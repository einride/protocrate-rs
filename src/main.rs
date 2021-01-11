mod module;

use clap::{App, Arg};
use codegen::Scope;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

use module::Module;

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
    let _ignore_err = std::fs::remove_dir_all(&src_dir);
    fs::create_dir_all(&src_dir).expect("error creating src dir");
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
        // And generate protobuf/gRPC code.
        tonic_build::configure()
            .out_dir(&src_dir)
            .format(!disable_rustfmt)
            .compile(&proto_paths[..], &proto_root_paths[..])?;
    }
    // Generate a lib.rs file containing all the module definitions and use statements.
    let lib_rs_path = src_dir.join("lib.rs");
    {
        let mut scope = Scope::new();
        scope.raw("#![allow(clippy::wrong_self_convention)]");
        scope.raw("#![allow(clippy::large_enum_variant)]");
        scope.raw("#![allow(clippy::unreadable_literal)]");
        mod_to_scope(&mut scope, &Module::build(Path::new(&src_dir)));
        File::create(&lib_rs_path)
            .expect("open lib.rs")
            .write_all(scope.to_string().as_bytes())
            .expect("error writing lib.rs");
    }
    if !disable_rustfmt {
        // Format with rustfmt if it is available otherwise skip it.
        if let Err(err) = Command::new("rustfmt")
            .args(&["--edition", "2018", lib_rs_path.to_str().unwrap()])
            .spawn()
        {
            println!("Failed to format lib.rs: {:?}", err);
        }
    }

    // Copy the Cargo template and set version
    write_cargo_toml(
        cargo_toml_template_path,
        &crate_dir.join("Cargo.toml"),
        pkg_name,
        pkg_authors,
        pkg_version,
    );
    Ok(())
}

fn mod_to_scope(scope: &mut Scope, module: &Module) {
    // Declare internal modules.
    for mod_name in module.sorted_priv_modules() {
        scope.new_module(&mod_name);
    }
    // Traverse child modules.
    for (child_name, child_mod) in module.sorted_children() {
        let module = scope.new_module(child_name).vis("pub");
        mod_to_scope(module.scope(), child_mod);
    }
    // Use public modules.
    for mod_name in module.sorted_pub_modules() {
        scope
            .import(&format!("super::{}", mod_name), "*")
            .vis("pub");
    }
}

fn write_cargo_toml(
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
    File::create(output_path)
        .expect("error creating Cargo.toml")
        .write_all(content.as_bytes())
        .expect("error writing Cargo.toml");
}
