//! T2-043: the general rides with its bodyguard regiment, projects an aura,
//! dies with a shock to the whole side, and has a fate (SIM-GEN-001..005,
//! SIM-MOR-013/014).

mod common;

use il_core::{PlayerId, RegimentId, S, Scalar, SoldierId, Tick, V2};
use il_sim_battle::combat::{Kill, Kills};
use il_sim_battle::components::{GeneralTag, Health, Morale, MoraleState, Regiment, Soldier};
use il_sim_battle::resources::{Ids, MeleeGateRes, Sides};
use il_sim_battle::{
    BattleEvent, BattleWorld, Command, CommandKind, GeneralFate, RegimentSetup, SetupError,
};

fn at(id: u32, unit: &str, count: u16, x: f32, y: f32, facing_deg: f32) -> RegimentSetup {
    RegimentSetup {
        id,
        unit_type: common::cid(unit),
        count,
        experience: 0,
        fatigue: 0.0,
        formation: None,
        position: Some([x, y]),
        facing_deg: Some(facing_deg),
    }
}

fn regiment_entity(w: &BattleWorld, id: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(id))
        .expect("regiment exists")
}

fn general_of(w: &BattleWorld, side: usize) -> SoldierId {
    w.ecs().resource::<Sides>().0[side]
        .general
        .expect("general spawned")
}

fn morale(w: &BattleWorld, id: u32) -> Morale {
    *w.ecs().get::<Morale>(regiment_entity(w, id)).unwrap()
}

fn in_aura(w: &BattleWorld, id: u32) -> bool {
    let i = w
        .ecs()
        .resource::<Ids>()
        .regiment_index(RegimentId(id))
        .unwrap();
    w.ecs().resource::<MeleeGateRes>().in_aura[i]
}

fn kill(w: &mut BattleWorld, soldier: SoldierId) {
    let e = w.ecs().resource::<Ids>().soldier_entity(soldier).unwrap();
    w.ecs_mut().get_mut::<Health>(e).unwrap().hp = S::ZERO;
    w.ecs_mut().resource_mut::<Kills>().0.push(Kill {
        victim: soldier,
        killer: None,
        killer_regiment: None,
    });
    w.recompute_hash();
}

fn step(w: &mut BattleWorld, n: u32) -> Vec<BattleEvent> {
    let mut events = Vec::new();
    for _ in 0..n {
        let out = w.step(&[]);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        events.extend(out.events);
    }
    events
}

/// SIM-GEN-001 / SIM-FLOW-019: the general is the bodyguard's last soldier
/// with its own unit, tripled hp and the tag; the default bodyguard is the
/// side's first regiment; the cap counts the generals.
#[test]
fn the_general_rides_with_its_bodyguard() {
    let mut setup = common::two_sides(10);
    setup.sides[0]
        .regiments
        .push(at(3, "rome:velites", 10, 300.0, 200.0, 0.0));
    setup.sides[0].general.bodyguard = Some(3);
    setup.sides[0].general.rank = 3;
    let w = BattleWorld::new(&setup, common::regs()).unwrap();
    assert_eq!(setup.soldier_total(), 32);
    assert_eq!(w.soldier_count(), 32);
    let sides = &w.ecs().resource::<Sides>().0;
    // Regiment ids: side 0 hastati 0, velites 1, side 1 hastati 2.
    assert_eq!(
        sides[0].general_regiment,
        Some(RegimentId(1)),
        "named bodyguard"
    );
    assert_eq!(
        sides[1].general_regiment,
        Some(RegimentId(2)),
        "first regiment by default"
    );
    assert!(!sides[0].general_dead && !sides[1].general_dead);
    for (side, expected_rank) in [(0usize, 3u8), (1, 1)] {
        let gid = general_of(&w, side);
        let re = regiment_entity(&w, sides[side].general_regiment.unwrap().0);
        let regiment = w.ecs().get::<Regiment>(re).unwrap();
        assert_eq!(regiment.soldiers.len(), 11, "count + 1");
        assert_eq!(*regiment.soldiers.last().unwrap(), gid, "the last soldier");
        assert_eq!(morale(&w, regiment.id.0).initial, 11);
        let e = w.ecs().resource::<Ids>().soldier_entity(gid).unwrap();
        assert_eq!(w.ecs().get::<GeneralTag>(e).unwrap().rank, expected_rank);
        let unit = w.ecs().get::<Soldier>(e).unwrap().unit;
        let regs = w.registries();
        assert_eq!(regs.units.id_of(unit).as_str(), "rome:general");
        assert_eq!(
            w.ecs().get::<Health>(e).unwrap().hp,
            regs.units.get(unit).hp * regs.rules.general.hp_mult
        );
        assert_eq!(w.view().soldier(gid).unwrap().general, Some(expected_rank));
    }
    // Every other soldier has no tag.
    let tagged = w.view().soldiers().filter(|s| s.general.is_some()).count();
    assert_eq!(tagged, 2);
}

#[test]
fn setup_validation_rejects_bad_generals() {
    let mut setup = common::two_sides(10);
    setup.sides[0].general.unit_type = common::cid("rome:hastati");
    assert!(matches!(
        BattleWorld::new(&setup, common::regs()),
        Err(SetupError::GeneralCategory { side: 0, .. })
    ));
    let mut setup = common::two_sides(10);
    setup.sides[1].general.bodyguard = Some(9);
    assert!(matches!(
        BattleWorld::new(&setup, common::regs()),
        Err(SetupError::UnknownBodyguard { side: 1, id: 9 })
    ));
    let mut setup = common::two_sides(10);
    setup.sides[1].regiments.clear();
    assert!(matches!(
        BattleWorld::new(&setup, common::regs()),
        Err(SetupError::NoBodyguard { side: 1 })
    ));
}

/// SIM-GEN-002: the aura reaches `aura_radius + aura_per_rank × (rank − 1)`
/// from the general; it lifts allied attack and the `general_aura` morale
/// factor, and is suspended while the bodyguard routs.
#[test]
fn the_aura_covers_allies_within_its_radius() {
    // Side 0: bodyguard at x 300 (the general in its rear rank, a couple of
    // metres behind the anchor), allies at 350 and 365 (about 52 m and
    // 67 m from the general). Side 1 far away.
    let mut setup = common::two_sides(20);
    setup.sides[0].regiments = vec![
        at(1, "rome:hastati", 20, 300.0, 150.0, 0.0),
        at(3, "rome:hastati", 20, 350.0, 150.0, 0.0),
        at(4, "rome:hastati", 20, 365.0, 150.0, 0.0),
    ];
    setup.sides[1].regiments = vec![at(2, "rome:hastati", 20, 700.0, 500.0, 180.0)];
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    step(&mut w, 1);
    assert!(in_aura(&w, 0), "the bodyguard itself (SIM-GEN-005)");
    assert!(in_aura(&w, 1), "50 m: inside 60 m");
    assert!(!in_aura(&w, 2), "67 m: outside 60 m");
    assert!(in_aura(&w, 3), "the enemy stands in its own general's aura");
    // Rank 3 adds ten metres.
    setup.sides[0].general.rank = 3;
    let mut w3 = BattleWorld::new(&setup, common::regs()).unwrap();
    step(&mut w3, 1);
    assert!(in_aura(&w3, 2), "70 m at rank 3");
    // The morale factor: an ally in the aura recovers faster than one outside.
    step(&mut w, 100);
    assert!(
        morale(&w, 1).m > morale(&w, 2).m,
        "{:?} vs {:?}",
        morale(&w, 1).m,
        morale(&w, 2).m
    );
    // Routing bodyguard: aura suspended.
    let e = regiment_entity(&w, 0);
    w.ecs_mut().get_mut::<Morale>(e).unwrap().m = S::ZERO;
    w.recompute_hash();
    step(&mut w, 2);
    assert!(matches!(
        morale(&w, 0).state,
        MoraleState::Routing | MoraleState::Shattered
    ));
    assert!(!in_aura(&w, 1), "suspended while the bodyguard routs");
    // The attack multiplier itself.
    let regs = common::regs();
    let g = &regs.rules.general;
    assert_eq!(
        il_sim_battle::combat::aura_attack_mult(true, g),
        S::ONE + g.aura_attack
    );
    assert_eq!(il_sim_battle::combat::aura_attack_mult(false, g), S::ONE);
}

/// SIM-GEN-003 / SIM-MOR-014: the general's death ends the aura, marks the
/// side, emits `GeneralDied`, and every regiment of the side loses the
/// shock next tick (half for one already Shaken).
#[test]
fn the_generals_death_shocks_the_side() {
    let mut setup = common::two_sides(20);
    setup.sides[0].regiments = vec![
        at(1, "rome:hastati", 20, 300.0, 150.0, 0.0),
        at(3, "rome:hastati", 20, 350.0, 150.0, 0.0),
    ];
    setup.sides[1].regiments = vec![at(2, "rome:hastati", 20, 700.0, 500.0, 180.0)];
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    step(&mut w, 200);
    let shaken = regiment_entity(&w, 1);
    w.ecs_mut().get_mut::<Morale>(shaken).unwrap().m = S::from_i32(45);
    w.ecs_mut().get_mut::<Morale>(shaken).unwrap().state = MoraleState::Shaken;
    w.recompute_hash();
    let general = general_of(&w, 0);
    kill(&mut w, general);
    let events = step(&mut w, 1);
    assert!(events.iter().any(
        |e| matches!(e, BattleEvent::GeneralDied { side: 0, soldier } if *soldier == general)
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, BattleEvent::SoldierDied { id, .. } if *id == general))
    );
    let sides = &w.ecs().resource::<Sides>().0;
    assert!(sides[0].general_dead);
    assert_eq!(
        sides[0].general,
        Some(general),
        "the id is kept for the fate"
    );
    let (m0, m1, m2) = (morale(&w, 0).m, morale(&w, 1).m, morale(&w, 2).m);
    step(&mut w, 1);
    // The gate recomputes the aura at Stage 9 of the tick after the death.
    assert!(!in_aura(&w, 0) && !in_aura(&w, 1), "aura gone");
    assert!(in_aura(&w, 2), "the other side keeps its own");
    // Recovery adds 0.15 per tick; the shock is 20 (10 for the Shaken one).
    assert!(
        m0 - morale(&w, 0).m > S::from_i32(19),
        "{m0:?} -> {:?}",
        morale(&w, 0).m
    );
    assert!(m1 - morale(&w, 1).m > S::from_i32(9) && m1 - morale(&w, 1).m < S::from_i32(11));
    assert!(morale(&w, 2).m >= m2, "the enemy is untouched");
    assert_eq!(w.general_fate(0, false), GeneralFate::Dead);
    assert_eq!(w.general_fate(1, true), GeneralFate::Alive);
}

/// SIM-GEN-004: Alive, Wounded (below 0.3 of the tripled hp), Captured
/// (losing side, bodyguard shattered).
#[test]
fn general_fate_table() {
    let mut w = common::world(20);
    assert_eq!(w.general_fate(0, false), GeneralFate::Alive);
    let general = general_of(&w, 0);
    let e = w.ecs().resource::<Ids>().soldier_entity(general).unwrap();
    let full = w.ecs().get::<Health>(e).unwrap().hp;
    assert_eq!(full, S::from_i32(360));
    w.ecs_mut().get_mut::<Health>(e).unwrap().hp = S::from_i32(107);
    w.recompute_hash();
    assert_eq!(w.general_fate(0, false), GeneralFate::Wounded);
    assert_eq!(w.general_fate(0, true), GeneralFate::Wounded);
    let re = regiment_entity(&w, 0);
    w.ecs_mut().get_mut::<Morale>(re).unwrap().state = MoraleState::Shattered;
    w.recompute_hash();
    assert_eq!(w.general_fate(0, true), GeneralFate::Captured);
    assert_eq!(
        w.general_fate(0, false),
        GeneralFate::Wounded,
        "only a losing side is captured"
    );
}

/// Determinism with generals in the fight: 1 vs 8 threads, a restore with
/// the general alive, and a restore after its death.
#[test]
fn generals_are_deterministic_across_threads_and_restore() {
    let mut setup = common::two_sides(120);
    setup.sides[1].regiments[0].position = Some([400.0, 150.0]);
    let commands = [
        Command {
            tick: Tick(1),
            player: PlayerId(0),
            seq: 0,
            kind: CommandKind::AttackRegiment {
                regiments: vec![RegimentId(0)],
                target: RegimentId(1),
            },
        },
        Command {
            tick: Tick(1),
            player: PlayerId(1),
            seq: 0,
            kind: CommandKind::AttackRegiment {
                regiments: vec![RegimentId(1)],
                target: RegimentId(0),
            },
        },
    ];
    let run = |w: &mut BattleWorld, until: u32| {
        let mut hashes = Vec::new();
        while w.tick().0 < until {
            let next = w.tick().next();
            let batch: Vec<Command> = commands
                .iter()
                .filter(|c| c.tick == next)
                .cloned()
                .collect();
            let out = w.step(&batch);
            assert!(out.rejected.is_empty());
            hashes.push(out.hash);
        }
        hashes
    };
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    let mut hashes = run(&mut w, 500);
    let snap_alive = w.snapshot();
    hashes.extend(run(&mut w, 800));
    let general = general_of(&w, 1);
    kill(&mut w, general);
    hashes.push(w.hash());
    hashes.extend(run(&mut w, 1_200));
    let snap_dead = w.snapshot();
    hashes.extend(run(&mut w, 2_000));

    let mut w8 = BattleWorld::new(&setup, common::regs()).unwrap();
    w8.set_threads(8);
    let mut hashes8 = run(&mut w8, 800);
    kill(&mut w8, general);
    hashes8.push(w8.hash());
    hashes8.extend(run(&mut w8, 2_000));
    if let Some(t) = hashes.iter().zip(&hashes8).position(|(a, b)| a != b) {
        panic!("1 vs 8 threads diverged at entry {t}");
    }

    let mut r1 = BattleWorld::restore(&snap_alive, common::regs()).unwrap();
    assert_eq!(r1.hash(), hashes[499]);
    let tail = run(&mut r1, 800);
    assert_eq!(tail, hashes[500..800], "restore with the general alive");
    let mut r2 = BattleWorld::restore(&snap_dead, common::regs()).unwrap();
    assert!(r2.ecs().resource::<Sides>().0[1].general_dead);
    let tail = run(&mut r2, 2_000);
    assert_eq!(tail, hashes[1_201..], "restore after the general's death");
    let _ = V2::ZERO;
}
