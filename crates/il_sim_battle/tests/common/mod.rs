//! Shared helpers for `il_sim_battle` integration tests.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;

use il_core::PlayerId;
use il_data::{ContentId, Registries};
use il_sim_battle::{BattleSetup, BattleWorld, GeneralSetup, RegimentSetup, SideSetup};

pub fn regs() -> Arc<Registries> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
    Arc::new(Registries::load_root(&root).unwrap_or_else(|d| panic!("{d}")))
}

/// The game plus a mod folder under `tests/mods/` (T3-010).
#[allow(dead_code)]
pub fn regs_with_mod(name: &str) -> Arc<Registries> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let roots = [base.join("game"), base.join("tests/mods").join(name)];
    Arc::new(il_data::load_roots(&roots).unwrap_or_else(|d| panic!("{d}")))
}

pub fn cid(s: &str) -> ContentId {
    ContentId::new(s).unwrap()
}

pub fn regiment(id: u32, unit: &str, count: u16, x: f32, facing_deg: f32) -> RegimentSetup {
    RegimentSetup {
        formation: None,
        position: Some([x, 150.0]),
        facing_deg: Some(facing_deg),
        ..RegimentSetup::single(id, cid(unit), count)
    }
}

pub fn side(player: u8, regiments: Vec<RegimentSetup>) -> SideSetup {
    SideSetup {
        faction: cid("rome:rome"),
        player: PlayerId(player),
        deployment_zone: 0,
        general: GeneralSetup {
            unit_type: cid("rome:general"),
            rank: 1,
            name_key: String::new(),
            bodyguard: None,
        },
        regiments,
        reinforcements: vec![],
        ai_profile: None,
    }
}

/// Two sides, players 0 and 1, one hastati regiment of `count` each, facing
/// each other 200 m apart on the test map. Regiment ids are 0 (side 0) and
/// 1 (side 1).
pub fn two_sides(count: u16) -> BattleSetup {
    BattleSetup {
        map_id: cid("rome:test_field"),
        seed: 42,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: vec![
            side(0, vec![regiment(1, "rome:hastati", count, 300.0, 0.0)]),
            side(1, vec![regiment(2, "rome:hastati", count, 500.0, 180.0)]),
        ],
        victory: Default::default(),
    }
}

pub fn world(count: u16) -> BattleWorld {
    BattleWorld::new(&two_sides(count), regs()).unwrap()
}

/// Pins every regiment's morale at 100 (T2-041): tests about the melee
/// itself want fights that run to the end, and a broken regiment stops
/// fighting and flees (T2-042). Call after every step; recomputes the hash.
pub fn pin_morale(w: &mut BattleWorld) {
    let entities: Vec<_> = w
        .ecs()
        .resource::<il_sim_battle::resources::Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in entities {
        if let Some(mut m) = w.ecs_mut().get_mut::<il_sim_battle::components::Morale>(e) {
            m.m = <il_core::S as il_core::Scalar>::from_i32(100);
        }
    }
    w.recompute_hash();
}
