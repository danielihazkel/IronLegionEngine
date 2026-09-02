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

pub fn cid(s: &str) -> ContentId {
    ContentId::new(s).unwrap()
}

pub fn regiment(id: u32, unit: &str, count: u16, x: f32, facing_deg: f32) -> RegimentSetup {
    RegimentSetup {
        id,
        unit_type: cid(unit),
        count,
        experience: 0,
        fatigue: 0.0,
        formation: None,
        position: Some([x, 0.0]),
        facing_deg: Some(facing_deg),
    }
}

pub fn side(player: u8, regiments: Vec<RegimentSetup>) -> SideSetup {
    SideSetup {
        faction: cid("rome:rome"),
        player: PlayerId(player),
        deployment_zone: 0,
        general: GeneralSetup {
            unit_type: cid("rome:hastati"),
            rank: 1,
            name_key: String::new(),
        },
        regiments,
        reinforcements: vec![],
    }
}

/// Two sides, players 0 and 1, one hastati regiment of `count` each, facing
/// each other across the origin. Regiment ids are 0 (side 0) and 1 (side 1).
pub fn two_sides(count: u16) -> BattleSetup {
    BattleSetup {
        map_id: None,
        seed: 42,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: vec![
            side(0, vec![regiment(1, "rome:hastati", count, -100.0, 0.0)]),
            side(1, vec![regiment(2, "rome:hastati", count, 100.0, 180.0)]),
        ],
        victory: Default::default(),
    }
}

pub fn world(count: u16) -> BattleWorld {
    BattleWorld::new(&two_sides(count), regs()).unwrap()
}
