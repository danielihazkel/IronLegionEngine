//! T0-033: Stage 17 hashes exactly the SIM-DET-004 fields, is stable across
//! process runs (golden), and swaps the interpolation buffers.

mod common;

use il_core::{Angle, S, Scalar, StateHash, StreamId, V2};
use il_sim_battle::BattleWorld;
use il_sim_battle::components::{
    Anchor, Body, FatigueC, Fsm, Health, Morale, MoraleState, Order, OrderKind, Pos, PrevPos,
    Regiment, SlotRef, SoldierState, Vel,
};
use il_sim_battle::resources::{BattlePhase, Ids, Phase, Rng};

/// Golden hash of the freshly spawned 2 x 500 hastati world at seed 42 on
/// the test map (re-baselined in T1-030 when the regiments moved onto it).
/// Stable across process runs; changes only when the hash layout, the
/// spawn placement, the content values or the RNG seeding change.
const GOLDEN_FRESH: u64 = 0x7af8_c96c_2094_821d;
/// Golden hash after 1,000 idle ticks of the same world.
const GOLDEN_1000: u64 = 0xc30d_3adf_2711_a672;

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
    cases.push((
        "ammo",
        Box::new(|w| {
            let e = regiment_entity(w, 1);
            w.ecs_mut().get_mut::<Regiment>(e).unwrap().ammo = 2;
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
