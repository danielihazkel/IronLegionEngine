//! T2-030: ranged targeting, volleys, ammo, fire modes and the friendly
//! block (SIM-PROJ-001..004, SIM-PROJ-009). T2-031: landing, delayed
//! damage, friendly fire, kill credit (SIM-PROJ-005, SIM-PROJ-006).

mod common;

use common::cid;
use il_core::{PlayerId, RegimentId, S, Scalar, SoldierId, StateHash, Tick, V2};
use il_data::ProjectileArc;
use il_sim_battle::combat::{Kill, Kills};
use il_sim_battle::components::{Anchor, Combat, Fire, Health, Pos, RangedState, Regiment};
use il_sim_battle::resources::{Ids, Pending, PendingDamage, Projectile, Projectiles};
use il_sim_battle::{
    BattleEvent, BattleSetup, BattleWorld, Command, CommandKind, FireMode, RegimentSetup,
    RejectReason, Snapshot,
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

/// 120 velites at x = 300 facing +x, 120 hastati (line) at `hastati_x`
/// facing them; nobody moves. Regiment ids: velites 0, hastati 1. The
/// velites stand in `rome:line` (40 m wide, like the hastati) so every one
/// of them has a hastati within 40 m; the §15.3 band file uses `loose`,
/// whose 96 m width leaves the wings out of range.
fn volley_setup(hastati_x: f32) -> BattleSetup {
    two_sides(
        vec![at(1, "rome:velites", 120, "rome:line", 300.0, 150.0, 0.0)],
        vec![at(
            2,
            "rome:hastati",
            120,
            "rome:line",
            hastati_x,
            150.0,
            180.0,
        )],
    )
}

fn world(setup: &BattleSetup) -> BattleWorld {
    BattleWorld::new(setup, common::regs()).unwrap()
}

fn command(tick: u32, player: u8, kind: CommandKind) -> Command {
    Command {
        tick: Tick(tick),
        player: PlayerId(player),
        seq: 0,
        kind,
    }
}

fn fire_mode(tick: u32, player: u8, regiment: u32, mode: FireMode) -> Command {
    command(
        tick,
        player,
        CommandKind::FireMode {
            regiments: vec![RegimentId(regiment)],
            mode,
        },
    )
}

/// `(tick, regiment, count)` of every `VolleyFired`, plus every
/// `FireBlocked` as `(tick, regiment, blocker)`.
#[derive(Default)]
struct Log {
    volleys: Vec<(u32, u32, u16)>,
    blocked: Vec<(u32, u32, u32)>,
    hashes: Vec<StateHash>,
    max_live: usize,
    /// `ProjectileLanded` events: total and hits (T2-031).
    landed: (u32, u32),
    deaths: Vec<(SoldierId, Option<SoldierId>, RegimentId)>,
}

fn run(world: &mut BattleWorld, commands: &[Command], until: u32, log: &mut Log) {
    while world.tick().0 < until {
        let next = world.tick().next();
        let batch: Vec<Command> = commands
            .iter()
            .filter(|c| c.tick == next)
            .cloned()
            .collect();
        let out = world.step(&batch);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
        for e in &out.events {
            match e {
                BattleEvent::VolleyFired { regiment, count } => {
                    log.volleys.push((next.0, regiment.0, *count));
                }
                BattleEvent::FireBlocked { regiment, blocker } => {
                    log.blocked.push((next.0, regiment.0, blocker.0));
                }
                BattleEvent::ProjectileLanded { hit, .. } => {
                    log.landed.0 += 1;
                    log.landed.1 += u32::from(*hit);
                }
                BattleEvent::SoldierDied {
                    id,
                    killer,
                    regiment,
                    ..
                } => log.deaths.push((*id, *killer, *regiment)),
                _ => {}
            }
        }
        log.hashes.push(out.hash);
        log.max_live = log.max_live.max(world.view().projectile_count());
    }
}

fn regiment_entity(w: &BattleWorld, id: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(id))
        .unwrap()
}

/// Ammo left per soldier of a regiment, ascending id.
fn ammo_of(w: &BattleWorld, regiment: u32) -> Vec<u16> {
    let ids = w.ecs().resource::<Ids>();
    w.ecs()
        .get::<Regiment>(regiment_entity(w, regiment))
        .unwrap()
        .soldiers
        .iter()
        .map(|s| {
            w.ecs()
                .get::<RangedState>(ids.soldier_entity(*s).unwrap())
                .unwrap()
                .ammo
        })
        .collect()
}

/// Moves a whole regiment (anchor and soldiers) by `delta` outside `step`.
fn teleport(w: &mut BattleWorld, regiment: u32, delta: V2) {
    let e = regiment_entity(w, regiment);
    let soldiers = w.ecs().get::<Regiment>(e).unwrap().soldiers.clone();
    w.ecs_mut().get_mut::<Anchor>(e).unwrap().pos += delta;
    for s in soldiers {
        let se = w.ecs().resource::<Ids>().soldier_entity(s).unwrap();
        w.ecs_mut().get_mut::<Pos>(se).unwrap().p += delta;
    }
    w.recompute_hash();
}

#[test]
fn velites_fire_eight_synchronised_volleys_and_run_dry() {
    let setup = volley_setup(335.0);
    let mut w = world(&setup);
    let mut log = Log::default();
    run(&mut w, &[], 1000, &mut log);

    let velites: Vec<&(u32, u32, u16)> = log.volleys.iter().filter(|v| v.1 == 0).collect();
    assert_eq!(velites.len(), 8, "{:?}", log.volleys);
    for pair in velites.windows(2) {
        assert_eq!(pair[1].0 - pair[0].0, 80, "volleys are reload_ticks apart");
    }
    assert!(
        velites.iter().all(|v| v.2 == 120),
        "every velite throws in every volley: {velites:?}"
    );
    assert!(ammo_of(&w, 0).iter().all(|a| *a == 0), "ammo reaches 0");
    // The hastati's pila (25 m) never reach across 35 m.
    assert!(log.volleys.iter().all(|v| v.1 == 0), "{:?}", log.volleys);
    assert!(ammo_of(&w, 1).iter().all(|a| *a == 2));
    assert!(log.blocked.is_empty());
    // One volley in flight at a time (35 ticks of flight, 80 of reload),
    // every javelin lands, and a good share of them hit.
    assert_eq!(log.max_live, 120);
    assert_eq!(w.view().projectile_count(), 0);
    assert_eq!(log.landed.0, 960);
    assert!(log.landed.1 > 200, "hits: {:?}", log.landed);
    let left = w.view().regiment(RegimentId(1)).unwrap().soldier_count;
    // The §15.3 row 5 band (loose velites, 50 seeds) pins the casualties;
    // here the line-formation velites only have to hurt without wiping out.
    assert!(left < 120 && left > 40, "hastati left: {left}");
    assert!(log.deaths.iter().all(|d| d.2 == RegimentId(1)));
    assert!(
        log.deaths.iter().all(|d| d.1.is_some()),
        "every death has a killer"
    );
    assert_eq!(
        w.ecs().get::<Combat>(regiment_entity(&w, 0)).unwrap().kills,
        120 - left,
        "kill credit reconciles with the losses"
    );

    // Determinism across thread counts.
    let mut w8 = world(&setup);
    w8.set_threads(8);
    let mut log8 = Log::default();
    run(&mut w8, &[], 1000, &mut log8);
    assert_eq!(log.hashes, log8.hashes);
    assert_eq!(log.volleys, log8.volleys);
}

#[test]
fn restore_with_a_volley_in_flight_continues_identically() {
    let setup = volley_setup(335.0);
    let mut original = world(&setup);
    let mut log = Log::default();
    // The third volley leaves at tick 161 and lands 35 ticks later.
    run(&mut original, &[], 170, &mut log);
    assert_eq!(log.volleys.len(), 3, "{:?}", log.volleys);
    // A scripted future entry keeps the damage queue non-empty across the
    // restore (the statistical path fills it for real in T2-032).
    let target = original
        .ecs()
        .get::<Regiment>(regiment_entity(&original, 1))
        .unwrap()
        .soldiers[5];
    original
        .ecs_mut()
        .resource_mut::<PendingDamage>()
        .0
        .push(Pending {
            apply_tick: Tick(180),
            target,
            damage: S::from_i32(5),
            shooter: SoldierId(0),
            shooter_regiment: RegimentId(0),
        });
    original.recompute_hash();
    let snap = original.snapshot();
    // The third volley is in flight; the first two have landed.
    assert_eq!(snap.projectiles.len(), 120);
    assert_eq!(snap.pending_damage.len(), 1);
    assert!(snap.projectiles.windows(2).all(|p| p[0].id < p[1].id));
    assert!(snap.regiments[0].fire.is_some());
    assert!(snap.soldiers[0].ranged.is_some());
    let decoded = Snapshot::from_bytes(&snap.to_bytes()).unwrap();
    let mut restored = BattleWorld::restore(&decoded, common::regs()).unwrap();
    assert_eq!(restored.hash(), original.hash());
    assert_eq!(restored.view().projectile_count(), 120);
    restored.set_threads(8);
    for tick in 0..500 {
        assert_eq!(
            original.step(&[]).hash,
            restored.step(&[]).hash,
            "diverged {tick} ticks after the restore"
        );
    }
    assert!(original.ecs().resource::<PendingDamage>().0.is_empty());
}

/// Pushes a projectile aimed at `end`, landing `flight` ticks from now.
fn scripted_projectile(w: &mut BattleWorld, shooter: SoldierId, end: V2, flight: u32) {
    let shooter_regiment = {
        let e = w.ecs().resource::<Ids>().soldier_entity(shooter).unwrap();
        w.ecs()
            .get::<il_sim_battle::components::Soldier>(e)
            .unwrap()
            .regiment
    };
    let side = w.view().regiment(shooter_regiment).unwrap().side;
    let launch = w.tick();
    let id = w.ecs_mut().resource_mut::<Ids>().projectiles.alloc();
    w.ecs_mut()
        .resource_mut::<Projectiles>()
        .0
        .push(Projectile {
            id,
            shooter,
            shooter_regiment,
            side,
            launch_tick: launch,
            land_tick: Tick(launch.0 + flight),
            start: end - V2::from_f32_data(30.0, 0.0),
            end,
            apex: S::from_i32(2),
            arc: ProjectileArc::Direct,
            damage: S::from_i32(1000),
            pen: S::ONE,
        });
    w.recompute_hash();
}

fn soldier_pos(w: &BattleWorld, id: SoldierId) -> V2 {
    w.view().soldier(id).unwrap().pos
}

#[test]
fn shields_only_count_from_the_front() {
    // Hastati facing the velites take frontal hits behind their shields;
    // turned away they take rear hits at 1.5x with no shield: far more die.
    let losses = |deg: f32| {
        let setup = two_sides(
            vec![at(1, "rome:velites", 120, "rome:line", 300.0, 150.0, 0.0)],
            vec![at(2, "rome:hastati", 120, "rome:line", 335.0, 150.0, deg)],
        );
        let mut w = world(&setup);
        let mut log = Log::default();
        run(&mut w, &[], 800, &mut log);
        120 - w.view().regiment(RegimentId(1)).unwrap().soldier_count
    };
    let front = losses(180.0);
    let rear = losses(0.0);
    assert!(rear > front + 10, "front {front}, rear {rear}");
}

#[test]
fn friendly_fire_lands_on_allies_under_the_arrows() {
    // Archers lob 100 m at hastati; a friendly hastati line stands five
    // metres in front of the target, inside the scatter ring.
    let setup = two_sides(
        vec![
            at(1, "persia:archer", 120, "rome:line", 300.0, 150.0, 0.0),
            at(2, "rome:hastati", 120, "rome:line", 393.0, 150.0, 0.0),
        ],
        vec![at(3, "rome:hastati", 120, "rome:line", 400.0, 150.0, 180.0)],
    );
    let mut w = world(&setup);
    let mut log = Log::default();
    // Both hastati lines hold their pila (7 m apart, they would throw).
    let hold = [
        fire_mode(1, 0, 1, FireMode::Hold),
        fire_mode(1, 1, 2, FireMode::Hold),
    ];
    run(&mut w, &hold, 1500, &mut log);
    assert!(log.blocked.is_empty(), "indirect fire is never blocked");
    let own = 120 - w.view().regiment(RegimentId(1)).unwrap().soldier_count;
    let enemy = 120 - w.view().regiment(RegimentId(2)).unwrap().soldier_count;
    // Arrows arrive from behind the friendly line (rear hits, no shield)
    // and from the front of the enemy line (frontal hits behind shields
    // and spread over 120 soldiers, so few reach a lethal total): the
    // friendly losses are the larger number. What the rule asks is that
    // allies under the arrows are hit like enemies.
    assert!(own > 0, "own {own}, enemy {enemy}");
    assert!(log.landed.1 > 100, "{:?}", log.landed);
    let archers: Vec<SoldierId> = w
        .ecs()
        .get::<Regiment>(regiment_entity(&w, 0))
        .unwrap()
        .soldiers
        .clone();
    assert!(
        log.deaths
            .iter()
            .any(|d| d.2 == RegimentId(1) && d.1.is_some_and(|k| archers.contains(&k))),
        "an ally was killed by an archer: {:?}",
        log.deaths
    );
}

#[test]
fn a_soldier_killed_in_melee_and_hit_by_a_javelin_dies_once() {
    let mut w = world(&volley_setup(335.0));
    let victim = w
        .ecs()
        .get::<Regiment>(regiment_entity(&w, 1))
        .unwrap()
        .soldiers[0];
    let ve = w.ecs().resource::<Ids>().soldier_entity(victim).unwrap();
    // The melee kill of this tick has already been applied (Stage 10)...
    w.ecs_mut().get_mut::<Health>(ve).unwrap().hp = S::ZERO;
    let killer = SoldierId(7);
    w.ecs_mut().resource_mut::<Kills>().0.push(Kill {
        victim,
        killer: Some(killer),
        killer_regiment: Some(RegimentId(0)),
    });
    // ...and a javelin lands on the same soldier at Stage 11.
    let next = w.tick().next();
    w.ecs_mut().resource_mut::<PendingDamage>().0.push(Pending {
        apply_tick: next,
        target: victim,
        damage: S::from_i32(50),
        shooter: SoldierId(9),
        shooter_regiment: RegimentId(0),
    });
    w.recompute_hash();
    let out = w.step(&[fire_mode(next.0, 0, 0, FireMode::Hold)]);
    let deaths: Vec<_> = out
        .events
        .iter()
        .filter(|e| matches!(e, BattleEvent::SoldierDied { .. }))
        .collect();
    assert_eq!(deaths.len(), 1, "{deaths:?}");
    assert!(matches!(
        deaths[0],
        BattleEvent::SoldierDied { id, killer: Some(k), .. } if *id == victim && *k == killer
    ));
    assert_eq!(
        w.ecs().get::<Combat>(regiment_entity(&w, 0)).unwrap().kills,
        1
    );
    assert!(w.ecs().resource::<PendingDamage>().0.is_empty());
}

#[test]
fn a_dead_shooter_still_credits_its_regiment() {
    let mut w = world(&volley_setup(335.0));
    // Hold fire so only the scripted javelin flies.
    let t = w.tick().next();
    assert!(
        w.step(&[fire_mode(t.0, 0, 0, FireMode::Hold)])
            .rejected
            .is_empty()
    );
    let shooter = w
        .ecs()
        .get::<Regiment>(regiment_entity(&w, 0))
        .unwrap()
        .soldiers[3];
    let target = w
        .ecs()
        .get::<Regiment>(regiment_entity(&w, 1))
        .unwrap()
        .soldiers[10];
    // Aim at where the (idle) target stands, landing in three ticks.
    let aim = soldier_pos(&w, target);
    scripted_projectile(&mut w, shooter, aim, 3);
    // The shooter falls this very tick.
    let se = w.ecs().resource::<Ids>().soldier_entity(shooter).unwrap();
    w.ecs_mut().get_mut::<Health>(se).unwrap().hp = S::ZERO;
    w.ecs_mut().resource_mut::<Kills>().0.push(Kill {
        victim: shooter,
        killer: None,
        killer_regiment: None,
    });
    w.recompute_hash();
    let mut log = Log::default();
    let until = w.tick().0 + 4;
    run(&mut w, &[], until, &mut log);
    assert_eq!(log.landed, (1, 1), "the javelin landed on someone");
    assert!(
        log.deaths
            .iter()
            .any(|d| d.0 == target && d.1 == Some(shooter) && d.2 == RegimentId(1)),
        "{:?}",
        log.deaths
    );
    assert_eq!(
        w.ecs().get::<Combat>(regiment_entity(&w, 0)).unwrap().kills,
        1,
        "credited although the shooter is gone"
    );
    assert!(w.view().soldier(shooter).is_none());
}

#[test]
fn a_javelin_hits_the_nearest_soldier_under_it() {
    let mut w = world(&volley_setup(335.0));
    let t = w.tick().next();
    assert!(
        w.step(&[fire_mode(t.0, 0, 0, FireMode::Hold)])
            .rejected
            .is_empty()
    );
    let hastati = w
        .ecs()
        .get::<Regiment>(regiment_entity(&w, 1))
        .unwrap()
        .soldiers
        .clone();
    let (near, far) = (hastati[20], hastati[21]);
    // Land 0.2 m from `near`, on the side away from `far` (neighbours in a
    // rank stand about a metre apart).
    let p_near = soldier_pos(&w, near);
    let p_far = soldier_pos(&w, far);
    let away = (p_near - p_far).normalized_or_zero() * S::from_f32_data(0.2);
    let shooter = w
        .ecs()
        .get::<Regiment>(regiment_entity(&w, 0))
        .unwrap()
        .soldiers[0];
    scripted_projectile(&mut w, shooter, p_near + away, 1);
    let mut log = Log::default();
    let until = w.tick().0 + 1;
    run(&mut w, &[], until, &mut log);
    assert_eq!(log.landed, (1, 1));
    assert_eq!(log.deaths.len(), 1, "{:?}", log.deaths);
    assert_eq!(log.deaths[0].0, near);
    assert!(w.view().soldier(far).is_some());
    // Landing on empty ground hits nobody.
    scripted_projectile(&mut w, shooter, V2::from_f32_data(600.0, 600.0), 1);
    let until = w.tick().0 + 1;
    run(&mut w, &[], until, &mut log);
    assert_eq!(log.landed, (2, 1));
}

#[test]
fn archer_battle_is_identical_at_one_and_eight_threads() {
    // 15 archer regiments (1,800 soldiers) lob at five hoplite phalanxes.
    let mut archers = Vec::new();
    for row in 0..5 {
        for col in 0..3 {
            archers.push(at(
                1 + row * 3 + col,
                "persia:archer",
                120,
                "rome:line",
                [280.0, 300.0, 320.0][col as usize],
                [60.0, 105.0, 150.0, 195.0, 240.0][row as usize],
                0.0,
            ));
        }
    }
    let hoplites = (0..5)
        .map(|row| {
            at(
                20 + row,
                "greece:hoplite",
                160,
                "rome:phalanx",
                400.0,
                [60.0, 105.0, 150.0, 195.0, 240.0][row as usize],
                180.0,
            )
        })
        .collect();
    let setup = two_sides(archers, hoplites);
    let mut a = world(&setup);
    let mut la = Log::default();
    run(&mut a, &[], 400, &mut la);
    let mut b = world(&setup);
    b.set_threads(8);
    let mut lb = Log::default();
    run(&mut b, &[], 400, &mut lb);
    assert_eq!(la.hashes, lb.hashes);
    assert!(la.landed.1 > 100, "{:?}", la.landed);
    assert!(la.max_live > 1000, "{}", la.max_live);
}

#[test]
fn hold_fires_nothing_and_fire_at_will_resumes() {
    let mut w = world(&volley_setup(335.0));
    let mut log = Log::default();
    let hold = fire_mode(1, 0, 0, FireMode::Hold);
    run(&mut w, &[hold], 200, &mut log);
    assert!(log.volleys.is_empty(), "{:?}", log.volleys);
    assert!(ammo_of(&w, 0).iter().all(|a| *a == 8));
    assert_eq!(
        w.view().regiment(RegimentId(0)).unwrap().fire,
        Some(FireMode::Hold)
    );

    let resume = fire_mode(201, 0, 0, FireMode::FireAtWill);
    run(&mut w, &[resume], 260, &mut log);
    assert_eq!(log.volleys.len(), 1, "{:?}", log.volleys);
    assert!(
        log.volleys[0].0 <= 212,
        "retargets within ranged_retarget_ticks"
    );
}

#[test]
fn target_mode_is_validated_and_falls_back_when_the_target_empties() {
    // Side 0: velites 0 and hoplites 1; side 1: hastati 2 and hastati 3.
    let setup = two_sides(
        vec![
            at(1, "rome:velites", 120, "rome:loose", 300.0, 150.0, 0.0),
            at(2, "greece:hoplite", 40, "rome:line", 300.0, 100.0, 0.0),
        ],
        vec![
            at(3, "rome:hastati", 120, "rome:line", 335.0, 150.0, 180.0),
            // 38 m from the velites' anchor: inside the 40 m annulus.
            at(4, "rome:hastati", 40, "rome:line", 335.0, 165.0, 180.0),
        ],
    );
    let mut w = world(&setup);
    let t = w.tick().next();
    let out = w.step(&[
        fire_mode(t.0, 0, 0, FireMode::Target(RegimentId(1))), // own side
        Command {
            seq: 1,
            ..fire_mode(t.0, 0, 1, FireMode::Hold) // hoplites do not shoot
        },
        Command {
            seq: 2,
            ..fire_mode(t.0, 0, 0, FireMode::Target(RegimentId(9))) // no such regiment
        },
        Command {
            seq: 3,
            ..fire_mode(t.0, 0, 0, FireMode::Target(RegimentId(3)))
        },
    ]);
    let reasons: Vec<&RejectReason> = out.rejected.iter().map(|(_, r)| r).collect();
    assert_eq!(
        reasons,
        vec![
            &RejectReason::InvalidTarget(RegimentId(1)),
            &RejectReason::NotRanged(RegimentId(1)),
            &RejectReason::UnknownRegiment(RegimentId(9)),
        ]
    );
    let row = w.view().regiment(RegimentId(0)).unwrap();
    assert_eq!(row.fire, Some(FireMode::Target(RegimentId(3))));

    // The ordered regiment is the one shot at, even though regiment 2 has
    // more soldiers in range (the first volley left inside the command's
    // own tick; the log starts after it, so it holds the second, at 81).
    let mut log = Log::default();
    run(&mut w, &[], 100, &mut log);
    assert_eq!(
        w.view().regiment(RegimentId(0)).unwrap().fire_target,
        Some(RegimentId(3))
    );
    assert_eq!(log.volleys.iter().filter(|v| v.1 == 0).count(), 1);
    assert!(
        ammo_of(&w, 0).contains(&6),
        "two throws at the ordered target"
    );

    // Empty it: the mode falls back to fire-at-will and regiment 2 is next.
    let e = regiment_entity(&w, 3);
    w.ecs_mut().get_mut::<Regiment>(e).unwrap().soldiers.clear();
    w.recompute_hash();
    run(&mut w, &[], 160, &mut log);
    let row = w.view().regiment(RegimentId(0)).unwrap();
    assert_eq!(row.fire, Some(FireMode::FireAtWill));
    assert_eq!(row.fire_target, Some(RegimentId(2)));
}

#[test]
fn friendly_line_blocks_direct_fire_but_not_arrows() {
    // Side 0: velites 0 with a friendly hastati line 1 five metres ahead;
    // side 1: hastati 2 at 35 m.
    let setup = two_sides(
        vec![
            at(1, "rome:velites", 120, "rome:line", 300.0, 150.0, 0.0),
            at(2, "rome:hastati", 120, "rome:line", 305.0, 150.0, 0.0),
        ],
        vec![at(3, "rome:hastati", 120, "rome:line", 335.0, 150.0, 180.0)],
    );
    let mut w = world(&setup);
    let mut log = Log::default();
    // The friendly hastati hold their pila so only the velites are in question.
    run(&mut w, &[fire_mode(1, 0, 1, FireMode::Hold)], 100, &mut log);
    // The friendly circle (extent 20 m) masks all but the outermost file,
    // whose line of fire passes 22 m from the friendly anchor.
    let through: u32 = log
        .volleys
        .iter()
        .filter(|v| v.1 == 0)
        .map(|v| u32::from(v.2))
        .sum();
    assert!(through <= 4, "{:?}", log.volleys);
    assert!(
        log.blocked.iter().any(|b| b.1 == 0 && b.2 == 1),
        "{:?}",
        log.blocked
    );
    let kept = ammo_of(&w, 0).iter().filter(|a| **a == 8).count();
    assert!(kept >= 115, "ammo kept by the blocked shooters: {kept}");
    assert_eq!(
        w.view().regiment(RegimentId(0)).unwrap().fire,
        Some(FireMode::FireAtWill),
        "the mode is unchanged"
    );

    // Out of the way: fire resumes by itself.
    teleport(&mut w, 1, V2::from_f32_data(0.0, 120.0));
    let before = log.volleys.len();
    run(&mut w, &[], 200, &mut log);
    assert!(
        log.volleys[before..].iter().any(|v| v.1 == 0),
        "{:?}",
        log.volleys
    );

    // Archers lob over the same line.
    let setup = two_sides(
        vec![
            at(1, "persia:archer", 120, "rome:line", 300.0, 150.0, 0.0),
            at(2, "rome:hastati", 120, "rome:line", 305.0, 150.0, 0.0),
        ],
        vec![at(3, "rome:hastati", 120, "rome:line", 380.0, 150.0, 180.0)],
    );
    let mut w = world(&setup);
    let mut log = Log::default();
    run(&mut w, &[fire_mode(1, 0, 1, FireMode::Hold)], 60, &mut log);
    assert!(log.blocked.is_empty(), "{:?}", log.blocked);
    let archers: Vec<_> = log.volleys.iter().filter(|v| v.1 == 0).collect();
    assert_eq!(archers.len(), 1, "{:?}", log.volleys);
    assert_eq!(archers[0].2, 120);
    assert!(ammo_of(&w, 0).iter().all(|a| *a == 19));
    // An 80 m lob at 40 m/s: v = sqrt(80 g) = 28 m/s, 4.04 s = 81 ticks, so
    // the arrows that left at tick 1 are still up at tick 60 and land near 82.
    assert_eq!(w.view().projectile_count(), 120);
    let p = w.view().projectiles().next().unwrap();
    assert!(
        (80..=86).contains(&p.land_tick.0) && p.apex > S::from_i32(15),
        "{p:?}"
    );
    run(&mut w, &[], 90, &mut log);
    assert_eq!(w.view().projectile_count(), 0);
    assert_eq!(log.landed.0, 120);
}

#[test]
fn out_of_range_soldiers_keep_their_ammo() {
    let mut w = world(&volley_setup(350.0));
    let mut log = Log::default();
    run(&mut w, &[], 300, &mut log);
    assert!(log.volleys.is_empty(), "{:?}", log.volleys);
    assert!(ammo_of(&w, 0).iter().all(|a| *a == 8));
    assert_eq!(w.view().regiment(RegimentId(0)).unwrap().fire_target, None);

    // Just inside: a line 39 m off has soldiers within 40 m of the anchor.
    let mut w = world(&volley_setup(339.0));
    let mut log = Log::default();
    run(&mut w, &[], 60, &mut log);
    assert_eq!(log.volleys.len(), 1, "{:?}", log.volleys);
    let e = regiment_entity(&w, 0);
    assert_eq!(w.ecs().get::<Fire>(e).unwrap().target, Some(RegimentId(1)));
}

/// Registries with a rules-override mod from `tests/mods/`.
fn regs_with_mod(name: &str) -> std::sync::Arc<il_data::Registries> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let roots = [root.join("game"), root.join("tests/mods").join(name)];
    std::sync::Arc::new(il_data::Registries::load_roots(&roots).unwrap_or_else(|d| panic!("{d}")))
}

#[test]
fn capped_at_zero_every_volley_resolves_statistically() {
    let regs = regs_with_mod("projectile_cap_zero");
    assert_eq!(regs.rules.combat.projectile_cap, 0);
    let setup = volley_setup(335.0);
    let mut w = BattleWorld::new(&setup, regs.clone()).unwrap();
    let mut log = Log::default();
    run(&mut w, &[], 170, &mut log);
    // Three volleys, no projectile ever, the damage waiting for its tick.
    assert_eq!(
        log.volleys.iter().filter(|v| v.1 == 0).count(),
        3,
        "{:?}",
        log.volleys
    );
    assert_eq!(log.max_live, 0);
    assert_eq!(log.landed, (0, 0), "no landing events on this path");
    let queue = w.ecs().resource::<PendingDamage>().0.clone();
    assert!(!queue.is_empty(), "the third volley's hits are queued");
    // Thrown at tick 161 with 35 to 42 ticks of flight (the wings shoot
    // the farthest).
    let ticks: Vec<u32> = queue.iter().map(|p| p.apply_tick.0).collect();
    assert!(ticks.iter().all(|t| (196..=205).contains(t)), "{ticks:?}");
    assert!(queue.iter().all(|p| p.shooter_regiment == RegimentId(0)));
    assert!(ammo_of(&w, 0).iter().all(|a| *a == 5));

    // Restore with the queue in flight, then both continue identically.
    let snap = w.snapshot();
    assert_eq!(snap.pending_damage.len(), queue.len());
    let mut restored = BattleWorld::restore(&snap, regs.clone()).unwrap();
    assert_eq!(restored.hash(), w.hash());
    restored.set_threads(8);
    let mut log_r = Log::default();
    run(&mut w, &[], 1000, &mut log);
    run(&mut restored, &[], 1000, &mut log_r);
    assert_eq!(log.hashes[170..], log_r.hashes[..]);
    let left = w.view().regiment(RegimentId(1)).unwrap().soldier_count;
    assert!(left < 120, "hastati fall to statistical hits: {left}");
    assert!(!log.deaths.is_empty() && log.deaths.iter().all(|d| d.1.is_some()));
    assert!(w.ecs().resource::<PendingDamage>().0.is_empty());

    // The same seeds at 1 and 8 threads agree from the start.
    let mut w8 = BattleWorld::new(&setup, regs).unwrap();
    w8.set_threads(8);
    let mut log8 = Log::default();
    run(&mut w8, &[], 300, &mut log8);
    assert_eq!(log.hashes[..300], log8.hashes[..]);
}

#[test]
fn a_volley_splits_at_the_cap_in_shooter_order() {
    let regs = regs_with_mod("projectile_cap_100");
    let mut w = BattleWorld::new(&volley_setup(335.0), regs).unwrap();
    let mut log = Log::default();
    run(&mut w, &[], 1, &mut log);
    assert_eq!(log.volleys, vec![(1, 0, 120)]);
    let projectiles: Vec<_> = w.view().projectiles().collect();
    assert_eq!(projectiles.len(), 100);
    // The first hundred shooters by id got projectiles; the rest went
    // statistical (ammo spent either way).
    let velites = w
        .ecs()
        .get::<Regiment>(regiment_entity(&w, 0))
        .unwrap()
        .soldiers
        .clone();
    let shooters: Vec<SoldierId> = w
        .ecs()
        .resource::<Projectiles>()
        .0
        .iter()
        .map(|p| p.shooter)
        .collect();
    assert_eq!(shooters, velites[..100].to_vec());
    assert!(ammo_of(&w, 0).iter().all(|a| *a == 7));
}
