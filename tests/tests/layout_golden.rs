//! T1-040 done-when: golden slot tables for every layout at
//! n ∈ {1, 7, 60, 160, 500} (`tests/golden/layouts.json`). Regenerate with
//! `IL_UPDATE_GOLDEN=1 cargo test -p il_tests --test layout_golden` and
//! review the diff: a change here is a change to every formation.

use std::path::PathBuf;

use il_core::{S, Scalar};
use il_data::{ContentId, Layout};
use il_sim_battle::formation::{Slot, effective_ranks, layout_slots};

const COUNTS: [u16; 5] = [1, 7, 60, 160, 500];
const TEMPLATES: [&str; 6] = [
    "rome:line",
    "rome:column",
    "rome:square",
    "rome:wedge",
    "rome:phalanx",
    "rome:loose",
];

fn golden_path() -> PathBuf {
    il_tests::workspace_root().join("tests/golden/layouts.json")
}

/// `[x, y, facing_rad, rank, file]` per slot, floats as their exact bits so
/// the table is a bit-level regression.
fn encode(slots: &[Slot]) -> serde_json::Value {
    serde_json::Value::Array(
        slots
            .iter()
            .map(|s| {
                serde_json::json!([
                    s.offset.x.to_bits(),
                    s.offset.y.to_bits(),
                    s.facing_offset.radians().to_bits(),
                    s.rank,
                    s.file
                ])
            })
            .collect(),
    )
}

#[test]
fn golden_slot_tables_match() {
    let regs = il_tests::game_regs();
    let hastati = regs.units.get(
        regs.units
            .lookup(&ContentId::new("rome:hastati").unwrap())
            .unwrap(),
    );
    let radius = hastati.soldier_radius;
    assert_eq!(radius, S::from_f32_data(0.4));

    let mut table = serde_json::Map::new();
    // The six game templates plus a synthetic custom one.
    let mut templates: Vec<il_data::FormationTemplate> = TEMPLATES
        .iter()
        .map(|id| {
            regs.formations
                .get(
                    regs.formations
                        .lookup(&ContentId::new(id).unwrap())
                        .unwrap(),
                )
                .clone()
        })
        .collect();
    let mut custom = templates[0].clone();
    custom.id = ContentId::new("test:custom").unwrap();
    custom.layout = Layout::Custom;
    custom.custom_slots = vec![
        il_core::V2::from_f32_data(0.0, 0.0),
        il_core::V2::from_f32_data(-1.5, -1.0),
        il_core::V2::from_f32_data(1.5, -1.0),
        il_core::V2::from_f32_data(-3.0, -2.0),
        il_core::V2::from_f32_data(3.0, -2.0),
    ];
    templates.push(custom);

    let mut slots = Vec::new();
    for t in &templates {
        for n in COUNTS {
            let ranks = effective_ranks(t, n, None);
            layout_slots(t, n, ranks, radius, &mut slots);
            assert_eq!(slots.len(), usize::from(n), "{} n={n}", t.id);
            let mut seen: Vec<(u32, u32)> = slots
                .iter()
                .map(|s| (s.offset.x.to_bits(), s.offset.y.to_bits()))
                .collect();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), slots.len(), "{} n={n}: duplicate slots", t.id);
            if t.layout != Layout::Custom {
                let front: Vec<&Slot> = slots
                    .iter()
                    .filter(|s| s.rank == 0 && s.facing_offset == il_core::Angle::ZERO)
                    .collect();
                let mut sum = S::ZERO;
                for s in &front {
                    sum = sum + s.offset.x;
                }
                if !front.is_empty() {
                    assert!(
                        (sum / S::from_i32(front.len() as i32)).abs() < S::from_f32_data(1e-4),
                        "{} n={n}: front rank not centred",
                        t.id
                    );
                }
            }
            table.insert(
                format!("{}/{n}/ranks{ranks}", t.id.as_str()),
                encode(&slots),
            );
        }
    }
    let actual = serde_json::Value::Object(table);
    let path = golden_path();
    if std::env::var_os("IL_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&actual).unwrap()).unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (run with IL_UPDATE_GOLDEN=1)", path.display()));
    let expected: serde_json::Value = serde_json::from_str(&text).unwrap();
    let (Some(exp), Some(act)) = (expected.as_object(), actual.as_object()) else {
        panic!("golden file is not an object");
    };
    let mut exp_keys: Vec<_> = exp.keys().collect();
    let mut act_keys: Vec<_> = act.keys().collect();
    exp_keys.sort();
    act_keys.sort();
    assert_eq!(exp_keys, act_keys, "the set of golden tables changed");
    for key in exp_keys {
        assert_eq!(exp[key], act[key], "layout table {key} changed");
    }
}
