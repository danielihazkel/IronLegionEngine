//! T0-033: Stage 17 hashes exactly the SIM-DET-004 fields, is stable across
//! process runs (golden), and swaps the interpolation buffers.

mod common;

use il_core::ProjectileId;
use il_core::Tick;
use il_core::{Angle, RegimentId, S, Scalar, SoldierId, StateHash, StreamId, V2};
use il_data::ProjectileArc;
use il_sim_battle::components::{
    Anchor, Body, Combat, Facing, FatigueC, Fire, FormationState, Fsm, Health, MeleeState, Morale,
    MoraleState, Order, OrderKind, Path, Pos, PrevPos, RangedState, Regiment, SlotRef,
    SoldierState, Vel, Waypoint,
};
use il_sim_battle::resources::{
    BattlePhase, Ids, Pending, PendingDamage, Phase, Projectile, Projectiles, Rng,
};
use il_sim_battle::{BattleWorld, FireMode, SpeedMode};

/// Golden hash of the freshly spawned 2 x 500 hastati world at seed 42 on
/// the test map (re-baselined in T1-030 when the regiments moved onto it and
/// in T1-041 when spawning switched to real Line slots; the 1,000-tick value
/// again in T1-043 when soldiers started steering and in T1-044 when
/// collisions started pushing; both again in T1-045 when the formation
/// frame was corrected so a line spans perpendicular to its facing; and in
/// T1-047 when the Phase 1 hash layout was fixed; and in T2-010 when regiments
/// started spawning with the unit's ranged ammo; in T2-020 when the combat
/// fields joined the layout; and in T2-030 when the regiment ammo gave way
/// to the optional fire and ranged states and the pending damage prefix).
/// Stable across process runs; changes only when the hash layout, the
/// spawn placement, the content values or the RNG seeding change.
const GOLDEN_FRESH: u64 = 0xae55_acb8_3fb9_7902;
/// Golden hash after 1,000 idle ticks of the same world.
const GOLDEN_1000: u64 = 0x8bd8_418e_1bb7_4d07;

type Mutation = Box<dyn Fn(&mut BattleWorld)>;

fn soldier_entity(w: &BattleWorld, index: usize) -> bevy_ecs::entity::Entity {
    w.ecs().resource::<Ids>().soldier_entities[index].1
}

fn regiment_entity(w: &BattleWorld, index: usize) -> bevy_ecs::entity::Entity {
    w.ecs().resource::<Ids>().regiment_entities[index].1
}

#[test]
fn fresh_world_hash_is_golden_and_stable() {
    let mut w = common::world(500);
    let fresh = w.hash();
    assert_eq!(fresh, w.recompute_hash());
    for _ in 0..1000 {
        w.step(&[]);
    }
    let after = w.hash();
    assert_eq!(
        (fresh.0, after.0),
        (GOLDEN_FRESH, GOLDEN_1000),
        "golden mismatch; actual: fresh 0x{:016x}, after 1000 ticks 0x{:016x}",
        fresh.0,
        after.0
    );
}

#[test]
fn every_hashed_field_changes_the_hash() {
    let base = common::world(20).hash();
    let mut cases: Vec<(&str, Mutation)> = Vec::new();

    // Soldier fields, SIM-DET-004 order: p, v, hp, fatigue, FSM state, slot.
    cases.push((
        "pos",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<Pos>(e).unwrap().p = V2::from_f32_data(1.0, 2.0);
        }),
    ));
    cases.push((
        "vel",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<Vel>(e).unwrap().v = V2::from_f32_data(0.0, 1.0);
        }),
    ));
    cases.push((
        "hp",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<Health>(e).unwrap().hp = S::from_i32(1);
        }),
    ));
    cases.push((
        "fatigue",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<FatigueC>(e).unwrap().f = S::HALF;
        }),
    ));
    cases.push((
        "fsm state",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<Fsm>(e).unwrap().state = SoldierState::MoveToSlot;
        }),
    ));
    cases.push((
        "slot",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<SlotRef>(e).unwrap().slot = Some(4);
        }),
    ));
    // Regiment fields: morale, morale state, soldier count, anchor, order kind, ammo.
    cases.push((
        "morale",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Morale>(e).unwrap().m = S::from_i32(10);
        }),
    ));
    cases.push((
        "morale state",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Morale>(e).unwrap().state = MoraleState::Shaken;
        }),
    ));
    cases.push((
        "soldier count",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Regiment>(e).unwrap().soldiers.pop();
        }),
    ));
    cases.push((
        "anchor pos",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Anchor>(e).unwrap().pos = V2::ZERO;
        }),
    ));
    cases.push((
        "anchor facing",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Anchor>(e).unwrap().facing = Angle::new(S::ONE);
        }),
    ));
    cases.push((
        "order kind",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Order>(e).unwrap().kind = OrderKind::Move;
        }),
    ));
    // T2-030: the regiment's fire state replaced `ammo` in the layout
    // (hastati carry pila, so both components exist in this world).
    cases.push((
        "fire mode",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Fire>(e).unwrap().mode = FireMode::Hold;
        }),
    ));
    cases.push((
        "fire target",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Fire>(e).unwrap().target = Some(RegimentId(0));
        }),
    ));
    cases.push((
        "fire cooldown",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Fire>(e).unwrap().cooldown = 11;
        }),
    ));
    cases.push((
        "fire presence",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().entity_mut(e).remove::<Fire>();
        }),
    ));
    // T1-047 layout: the rest of the order, the formation state, the path
    // and the soldier facing.
    cases.push((
        "order target",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Order>(e).unwrap().target = V2::from_f32_data(1.0, 2.0);
        }),
    ));
    cases.push((
        "order facing",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Order>(e).unwrap().facing = Some(Angle::new(S::ONE));
        }),
    ));
    cases.push((
        "order speed",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Order>(e).unwrap().speed = SpeedMode::Run;
        }),
    ));
    cases.push((
        "order since",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Order>(e).unwrap().since = Tick(5);
        }),
    ));
    cases.push((
        "formation template",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            let column = w
                .registries()
                .formations
                .lookup(&common::cid("rome:column"))
                .unwrap();
            w.ecs_mut().get_mut::<FormationState>(e).unwrap().template = column;
        }),
    ));
    cases.push((
        "formation ranks",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<FormationState>(e).unwrap().ranks += 1;
        }),
    ));
    cases.push((
        "formation files",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<FormationState>(e).unwrap().files += 1;
        }),
    ));
    cases.push((
        "integrity",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<FormationState>(e).unwrap().integrity = S::HALF;
        }),
    ));
    cases.push((
        "morph_until",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut()
                .get_mut::<FormationState>(e)
                .unwrap()
                .morph_until = Tick(9);
        }),
    ));
    cases.push((
        "needs_reform",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut()
                .get_mut::<FormationState>(e)
                .unwrap()
                .needs_reform = true;
        }),
    ));
    cases.push((
        "prior template",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            let line = w
                .registries()
                .formations
                .lookup(&common::cid("rome:line"))
                .unwrap();
            w.ecs_mut()
                .get_mut::<FormationState>(e)
                .unwrap()
                .prior_template = Some(line);
        }),
    ));
    cases.push((
        "laid-out facing",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut()
                .get_mut::<FormationState>(e)
                .unwrap()
                .laid_out_facing = Angle::new(S::ONE);
        }),
    ));
    cases.push((
        "path waypoints",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut()
                .get_mut::<Path>(e)
                .unwrap()
                .waypoints
                .push(Waypoint {
                    p: V2::from_f32_data(3.0, 4.0),
                    corridor: S::from_i32(8),
                });
        }),
    ));
    cases.push((
        "path next",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Path>(e).unwrap().next = 3;
        }),
    ));
    cases.push((
        "path requested",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Path>(e).unwrap().requested = true;
        }),
    ));
    cases.push((
        "soldier facing",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<Facing>(e).unwrap().theta = Angle::new(S::ONE);
        }),
    ));
    // T2-020: melee state per soldier, combat state and casualty ring per
    // regiment, the order's target regiment.
    cases.push((
        "melee target",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<MeleeState>(e).unwrap().target = Some(SoldierId(9));
        }),
    ));
    cases.push((
        "melee cooldown",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<MeleeState>(e).unwrap().cooldown = 7;
        }),
    ));
    cases.push((
        "order target regiment",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Order>(e).unwrap().target_regiment = Some(RegimentId(0));
        }),
    ));
    // T2-030: per-soldier ranged state, the projectile list and the pending
    // damage queue.
    cases.push((
        "ranged ammo",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<RangedState>(e).unwrap().ammo += 1;
        }),
    ));
    cases.push((
        "ranged cooldown",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().get_mut::<RangedState>(e).unwrap().cooldown = 4;
        }),
    ));
    cases.push((
        "ranged presence",
        Box::new(|w| {
            let e = soldier_entity(w, 3);
            w.ecs_mut().entity_mut(e).remove::<RangedState>();
        }),
    ));
    cases.push((
        "projectile",
        Box::new(|w| {
            w.ecs_mut()
                .resource_mut::<Projectiles>()
                .0
                .push(Projectile {
                    id: ProjectileId(0),
                    shooter: SoldierId(1),
                    shooter_regiment: RegimentId(0),
                    side: 0,
                    launch_tick: Tick(1),
                    land_tick: Tick(20),
                    start: V2::from_f32_data(1.0, 1.0),
                    end: V2::from_f32_data(30.0, 1.0),
                    apex: S::from_i32(2),
                    arc: ProjectileArc::Direct,
                    damage: S::from_i32(30),
                    pen: S::HALF,
                });
        }),
    ));
    cases.push((
        "pending damage",
        Box::new(|w| {
            w.ecs_mut().resource_mut::<PendingDamage>().0.push(Pending {
                apply_tick: Tick(5),
                target: SoldierId(2),
                damage: S::from_i32(3),
                shooter: SoldierId(30),
                shooter_regiment: RegimentId(1),
            });
        }),
    ));
    cases.push((
        "combat engaged",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Combat>(e).unwrap().engaged = true;
        }),
    ));
    cases.push((
        "combat last_fighting",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Combat>(e).unwrap().last_fighting = Tick(3);
        }),
    ));
    cases.push((
        "combat charge_until",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Combat>(e).unwrap().charge_until = Tick(60);
        }),
    ));
    cases.push((
        "combat experience",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Combat>(e).unwrap().experience = 2;
        }),
    ));
    cases.push((
        "combat kills",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Combat>(e).unwrap().kills = 5;
        }),
    ));
    cases.push((
        "casualty ring",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Morale>(e).unwrap().deaths_5s[3] = 1;
        }),
    ));
    cases.push((
        "initial strength",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Morale>(e).unwrap().initial = 7;
        }),
    ));
    // Globals: phase, RNG.
    cases.push((
        "phase",
        Box::new(|w| {
            w.ecs_mut().resource_mut::<Phase>().0 = BattlePhase::Pursuit;
        }),
    ));
    cases.push((
        "rng",
        Box::new(|w| {
            w.ecs_mut()
                .resource_mut::<Rng>()
                .stream(StreamId::Morale)
                .next_u32();
        }),
    ));

    for (name, mutate) in cases {
        let mut w = common::world(20);
        mutate(&mut w);
        assert_ne!(w.recompute_hash(), base, "{name} is not hashed");
    }
}

#[test]
fn derived_and_render_only_fields_do_not_change_the_hash() {
    let base = common::world(20).hash();
    let mut w = common::world(20);
    let e = soldier_entity(&w, 0);
    w.ecs_mut().get_mut::<Body>(e).unwrap().r = S::from_i32(9);
    w.ecs_mut().get_mut::<PrevPos>(e).unwrap().p = V2::from_f32_data(5.0, 5.0);
    assert_eq!(w.recompute_hash(), base);
}

#[test]
fn stage_17_copies_pos_to_prev_pos() {
    let mut w = common::world(20);
    let e = soldier_entity(&w, 7);
    let moved = V2::from_f32_data(3.0, -4.0);
    w.ecs_mut().get_mut::<Pos>(e).unwrap().p = moved;
    assert_ne!(w.ecs().get::<PrevPos>(e).unwrap().p, moved);
    w.step(&[]);
    assert_eq!(w.ecs().get::<PrevPos>(e).unwrap().p, moved);
    assert_ne!(w.hash(), StateHash(0));
}
