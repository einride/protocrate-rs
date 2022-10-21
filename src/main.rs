mod module;

use anyhow::{Context, Result};
use codegen::Scope;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use structopt::StructOpt;
use walkdir::WalkDir;

use module::Module;

#[derive(StructOpt, Debug)]
#[structopt(name = env!("CARGO_PKG_NAME"))]
struct Opt {
    /// Where crate should be generated
    #[structopt(short, long, parse(from_os_str))]
    output_dir: PathBuf,
    /// Cargo.toml template file to use
    #[structopt(short, long, parse(from_os_str))]
    cargo_toml_template: Option<PathBuf>,
    /// Set crate name
    #[structopt(short, long)]
    pkg_name: String,
    /// Set crate version
    #[structopt(long, default_value = "0.1.0")]
    pkg_version: String,
    /// Set crate author(s)
    #[structopt(long)]
    pkg_author: Vec<String>,
    /// Disable rustfmt to be run on generated code (will otherwise be run if present)
    #[structopt(long)]
    disable_rustfmt: bool,
    /// Root directory of protobuf tree (can be multiple)
    #[structopt(name = "DIR", required = true, min_values = 1)]
    root: Vec<String>,
}

fn main() -> Result<()> {
    let opt = Opt::from_args();
    let src_dir = opt.output_dir.join("src");
    let resources_dir = opt.output_dir.join("resources");
    let _ignore_err = std::fs::remove_dir_all(&src_dir);
    fs::create_dir_all(&resources_dir)
        .context(format!("create dir ({})", resources_dir.display()))?;
    fs::create_dir_all(&src_dir).context(format!("create dir ({})", src_dir.display()))?;
    let descriptor_path = PathBuf::from(&resources_dir).join("file_descriptor_set.bin");

    {
        // Find all .proto files in any of the root paths.
        let mut proto_paths: Vec<String> = opt
            .root
            .iter()
            .flat_map(|path| {
                WalkDir::new(path)
                    .into_iter()
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension() == Some(OsStr::new("proto")))
                    .map(|e| e.path().to_str().unwrap().to_owned())
            })
            .collect();
        proto_paths.sort();

        let mut config = prost_build::Config::new();
        config.out_dir(&src_dir);
        prost_reflect_build::Builder::new()
            .file_descriptor_set_path(&descriptor_path)
            .configure(&mut config, &proto_paths[..], &opt.root[..])
            .context(format!(
                "generate reflective protobuf ({})",
                src_dir.display()
            ))?;
        tonic_build::configure()
            .out_dir(&src_dir) // needed because tonic looks in a different place for the out dir than prost
            .compile_with_config(config, &proto_paths[..], &opt.root[..])
            .context(format!("generate protobuf ({})", src_dir.display()))?;
    }
    // Generate a lib.rs file containing all the module definitions and use statements.
    let lib_rs_path = src_dir.join("lib.rs");
    {
        let mut scope = Scope::new();
        scope.raw("#![allow(clippy::wrong_self_convention)]");
        scope.raw("#![allow(clippy::large_enum_variant)]");
        scope.raw("#![allow(clippy::unreadable_literal)]");

        // Adding getter for descriptor pool
        scope.raw("use prost_reflect::DescriptorPool;");
        scope.raw("use once_cell::sync::Lazy;");
        let line = format!(
            "static DESCRIPTOR_POOL: Lazy<DescriptorPool>
        = Lazy::new(|| DescriptorPool::decode(include_bytes!(\"..{}\").as_ref()).unwrap());",
            strip_prefix(descriptor_path, &opt.output_dir).display()
        );
        scope.raw(line.as_str());

        Module::build(Path::new(&src_dir), &[&lib_rs_path])?.codegen(&mut scope);
        File::create(&lib_rs_path)
            .context("create lib.rs")?
            .write_all(scope.to_string().as_bytes())
            .context("write lib.rs")?;
    }
    if !opt.disable_rustfmt {
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
        opt.cargo_toml_template,
        &opt.output_dir.join("Cargo.toml"),
        &opt.pkg_name,
        opt.pkg_author,
        &opt.pkg_version,
    )
}

fn strip_prefix(path: PathBuf, prefix: &Path) -> PathBuf {
    if !path.starts_with(prefix) {
        return path;
    }

    PathBuf::from(path.to_str().unwrap()[prefix.to_str().unwrap().len()..].to_owned())
}

fn write_cargo_toml(
    template_path: Option<PathBuf>,
    output_path: &Path,
    pkg_name: &str,
    pkg_authors: Vec<String>,
    pkg_version: &str,
) -> Result<()> {
    let content = if let Some(template_path) = template_path {
        // Read template file
        let mut content = String::new();
        let mut template_file = File::open(template_path).context("open template")?;
        template_file
            .read_to_string(&mut content)
            .context("read template")?;
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
        .context("error creating Cargo.toml")?
        .write_all(content.as_bytes())
        .context("error writing Cargo.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip() {
        let path = PathBuf::from("path/to/jassob");
        let prefix = PathBuf::from("path/to/");

        assert_eq!("jassob", strip_prefix(path, &prefix).to_str().unwrap())
    }
}
