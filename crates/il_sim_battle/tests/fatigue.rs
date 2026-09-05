//! T2-040: fatigue accumulates by activity and terrain, recovers when
//! idle, slows soldiers and anchors, is averaged per regiment every ten
//! ticks, and is deterministic across threads and a restore (SIM-FAT-001..005).

mod common;

use il_core::{PlayerId, RegimentId, S, Scalar, StateHash, Tick, V2};
use il_sim_battle::components::{FatigueC, Regiment, RegimentFatigue};
use il_sim_battle::morale::FATIGUE_MEAN_PERIOD;
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleSetup, BattleWorld, Command, CommandKind, SpeedMode};

fn one_side(count: u16, fatigue: f32) -> BattleSetup {
    let mut r = common::regiment(1, "rome:hastati", count, 100.0, 0.0);
    r.fatigue = fatigue;
    BattleSetup {
        map_id: common::cid("rome:test_field"),
        seed: 42,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: vec![common::side(0, vec![r])],
        victory: Default::default(),
    }
}

fn move_cmd(tick: u32, player: u8, regiment: u32, x: f32, y: f32, speed: SpeedMode) -> Command {
    Command {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        kind: CommandKind::Move {
            regiments: vec![RegimentId(regiment)],
            target: V2::from_f32_data(x, y),
            facing: None,
            speed,
        },
    }
}

/// Steps to `until`, feeding the commands stamped for each tick.
fn run(w: &mut BattleWorld, commands: &[Command], until: u32) -> Vec<StateHash> {
    let mut hashes = Vec::new();
    while w.tick().0 < until {
        let next = w.tick().next();
        let batch: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let out = w.step(&batch);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        hashes.push(out.hash);
    }
    hashes
}

fn fatigues(w: &BattleWorld, regiment: u32) -> Vec<S> {
    let ids = w.ecs().resource::<Ids>();
    let e = ids.regiment_entity(RegimentId(regiment)).unwrap();
    let soldiers = w.ecs().get::<Regiment>(e).unwrap().soldiers.clone();
    soldiers
        .iter()
        .map(|s| {
            w.ecs()
                .get::<FatigueC>(ids.soldier_entity(*s).unwrap())
                .unwrap()
                .f
        })
        .collect()
}

fn mean_of(w: &BattleWorld, regiment: u32) -> S {
    let e = w
        .ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(regiment))
        .unwrap();
    w.ecs().get::<RegimentFatigue>(e).unwrap().mean
}

fn anchor_x(w: &BattleWorld) -> S {
    w.view().regiment(RegimentId(0)).unwrap().anchor_pos.x
}

/// Done-when: a regiment that runs for three minutes reaches Exhausted
/// (SIM-FAT-002/003) and never recovers while it keeps running.
#[test]
fn running_three_minutes_reaches_exhausted() {
    let mut w = BattleWorld::new(&one_side(120, 0.0), common::regs()).unwrap();
    let commands = [
        move_cmd(1, 0, 0, 700.0, 150.0, SpeedMode::Run),
        move_cmd(1_800, 0, 0, 100.0, 150.0, SpeedMode::Run),
    ];
    run(&mut w, &commands, 1_500);
    // 0.0216 per second (run 0.020 plus armour 8 x 0.0002) saturates in 47 s.
    assert!(fatigues(&w, 0).iter().all(|f| *f == S::ONE), "saturated");
    run(&mut w, &commands, 3_600);
    let three_quarters = S::from_f32_data(0.75);
    assert!(
        fatigues(&w, 0).iter().all(|f| *f >= three_quarters),
        "exhausted after three minutes of running"
    );
    assert!(mean_of(&w, 0) >= three_quarters);
}

/// SIM-FAT-004 / SIM-MOVE-011: an exhausted regiment's anchor runs at
/// `1 - speed_loss` of a fresh one's over the same ground.
#[test]
fn exhausted_regiment_runs_at_the_speed_multiplier() {
    let mut tired = BattleWorld::new(&one_side(120, 1.0), common::regs()).unwrap();
    let mut fresh = BattleWorld::new(&one_side(120, 0.0), common::regs()).unwrap();
    let commands = [move_cmd(1, 0, 0, 700.0, 150.0, SpeedMode::Run)];
    for w in [&mut tired, &mut fresh] {
        run(w, &commands, 20);
    }
    let (t0, f0) = (anchor_x(&tired), anchor_x(&fresh));
    for w in [&mut tired, &mut fresh] {
        run(w, &commands, 120);
    }
    let tired_step = anchor_x(&tired) - t0;
    let fresh_step = anchor_x(&fresh) - f0;
    assert!(
        tired_step > S::ZERO && fresh_step > tired_step,
        "both ran east"
    );
    // The fresh regiment tires a little over the window (mean f ~ 0.076,
    // mult ~ 0.977), so the ratio sits just above 0.7.
    let ratio = (tired_step / fresh_step).to_f32_render();
    assert!((ratio - 0.716).abs() < 0.03, "ratio {ratio}");
    assert!(fatigues(&tired, 0).iter().all(|f| *f == S::ONE));
}

/// SIM-FAT-002: idle soldiers recover at `rate_idle`; from 1 that is 100 s.
#[test]
fn idle_regiment_recovers_to_zero_in_100_seconds() {
    let mut w = BattleWorld::new(&one_side(60, 1.0), common::regs()).unwrap();
    run(&mut w, &[], 1_990);
    assert!(
        fatigues(&w, 0).iter().all(|f| *f > S::ZERO),
        "still tired at 99.5 s"
    );
    run(&mut w, &[], 2_001);
    assert!(
        fatigues(&w, 0).iter().all(|f| *f == S::ZERO),
        "fresh at 100 s"
    );
    assert_eq!(mean_of(&w, 0), S::ZERO);
}

/// SIM-FAT-005: the regiment mean refreshes every ten ticks and holds
/// between refreshes.
#[test]
fn regiment_mean_refreshes_every_ten_ticks() {
    let mut w = BattleWorld::new(&one_side(60, 0.0), common::regs()).unwrap();
    let commands = [move_cmd(1, 0, 0, 700.0, 150.0, SpeedMode::Run)];
    let mut last = mean_of(&w, 0);
    for _ in 0..200 {
        let next = w.tick().0 + 1;
        run(&mut w, &commands, next);
        let mean = mean_of(&w, 0);
        if w.tick().0.is_multiple_of(FATIGUE_MEAN_PERIOD) {
            let fs = fatigues(&w, 0);
            let sum = fs.iter().fold(S::ZERO, |a, b| a + *b);
            assert_eq!(mean, sum / S::from_i32(fs.len() as i32), "refreshed");
        } else {
            assert_eq!(mean, last, "held between refreshes");
        }
        last = mean;
    }
    assert!(last > S::ZERO);
}

/// Determinism (TDD §18): a march, a charge and a melee at 1 and 8
/// threads, with a restore mid-fight.
#[test]
fn fatigue_is_deterministic_across_threads_and_restore() {
    let setup = common::two_sides(120);
    let commands = [Command {
        tick: Tick(1),
        player: PlayerId(0),
        seq: 0,
        kind: CommandKind::AttackRegiment {
            regiments: vec![RegimentId(0)],
            target: RegimentId(1),
        },
    }];
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    let mut hashes = run(&mut w, &commands, 1_000);
    let snap = w.snapshot();
    hashes.extend(run(&mut w, &commands, 2_000));
    assert!(
        fatigues(&w, 0).iter().any(|f| *f > S::HALF),
        "the charge tired them"
    );

    let mut w8 = BattleWorld::new(&setup, common::regs()).unwrap();
    w8.set_threads(8);
    let hashes8 = run(&mut w8, &commands, 2_000);
    if let Some(t) = hashes.iter().zip(&hashes8).position(|(a, b)| a != b) {
        panic!("1 vs 8 threads diverged at tick {}", t + 1);
    }

    let mut restored = BattleWorld::restore(&snap, common::regs()).unwrap();
    assert_eq!(restored.hash(), hashes[999]);
    let tail = run(&mut restored, &commands, 2_000);
    if let Some(t) = hashes[1_000..].iter().zip(&tail).position(|(a, b)| a != b) {
        panic!("restore diverged at tick {}", 1_001 + t);
    }
}
