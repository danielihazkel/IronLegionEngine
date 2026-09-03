//! The load pipeline over a resolved `ModSet` (TDD §3.3): parse every
//! content file of every mod in load order, merge per kind (T1-022),
//! validate the merged objects (T1-021), deserialise into typed registries.
//! Diagnostics are collected across all mods before failing.

use std::collections::BTreeMap;
use std::path::Path;

use crate::content_id::ContentId;
use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::load_order::ModSet;
use crate::loader::{Registries, json5_files, parse_content_file};
use crate::merge::{ApplyCtx, KindAccumulator, MergedItem};
use crate::registry::{ContentKind, Registry};
use crate::source::Sources;
use crate::unit_type::UnitType;
use crate::validate::validate_merged;

/// Parses and merges every file of `T::DIR` across the mod set.
pub fn merge_kind<T: ContentKind>(
    set: &ModSet,
    sources: &mut Sources,
    diags: &mut Diagnostics,
) -> KindAccumulator {
    let mut acc = KindAccumulator::new(T::TAG);
    for (mod_index, m) in set.mods.iter().enumerate() {
        let dir = m.content_dir().join(T::DIR);
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        if let Err(e) = json5_files(&dir, &mut files) {
            diags.push(Diagnostic::file_level(&dir, format!("cannot list: {e}")));
            continue;
        }
        let mut objects = Vec::new();
        for abs in &files {
            let rel = abs.strip_prefix(&m.root).unwrap_or(abs);
            let file_id = sources.add(mod_index, &m.manifest.id, rel, abs);
            let display = sources.display(file_id);
            match parse_content_file(abs, &display, file_id) {
                Ok(values) => objects.extend(values),
                Err(d) => diags.push(d),
            }
        }
        let namespaces: Vec<String> = m.namespaces().map(str::to_string).collect();
        let ctx = ApplyCtx {
            mod_index,
            mod_id: &m.manifest.id,
            namespaces: &namespaces,
            sources,
        };
        acc.apply_mod(objects, &ctx, diags);
    }
    acc
}

/// Validates every merged item of `acc` and deserialises the valid ones
/// into a registry, in ascending `ContentId` order so handles never depend
/// on file order.
pub fn build_registry<T: ContentKind>(
    acc: &KindAccumulator,
    sources: &Sources,
    diags: &mut Diagnostics,
) -> Registry<T> {
    let mut registry = Registry::new();
    for (id, item) in &acc.items {
        if !validate_merged(T::TAG, item, sources, diags) {
            continue;
        }
        match serde_json::from_value::<T>(item.value.to_json()) {
            Ok(typed) => {
                registry
                    .insert(typed)
                    .expect("ids are unique in a BTreeMap");
            }
            Err(e) => {
                let span = item.defined_at;
                diags.push(
                    Diagnostic::file_level(
                        sources.display(span.file),
                        format!(
                            "internal: the schema accepted {} {id} but the loader cannot read it: {e}",
                            T::TAG.label()
                        ),
                    )
                    .at(span.line, span.col),
                );
            }
        }
    }
    registry
}

/// The merged objects of one kind as plain JSON, for tools and golden tests.
pub fn merged_json<T: ContentKind>(
    set: &ModSet,
) -> Result<BTreeMap<ContentId, serde_json::Value>, Diagnostics> {
    let mut sources = Sources::new();
    let mut diags = Diagnostics::new();
    let acc = merge_kind::<T>(set, &mut sources, &mut diags);
    let out = acc
        .items
        .iter()
        .map(|(id, item): (&ContentId, &MergedItem)| (id.clone(), item.value.to_json()))
        .collect();
    diags.into_result(out)
}

/// Loads every registry from a resolved mod set.
pub fn load(set: &ModSet) -> Result<Registries, Diagnostics> {
    let mut sources = Sources::new();
    let mut diags = Diagnostics::new();
    let units_acc = merge_kind::<UnitType>(set, &mut sources, &mut diags);
    let units = build_registry::<UnitType>(&units_acc, &sources, &mut diags);
    diags.into_result(Registries { units })
}

/// Discovers, orders and loads the mods under `roots` (first root = game).
pub fn load_roots(roots: &[std::path::PathBuf]) -> Result<Registries, Diagnostics> {
    let found = crate::discover::discover(roots)?;
    let set = ModSet::all(&found).map_err(|errors| {
        Diagnostics(
            errors
                .iter()
                .map(|e| {
                    Diagnostic::file_level(
                        roots
                            .first()
                            .map_or(Path::new("."), |p| p.as_path())
                            .join("mod.json5"),
                        e.to_string(),
                    )
                })
                .collect(),
        )
    })?;
    load(&set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/il_data_test/pipeline")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_mod(dir: &Path, manifest: &str, files: &[(&str, &str)]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("mod.json5"), manifest).unwrap();
        for (rel, text) in files {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
    }

    const GAME: &str = r#"{ id: "rome", name_key: "rome.mod.name", version: "0.1.0", engine_version: "*", namespaces: ["rome", "greece"] }"#;
    const MYMOD: &str = r#"{ id: "mymod", name_key: "mymod.mod.name", version: "1.0.0", engine_version: "*", dependencies: [{ id: "rome", version: ">=0.1.0" }] }"#;

    fn unit(id: &str, hp: u32) -> String {
        format!(
            r#"{{ id: "{id}", name_key: "t.units.x.name", category: "infantry",
        hp: {hp}, speed_walk: 1.6, speed_run: 4.0, attack: 1, defence: 1, damage: 1,
        formations: ["rome:line"], sprite_set: "sprites/units/x", cost: 1, upkeep: 1 }}"#
        )
    }

    #[test]
    fn a_second_mod_overrides_a_game_unit_by_merge() {
        let base = scratch("override");
        write_mod(
            &base.join("game"),
            GAME,
            &[("content/units/a.json5", &unit("rome:a", 100))],
        );
        write_mod(
            &base.join("mods/mymod"),
            MYMOD,
            &[(
                "content/units/tweak.json5",
                r#"{ id: "rome:a", hp: 150, speed_walk: 2.5 }"#,
            )],
        );
        let regs = load_roots(&[base.join("game"), base.join("mods")]).unwrap();
        let h = regs
            .units
            .lookup(&ContentId::new("rome:a").unwrap())
            .unwrap();
        assert_eq!(
            regs.units.get(h).hp,
            <il_core::S as il_core::Scalar>::from_f32_data(150.0)
        );
        assert_eq!(
            regs.units.get(h).speed_walk,
            <il_core::S as il_core::Scalar>::from_f32_data(2.5)
        );
    }

    #[test]
    fn merged_result_errors_name_both_locations() {
        let base = scratch("provenance");
        let hastati = "{\n  id: \"rome:hastati\",\n  armour: 8,\n  name_key: \"t.units.x.name\", category: \"infantry\",\n  hp: 100, speed_walk: 1.6, speed_run: 4.0, attack: 1, defence: 1, damage: 1,\n  formations: [\"rome:line\"], sprite_set: \"sprites/units/x\", cost: 1, upkeep: 1,\n}";
        write_mod(
            &base.join("game"),
            GAME,
            &[("content/units/hastati.json5", hastati)],
        );
        write_mod(
            &base.join("mods/mymod"),
            MYMOD,
            &[(
                "content/units/tweak.json5",
                "{\n  id: \"rome:hastati\",\n  armour: -2,\n}",
            )],
        );
        let err = load_roots(&[base.join("game"), base.join("mods")]).unwrap_err();
        assert_eq!(err.len(), 1, "{err}");
        assert_eq!(
            err.0[0].to_string(),
            "rome/content/units/hastati.json5:3:3 armour: after merge by \"mymod\" (content/units/tweak.json5:3:11): value -2 out of range (expected 0..=100)"
        );
    }

    #[test]
    fn registries_are_in_content_id_order_regardless_of_files() {
        let base = scratch("order");
        write_mod(
            &base.join("game"),
            GAME,
            &[
                ("content/units/z.json5", &unit("rome:a", 1)),
                ("content/units/a.json5", &unit("rome:b", 1)),
                ("content/units/m.json5", &unit("greece:c", 1)),
            ],
        );
        let regs = load_roots(&[base.join("game")]).unwrap();
        let ids: Vec<&str> = regs.units.ids().map(ContentId::as_str).collect();
        assert_eq!(ids, vec!["greece:c", "rome:a", "rome:b"]);
    }

    #[test]
    fn diagnostics_are_collected_across_mods() {
        let base = scratch("collect");
        write_mod(
            &base.join("game"),
            GAME,
            &[("content/units/a.json5", "{ id: 5 }")],
        );
        write_mod(
            &base.join("mods/mymod"),
            MYMOD,
            &[
                ("content/units/b.json5", "not json5"),
                ("content/units/c.json5", "{ id: \"mymod:c\" }"),
            ],
        );
        let err = load_roots(&[base.join("game"), base.join("mods")]).unwrap_err();
        let mut files: Vec<String> = err
            .0
            .iter()
            .map(|d| d.file.to_string_lossy().replace('\\', "/"))
            .collect();
        files.dedup();
        assert_eq!(
            files,
            vec![
                "rome/content/units/a.json5",
                "mymod/content/units/b.json5",
                "mymod/content/units/c.json5"
            ],
            "{err}"
        );
    }
}
