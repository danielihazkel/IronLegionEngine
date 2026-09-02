//! T0-032: `BattleWorld::new` validation and spawning against `game/` content.

use std::path::Path;
use std::sync::Arc;

use il_core::{PlayerId, SoldierId, Tick};
use il_data::{ContentId, Registries};
use il_sim_battle::{
    BattleSetup, BattleWorld, GeneralSetup, RegimentSetup, ReinforcementGroup, SOLDIER_CAP,
    SetupError, SideSetup,
};

fn regs() -> Arc<Registries> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
    Arc::new(Registries::load_root(&root).unwrap_or_else(|d| panic!("{d}")))
}

fn cid(s: &str) -> ContentId {
    ContentId::new(s).unwrap()
}

fn regiment(id: u32, unit: &str, count: u16, x: f32, facing_deg: f32) -> RegimentSetup {
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

fn side(player: u8, regiments: Vec<RegimentSetup>) -> SideSetup {
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

fn two_sides(count: u16) -> BattleSetup {
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

#[test]
fn two_sides_of_500_spawn_1000_soldiers_with_ascending_ids() {
    let w = BattleWorld::new(&two_sides(500), regs()).unwrap();
    assert_eq!(w.soldier_count(), 1000);
    assert_eq!(w.regiment_count(), 2);
    let ids: Vec<SoldierId> = w.soldier_ids().collect();
    assert_eq!(ids.first(), Some(&SoldierId(0)));
    assert_eq!(ids.last(), Some(&SoldierId(999)));
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(w.tick(), Tick::ZERO);
    assert_eq!(w.setup().map(|s| s.seed), Some(42));
}

#[test]
fn over_cap_is_rejected() {
    // 40,000 soldiers across two sides.
    let err = BattleWorld::new(&two_sides(20_000), regs()).unwrap_err();
    assert_eq!(
        err,
        SetupError::OverCap {
            count: 40_000,
            cap: SOLDIER_CAP
        }
    );
    // Reinforcements count toward the cap too (SIM-CORE-006).
    let mut setup = two_sides(16_000);
    setup.sides[0].reinforcements.push(ReinforcementGroup {
        arrival_tick: 100,
        edge: 0,
        regiments: vec![regiment(9, "rome:hastati", 1_000, 0.0, 0.0)],
    });
    assert!(matches!(
        BattleWorld::new(&setup, regs()).unwrap_err(),
        SetupError::OverCap { count: 33_000, .. }
    ));
}

#[test]
fn unknown_unit_types_and_missing_sides_are_rejected() {
    let mut setup = two_sides(10);
    setup.sides[1].regiments[0].unit_type = cid("rome:nope");
    assert_eq!(
        BattleWorld::new(&setup, regs()).unwrap_err(),
        SetupError::UnknownUnitType {
            side: 1,
            unit_type: cid("rome:nope")
        }
    );
    let mut setup = two_sides(10);
    setup.sides[0].general.unit_type = cid("rome:ghost");
    assert!(matches!(
        BattleWorld::new(&setup, regs()).unwrap_err(),
        SetupError::UnknownGeneralUnitType { side: 0, .. }
    ));
    let mut setup = two_sides(10);
    setup.sides.clear();
    assert_eq!(
        BattleWorld::new(&setup, regs()).unwrap_err(),
        SetupError::NoSides
    );
}

#[test]
fn spawned_world_steps_deterministically() {
    let mut a = BattleWorld::new(&two_sides(50), regs()).unwrap();
    let mut b = BattleWorld::new(&two_sides(50), regs()).unwrap();
    assert_eq!(a.hash(), b.hash());
    for _ in 0..20 {
        assert_eq!(a.step(&[]).hash, b.step(&[]).hash);
    }
    let mut c = BattleWorld::new(&two_sides(51), regs()).unwrap();
    assert_ne!(a.hash(), c.step(&[]).hash);
}
