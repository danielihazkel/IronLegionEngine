//! Single-root loader: JSON5 files from one mod into registries (TDD §3.3
//! steps 3 and 4, reduced). Discovery, load order and manifests are in their
//! own modules (T1-020); schema validation, overrides and the multi-mod
//! pipeline replace this file's body in T1-021..T1-023.

use std::path::{Path, PathBuf};

use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::json5::{FileId, SpannedValue, ValueKind, parse_json5};
use crate::manifest::read_manifest;
use crate::registry::{ContentKind, Registry};
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
/// array of objects (Modding SDK §2.1). Positions are kept.
pub fn parse_content_file(path: &Path, file: FileId) -> Result<Vec<SpannedValue>, Diagnostic> {
    let text = std::fs::read_to_string(path)
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

/// Loads every file of kind `T` under `content_dir/T::DIR` into a registry,
/// collecting all diagnostics. A missing kind folder is an empty registry.
pub fn load_kind<T: ContentKind>(content_dir: &Path) -> Result<Registry<T>, Diagnostics> {
    let dir = content_dir.join(T::DIR);
    let mut registry = Registry::new();
    let mut diags = Diagnostics::new();
    if !dir.is_dir() {
        return Ok(registry);
    }
    let mut files = Vec::new();
    if let Err(e) = json5_files(&dir, &mut files) {
        diags.push(Diagnostic::file_level(&dir, format!("cannot list: {e}")));
        return Err(diags);
    }
    for (file_index, path) in files.iter().enumerate() {
        let values = match parse_content_file(path, FileId(file_index as u32)) {
            Ok(v) => v,
            Err(d) => {
                diags.push(d);
                continue;
            }
        };
        for (i, value) in values.into_iter().enumerate() {
            let item: T = match serde_json::from_value(value.to_json()) {
                Ok(item) => item,
                Err(e) => {
                    // Typed errors name the field; schema validation adds
                    // per-field positions (T1-021).
                    diags.push(
                        Diagnostic::file_level(path, e.to_string())
                            .at(value.span.line, value.span.col)
                            .field(format!("[{i}]")),
                    );
                    continue;
                }
            };
            if let Err(dup) = registry.insert(item) {
                let span = value.key_span("id").unwrap_or(value.span);
                diags.push(
                    Diagnostic::file_level(path, dup.to_string())
                        .at(span.line, span.col)
                        .field(format!("[{i}].id")),
                );
            }
        }
    }
    diags.into_result(registry)
}

/// Every registry (TDD §3.2 `Registries`, Phase 0 subset).
#[derive(Debug, Default)]
pub struct Registries {
    pub units: Registry<UnitType>,
}

impl Registries {
    /// Loads one mod root (the flagship game at `game/`).
    pub fn load_root(mod_root: &Path) -> Result<Registries, Diagnostics> {
        let manifest = read_manifest(mod_root, true)?;
        let content_dir = mod_root.join(&manifest.manifest.content_root);
        let units = load_kind::<UnitType>(&content_dir)?;
        Ok(Registries { units })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_id::ContentId;

    /// A fresh scratch mod folder under the target directory.
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

    const GOOD: &str = r#"{ id: "t:good", name_key: "t.good", category: "infantry",
        hp: 100, speed_walk: 1.6, speed_run: 4.0, attack: 1 }"#;

    #[test]
    fn loads_objects_and_arrays_in_path_order() {
        let root = scratch("ok");
        std::fs::write(root.join("content/units/b.json5"), GOOD).unwrap();
        std::fs::write(
            root.join("content/units/a.json5"),
            r#"[
              { id: "t:a1", name_key: "t.a", category: "cavalry", hp: 1, speed_walk: 1, speed_run: 2 },
              { id: "t:a2", name_key: "t.a", category: "ranged", hp: 1, speed_walk: 1, speed_run: 2 },
            ]"#,
        )
        .unwrap();
        let regs = Registries::load_root(&root).unwrap();
        let ids: Vec<&str> = regs.units.ids().map(|i| i.as_str()).collect();
        assert_eq!(ids, vec!["t:a1", "t:a2", "t:good"]);
        let h = regs
            .units
            .lookup(&ContentId::new("t:good").unwrap())
            .unwrap();
        assert_eq!(
            regs.units.get(h).speed_walk,
            <il_core::S as il_core::Scalar>::from_f32_data(1.6)
        );
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
        assert!(d.file.ends_with("bad.json5"));
        assert_eq!(d.line, 4, "{d}");
        assert_eq!(d.col, 11, "{d}");
    }

    #[test]
    fn collects_every_diagnostic_before_failing() {
        let root = scratch("many");
        std::fs::write(root.join("content/units/a.json5"), "{ id: 5 }").unwrap();
        std::fs::write(root.join("content/units/b.json5"), GOOD).unwrap();
        std::fs::write(root.join("content/units/c.json5"), GOOD).unwrap();
        std::fs::write(root.join("content/units/d.json5"), "not json5 at all").unwrap();
        let err = Registries::load_root(&root).unwrap_err();
        let files: Vec<String> = err
            .0
            .iter()
            .map(|d| d.file.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files, vec!["a.json5", "c.json5", "d.json5"], "{err}");
        assert!(err.0[1].message.contains("duplicate"), "{err}");
        assert_eq!(
            (err.0[1].line, err.0[1].col),
            (1, 3),
            "duplicate points at the id key"
        );
    }

    #[test]
    fn missing_manifest_is_a_diagnostic() {
        let root = scratch("nomanifest");
        std::fs::remove_file(root.join("mod.json5")).unwrap();
        let err = Registries::load_root(&root).unwrap_err();
        assert!(err.0[0].message.contains("cannot read"));
    }
}
