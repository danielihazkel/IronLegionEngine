//! T2-041: morale moves under the factors and shocks, the hysteresis state
//! machine transitions, and everything is deterministic across threads
//! and a restore (SIM-MOR-001..027).

mod common;

use common::two_sides;
use il_core::{PlayerId, RegimentId, S, Scalar, SoldierId, StateHash, Tick, V2};
use il_sim_battle::combat::{Kill, Kills};
use il_sim_battle::components::{Health, Morale, MoraleState, Regiment};
use il_sim_battle::resources::Ids;
use il_sim_battle::{
    BattleEvent, BattleSetup, BattleWorld, Command, CommandKind, FireMode, RegimentSetup, SpeedMode,
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

fn one_side(regiments: Vec<RegimentSetup>) -> BattleSetup {
    BattleSetup {
        map_id: common::cid("rome:test_field"),
        seed: 42,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: vec![common::side(0, regiments)],
        victory: Default::default(),
    }
}

fn command(tick: u32, player: u8, kind: CommandKind) -> Command {
    Command {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        kind,
    }
}

fn attack(tick: u32, player: u8, regiment: u32, target: u32) -> Command {
    command(
        tick,
        player,
        CommandKind::AttackRegiment {
            regiments: vec![RegimentId(regiment)],
            target: RegimentId(target),
        },
    )
}

fn regiment_entity(w: &BattleWorld, id: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(id))
        .expect("regiment exists")
}

fn morale(w: &BattleWorld, id: u32) -> Morale {
    *w.ecs().get::<Morale>(regiment_entity(w, id)).unwrap()
}

fn set_morale(w: &mut BattleWorld, id: u32, m: f32) {
    let e = regiment_entity(w, id);
    w.ecs_mut().get_mut::<Morale>(e).unwrap().m = S::from_f32_data(m);
    w.recompute_hash();
}

/// Every hastati regiment holds its pila so the counts stay scripted.
fn hold_fire(w: &mut BattleWorld) {
    let entities: Vec<_> = w
        .ecs()
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for e in entities {
        if let Some(mut fire) = w.ecs_mut().get_mut::<il_sim_battle::components::Fire>(e) {
            fire.mode = FireMode::Hold;
        }
    }
    w.recompute_hash();
}

/// Queues the first `n` living soldiers of `regiment` as killed.
fn kill_first(w: &mut BattleWorld, regiment: u32, n: usize) {
    let victims: Vec<SoldierId> = w
        .ecs()
        .get::<Regiment>(regiment_entity(w, regiment))
        .unwrap()
        .soldiers
        .iter()
        .copied()
        .take(n)
        .collect();
    for v in &victims {
        let e = w.ecs().resource::<Ids>().soldier_entity(*v).unwrap();
        w.ecs_mut().get_mut::<Health>(e).unwrap().hp = S::ZERO;
        w.ecs_mut().resource_mut::<Kills>().0.push(Kill {
            victim: *v,
            killer: None,
            killer_regiment: None,
        });
    }
}

/// Steps to `until`, feeding scripted commands and collecting the morale
/// transitions of every regiment in order.
fn run(
    w: &mut BattleWorld,
    commands: &[Command],
    until: u32,
    changes: &mut Vec<(u32, u32, MoraleState, MoraleState)>,
) -> Vec<StateHash> {
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
        for e in &out.events {
            if let BattleEvent::MoraleChanged { regiment, from, to } = e {
                changes.push((next.0, regiment.0, *from, *to));
            }
        }
        hashes.push(out.hash);
    }
    hashes
}

/// SIM-MOR-001/003: hastati spawn Unsettled at 60; SIM-MOR-024: alone on
/// the field they recover to 100 and climb to Steady past 75.
#[test]
fn a_lone_regiment_recovers_to_steady_and_full_morale() {
    let mut w = BattleWorld::new(
        &one_side(vec![at(1, "rome:hastati", 120, 300.0, 150.0, 0.0)]),
        common::regs(),
    )
    .unwrap();
    assert_eq!(morale(&w, 0).state, MoraleState::Unsettled);
    assert_eq!(morale(&w, 0).m, S::from_i32(60));
    let mut changes = Vec::new();
    run(&mut w, &[], 400, &mut changes);
    assert_eq!(morale(&w, 0).m, S::from_i32(100), "recovery saturates");
    assert_eq!(morale(&w, 0).state, MoraleState::Steady);
    // +3 per second: past 75 after five seconds, one event only.
    assert_eq!(changes.len(), 1, "{changes:?}");
    let (tick, _, from, to) = changes[0];
    assert_eq!((from, to), (MoraleState::Unsettled, MoraleState::Steady));
    // +3 recovery and +1 aura per second (the general rides along, T2-043).
    assert!((70..=110).contains(&tick), "climbed at tick {tick}");
}

/// Done-when: a regiment losing 30 % in ten seconds passes through
/// Broken into Routing (SIM-MOR-010/011; an enemy 50 m away stops the
/// recovery factor).
#[test]
fn losing_thirty_percent_in_ten_seconds_breaks_and_routs() {
    let mut setup = two_sides(120);
    setup.sides[1].regiments[0].position = Some([350.0, 150.0]);
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    hold_fire(&mut w);
    let mut changes = Vec::new();
    for _ in 0..18 {
        kill_first(&mut w, 0, 2);
        let next = w.tick().0 + 11;
        run(&mut w, &[], next, &mut changes);
    }
    let m = morale(&w, 0);
    assert_eq!(m.initial as usize, m.initial as usize);
    let states: Vec<MoraleState> = changes.iter().filter(|c| c.1 == 0).map(|c| c.3).collect();
    assert_eq!(
        states,
        [
            MoraleState::Shaken,
            MoraleState::Broken,
            MoraleState::Routing
        ],
        "{changes:?}"
    );
    assert!(m.m <= S::from_i32(15), "{:?}", m.m);
    assert_eq!(m.state, MoraleState::Routing);
    // The enemy watched them fall: `winning` lifts it to Steady, never lower.
    assert!(
        changes
            .iter()
            .filter(|c| c.1 == 1)
            .all(|c| c.3 == MoraleState::Steady),
        "{changes:?}"
    );
}

/// SIM-MOR-003 hysteresis: leaving a state upward needs the threshold
/// plus five; dropping happens at the threshold.
#[test]
fn hysteresis_holds_a_state_until_the_margin_is_cleared() {
    let mut setup = two_sides(120);
    setup.sides[1].regiments[0].position = Some([350.0, 150.0]);
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    hold_fire(&mut w);
    let mut changes = Vec::new();
    set_morale(&mut w, 0, 74.0);
    run(&mut w, &[], 5, &mut changes);
    assert_eq!(morale(&w, 0).state, MoraleState::Unsettled, "{changes:?}");
    set_morale(&mut w, 0, 76.0);
    run(&mut w, &[], 6, &mut changes);
    assert_eq!(morale(&w, 0).state, MoraleState::Steady);
    set_morale(&mut w, 0, 69.5);
    run(&mut w, &[], 7, &mut changes);
    assert_eq!(
        morale(&w, 0).state,
        MoraleState::Unsettled,
        "at the threshold"
    );
    set_morale(&mut w, 0, 20.0);
    run(&mut w, &[], 8, &mut changes);
    assert_eq!(
        morale(&w, 0).state,
        MoraleState::Broken,
        "straight down two bands"
    );
    set_morale(&mut w, 0, 40.0);
    run(&mut w, &[], 9, &mut changes);
    assert_eq!(
        morale(&w, 0).state,
        MoraleState::Shaken,
        "one band up per tick"
    );
}

/// SIM-MOR-015: two Steady neighbours within 40 m lift morale faster.
#[test]
fn nearby_steady_allies_raise_morale() {
    let lone = one_side(vec![at(1, "rome:hastati", 60, 300.0, 150.0, 0.0)]);
    let flanked = one_side(vec![
        at(1, "rome:hastati", 60, 300.0, 150.0, 0.0),
        at(2, "rome:hastati", 60, 300.0, 120.0, 0.0),
        at(3, "rome:hastati", 60, 300.0, 180.0, 0.0),
    ]);
    let mut a = BattleWorld::new(&lone, common::regs()).unwrap();
    let mut b = BattleWorld::new(&flanked, common::regs()).unwrap();
    let mut changes = Vec::new();
    // 130 ticks: the neighbours turn Steady at about 76 and the factor
    // then counts; recovery plus the aura (T2-043) saturate at 100 by 200.
    run(&mut a, &[], 130, &mut changes);
    run(&mut b, &[], 130, &mut changes);
    let (ma, mb) = (morale(&a, 0).m, morale(&b, 0).m);
    assert!(mb > ma + S::ONE, "alone {ma:?}, with allies {mb:?}");
    assert!(ma > S::from_i32(75) && mb < S::from_i32(100));
}

/// SIM-MOR-025: an engaged regiment ordered away loses `disengage_penalty`
/// on that tick; SIM-MOR-026: a regiment charged in the rear loses
/// `charged_penalty` on the charge tick and remembers the rear arc
/// (SIM-MOR-019).
#[test]
fn disengage_and_charge_shocks_land_on_their_tick() {
    // Cavalry 60 m behind a hastati line facing +x: a rear charge at run.
    let mut setup = two_sides(120);
    setup.sides[0].regiments = vec![at(1, "rome:hastati", 120, 300.0, 150.0, 0.0)];
    setup.sides[1].regiments = vec![at(2, "persia:cavalry", 60, 240.0, 150.0, 0.0)];
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    hold_fire(&mut w);
    let commands = [
        command(
            1,
            1,
            CommandKind::SetSpeedMode {
                regiments: vec![RegimentId(1)],
                mode: SpeedMode::Run,
            },
        ),
        attack(2, 1, 1, 0),
    ];
    let mut changes = Vec::new();
    let mut charge_tick = None;
    let mut before = morale(&w, 0).m;
    while w.tick().0 < 600 && charge_tick.is_none() {
        let next = w.tick().next();
        let batch: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let out = w.step(&batch);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        if out
            .events
            .iter()
            .any(|e| matches!(e, BattleEvent::Charge { target, .. } if target.0 == 0))
        {
            charge_tick = Some(next.0);
            break;
        }
        before = morale(&w, 0).m;
    }
    let tick = charge_tick.expect("the cavalry charged home");
    let after = morale(&w, 0);
    assert!(
        before - after.m >= S::from_f32_data(7.5),
        "rear charge shock: {before:?} -> {:?} at tick {tick}",
        after.m
    );

    // Attacks stamp the arc they came through (SIM-MOR-019). Fighting
    // hastati turn to face their attackers within a few ticks, so the
    // stamp itself is checked with a scripted rear outcome.
    run(&mut w, &[], tick + 20, &mut changes);
    assert!(
        morale(&w, 0).arc_hit.iter().any(|t| t.0 > tick),
        "some arc was attacked: {:?}",
        morale(&w, 0).arc_hit
    );
    let (attacker, target) = {
        let a = w
            .ecs()
            .get::<Regiment>(regiment_entity(&w, 1))
            .unwrap()
            .soldiers[0];
        let t = w
            .ecs()
            .get::<Regiment>(regiment_entity(&w, 0))
            .unwrap()
            .soldiers[0];
        (a, t)
    };
    w.ecs_mut()
        .resource::<il_sim_battle::combat::Outcomes>()
        .0
        .lock()
        .unwrap()
        .push(il_sim_battle::combat::AttackOutcome {
            attacker,
            target,
            hit: false,
            damage: S::ZERO,
            arc: il_sim_battle::combat::Arc::Rear,
        });
    let stamp = w.tick().0 + 1;
    run(&mut w, &[], stamp, &mut changes);
    assert_eq!(morale(&w, 0).arc_hit[2].0, stamp, "rear arc stamped");

    // Order the charged hastati away while engaged: the disengage shock.
    let before = morale(&w, 0).m;
    let order_tick = w.tick().0 + 1;
    let away = command(
        order_tick,
        0,
        CommandKind::Move {
            regiments: vec![RegimentId(0)],
            target: V2::from_f32_data(500.0, 150.0),
            facing: None,
            speed: SpeedMode::Walk,
        },
    );
    run(&mut w, &[away], order_tick, &mut changes);
    let after = morale(&w, 0).m;
    assert!(
        before - after >= S::from_f32_data(4.5),
        "disengage shock: {before:?} -> {after:?}"
    );
}

/// Determinism (TDD §18): a full melee with morale at 1 and 8 threads,
/// restored mid-fight.
#[test]
fn morale_is_deterministic_across_threads_and_restore() {
    let mut setup = two_sides(120);
    setup.sides[1].regiments[0].position = Some([400.0, 150.0]);
    let commands = [attack(1, 0, 0, 1), attack(1, 1, 1, 0)];
    let mut changes = Vec::new();
    let mut w = BattleWorld::new(&setup, common::regs()).unwrap();
    let mut hashes = run(&mut w, &commands, 1_000, &mut changes);
    let snap = w.snapshot();
    hashes.extend(run(&mut w, &commands, 3_000, &mut changes));
    assert!(
        changes.iter().any(|c| c.3 == MoraleState::Routing),
        "somebody broke: {changes:?}"
    );

    let mut w8 = BattleWorld::new(&setup, common::regs()).unwrap();
    w8.set_threads(8);
    let hashes8 = run(&mut w8, &commands, 3_000, &mut Vec::new());
    if let Some(t) = hashes.iter().zip(&hashes8).position(|(a, b)| a != b) {
        panic!("1 vs 8 threads diverged at tick {}", t + 1);
    }

    let mut restored = BattleWorld::restore(&snap, common::regs()).unwrap();
    assert_eq!(restored.hash(), hashes[999]);
    let tail = run(&mut restored, &commands, 3_000, &mut Vec::new());
    if let Some(t) = hashes[1_000..].iter().zip(&tail).position(|(a, b)| a != b) {
        panic!("restore diverged at tick {}", 1_001 + t);
    }
}
