use clap::{App, Arg};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::Write;
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
    internal_mod: Vec<String>,
    pub_mod: Vec<String>,
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
                .internal_mod
                .push(escape_reserved_keywords(mod_name).trim().to_owned());
        }
    } else {
        modules
            .pub_mod
            .push(escape_reserved_keywords(mod_name).trim().to_owned());
    }
}

// Write whitespace according to the current tree depth
fn depth_to_ws(tree_depth: usize) -> String {
    let mut tmp = String::new();
    for _ in 0..tree_depth {
        write!(&mut tmp, "    ").unwrap();
    }
    tmp
}

fn write_lib_rs(content: &mut String, modules: &Module, tree_depth: usize) {
    let mut int_mods_sorted = modules.internal_mod.clone();
    int_mods_sorted.sort();
    for mod_name in int_mods_sorted {
        writeln!(content, "{}mod {};", depth_to_ws(tree_depth), mod_name).unwrap();
    }
    let mut children_sorted: Vec<(&str, &Module)> = modules
        .children
        .iter()
        .map(|(name, module)| (name.as_str(), module))
        .collect();
    children_sorted.sort_by(|a, b| a.0.cmp(b.0));
    for child in children_sorted {
        writeln!(
            content,
            "{}pub mod {} {{",
            depth_to_ws(tree_depth),
            escape_reserved_keywords(child.0)
        )
        .unwrap();
        write_lib_rs(content, child.1, tree_depth + 1);
        writeln!(content, "{}}}", depth_to_ws(tree_depth),).unwrap();
    }
    let mut pub_mods_sorted = modules.pub_mod.clone();
    pub_mods_sorted.sort();
    for mod_name in pub_mods_sorted {
        writeln!(
            content,
            "{}pub use super::{}::*;",
            depth_to_ws(tree_depth),
            mod_name
        )
        .unwrap();
    }
}

fn generate_lib(src_path: &Path) {
    // Build a dictionary of modules based on file name (each dot separated
    // part of the file name is a submodule').
    let mut root_tree = Module::default();
    let lib_rs_path = src_path.join("lib.rs");
    fs::read_dir(src_path)
        .expect("failed to read src dir")
        .map(|res| res.map(|e| e.path()))
        .filter(|f| match f {
            // don't include lib.rs
            Ok(path) => path != &lib_rs_path,
            _ => false,
        })
        .for_each(|f| {
            if let Ok(path) = f {
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
                    std::fs::rename(&path, new_dir.join(&new_file_name))
                        .expect("error moving file");
                }
                file_name_to_mod_path(&internal_mod_name, &mut root_tree, &mod_path);
            }
        });
    let mut content = String::new();
    // Allow some clippy warnings in the generated protobuf code
    writeln!(content, "#![allow(clippy::wrong_self_convention)]").unwrap();
    writeln!(content, "#![allow(clippy::large_enum_variant)]").unwrap();
    writeln!(content, "#![allow(clippy::unreadable_literal)]").unwrap();
    writeln!(content).unwrap();
    write_lib_rs(&mut content, &root_tree, 0);
    let mut file = File::create(&lib_rs_path).expect("error creating lib.rs");
    file.write_all(content.as_bytes())
        .expect("error writing lib.rs");
    // Format if rustfmt is available otherwise skip it
    if let Err(err) = Command::new("rustfmt")
        .args(&["--edition", "2018", lib_rs_path.to_str().unwrap()])
        .spawn()
    {
        println!("Failed to format lib.rs: {:?}", err);
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

    let _e = std::fs::remove_dir_all(&src_dir);
    create_dir_all(&src_dir).expect("error creating src dir");
    {
        // Find all .proto files in any of the root paths.
        let proto_paths: Vec<String> = proto_root_paths
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
        tonic_build::configure()
            .out_dir(&src_dir)
            .compile(&proto_paths[..], &proto_root_paths[..])?;
    }
    // Generate a lib.rs file containing all the module definitions and include statements.
    generate_lib(Path::new(&src_dir));

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
