use clap::{App, Arg};
use codegen::Scope;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::create_dir_all;
use std::fs::{self, File};
use std::io::Read;
use std::io::Write as OtherWrite;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

#[derive(Clone, Debug, Default)]
struct Module {
    children: HashMap<String, Module>,
    priv_mod: Vec<String>,
    pub_mod: Vec<String>,
}

impl Module {
    fn sorted_children(&self) -> Vec<(&str, &Module)> {
        let mut child_mods: Vec<(&str, &Module)> = self
            .children
            .iter()
            .map(|(name, module)| (name.as_str(), module))
            .collect();
        child_mods.sort_unstable_by(|a, b| a.0.cmp(b.0));
        child_mods
    }
    fn sorted_priv_modules(&self) -> Vec<&str> {
        let mut mods: Vec<&str> = self.priv_mod.iter().map(|s| s.as_str()).collect();
        mods.sort_unstable();
        mods
    }
    fn sorted_pub_modules(&self) -> Vec<&str> {
        let mut mods: Vec<&str> = self.pub_mod.iter().map(|s| s.as_str()).collect();
        mods.sort_unstable();
        mods
    }
}

// Modules with name matching Rust reserved keywords needs escaping.
// Most of them can use the raw identifier (r#) to work around the overlap while some will
// be postfixed with '_'.
fn escape_reserved_keywords(ident: &str) -> String {
    let mut ident = ident.to_owned();
    match ident.as_str() {
        "as" | "break" | "const" | "continue" | "else" | "enum" | "false" | "fn" | "for" | "if"
        | "impl" | "in" | "let" | "loop" | "match" | "mod" | "move" | "mut" | "pub" | "ref"
        | "return" | "static" | "struct" | "trait" | "true" | "type" | "unsafe" | "use"
        | "where" | "while" | "dyn" | "abstract" | "become" | "box" | "do" | "final" | "macro"
        | "override" | "priv" | "typeof" | "unsized" | "virtual" | "yield" | "async" | "await"
        | "try" => ident.insert_str(0, "r#"),
        "self" | "super" | "extern" | "crate" => ident += "_",
        _ => (),
    }
    ident
}

fn file_name_to_mod_path(mod_name: &str, modules: &mut Module, path: &[&str]) {
    if !path.is_empty() {
        let mut child = modules
            .children
            .entry(path[0].to_owned())
            .or_insert_with(Module::default);
        file_name_to_mod_path(mod_name, &mut child, &path[1..]);
        if path.len() == 1 {
            modules
                .priv_mod
                .push(escape_reserved_keywords(mod_name).trim().to_owned());
        }
    } else {
        modules
            .pub_mod
            .push(escape_reserved_keywords(mod_name).trim().to_owned());
    }
}

fn write_lib_rs(scope: &mut Scope, module: &Module) {
    // Declare internal modules.
    for mod_name in module.sorted_priv_modules() {
        scope.new_module(&mod_name);
    }
    // Traverse child modules.
    for child in module.sorted_children() {
        let module = scope
            .new_module(&escape_reserved_keywords(child.0))
            .vis("pub");
        write_lib_rs(module.scope(), child.1);
    }
    // Use public modules.
    for mod_name in module.sorted_pub_modules() {
        scope
            .import(&format!("super::{}", mod_name), "*")
            .vis("pub");
    }
}

fn generate_lib(src_path: &Path, disable_rustfmt: bool) {
    // Build a dictionary of modules based on file name (each dot separated
    // part of the file name is a submodule).
    let mut root_tree = Module::default();
    let lib_rs_path = src_path.join("lib.rs");
    let mut file_paths: Vec<PathBuf> = fs::read_dir(src_path)
        .expect("failed to read src dir")
        .map(|res| res.map(|e| e.path()))
        .filter_map(|f| match f {
            // don't include lib.rs
            Ok(path) if path != lib_rs_path => Some(path),
            _ => None,
        })
        .collect();
    file_paths.sort();
    for path in file_paths {
        // Rename and move each file into directory tree and build a module tree structure.
        // A file is named 'src/simian_public.simulator.v2.rs' will be moved to
        // 'src/simian_public/simulator/simian_public_simulator_v2_internal.rs'
        // and its content will be made public at module path 'simian_public::simulator::v2'.
        let file_stem = path
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .replace("r#", "");
        let mod_path = file_stem.split('.').collect::<Vec<&str>>();
        let internal_mod_name = file_stem.replace(".", "_") + "_internal";
        {
            let new_file_name = internal_mod_name.clone() + ".rs";
            let mut new_dir = path.parent().unwrap().to_path_buf();
            for mod_name in &mod_path[0..mod_path.len() - 1] {
                new_dir = new_dir.join(mod_name);
            }
            std::fs::create_dir_all(&new_dir).expect("error creating mod directory");
            std::fs::rename(&path, new_dir.join(&new_file_name)).expect("error moving file");
        }
        file_name_to_mod_path(&internal_mod_name, &mut root_tree, &mod_path);
    }
    let mut scope = Scope::new();
    scope.raw("#![allow(clippy::wrong_self_convention)]");
    scope.raw("#![allow(clippy::large_enum_variant)]");
    scope.raw("#![allow(clippy::unreadable_literal)]");
    write_lib_rs(&mut scope, &root_tree);
    File::create(&lib_rs_path)
        .expect("error creating lib.rs")
        .write_all(scope.to_string().as_bytes())
        .expect("error writing lib.rs");
    if !disable_rustfmt {
        // Format with rustfmt if is available otherwise skip it
        if let Err(err) = Command::new("rustfmt")
            .args(&["--edition", "2018", lib_rs_path.to_str().unwrap()])
            .spawn()
        {
            println!("Failed to format lib.rs: {:?}", err);
        }
    }
}

fn generate_cargo_toml(
    template_path: Option<PathBuf>,
    output_path: &Path,
    pkg_name: &str,
    pkg_authors: Vec<String>,
    pkg_version: &str,
) {
    let content = if let Some(template_path) = template_path {
        // Read template file
        let mut content = String::new();
        let mut template_file = File::open(template_path).expect("error opening template file");
        template_file.read_to_string(&mut content).unwrap();
        content
    } else {
        // Use default template if no file was provided
        include_str!("Cargo.toml.tmpl").to_string()
    };
    let content = content
        .replace("_PKG_NAME_", &format!("\"{}\"", pkg_name))
        .replace(
            "_PKG_AUTHORS_",
            &pkg_authors
                .iter()
                .map(|v| format!("\"{}\"", v))
                .collect::<Vec<String>>()
                .join(","),
        )
        .replace("_PKG_VERSION_", &format!("\"{}\"", pkg_version));

    let mut output_file = File::create(output_path).expect("error creating Cargo.toml");
    output_file
        .write_all(content.as_bytes())
        .expect("error writing Cargo.toml");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let matches = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .about("Generate rust/tonic code from protobuf")
        .arg(
            Arg::with_name("root")
                .long("root")
                .value_name("FILE")
                .help("Root of protobuf tree")
                .required(true)
                .multiple(true),
        )
        .arg(
            Arg::with_name("output-directory")
                .long("output")
                .value_name("DIR")
                .help("Where crate should be generated")
                .required(true),
        )
        .arg(
            Arg::with_name("cargo-toml-template")
                .long("cargo-toml-template")
                .value_name("FILE")
                .help("Use this Cargo.toml template file"),
        )
        .arg(
            Arg::with_name("pkg-name")
                .long("pkg-name")
                .value_name("NAME")
                .help("Set package name")
                .required(true),
        )
        .arg(
            Arg::with_name("pkg-author")
                .long("pkg-author")
                .value_name("AUTHOR")
                .help("Set package author(s)")
                .multiple(true),
        )
        .arg(
            Arg::with_name("pkg-version")
                .long("pkg-version")
                .value_name("VERSION")
                .help("Set package version"),
        )
        .arg(
            Arg::with_name("disable-rustfmt")
                .long("disable-rustfmt")
                .help(
                    "Disable rustfmt to be run on generated code. Will otherwise run if present.",
                ),
        )
        .get_matches();

    // Parse cli arguments
    let proto_root_paths: Vec<String> = matches
        .values_of("root")
        .unwrap()
        .map(|v| v.to_owned())
        .collect();
    let crate_dir = Path::new(matches.value_of("output-directory").unwrap());
    let src_dir = crate_dir.join("src");
    let cargo_toml_template_path = matches
        .value_of("cargo-toml-template")
        .map(|v| Path::new(v).to_path_buf());
    let pkg_name = matches.value_of("pkg-name").unwrap();
    let pkg_version = matches.value_of("pkg-version").unwrap_or("0.1.0");
    let pkg_authors: Vec<String> = if let Some(authors) = matches.values_of("pkg-author") {
        authors.map(|v| v.to_owned()).collect()
    } else {
        vec![]
    };
    let disable_rustfmt = matches.is_present("disable-rustfmt");
    let _e = std::fs::remove_dir_all(&src_dir);
    create_dir_all(&src_dir).expect("error creating src dir");
    {
        // Find all .proto files in any of the root paths.
        let mut proto_paths: Vec<String> = proto_root_paths
            .iter()
            .map(|path| {
                WalkDir::new(path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension() == Some(OsStr::new("proto")))
                    .map(|e| e.path().to_str().expect("error walking tree").to_owned())
            })
            .flatten()
            .collect();
        proto_paths.sort();
        tonic_build::configure()
            .out_dir(&src_dir)
            .format(!disable_rustfmt)
            .compile(&proto_paths[..], &proto_root_paths[..])?;
    }
    // Generate a lib.rs file containing all the module definitions and include statements.
    generate_lib(Path::new(&src_dir), disable_rustfmt);

    // Copy the Cargo template and set version
    generate_cargo_toml(
        cargo_toml_template_path,
        &crate_dir.join("Cargo.toml"),
        pkg_name,
        pkg_authors,
        pkg_version,
    );
    Ok(())
}
