//! File helpers and the `Registries` entry points (TDD §3.3). The pipeline
//! itself lives in [`crate::pipeline`].

use std::path::{Path, PathBuf};

use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::json5::{FileId, SpannedValue, ValueKind, parse_json5};
use crate::registry::Registry;
use crate::unit_type::UnitType;

/// Every `*.json5` under `dir`, recursively, sorted by path so load order
/// does not depend on the filesystem.
pub fn json5_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            json5_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "json5") {
            out.push(path);
        }
    }
    out.sort();
    Ok(())
}

/// Parses one content file into its objects: a file holds one object or an
/// array of objects (Modding SDK §2.1). Positions are kept. `display` is the
/// path used in diagnostics (`<mod id>/<mod-relative path>`).
pub fn parse_content_file(
    abs: &Path,
    display: &Path,
    file: FileId,
) -> Result<Vec<SpannedValue>, Diagnostic> {
    let path = display;
    let text = std::fs::read_to_string(abs)
        .map_err(|e| Diagnostic::file_level(path, format!("cannot read: {e}")))?;
    let value = parse_json5(&text, file)
        .map_err(|e| Diagnostic::file_level(path, e.message).at(e.span.line, e.span.col))?;
    match value.kind {
        ValueKind::Array(items) => {
            if let Some(bad) = items.iter().find(|v| v.as_object().is_none()) {
                return Err(Diagnostic::file_level(
                    path,
                    format!("expected an object in the array, found {}", bad.type_name()),
                )
                .at(bad.span.line, bad.span.col));
            }
            Ok(items)
        }
        ValueKind::Object(_) => Ok(vec![value]),
        _ => Err(Diagnostic::file_level(
            path,
            format!(
                "expected an object or an array of objects, found {}",
                value.type_name()
            ),
        )
        .at(value.span.line, value.span.col)),
    }
}

/// Every registry (TDD §3.2 `Registries`, Phase 1 subset so far).
#[derive(Debug, Default)]
pub struct Registries {
    pub units: Registry<UnitType>,
}

impl Registries {
    /// Loads a single mod root (the flagship game at `game/`); the folder
    /// must hold a `mod.json5`.
    pub fn load_root(mod_root: &Path) -> Result<Registries, Diagnostics> {
        crate::manifest::read_manifest(mod_root, true)?;
        crate::pipeline::load_roots(&[mod_root.to_path_buf()])
    }

    /// Discovers, orders and loads the mods under `roots`; the first root is
    /// the game (Modding SDK §3.1).
    pub fn load_roots(roots: &[PathBuf]) -> Result<Registries, Diagnostics> {
        crate::pipeline::load_roots(roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/il_data_test")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("content/units")).unwrap();
        std::fs::write(
            root.join("mod.json5"),
            r#"{ id: "t", name_key: "t.mod", version: "0.1.0", engine_version: "*" }"#,
        )
        .unwrap();
        root
    }

    #[test]
    fn malformed_file_reports_line_and_column() {
        let root = scratch("malformed");
        std::fs::write(
            root.join("content/units/bad.json5"),
            "{\n  id: \"t:bad\",\n  name_key: \"t.bad\",\n  hp: 100 speed_walk: 1\n}\n",
        )
        .unwrap();
        let err = Registries::load_root(&root).unwrap_err();
        assert_eq!(err.len(), 1, "{err}");
        let d = &err.0[0];
        assert_eq!(
            d.file.to_string_lossy().replace('\\', "/"),
            "t/content/units/bad.json5"
        );
        assert_eq!((d.line, d.col), (4, 11), "{d}");
    }

    #[test]
    fn missing_manifest_is_a_diagnostic() {
        let root = scratch("nomanifest");
        std::fs::remove_file(root.join("mod.json5")).unwrap();
        let err = Registries::load_root(&root).unwrap_err();
        assert!(err.0[0].message.contains("cannot read"), "{err}");
    }

    #[test]
    fn game_content_loads() {
        let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        let regs = Registries::load_root(&game).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            regs.units
                .contains(&crate::ContentId::new("rome:hastati").unwrap())
        );
    }
}
