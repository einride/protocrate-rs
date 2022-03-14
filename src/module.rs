use anyhow::{Context, Result};
use codegen::Scope;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Module {
    child_mod: HashMap<String, Module>,
    priv_mod: Vec<String>,
    use_mod: Vec<String>,
}

impl Module {
    pub fn build(src_path: &Path, ignore_files: &[&Path]) -> Result<Self> {
        let mut root = Module::default();
        let mut file_paths: Vec<PathBuf> = fs::read_dir(src_path)
            .context("read src dir")?
            .map(|res| res.map(|e| e.path()))
            .filter_map(|f| match f {
                // Ignore some files.
                Ok(path) if !ignore_files.contains(&path.as_path()) => Some(path),
                _ => None,
            })
            .collect();
        file_paths.sort();
        for path in file_paths {
            // Rename and move each file into the directory tree and build a module structure.
            // A file named 'src/foo.bar.v2.rs' would be moved to 'src/foo/bar/foo_bar_v2_internal.rs'
            // and its content would be made public as module 'foo::bar::v2'.
            let file_stem = path
                .file_stem()
                .context("file stem")?
                .to_str()
                .context("file stem to string")?
                .replace("r#", "");
            let mod_path: Vec<&str> = file_stem.split('.').collect();
            let internal_mod_name = file_stem.replace('.', "_") + "_internal";
            {
                let new_file_name = internal_mod_name.clone() + ".rs";
                let mut new_dir = path.parent().context("path parent")?.to_path_buf();
                for mod_name in &mod_path[0..mod_path.len() - 1] {
                    new_dir = new_dir.join(mod_name);
                }
                std::fs::create_dir_all(&new_dir)
                    .context(format!("create directory ({})", new_dir.display()))?;
                std::fs::rename(&path, new_dir.join(&new_file_name))
                    .context(format!("move file ({})", path.display()))?;
            }
            root.path_to_mod(&internal_mod_name, &mod_path);
        }
        Ok(root)
    }
    fn path_to_mod(&mut self, mod_name: &str, path: &[&str]) {
        if !path.is_empty() {
            let child = self
                .child_mod
                .entry(escape_reserved_keywords(path[0]))
                .or_insert_with(Module::default);
            child.path_to_mod(mod_name, &path[1..]);
            if path.len() == 1 {
                self.priv_mod
                    .push(escape_reserved_keywords(mod_name).trim().to_owned());
            }
        } else {
            self.use_mod
                .push(escape_reserved_keywords(mod_name).trim().to_owned());
        }
    }
    fn sorted_children(&self) -> Vec<(&str, &Module)> {
        let mut child_mod: Vec<(&str, &Module)> = self
            .child_mod
            .iter()
            .map(|(name, module)| (name.as_str(), module))
            .collect();
        child_mod.sort_unstable_by(|a, b| a.0.cmp(b.0));
        child_mod
    }
    fn sorted_priv_modules(&self) -> Vec<&str> {
        let mut mods: Vec<&str> = self.priv_mod.iter().map(|s| s.as_str()).collect();
        mods.sort_unstable();
        mods
    }
    fn sorted_use(&self) -> Vec<&str> {
        let mut mods: Vec<&str> = self.use_mod.iter().map(|s| s.as_str()).collect();
        mods.sort_unstable();
        mods
    }
    pub fn codegen(&self, scope: &mut Scope) {
        // Declare internal modules.
        for mod_name in self.sorted_priv_modules() {
            scope.raw(&format!("mod {};", &mod_name));
        }
        // Traverse child modules.
        for (child_name, child_mod) in self.sorted_children() {
            let module = scope.new_module(child_name).vis("pub");
            child_mod.codegen(module.scope());
        }
        // Use public modules.
        for mod_name in self.sorted_use() {
            scope
                .import(&format!("super::{}", mod_name), "*")
                .vis("pub");
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    fn create_files(paths: &[&Path]) {
        for path in paths {
            let dir = path.parent().unwrap();
            fs::create_dir_all(dir).unwrap();
            std::fs::File::create(path).unwrap();
        }
    }
    // Remove unnecessary whitespace (\n and mulitple spaces)
    fn strip(text: &str) -> String {
        let mut tmp = text.replace("\n", "").trim().to_string();
        loop {
            let stripped = tmp.replace("  ", "");
            if stripped != tmp {
                tmp = stripped;
            } else {
                return tmp;
            }
        }
    }

    #[test]
    fn single_file_no_child() {
        // Given
        let root = TempDir::new("root").unwrap();
        create_files(&[&root.path().join("foo.rs")]);

        // When
        let module = Module::build(&root.path(), &[]).unwrap();

        // Then
        let mut scope = Scope::new();
        module.codegen(&mut scope);
        assert_eq!(
            strip(&scope.to_string()),
            strip(
                r#"
                mod foo_internal;
                pub mod foo {
                    pub use super::foo_internal::*;
                }
                "#
            )
        );
    }
    #[test]
    fn single_file_with_child() {
        // Given
        let root = TempDir::new("root").unwrap();
        create_files(&[&root.path().join("foo.v1.rs")]);

        // When
        let module = Module::build(&root.path(), &[]).unwrap();

        // Then
        let mut scope = Scope::new();
        module.codegen(&mut scope);
        assert_eq!(
            strip(&scope.to_string()),
            strip(
                r#"
                pub mod foo {
                    mod foo_v1_internal;
                    pub mod v1 {
                        pub use super::foo_v1_internal::*;
                    }
                }
                "#
            )
        );
    }
    #[test]
    fn multiple_files() {
        // Given
        let root = TempDir::new("root").unwrap();
        create_files(&[
            &root.path().join("first.rs"),
            &root.path().join("foo.v1.one.rs"),
            &root.path().join("foo.v1.two.rs"),
        ]);

        // When
        let module = Module::build(&root.path(), &[]).unwrap();

        // Then
        let mut scope = Scope::new();
        module.codegen(&mut scope);
        assert_eq!(
            strip(&scope.to_string()),
            strip(
                r#"
                mod first_internal;
                pub mod first {
                    pub use super::first_internal::*;
                }
                pub mod foo {
                    pub mod v1 {
                        mod foo_v1_one_internal;
                        mod foo_v1_two_internal;
                        pub mod one {
                            pub use super::foo_v1_one_internal::*;
                        }
                        pub mod two {
                            pub use super::foo_v1_two_internal::*;
                        }
                    }
                }"#
            )
        );
    }
}
