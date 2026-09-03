//! `BattleView` (T1-052): read-only, id-ordered, and in step with the world.

mod common;

use il_core::{Angle, S, Scalar, SoldierId, V2};
use il_sim_battle::BattleView;

fn assert_read_only(_: &BattleView<'_>) {}

#[test]
fn view_iterates_soldiers_and_regiments_in_id_order() {
    let world = common::world(60);
    let view = world.view();
    assert_read_only(&view);
    assert_eq!(view.soldier_count(), 120);
    assert_eq!(view.regiment_count(), 2);

    let ordered: Vec<SoldierId> = view.soldiers().map(|s| s.id).collect();
    let expected: Vec<SoldierId> = world.soldier_ids().collect();
    assert_eq!(ordered, expected);
    assert_eq!(view.soldiers_unordered().count(), 120);

    let regs: Vec<_> = view.regiments().collect();
    assert!(regs.windows(2).all(|w| w[0].id < w[1].id));
    assert_eq!(regs[0].side, 0);
    assert_eq!(regs[1].side, 1);
    assert_eq!(regs[0].soldier_count, 60);

    let first = ordered[0];
    let row = view.soldier(first).expect("lookup by id");
    assert_eq!(row.regiment, regs[0].id);
    assert!(view.soldier(SoldierId(9_999)).is_none());
    assert_eq!(view.regiment(regs[1].id).map(|r| r.side), Some(1));
}

#[test]
fn view_reflects_steps_and_tool_moves() {
    let mut world = common::world(10);
    let before = world.view().soldier(SoldierId(0)).unwrap();
    assert_eq!(before.pos, before.prev_pos, "fresh world: prev == cur");

    let delta = V2::new(S::from_i32(3), S::from_i32(-1));
    let facing = Angle::new(S::from_f32_data(1.0));
    world.debug_translate_all(delta, Some(facing));
    let moved = world.view().soldier(SoldierId(0)).unwrap();
    assert_eq!(moved.pos, before.pos + delta);
    assert_eq!(
        moved.prev_pos, before.prev_pos,
        "prev only advances at Stage 17"
    );
    assert_eq!(moved.facing, facing);
    assert_eq!(
        world.view().regiments().next().unwrap().anchor_facing,
        facing
    );

    world.step(&[]);
    let stepped = world.view().soldier(SoldierId(0)).unwrap();
    assert_eq!(
        stepped.prev_pos, stepped.pos,
        "Stage 17 copied Pos into PrevPos"
    );
    assert_eq!(world.view().tick(), world.tick());
}
