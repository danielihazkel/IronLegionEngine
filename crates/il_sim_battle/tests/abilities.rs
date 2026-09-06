//! T2-050: abilities (SIM-ABIL-001..007). Testudo blunts a javelin volley
//! and expires to the tick; cooldowns, ownership, targeting and the
//! engagement condition reject as SIM-ABIL-003 says; war cry drains an
//! enemy's morale; the multipliers combine per SIM-ABIL-005; everything is
//! identical at 1 and 8 threads and across a mid-status restore.

mod common;

use common::cid;
use il_core::{PlayerId, RegimentId, S, Scalar, StateHash, Tick};
use il_sim_battle::combat::{StatMults, status_mults};
use il_sim_battle::components::{StatusEffect, Statuses};
use il_sim_battle::resources::Ids;
use il_sim_battle::{
    AbilityFail, AbilityTarget, BattleEvent, BattleSetup, BattleWorld, Command, CommandKind,
    FireMode, RegimentSetup, RejectReason,
};

fn at(id: u32, unit: &str, count: u16, formation: &str, x: f32, y: f32, deg: f32) -> RegimentSetup {
    RegimentSetup {
        id,
        unit_type: cid(unit),
        count,
        experience: 0,
        fatigue: 0.0,
        formation: Some(cid(formation)),
        position: Some([x, y]),
        facing_deg: Some(deg),
    }
}

fn two_sides(side0: Vec<RegimentSetup>, side1: Vec<RegimentSetup>) -> BattleSetup {
    BattleSetup {
        map_id: cid("rome:test_field"),
        seed: 42,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: vec![common::side(0, side0), common::side(1, side1)],
        victory: Default::default(),
    }
}

/// Row 5 of §15.3: 120 velites (line) at 300 m throw at 120 hastati
/// (line) at 335 m; nobody moves. Regiment ids: velites 0, hastati 1.
fn volley_setup() -> BattleSetup {
    two_sides(
        vec![at(1, "rome:velites", 120, "rome:line", 300.0, 150.0, 0.0)],
        vec![at(2, "rome:hastati", 120, "rome:line", 335.0, 150.0, 180.0)],
    )
}

fn command(tick: u32, player: u8, kind: CommandKind) -> Command {
    Command {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        kind,
    }
}

fn use_ability(
    tick: u32,
    player: u8,
    regiment: u32,
    ability: &str,
    target: AbilityTarget,
) -> Command {
    command(
        tick,
        player,
        CommandKind::UseAbility {
            regiment: RegimentId(regiment),
            ability: cid(ability),
            target,
        },
    )
}

/// Steps to `until`, feeding the commands stamped for each tick, pinning
/// morale, collecting events and rejections.
fn run(
    w: &mut BattleWorld,
    commands: &[Command],
    until: u32,
    events: &mut Vec<(u32, BattleEvent)>,
    rejected: &mut Vec<(u32, RejectReason)>,
) -> Vec<StateHash> {
    let mut hashes = Vec::new();
    while w.tick().0 < until {
        let next = w.tick().next();
        let due: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let out = w.step(&due);
        common::pin_morale(w);
        hashes.push(w.hash());
        events.extend(out.events.into_iter().map(|e| (next.0, e)));
        rejected.extend(out.rejected.into_iter().map(|(_, r)| (next.0, r)));
    }
    hashes
}

fn soldiers(w: &BattleWorld, regiment: u32) -> u32 {
    w.view()
        .regiment(RegimentId(regiment))
        .map_or(0, |r| r.soldier_count)
}

fn statuses(w: &BattleWorld, regiment: u32) -> Statuses {
    let e = w
        .ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(regiment))
        .unwrap();
    w.ecs().get::<Statuses>(e).unwrap().clone()
}

#[test]
fn testudo_reduces_javelin_casualties() {
    let setup = volley_setup();
    let regs = common::regs();
    let mut bare = BattleWorld::new(&setup, regs.clone()).unwrap();
    let mut shielded = BattleWorld::new(&setup, regs).unwrap();
    let (mut ev, mut rej) = (Vec::new(), Vec::new());
    run(&mut bare, &[], 1_200, &mut ev, &mut rej);
    let testudo = [use_ability(
        1,
        1,
        1,
        "rome:testudo",
        AbilityTarget::SelfTarget,
    )];
    run(&mut shielded, &testudo, 1_200, &mut ev, &mut rej);
    assert!(rej.is_empty(), "{rej:?}");
    assert!(
        ev.iter()
            .any(|(t, e)| *t == 1 && matches!(e, BattleEvent::AbilityUsed { regiment, targets: 1, .. } if *regiment == RegimentId(1)))
    );
    let lost_bare = 120 - soldiers(&bare, 1);
    let lost_shielded = 120 - soldiers(&shielded, 1);
    assert!(lost_bare >= 10, "the bare hastati lost only {lost_bare}");
    assert!(
        f64::from(lost_shielded) <= 0.6 * f64::from(lost_bare),
        "testudo: {lost_shielded} lost against {lost_bare} without"
    );
}

#[test]
fn testudo_expires_on_time_and_restores_the_multipliers() {
    let regs = common::regs();
    let mut w = BattleWorld::new(&volley_setup(), regs.clone()).unwrap();
    // Nobody throws, so only the status clock runs.
    let hold = |player, regiment| {
        command(
            1,
            player,
            CommandKind::FireMode {
                regiments: vec![RegimentId(regiment)],
                mode: FireMode::Hold,
            },
        )
    };
    let commands = [
        hold(0, 0),
        hold(1, 1),
        use_ability(1, 1, 1, "rome:testudo", AbilityTarget::SelfTarget),
    ];
    let (mut ev, mut rej) = (Vec::new(), Vec::new());
    run(&mut w, &commands, 1, &mut ev, &mut rej);
    assert!(rej.is_empty(), "{rej:?}");
    let s = statuses(&w, 1);
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].remaining, 399, "one tick of the 400 has run");
    assert!(!s.list[0].hostile);
    assert_eq!(s.mults.armour_mult, S::from_f32_data(3.5));
    assert_eq!(s.mults.speed, S::HALF);
    assert_eq!(s.mults.armour(S::from_i32(8)), S::from_i32(28));
    run(&mut w, &commands, 399, &mut ev, &mut rej);
    assert_eq!(statuses(&w, 1).list.len(), 1, "still up after tick 399");
    run(&mut w, &commands, 400, &mut ev, &mut rej);
    let s = statuses(&w, 1);
    assert!(s.list.is_empty(), "gone after tick 400");
    assert_eq!(s.mults, StatMults::default());
    assert!(ev.iter().any(|(t, e)| *t == 400
        && matches!(e, BattleEvent::StatusExpired { regiment, ability } if *regiment == RegimentId(1) && ability == &cid("rome:testudo"))));
    // Cooldown: refused at tick 2, accepted again once 1,200 ticks passed.
    let again = [
        use_ability(2, 1, 1, "rome:testudo", AbilityTarget::SelfTarget),
        use_ability(1_200, 1, 1, "rome:testudo", AbilityTarget::SelfTarget),
        use_ability(1_201, 1, 1, "rome:testudo", AbilityTarget::SelfTarget),
    ];
    let mut w = BattleWorld::new(&volley_setup(), regs).unwrap();
    let all: Vec<Command> = commands.iter().chain(&again).cloned().collect();
    let (mut ev, mut rej) = (Vec::new(), Vec::new());
    run(&mut w, &all, 1_201, &mut ev, &mut rej);
    let cooldown = |t| {
        RejectReason::Ability {
            regiment: RegimentId(1),
            ability: cid("rome:testudo"),
            why: AbilityFail::OnCooldown,
        } == rej.iter().find(|(tick, _)| *tick == t).unwrap().1
    };
    assert!(cooldown(2) && cooldown(1_200), "{rej:?}");
    assert_eq!(rej.len(), 2, "{rej:?}");
    assert_eq!(
        statuses(&w, 1).list.len(),
        1,
        "the tick 1,201 use went through"
    );
}

#[test]
fn ownership_target_kind_range_and_engagement_are_checked() {
    // Hoplites do not know testudo; a bad id is unknown content; a hastati
    // regiment cannot testudo an ally; war cry needs an enemy within 60 m.
    let setup = two_sides(
        vec![
            at(1, "rome:hastati", 60, "rome:line", 300.0, 150.0, 0.0),
            at(2, "persia:cavalry", 30, "rome:wedge", 300.0, 120.0, 0.0),
        ],
        vec![
            at(3, "greece:hoplite", 60, "rome:line", 420.0, 150.0, 180.0),
            at(4, "rome:hastati", 60, "rome:line", 340.0, 150.0, 180.0),
        ],
    );
    let regs = common::regs();
    let mut w = BattleWorld::new(&setup, regs.clone()).unwrap();
    let commands = [
        use_ability(1, 1, 2, "rome:testudo", AbilityTarget::SelfTarget),
        use_ability(2, 1, 2, "rome:nope", AbilityTarget::SelfTarget),
        use_ability(
            3,
            0,
            0,
            "rome:testudo",
            AbilityTarget::Regiment(RegimentId(1)),
        ),
        use_ability(
            4,
            0,
            1,
            "persia:war_cry",
            AbilityTarget::Regiment(RegimentId(2)),
        ),
        use_ability(
            5,
            0,
            1,
            "persia:war_cry",
            AbilityTarget::Regiment(RegimentId(0)),
        ),
        use_ability(
            6,
            0,
            1,
            "persia:war_cry",
            AbilityTarget::Regiment(RegimentId(3)),
        ),
    ];
    let (mut ev, mut rej) = (Vec::new(), Vec::new());
    run(&mut w, &commands, 6, &mut ev, &mut rej);
    let why = |t: u32| match &rej.iter().find(|(tick, _)| *tick == t).unwrap().1 {
        RejectReason::Ability { why, .. } => Some(*why),
        RejectReason::UnknownContent(_) => None,
        other => panic!("{other:?}"),
    };
    assert_eq!(why(1), Some(AbilityFail::NotOwned));
    assert_eq!(why(2), None, "unknown content");
    assert_eq!(why(3), Some(AbilityFail::BadTarget), "wrong target kind");
    assert_eq!(why(4), Some(AbilityFail::OutOfRange), "hoplites at 120 m");
    assert_eq!(why(5), Some(AbilityFail::BadTarget), "own side");
    assert_eq!(rej.len(), 5, "{rej:?}");
    let s = statuses(&w, 3);
    assert_eq!(
        s.list.len(),
        1,
        "the tick 6 war cry on the near hastati landed"
    );
    assert!(s.list[0].hostile);
    assert_eq!(s.mults.morale_per_s, S::from_i32(-2));
    assert_eq!(s.mults.attack, S::from_f32_data(0.9));

    // Engagement: hastati locked in melee cannot form testudo.
    let mut w = BattleWorld::new(&setup, regs).unwrap();
    let commands = [
        command(
            1,
            0,
            CommandKind::FireMode {
                regiments: vec![RegimentId(0)],
                mode: FireMode::Hold,
            },
        ),
        command(
            1,
            0,
            CommandKind::AttackRegiment {
                regiments: vec![RegimentId(0)],
                target: RegimentId(3),
            },
        ),
        use_ability(400, 0, 0, "rome:testudo", AbilityTarget::SelfTarget),
    ];
    let (mut ev, mut rej) = (Vec::new(), Vec::new());
    run(&mut w, &commands, 400, &mut ev, &mut rej);
    assert!(
        w.view().regiment(RegimentId(0)).unwrap().engaged,
        "the hastati never closed"
    );
    assert_eq!(
        rej.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>(),
        vec![RejectReason::Ability {
            regiment: RegimentId(0),
            ability: cid("rome:testudo"),
            why: AbilityFail::Engaged,
        }]
    );
}

#[test]
fn war_cry_drains_the_targets_morale() {
    // Cavalry 30 m from hastati that stand and do nothing; no fire.
    let setup = two_sides(
        vec![at(1, "persia:cavalry", 30, "rome:wedge", 300.0, 150.0, 0.0)],
        vec![at(2, "rome:hastati", 120, "rome:line", 330.0, 150.0, 180.0)],
    );
    let regs = common::regs();
    let hold = command(
        1,
        1,
        CommandKind::FireMode {
            regiments: vec![RegimentId(1)],
            mode: FireMode::Hold,
        },
    );
    let morale_at = |commands: &[Command], until: u32| {
        let mut w = BattleWorld::new(&setup, regs.clone()).unwrap();
        while w.tick().0 < until {
            let next = w.tick().next();
            let due: Vec<Command> = commands
                .iter()
                .filter(|c| c.tick == next)
                .cloned()
                .collect();
            let out = w.step(&due);
            assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        }
        w.view().regiment(RegimentId(1)).unwrap().morale
    };
    let control = morale_at(std::slice::from_ref(&hold), 300);
    let cried = morale_at(
        &[
            hold.clone(),
            use_ability(
                1,
                0,
                0,
                "persia:war_cry",
                AbilityTarget::Regiment(RegimentId(1)),
            ),
        ],
        300,
    );
    // Two morale a second for fifteen seconds, within the clamp.
    assert!(
        control - cried >= S::from_i32(20),
        "control {control:?}, cried {cried:?}"
    );
}

#[test]
fn multipliers_combine_per_status_and_side() {
    let regs = common::regs();
    let wall = regs.abilities.lookup(&cid("greece:shield_wall")).unwrap();
    let cry = regs.abilities.lookup(&cid("persia:war_cry")).unwrap();
    let status = |source, stacks, hostile| StatusEffect {
        source,
        remaining: 10,
        stacks,
        hostile,
    };
    let m = status_mults(&[status(wall, 1, false)], &regs);
    assert_eq!(m.defence, S::from_f32_data(1.25));
    assert_eq!(m.armour_add, S::from_i32(2));
    assert_eq!(m.armour_mult, S::ONE);
    assert_eq!(m.speed, S::from_f32_data(0.7));
    assert_eq!(m.attack_interval, S::from_f32_data(1.2));
    assert_eq!(m.attack, S::ONE);
    // Both at once: multiplicative parts multiply.
    let both = status_mults(&[status(wall, 1, false), status(cry, 1, true)], &regs);
    assert_eq!(both.attack, S::from_f32_data(0.9));
    assert_eq!(both.morale_per_s, S::from_i32(-2));
    assert_eq!(both.defence, S::from_f32_data(1.25));
    // A hostile shield wall would grant its buffs to nobody: debuffs only.
    let hostile = status_mults(&[status(wall, 1, true)], &regs);
    assert_eq!(hostile.defence, S::ONE);
    assert_eq!(hostile.armour_add, S::ZERO);
    assert_eq!(hostile.speed, S::from_f32_data(0.7));
    // Stacks scale the additive parts only.
    let stacked = status_mults(&[status(cry, 3, true)], &regs);
    assert_eq!(stacked.morale_per_s, S::from_i32(-6));
    assert_eq!(stacked.attack, S::from_f32_data(0.9));
    assert_eq!(status_mults(&[], &regs), StatMults::default());
}

#[test]
fn abilities_are_deterministic_across_threads_and_a_mid_status_restore() {
    let setup = volley_setup();
    let regs = common::regs();
    let commands = [use_ability(
        1,
        1,
        1,
        "rome:testudo",
        AbilityTarget::SelfTarget,
    )];
    let (mut ev, mut rej) = (Vec::new(), Vec::new());
    let mut w = BattleWorld::new(&setup, regs.clone()).unwrap();
    let mut hashes = run(&mut w, &commands, 200, &mut ev, &mut rej);
    let snap = w.snapshot();
    assert_eq!(
        snap.regiments[1].statuses.len(),
        1,
        "snapshot carries the status"
    );
    assert_eq!(snap.regiments[1].cooldowns, vec![1_000]);
    hashes.extend(run(&mut w, &commands, 1_500, &mut ev, &mut rej));
    assert!(rej.is_empty(), "{rej:?}");

    let mut w8 = BattleWorld::new(&setup, regs.clone()).unwrap();
    w8.set_threads(8);
    let hashes8 = run(&mut w8, &commands, 1_500, &mut ev, &mut rej);
    if let Some(i) = hashes.iter().zip(&hashes8).position(|(a, b)| a != b) {
        panic!("1 vs 8 threads diverge at tick {}", i + 1);
    }
    let mut restored = BattleWorld::restore(&snap, regs).unwrap();
    assert_eq!(restored.hash(), hashes[199], "restore reproduces tick 200");
    assert_eq!(
        statuses(&restored, 1).mults.armour_mult,
        S::from_f32_data(3.5)
    );
    let tail = run(&mut restored, &commands, 1_500, &mut ev, &mut rej);
    if let Some(i) = hashes[200..].iter().zip(&tail).position(|(a, b)| a != b) {
        panic!("restored run diverges at tick {}", 201 + i);
    }
}
