//! The table behind `FileId`: which mod and file a span belongs to, so a
//! diagnostic can say `mymod/content/units/x.json5:14:5` and a merged field
//! can be traced to the mod that last wrote it (Modding SDK §3.6).

use std::path::{Path, PathBuf};

use crate::json5::FileId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceFile {
    pub mod_index: usize,
    pub mod_id: String,
    /// Mod-relative path with forward slashes, e.g. `content/units/x.json5`.
    pub rel: String,
    pub abs: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct Sources {
    files: Vec<SourceFile>,
}

impl Sources {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a file and returns its id. `rel` is relative to the mod root.
    pub fn add(&mut self, mod_index: usize, mod_id: &str, rel: &Path, abs: &Path) -> FileId {
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        self.files.push(SourceFile {
            mod_index,
            mod_id: mod_id.to_string(),
            rel,
            abs: abs.to_path_buf(),
        });
        FileId(self.files.len() as u32 - 1)
    }

    pub fn get(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }

    /// `<mod id>/<mod-relative path>`, the file form used in diagnostics.
    pub fn display(&self, id: FileId) -> PathBuf {
        let f = self.get(id);
        PathBuf::from(format!("{}/{}", f.mod_id, f.rel))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_uses_mod_id_and_forward_slashes() {
        let mut s = Sources::new();
        let id = s.add(
            1,
            "mymod",
            Path::new("content").join("units").join("x.json5").as_path(),
            Path::new("C:/mods/mymod/content/units/x.json5"),
        );
        assert_eq!(s.display(id), PathBuf::from("mymod/content/units/x.json5"));
        assert_eq!(s.get(id).mod_index, 1);
        assert_eq!(s.len(), 1);
    }
}
