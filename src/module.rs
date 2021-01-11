use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct Module {
    child_mod: HashMap<String, Module>,
    priv_mod: Vec<String>,
    pub_mod: Vec<String>,
}

impl Module {
    pub fn build(src_path: &Path) -> Self {
        let mut root = Module::default();
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
            // Rename and move each file into the directory tree and build a module structure.
            // A file named 'src/foo.bar.v2.rs' would be moved to 'src/foo/bar/foo_bar_v2_internal.rs'
            // and its content would be made public as module 'foo::bar::v2'.
            let file_stem = path
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .replace("r#", "");
            let mod_path: Vec<&str> = file_stem.split('.').collect();
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
            root.path_to_mod(&internal_mod_name, &mod_path);
        }
        root
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
            self.pub_mod
                .push(escape_reserved_keywords(mod_name).trim().to_owned());
        }
    }
    pub fn sorted_children(&self) -> Vec<(&str, &Module)> {
        let mut child_mod: Vec<(&str, &Module)> = self
            .child_mod
            .iter()
            .map(|(name, module)| (name.as_str(), module))
            .collect();
        child_mod.sort_unstable_by(|a, b| a.0.cmp(b.0));
        child_mod
    }
    pub fn sorted_priv_modules(&self) -> Vec<&str> {
        let mut mods: Vec<&str> = self.priv_mod.iter().map(|s| s.as_str()).collect();
        mods.sort_unstable();
        mods
    }
    pub fn sorted_pub_modules(&self) -> Vec<&str> {
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
