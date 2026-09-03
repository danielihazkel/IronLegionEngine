//! T1-041: spawning on real Line slots, reforming on a facing change,
//! resizing after losses, and the same result at 1 and 8 threads.

mod common;

use std::collections::BTreeSet;

use il_core::{Angle, RegimentId, S, Scalar, SoldierId};
use il_sim_battle::components::{Anchor, FormationState, Pos, Rank, Regiment, SlotRef};
use il_sim_battle::resources::Ids;
use il_sim_battle::{BattleWorld, slot_world};

fn regiment_entity(w: &BattleWorld, rid: u32) -> bevy_ecs::entity::Entity {
    w.ecs()
        .resource::<Ids>()
        .regiment_entity(RegimentId(rid))
        .unwrap()
}

/// `(slot, rank, file, position)` per soldier of the regiment, in id order.
fn members(w: &BattleWorld, rid: u32) -> Vec<(Option<u16>, Rank, il_core::V2)> {
    let e = regiment_entity(w, rid);
    let ids: Vec<SoldierId> = w.ecs().get::<Regiment>(e).unwrap().soldiers.clone();
    ids.iter()
        .map(|sid| {
            let se = w.ecs().resource::<Ids>().soldier_entity(*sid).unwrap();
            (
                w.ecs().get::<SlotRef>(se).unwrap().slot,
                *w.ecs().get::<Rank>(se).unwrap(),
                w.ecs().get::<Pos>(se).unwrap().p,
            )
        })
        .collect()
}

fn assert_one_to_one(w: &BattleWorld, rid: u32) {
    let m = members(w, rid);
    let state = w.view().formation_state(RegimentId(rid)).unwrap();
    assert_eq!(state.slots.len(), m.len(), "one slot per soldier");
    let used: BTreeSet<u16> = m.iter().map(|(s, _, _)| s.expect("assigned")).collect();
    assert_eq!(used.len(), m.len(), "no two soldiers share a slot");
    for (slot, rank, _) in &m {
        let s = &state.slots[usize::from(slot.unwrap())];
        assert_eq!((rank.rank, rank.file), (s.rank, s.file));
    }
    assert!(!state.needs_reform && !state.dirty);
}

#[test]
fn spawning_places_soldiers_on_line_slots() {
    let w = common::world(60);
    for rid in 0..2 {
        assert_one_to_one(&w, rid);
        let state = w.view().formation_state(RegimentId(rid)).unwrap();
        let anchor = *w.ecs().get::<Anchor>(regiment_entity(&w, rid)).unwrap();
        assert_eq!(state.ranks, 4, "rome:line default ranks");
        assert_eq!(state.files, 15);
        for (slot, _, p) in members(&w, rid) {
            assert_eq!(
                p,
                slot_world(&anchor, &state.slots[usize::from(slot.unwrap())])
            );
        }
        assert_eq!(state.laid_out_facing, anchor.facing);
    }
}

#[test]
fn a_facing_change_beyond_reform_angle_reforms_and_is_thread_independent() {
    let mut a = common::world(120);
    let mut b = common::world(120);
    b.set_threads(8);
    for w in [&mut a, &mut b] {
        let e = regiment_entity(w, 0);
        w.ecs_mut().get_mut::<Anchor>(e).unwrap().facing = Angle::from_degrees_data(90.0);
        w.recompute_hash();
        w.step(&[]);
    }
    assert_one_to_one(&a, 0);
    let state = a.view().formation_state(RegimentId(0)).unwrap();
    assert_eq!(state.laid_out_facing, Angle::from_degrees_data(90.0));
    assert_eq!(members(&a, 0), members(&b, 0));
    assert_eq!(a.hash(), b.hash());
    // A small facing change does not reform.
    let before = members(&a, 1);
    let e = regiment_entity(&a, 1);
    let facing = a.ecs().get::<Anchor>(e).unwrap().facing;
    a.ecs_mut().get_mut::<Anchor>(e).unwrap().facing =
        Angle::new(facing.radians() + S::from_f32_data(0.05));
    a.step(&[]);
    assert_eq!(members(&a, 1), before);
}

#[test]
fn losses_shrink_the_layout_and_close_ranks_from_the_rear() {
    let mut w = common::world(40);
    let e = regiment_entity(&w, 0);
    // Remove five soldiers of the front rank (ids 0..5) the way Stage 15 will.
    let victims: Vec<SoldierId> = w.ecs().get::<Regiment>(e).unwrap().soldiers[..5].to_vec();
    for sid in &victims {
        let se = w.ecs().resource::<Ids>().soldier_entity(*sid).unwrap();
        w.ecs_mut().despawn(se);
        let mut ids = w.ecs_mut().resource_mut::<Ids>();
        ids.soldier_entities.retain(|(id, _)| id != sid);
    }
    w.ecs_mut()
        .get_mut::<Regiment>(e)
        .unwrap()
        .soldiers
        .retain(|id| !victims.contains(id));
    w.recompute_hash();
    w.step(&[]);
    assert_one_to_one(&w, 0);
    let state = w.view().formation_state(RegimentId(0)).unwrap();
    assert_eq!(state.slots.len(), 35);
    assert_eq!(state.ranks, 4);
    // The front rank is full again: nine files, all present.
    let front = members(&w, 0)
        .iter()
        .filter(|(_, r, _)| r.rank == 0)
        .count();
    assert_eq!(front, state.files as usize);
    let _ = FormationState::new;
}
