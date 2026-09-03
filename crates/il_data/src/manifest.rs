//! `mod.json5` (T1-020, Modding SDK §2.2, `mod-manifest.schema.json`).

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::json5::{FileId, parse_json5};

/// A dependency on another mod at a version range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub id: String,
    pub version: semver::VersionReq,
}

/// A parsed, validated manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub name_key: String,
    pub version: semver::Version,
    /// Engine versions this mod targets; loading outside the range warns.
    pub engine_version: semver::VersionReq,
    pub dependencies: Vec<Dependency>,
    pub load_after: Vec<String>,
    pub load_before: Vec<String>,
    pub content_root: String,
    pub scripts_root: String,
    pub assets_root: String,
    pub locales: Vec<String>,
    /// Extra ContentId namespaces; honoured only for the flagship game.
    pub namespaces: Vec<String>,
}

/// A manifest with the folder it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestWithPath {
    pub manifest: Manifest,
    pub root: PathBuf,
    /// The flagship game at `game/`: always enabled, loads first, may declare
    /// namespaces.
    pub is_game: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependency {
    id: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    id: String,
    name_key: String,
    version: String,
    engine_version: String,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    load_after: Vec<String>,
    #[serde(default)]
    load_before: Vec<String>,
    #[serde(default = "default_content_root")]
    content_root: String,
    #[serde(default = "default_scripts_root")]
    scripts_root: String,
    #[serde(default = "default_assets_root")]
    assets_root: String,
    #[serde(default)]
    locales: Vec<String>,
    #[serde(default)]
    namespaces: Vec<String>,
}

fn default_content_root() -> String {
    "content".to_string()
}
fn default_scripts_root() -> String {
    "scripts".to_string()
}
fn default_assets_root() -> String {
    "assets".to_string()
}

pub(crate) fn is_mod_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn is_relative_root(s: &str) -> bool {
    !s.starts_with("..") && !s.starts_with('/') && !s.contains('\\')
}

/// Parses a semver range as the SDK writes it: comparators separated by
/// spaces (`>=0.4.0 <0.6.0`), `*`, or a single comparator. An operator and
/// its version must not be separated by a space.
pub fn parse_version_req(s: &str) -> Result<semver::VersionReq, semver::Error> {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(", ");
    semver::VersionReq::parse(if joined.is_empty() { "*" } else { &joined })
}

/// The engine version the running binary reports.
pub fn engine_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).expect("crate version is semver")
}

/// Reads and validates `root/mod.json5`. Every problem becomes a diagnostic
/// against that file; positional detail arrives with schema validation
/// (T1-021).
pub fn read_manifest(root: &Path, is_game: bool) -> Result<ManifestWithPath, Diagnostics> {
    let path = root.join("mod.json5");
    let mut diags = Diagnostics::new();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            diags.push(Diagnostic::file_level(&path, format!("cannot read: {e}")));
            return Err(diags);
        }
    };
    let value = match parse_json5(&text, FileId(0)) {
        Ok(v) => v,
        Err(e) => {
            diags.push(Diagnostic::file_level(&path, e.message).at(e.span.line, e.span.col));
            return Err(diags);
        }
    };
    let raw: RawManifest = match serde_json::from_value(value.to_json()) {
        Ok(r) => r,
        Err(e) => {
            diags.push(Diagnostic::file_level(&path, e.to_string()));
            return Err(diags);
        }
    };
    let field_diag = |field: &str, message: String| {
        let (line, col) = value.key_span(field).map_or((1, 1), |s| (s.line, s.col));
        Diagnostic::file_level(&path, message)
            .at(line, col)
            .field(field)
    };

    if !is_mod_id(&raw.id) {
        diags.push(
            field_diag("id", format!("invalid mod id {:?}", raw.id))
                .expected("^[a-z0-9_]+$, at most 64 characters"),
        );
    }
    let version = match semver::Version::parse(&raw.version) {
        Ok(v) => v,
        Err(e) => {
            diags.push(
                field_diag("version", format!("invalid version {:?}: {e}", raw.version))
                    .expected("MAJOR.MINOR.PATCH"),
            );
            semver::Version::new(0, 0, 0)
        }
    };
    let engine_version_req = match parse_version_req(&raw.engine_version) {
        Ok(r) => r,
        Err(e) => {
            diags.push(
                field_diag(
                    "engine_version",
                    format!(
                        "cannot parse engine version range {:?}: {e}",
                        raw.engine_version
                    ),
                )
                .expected("a semver range such as \">=0.1.0 <0.2.0\""),
            );
            semver::VersionReq::STAR
        }
    };
    let mut dependencies = Vec::new();
    for (i, d) in raw.dependencies.iter().enumerate() {
        if !is_mod_id(&d.id) {
            diags.push(field_diag(
                "dependencies",
                format!("dependencies[{i}].id: invalid mod id {:?}", d.id),
            ));
        }
        match parse_version_req(&d.version) {
            Ok(version) => dependencies.push(Dependency {
                id: d.id.clone(),
                version,
            }),
            Err(e) => diags.push(field_diag(
                "dependencies",
                format!(
                    "dependencies[{i}].version: cannot parse range {:?}: {e}",
                    d.version
                ),
            )),
        }
    }
    for (field, list) in [
        ("load_after", &raw.load_after),
        ("load_before", &raw.load_before),
        ("namespaces", &raw.namespaces),
    ] {
        for (i, id) in list.iter().enumerate() {
            if !is_mod_id(id) {
                diags.push(field_diag(
                    field,
                    format!("{field}[{i}]: invalid mod id {id:?}"),
                ));
            }
        }
    }
    for (field, dir) in [
        ("content_root", &raw.content_root),
        ("scripts_root", &raw.scripts_root),
        ("assets_root", &raw.assets_root),
    ] {
        if !is_relative_root(dir) {
            diags.push(field_diag(
                field,
                format!("{dir:?} must be a relative path with forward slashes"),
            ));
        }
    }
    if !raw.namespaces.is_empty() && !is_game {
        // Not an error: the field is simply ignored outside game/ (SDK §3.5).
        diags.push(
            field_diag(
                "namespaces",
                "namespaces are honoured only for the flagship game at game/; ignored".to_string(),
            )
            .warning(),
        );
    }

    let manifest = Manifest {
        id: raw.id,
        name_key: raw.name_key,
        version,
        engine_version: engine_version_req,
        dependencies,
        load_after: raw.load_after,
        load_before: raw.load_before,
        content_root: raw.content_root,
        scripts_root: raw.scripts_root,
        assets_root: raw.assets_root,
        locales: raw.locales,
        namespaces: if is_game { raw.namespaces } else { Vec::new() },
    };
    if diags.has_errors() {
        return Err(diags);
    }
    Ok(ManifestWithPath {
        manifest,
        root: root.to_path_buf(),
        is_game,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str, manifest: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/il_data_test/manifest")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("mod.json5"), manifest).unwrap();
        root
    }

    #[test]
    fn full_manifest_parses_with_defaults() {
        let root = scratch(
            "full",
            r#"{ id: "mymod", name_key: "mymod.mod.name", version: "1.2.3",
               engine_version: ">=0.1.0 <0.2.0",
               dependencies: [{ id: "rome", version: ">=0.1.0" }],
               load_after: ["better_ai"], locales: ["en", "de"] }"#,
        );
        let m = read_manifest(&root, false).unwrap();
        assert_eq!(m.manifest.id, "mymod");
        assert_eq!(m.manifest.version, semver::Version::new(1, 2, 3));
        assert!(
            m.manifest
                .engine_version
                .matches(&semver::Version::new(0, 1, 5))
        );
        assert!(
            !m.manifest
                .engine_version
                .matches(&semver::Version::new(0, 2, 0))
        );
        assert_eq!(m.manifest.dependencies[0].id, "rome");
        assert_eq!(m.manifest.content_root, "content");
        assert_eq!(m.manifest.assets_root, "assets");
        assert!(!m.is_game);
    }

    #[test]
    fn namespaces_only_for_the_game() {
        let src = r#"{ id: "rome", name_key: "rome.mod.name", version: "0.1.0", engine_version: "*", namespaces: ["greece"] }"#;
        let game = read_manifest(&scratch("ns_game", src), true).unwrap();
        assert_eq!(game.manifest.namespaces, vec!["greece"]);
        let other = read_manifest(&scratch("ns_other", src), false).unwrap();
        assert!(other.manifest.namespaces.is_empty());
    }

    #[test]
    fn invalid_fields_are_positioned_diagnostics() {
        let root = scratch(
            "bad",
            "{\n  id: \"My Mod\",\n  name_key: \"x.y\",\n  version: \"1.2\",\n  engine_version: \">= 1\",\n}",
        );
        let err = read_manifest(&root, false).unwrap_err();
        let lines: Vec<(String, u32)> = err.0.iter().map(|d| (d.field.clone(), d.line)).collect();
        assert_eq!(
            lines,
            vec![
                ("id".to_string(), 2),
                ("version".to_string(), 4),
                ("engine_version".to_string(), 5)
            ],
            "{err}"
        );
    }

    #[test]
    fn unknown_fields_and_syntax_errors_are_reported() {
        let err = read_manifest(
            &scratch(
                "unknown",
                r#"{ id: "a", name_key: "a.b", version: "0.1.0", engine_version: "*", bogus: 1 }"#,
            ),
            false,
        )
        .unwrap_err();
        assert!(err.0[0].message.contains("bogus"), "{err}");
        let err = read_manifest(&scratch("syntax", "{ id: }"), false).unwrap_err();
        assert_eq!((err.0[0].line, err.0[0].col), (1, 7));
    }

    #[test]
    fn version_req_grammar() {
        assert!(
            parse_version_req(">=0.4.0 <0.6.0")
                .unwrap()
                .matches(&semver::Version::new(0, 5, 0))
        );
        assert!(
            parse_version_req("*")
                .unwrap()
                .matches(&semver::Version::new(9, 9, 9))
        );
        assert!(
            parse_version_req("=0.1.0")
                .unwrap()
                .matches(&semver::Version::new(0, 1, 0))
        );
        assert!(parse_version_req(">= 1.0.0").is_err());
    }
}
