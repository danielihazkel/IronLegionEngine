//! The load pipeline over a resolved `ModSet` (TDD §3.3 steps 3–6): parse
//! every content file of every mod in load order, merge per kind (T1-022),
//! validate the merged objects (T1-021), register every valid id (pass 1),
//! deserialise and resolve references (pass 2), compute the hashes.
//! Diagnostics are collected across all mods before failing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::content_id::ContentId;
use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::faction::Faction;
use crate::formation::{FormationTemplate, GroupFormationTemplate};
use crate::json5::{PathSeg, Span, SpannedValue};
use crate::load_order::ModSet;
use crate::loader::{json5_files, parse_content_file};
use crate::locale::load_locales;
use crate::map_def::MapDef;
use crate::merge::{ApplyCtx, KindAccumulator, MergedItem, merge_singleton};
use crate::registries::{ModInfo, Registries};
use crate::registry::{ContentKind, Lookup, Registry, ResolveError};
use crate::rules::{FormationRules, InputBindings, MovementRules, Rules};
use crate::schema::KindTag;
use crate::source::Sources;
use crate::sprite_set::SpriteSet;
use crate::text::nearest;
use crate::unit_type::UnitType;
use crate::validate::{validate_merged, validate_value};
use crate::zone::ZoneType;

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

/// A merged singleton file (`content/<dir>/<stem>.json5`), or `None` when no
/// enabled mod ships it.
pub struct Singleton {
    pub value: Option<SpannedValue>,
}

/// Merges `content/<dir>/<stem>.json5` across the mod set.
pub fn merge_singleton_file(
    set: &ModSet,
    dir: &str,
    stem: &str,
    sources: &mut Sources,
    diags: &mut Diagnostics,
) -> Singleton {
    let mut value: Option<SpannedValue> = None;
    for (mod_index, m) in set.mods.iter().enumerate() {
        let rel = Path::new(&m.manifest.content_root)
            .join(dir)
            .join(format!("{stem}.json5"));
        let abs = m.root.join(&rel);
        if !abs.is_file() {
            continue;
        }
        let file_id = sources.add(mod_index, &m.manifest.id, &rel, &abs);
        let display = sources.display(file_id);
        let objects = match parse_content_file(&abs, &display, file_id) {
            Ok(v) => v,
            Err(d) => {
                diags.push(d);
                continue;
            }
        };
        let namespaces: Vec<String> = m.namespaces().map(str::to_string).collect();
        let ctx = ApplyCtx {
            mod_index,
            mod_id: &m.manifest.id,
            namespaces: &namespaces,
            sources,
        };
        for obj in objects {
            merge_singleton(&mut value, obj, &ctx, diags);
        }
    }
    Singleton { value }
}

/// The ids of `acc` whose merged object passes the schema.
fn valid_ids(
    acc: &KindAccumulator,
    sources: &Sources,
    diags: &mut Diagnostics,
) -> BTreeSet<ContentId> {
    acc.items
        .iter()
        .filter(|(_, item)| validate_merged(acc.kind, item, sources, diags))
        .map(|(id, _)| id.clone())
        .collect()
}

/// `formations[1]`, `zones[0].type` → path segments.
fn parse_field_path(field: &str) -> Vec<PathSeg<'_>> {
    let mut out = Vec::new();
    for part in field.split('.') {
        let (name, rest) = part.split_once('[').unwrap_or((part, ""));
        if !name.is_empty() {
            out.push(PathSeg::Key(name));
        }
        for idx in rest.split('[') {
            if let Some(n) = idx.strip_suffix(']')
                && let Ok(i) = n.parse::<usize>()
            {
                out.push(PathSeg::Index(i));
            }
        }
    }
    out
}

type Tombstones = BTreeMap<KindTag, BTreeMap<ContentId, Span>>;

fn resolve_diagnostic(
    item: &MergedItem,
    err: &ResolveError,
    lookup: &Lookup,
    tombstones: &Tombstones,
    sources: &Sources,
) -> Diagnostic {
    let segs = parse_field_path(&err.field);
    let span = item
        .value
        .at_path(&segs)
        .map_or(item.defined_at, |v| v.span);
    let message = match &err.message {
        Some(m) => m.clone(),
        None => {
            let deleted = tombstones
                .get(&err.kind)
                .and_then(|t| t.get(&err.id))
                .map(|s| {
                    let f = sources.get(s.file);
                    format!(
                        "; deleted by {:?} ({}:{}:{})",
                        f.mod_id, f.rel, s.line, s.col
                    )
                })
                .unwrap_or_default();
            format!(
                "unknown reference in {} {:?}{deleted}",
                err.field,
                err.id.as_str()
            )
        }
    };
    let mut expected = format!("an existing {} ContentId", err.kind.label());
    if let Some(n) = nearest(err.id.as_str(), lookup.ids(err.kind).map(ContentId::as_str)) {
        expected.push_str(&format!("; nearest: {n:?}"));
    }
    Diagnostic::file_level(sources.display(span.file), message)
        .at(span.line, span.col)
        .field(err.field.clone())
        .expected(expected)
}

/// Deserialises and resolves the valid items of `acc`, in ascending
/// `ContentId` order so handles never depend on file order (pass 2).
fn build_registry<T: ContentKind>(
    acc: &KindAccumulator,
    valid: &BTreeSet<ContentId>,
    lookup: &Lookup,
    tombstones: &Tombstones,
    sources: &Sources,
    diags: &mut Diagnostics,
) -> Registry<T> {
    let mut registry = Registry::new();
    for (id, item) in &acc.items {
        if !valid.contains(id) {
            continue;
        }
        let mut typed: T = match serde_json::from_value(item.value.to_json()) {
            Ok(t) => t,
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
                continue;
            }
        };
        let mut errors = Vec::new();
        typed.resolve(lookup, &mut errors);
        if !errors.is_empty() {
            // References to items that failed their own validation are not
            // reported again; the item is still dropped.
            for err in errors.iter().filter(|e| !lookup.is_invalid(e.kind, &e.id)) {
                diags.push(resolve_diagnostic(item, err, lookup, tombstones, sources));
            }
            continue;
        }
        registry
            .insert(typed)
            .expect("ids are unique in a BTreeMap");
    }
    registry
}

fn build_singleton<T: serde::de::DeserializeOwned>(
    single: &Singleton,
    kind: KindTag,
    what: &str,
    sources: &Sources,
    diags: &mut Diagnostics,
    set: &ModSet,
) -> Option<T> {
    let Some(value) = &single.value else {
        let game = set.mods.first().map_or("game", |m| m.manifest.id.as_str());
        diags.push(Diagnostic::file_level(
            format!("{game}/{what}"),
            "missing: no enabled mod ships this file and every field is required".to_string(),
        ));
        return None;
    };
    let file = sources.display(value.span.file);
    if !validate_value(kind, value, &file, diags) {
        return None;
    }
    match serde_json::from_value::<T>(value.to_json()) {
        Ok(t) => Some(t),
        Err(e) => {
            diags.push(
                Diagnostic::file_level(
                    file,
                    format!(
                        "internal: the schema accepted {what} but the loader cannot read it: {e}"
                    ),
                )
                .at(value.span.line, value.span.col),
            );
            None
        }
    }
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

/// The outcome of a load: registries when there were no errors, plus every
/// diagnostic (warnings included) either way.
pub struct LoadReport {
    pub registries: Option<Registries>,
    pub diagnostics: Diagnostics,
}

/// Loads every registry from a resolved mod set.
pub fn load(set: &ModSet) -> Result<Registries, Diagnostics> {
    let report = load_report(set);
    match report.registries {
        Some(regs) => Ok(regs),
        None => Err(report.diagnostics),
    }
}

/// [`load`] keeping the warnings of a successful load (for `il_cli validate`).
pub fn load_report(set: &ModSet) -> LoadReport {
    let mut sources = Sources::new();
    let mut diags = Diagnostics::new();

    // Parse and merge.
    let units = merge_kind::<UnitType>(set, &mut sources, &mut diags);
    let formations = merge_kind::<FormationTemplate>(set, &mut sources, &mut diags);
    let group_formations = merge_kind::<GroupFormationTemplate>(set, &mut sources, &mut diags);
    let factions = merge_kind::<Faction>(set, &mut sources, &mut diags);
    let zones = merge_kind::<ZoneType>(set, &mut sources, &mut diags);
    let maps = merge_kind::<MapDef>(set, &mut sources, &mut diags);
    let sprite_sets = merge_kind::<SpriteSet>(set, &mut sources, &mut diags);
    let movement = merge_singleton_file(set, "rules", "movement", &mut sources, &mut diags);
    let formation_rules = merge_singleton_file(set, "rules", "formation", &mut sources, &mut diags);
    let input = merge_singleton_file(set, "input", "bindings", &mut sources, &mut diags);
    let locale = load_locales(set, &mut sources, &mut diags);

    // Pass 1: validate, register every valid id.
    let mut lookup = Lookup::new();
    let mut tombstones: Tombstones = BTreeMap::new();
    let mut pass1 = |acc: &KindAccumulator, diags: &mut Diagnostics| -> BTreeSet<ContentId> {
        let valid = valid_ids(acc, &sources, diags);
        lookup.register(acc.kind, valid.iter());
        lookup.register_invalid(acc.kind, acc.items.keys().filter(|id| !valid.contains(*id)));
        tombstones.insert(
            acc.kind,
            acc.tombstones
                .iter()
                .map(|(id, t)| (id.clone(), t.span))
                .collect(),
        );
        valid
    };
    let units_ok = pass1(&units, &mut diags);
    let formations_ok = pass1(&formations, &mut diags);
    let group_ok = pass1(&group_formations, &mut diags);
    let factions_ok = pass1(&factions, &mut diags);
    let zones_ok = pass1(&zones, &mut diags);
    let maps_ok = pass1(&maps, &mut diags);
    let sprites_ok = pass1(&sprite_sets, &mut diags);

    // Pass 2: deserialise and resolve.
    let movement: Option<MovementRules> = build_singleton(
        &movement,
        KindTag::RulesMovement,
        "content/rules/movement.json5",
        &sources,
        &mut diags,
        set,
    );
    let formation: Option<FormationRules> = build_singleton(
        &formation_rules,
        KindTag::RulesFormation,
        "content/rules/formation.json5",
        &sources,
        &mut diags,
        set,
    );
    let rules = match (movement, formation) {
        (Some(movement), Some(formation)) => Rules {
            movement,
            formation,
        },
        _ => Rules::zeroed(),
    };
    let input = build_singleton::<InputBindings>(
        &input,
        KindTag::InputBindings,
        "content/input/bindings.json5",
        &sources,
        &mut diags,
        set,
    )
    .unwrap_or_default();

    let mut regs = Registries {
        units: build_registry(
            &units,
            &units_ok,
            &lookup,
            &tombstones,
            &sources,
            &mut diags,
        ),
        formations: build_registry(
            &formations,
            &formations_ok,
            &lookup,
            &tombstones,
            &sources,
            &mut diags,
        ),
        group_formations: build_registry(
            &group_formations,
            &group_ok,
            &lookup,
            &tombstones,
            &sources,
            &mut diags,
        ),
        factions: build_registry(
            &factions,
            &factions_ok,
            &lookup,
            &tombstones,
            &sources,
            &mut diags,
        ),
        zones: build_registry(
            &zones,
            &zones_ok,
            &lookup,
            &tombstones,
            &sources,
            &mut diags,
        ),
        maps: build_registry(&maps, &maps_ok, &lookup, &tombstones, &sources, &mut diags),
        sprite_sets: build_registry(
            &sprite_sets,
            &sprites_ok,
            &lookup,
            &tombstones,
            &sources,
            &mut diags,
        ),
        rules,
        input,
        locale,
        mods: set
            .mods
            .iter()
            .map(|m| ModInfo {
                id: m.manifest.id.clone(),
                version: m.manifest.version.to_string(),
            })
            .collect(),
        mod_list_hash: set.mod_list_hash(),
        content_registry_hash: 0,
    };
    regs.content_registry_hash = regs.compute_content_hash();
    LoadReport {
        registries: if diags.has_errors() { None } else { Some(regs) },
        diagnostics: diags,
    }
}

/// Discovers, orders and loads the mods under `roots` (first root = game).
pub fn load_roots(roots: &[PathBuf]) -> Result<Registries, Diagnostics> {
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
    use il_core::{S, Scalar};

    fn scratch(name: &str) -> PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/il_data_test/pipeline")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn game_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game")
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    /// A copy of game/ (content only) under the scratch dir, so tests can
    /// edit it.
    fn game_copy(name: &str) -> PathBuf {
        let base = scratch(name);
        let dst = base.join("game");
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::copy(game_dir().join("mod.json5"), dst.join("mod.json5")).unwrap();
        copy_dir(&game_dir().join("content"), &dst.join("content"));
        dst
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

    const MYMOD: &str = r#"{ id: "mymod", name_key: "mymod.mod.name", version: "1.0.0", engine_version: "*", dependencies: [{ id: "rome", version: ">=0.1.0" }] }"#;

    fn unit(id: &str, hp: u32) -> String {
        format!(
            r#"{{ id: "{id}", name_key: "t.units.x.name", category: "infantry",
        hp: {hp}, speed_walk: 1.6, speed_run: 4.0, attack: 1, defence: 1, damage: 1,
        formations: ["rome:line"], sprite_set: "rome:sprites_infantry", cost: 1, upkeep: 1 }}"#
        )
    }

    #[test]
    fn game_root_populates_every_phase_1_registry() {
        let regs = load_roots(&[game_dir()]).unwrap_or_else(|e| panic!("{e}"));
        assert!(!regs.units.is_empty());
        assert!(!regs.formations.is_empty());
        assert!(!regs.group_formations.is_empty());
        assert!(!regs.factions.is_empty());
        assert!(!regs.zones.is_empty());
        assert!(!regs.sprite_sets.is_empty());
        assert!(regs.maps.is_empty(), "the test map arrives with T1-030");
        assert_eq!(regs.rules.movement.nav_cell, S::from_f32_data(4.0));
        assert_eq!(regs.rules.formation.swap_passes, 2);
        assert!(!regs.input.keys_for("camera_pan_up").is_empty());
        assert_eq!(regs.locale.get("rome.units.hastati.name"), "Hastati");
        assert_eq!(regs.locale.get("rome.zones.ford.name"), "Ford");
        let h = regs
            .units
            .lookup(&ContentId::new("rome:hastati").unwrap())
            .unwrap();
        let hastati = regs.units.get(h);
        assert_eq!(hastati.formations.len(), 3);
        assert_eq!(
            regs.formations.id_of(hastati.default_formation()).as_str(),
            "rome:line"
        );
        assert_eq!(
            regs.sprite_sets.id_of(hastati.sprite_set()).as_str(),
            "rome:sprites_infantry"
        );
        assert_eq!(regs.mods.len(), 1);
        assert_ne!(regs.content_registry_hash, 0);
        assert_ne!(regs.mod_list_hash, 0);
    }

    #[test]
    fn a_second_mod_overrides_a_game_unit_by_merge() {
        let game = game_copy("override");
        let mods = game.parent().unwrap().join("mods");
        write_mod(
            &mods.join("mymod"),
            MYMOD,
            &[(
                "content/units/tweak.json5",
                r#"{ id: "rome:hastati", hp: 150, speed_walk: 2.5 }"#,
            )],
        );
        let regs = load_roots(&[game, mods]).unwrap_or_else(|e| panic!("{e}"));
        let h = regs
            .units
            .lookup(&ContentId::new("rome:hastati").unwrap())
            .unwrap();
        assert_eq!(regs.units.get(h).hp, S::from_f32_data(150.0));
        assert_eq!(regs.units.get(h).speed_walk, S::from_f32_data(2.5));
        assert_eq!(regs.mods.len(), 2);
    }

    #[test]
    fn merged_result_errors_name_both_locations() {
        let game = game_copy("provenance");
        let mods = game.parent().unwrap().join("mods");
        write_mod(
            &mods.join("mymod"),
            MYMOD,
            &[(
                "content/units/tweak.json5",
                "{\n  id: \"rome:hastati\",\n  armour: -2,\n}",
            )],
        );
        let err = load_roots(&[game, mods]).unwrap_err();
        assert_eq!(err.len(), 1, "{err}");
        let text = err.0[0].to_string();
        assert!(
            text.starts_with("rome/content/units/hastati.json5:"),
            "{text}"
        );
        assert!(
            text.contains(" armour: after merge by \"mymod\" (content/units/tweak.json5:3:11): value -2 out of range (expected 0..=100)"),
            "{text}"
        );
    }

    #[test]
    fn handles_resolve_regardless_of_file_order_and_registries_sort_by_id() {
        let game = game_copy("order");
        std::fs::write(
            game.join("content/units/a_first.json5"),
            unit("rome:zed", 1),
        )
        .unwrap();
        std::fs::write(
            game.join("content/factions/z_last.json5"),
            r##"{ id: "rome:late", name_key: "rome.factions.late.name", culture: "latin", colour_primary: "#000000", colour_secondary: "#ffffff", units: ["rome:zed", "rome:hastati"], ai_profile: "rome:x", tech_tree: "rome:y" }"##,
        )
        .unwrap();
        let regs = load_roots(&[game]).unwrap_or_else(|e| panic!("{e}"));
        let ids: Vec<&str> = regs.units.ids().map(ContentId::as_str).collect();
        assert_eq!(ids, vec!["rome:hastati", "rome:zed"]);
        let f = regs.factions.get(
            regs.factions
                .lookup(&ContentId::new("rome:late").unwrap())
                .unwrap(),
        );
        assert_eq!(f.units.len(), 2);
        assert_eq!(regs.units.id_of(f.units[0]).as_str(), "rome:zed");
    }

    #[test]
    fn unknown_references_are_positioned_with_a_suggestion() {
        let game = game_copy("unknown_ref");
        std::fs::write(
            game.join("content/units/bad.json5"),
            "{\n  id: \"rome:bad\", name_key: \"t.units.x.name\", category: \"infantry\",\n  hp: 1, speed_walk: 1.6, speed_run: 4.0, attack: 1, defence: 1, damage: 1,\n  formations: [\"rome:line\", \"rome:lin\"],\n  sprite_set: \"rome:sprites_infantry\", cost: 1, upkeep: 1,\n}",
        )
        .unwrap();
        let err = load_roots(&[game]).unwrap_err();
        assert_eq!(err.len(), 1, "{err}");
        assert_eq!(
            err.0[0].to_string(),
            "rome/content/units/bad.json5:4:29 formations[1]: unknown reference in formations[1] \"rome:lin\" (expected an existing formation template ContentId; nearest: \"rome:line\")"
        );
    }

    #[test]
    fn rules_require_every_field_and_the_file() {
        let game = game_copy("rules_missing_field");
        let path = game.join("content/rules/formation.json5");
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replace("  swap_passes: 2,\n", "");
        std::fs::write(&path, text).unwrap();
        let err = load_roots(&[game]).unwrap_err();
        assert!(
            err.0
                .iter()
                .any(|d| d.message == "missing required field \"swap_passes\""
                    && d.file.to_string_lossy().replace('\\', "/")
                        == "rome/content/rules/formation.json5"),
            "{err}"
        );

        let game = game_copy("rules_missing_file");
        std::fs::remove_file(game.join("content/rules/movement.json5")).unwrap();
        let err = load_roots(&[game]).unwrap_err();
        assert!(
            err.0
                .iter()
                .any(|d| d.file.to_string_lossy().replace('\\', "/")
                    == "rome/content/rules/movement.json5"
                    && d.message.starts_with("missing")),
            "{err}"
        );
    }

    #[test]
    fn a_mod_can_tweak_one_rule_and_the_hash_changes() {
        let game = game_copy("rules_tweak");
        let mods = game.parent().unwrap().join("mods");
        let base = load_roots(std::slice::from_ref(&game)).unwrap();
        write_mod(
            &mods.join("mymod"),
            MYMOD,
            &[("content/rules/movement.json5", "{ paths_per_tick: 16 }")],
        );
        let regs = load_roots(&[game, mods]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(regs.rules.movement.paths_per_tick, 16);
        assert_eq!(
            regs.rules.movement.nav_cell,
            S::from_f32_data(4.0),
            "other fields inherited"
        );
        assert_ne!(regs.content_registry_hash, base.content_registry_hash);
        assert_ne!(regs.mod_list_hash, base.mod_list_hash);
    }

    #[test]
    fn content_hash_ignores_file_layout_whitespace_key_order_and_number_spelling() {
        let a = game_copy("hash_a");
        let base = load_roots(&[a]).unwrap();

        // Same content: unit moved to another file, a key reordered, comments,
        // whitespace, and integer-valued floats spelled differently.
        let b = game_copy("hash_b");
        let text = std::fs::read_to_string(b.join("content/units/hastati.json5")).unwrap();
        std::fs::remove_file(b.join("content/units/hastati.json5")).unwrap();
        let reordered = text
            .replace("  hp: 100,\n", "")
            .replace(
                "  mass: 80,\n",
                "  mass: 80.0, // heavier spelling\n  hp: 1e2,\n",
            )
            .replace("  speed_walk: 1.6,", "\n\n  speed_walk:   1.60,");
        assert_ne!(reordered, text);
        std::fs::write(b.join("content/units/zz_moved.json5"), reordered).unwrap();
        let same = load_roots(&[b]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(same.content_registry_hash, base.content_registry_hash);
        assert_eq!(same.units.len(), base.units.len());

        // A value change is visible.
        let c = game_copy("hash_c");
        let text = std::fs::read_to_string(c.join("content/units/hastati.json5")).unwrap();
        std::fs::write(
            c.join("content/units/hastati.json5"),
            text.replace("hp: 100,", "hp: 101,"),
        )
        .unwrap();
        let changed = load_roots(&[c]).unwrap();
        assert_ne!(changed.content_registry_hash, base.content_registry_hash);
    }

    #[test]
    fn later_mods_override_locale_keys_and_bad_leaves_are_diagnostics() {
        let game = game_copy("locale");
        std::fs::create_dir_all(game.join("locale")).unwrap();
        std::fs::write(game.join("locale/en.json5"), r#"{ rome: { units: { hastati: { name: "Hastati" } } }, il: { app: { title: "Iron Legion" } } }"#).unwrap();
        let mods = game.parent().unwrap().join("mods");
        write_mod(
            &mods.join("mymod"),
            MYMOD,
            &[
                (
                    "locale/en.json5",
                    r#"{ rome: { units: { hastati: { name: "Spearmen" } } }, mymod: { mod: { name: "My Mod" } } }"#,
                ),
                (
                    "locale/de.json5",
                    r#"{ mymod: { mod: { name: "Mein Mod" } } }"#,
                ),
            ],
        );
        let regs = load_roots(&[game.clone(), mods.clone()]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(regs.locale.get("rome.units.hastati.name"), "Spearmen");
        assert_eq!(regs.locale.get("il.app.title"), "Iron Legion");
        assert_eq!(regs.locale.get("mymod.mod.name"), "My Mod");
        assert_eq!(
            regs.locale.languages().collect::<Vec<_>>(),
            vec!["de", "en"]
        );

        std::fs::write(
            mods.join("mymod/locale/en.json5"),
            "{ mymod: { mod: { name: 7 } } }",
        )
        .unwrap();
        let err = load_roots(&[game, mods]).unwrap_err();
        assert!(
            err.0.iter().any(|d| d.field == "mymod.mod.name"
                && d.file.to_string_lossy().replace('\\', "/") == "mymod/locale/en.json5"),
            "{err}"
        );
    }

    #[test]
    fn diagnostics_are_collected_across_mods() {
        let game = game_copy("collect");
        std::fs::write(game.join("content/units/a.json5"), "{ id: 5 }").unwrap();
        let mods = game.parent().unwrap().join("mods");
        write_mod(
            &mods.join("mymod"),
            MYMOD,
            &[
                ("content/units/b.json5", "not json5"),
                ("content/units/c.json5", "{ id: \"mymod:c\" }"),
            ],
        );
        let err = load_roots(&[game, mods]).unwrap_err();
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
