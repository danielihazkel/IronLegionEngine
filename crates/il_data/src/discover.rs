//! Mod discovery (T1-020, Modding SDK §3.1).

use std::path::{Path, PathBuf};

use crate::diagnostic::Diagnostics;
use crate::manifest::{ManifestWithPath, read_manifest};

/// Scans `roots` for mods. The first root is the flagship game: if it holds
/// `mod.json5` it is the game mod itself; otherwise each immediate child
/// folder with a `mod.json5` is a mod. Later roots (`<install>/mods/`,
/// `<user data>/IronLegion/mods/`) are scanned the same way with
/// `is_game = false`. Children are visited in sorted name order so the result
/// never depends on the filesystem. Zipped mods arrive with SDK §10.
pub fn discover(roots: &[PathBuf]) -> Result<Vec<ManifestWithPath>, Diagnostics> {
    let mut found = Vec::new();
    let mut diags = Diagnostics::new();
    for (i, root) in roots.iter().enumerate() {
        let is_game_root = i == 0;
        if root.join("mod.json5").is_file() {
            collect(root, is_game_root, &mut found, &mut diags);
            continue;
        }
        let Ok(entries) = std::fs::read_dir(root) else {
            continue; // a missing mods folder is not an error
        };
        let mut children: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join("mod.json5").is_file())
            .collect();
        children.sort();
        for (j, child) in children.iter().enumerate() {
            // Only the first mod under the game root can be the game.
            collect(child, is_game_root && j == 0, &mut found, &mut diags);
        }
    }
    diags.into_result(found)
}

fn collect(root: &Path, is_game: bool, found: &mut Vec<ManifestWithPath>, diags: &mut Diagnostics) {
    match read_manifest(root, is_game) {
        Ok(m) => found.push(m),
        Err(d) => diags.extend(d),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/il_data_test/discover")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_mod(dir: &Path, id: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("mod.json5"),
            format!(r#"{{ id: "{id}", name_key: "{id}.mod.name", version: "0.1.0", engine_version: "*" }}"#),
        )
        .unwrap();
    }

    #[test]
    fn game_root_then_mod_folders_in_name_order() {
        let base = scratch("layout");
        write_mod(&base.join("game"), "rome");
        write_mod(&base.join("mods/zeta"), "zeta");
        write_mod(&base.join("mods/alpha"), "alpha");
        std::fs::create_dir_all(base.join("mods/not_a_mod")).unwrap();
        let found =
            discover(&[base.join("game"), base.join("mods"), base.join("missing")]).unwrap();
        let ids: Vec<(&str, bool)> = found
            .iter()
            .map(|m| (m.manifest.id.as_str(), m.is_game))
            .collect();
        assert_eq!(ids, vec![("rome", true), ("alpha", false), ("zeta", false)]);
    }

    #[test]
    fn broken_manifests_are_collected_not_fatal_one_by_one() {
        let base = scratch("broken");
        write_mod(&base.join("game"), "rome");
        std::fs::create_dir_all(base.join("mods/bad")).unwrap();
        std::fs::write(base.join("mods/bad/mod.json5"), "{ id: 1 }").unwrap();
        write_mod(&base.join("mods/good"), "good");
        let err = discover(&[base.join("game"), base.join("mods")]).unwrap_err();
        assert_eq!(err.len(), 1, "{err}");
        assert!(
            err.0[0].file.ends_with("bad/mod.json5") || err.0[0].file.ends_with("bad\\mod.json5")
        );
    }
}
