//! `BattleFrame` (T3-032): an owned capture equals the live view it was
//! taken from, reuses its buffers when refilled, and never touches the
//! state hash.

mod common;

use std::sync::Arc;

use il_sim_battle::{BattleFrame, BattleWorld, FrameDetail, FrameStatics, Scenario};

const AI_SKIRMISH: &str = include_str!("../../../tests/scenarios/ai_skirmish_300.json5");

fn skirmish_at(ticks: u32) -> BattleWorld {
    let s: Scenario = json5::from_str(AI_SKIRMISH).expect("scenario parses");
    let mut w = BattleWorld::new(&s.setup, common::regs()).unwrap();
    for _ in 0..ticks {
        w.step(&[]);
    }
    w
}

#[test]
fn capture_matches_the_view() {
    let w = skirmish_at(200);
    let f = BattleFrame::capture(&w, FrameDetail::ALL);
    let v = w.view();
    assert_eq!((f.tick(), f.phase()), (v.tick(), v.phase()));
    assert!(Arc::ptr_eq(f.regs(), v.regs()));
    assert_eq!(f.sides(), v.sides());
    assert_eq!(f.soldier_count(), v.soldier_count());
    assert!(f.soldiers_unordered().eq(v.soldiers_unordered()));
    assert!(f.regiments().eq(v.regiments()));
    assert!(f.projectiles().eq(v.projectiles()));
    assert_eq!(f.nav_grid(), v.nav_grid());
    for (s, side) in v.sides().iter().enumerate() {
        let s = s as u8;
        assert_eq!(f.flow_field(s), v.flow_field(s));
        assert_eq!(f.ai_plan(s), v.ai_plan(s));
        assert_eq!(
            f.general(s),
            side.general.and_then(|id| v.soldier(id)),
            "side {s}"
        );
    }
    for r in v.regiments() {
        let id = r.id;
        assert_eq!(f.regiment(id), Some(r));
        assert_eq!(f.regiment_units(id), v.regiment_units(id));
        assert_eq!(f.regiment_formations(id), v.regiment_formations(id));
        assert_eq!(
            f.regiment_living_by_group(id),
            v.regiment_living_by_group(id)
        );
        assert_eq!(f.abilities(id), v.abilities(id));
        assert_eq!(f.statuses(id), v.statuses(id));
        assert_eq!(f.ammo(id), v.ammo(id));
        assert_eq!(f.los_radius(id), v.los_radius(id));
        assert_eq!(f.formation_state(id), v.formation_state(id));
        assert_eq!(f.path(id), v.path(id));
        for s in 0..v.sides().len() as u8 {
            assert_eq!(
                f.visible(s, id),
                v.visible(s, id),
                "side {s} regiment {id:?}"
            );
            assert_eq!(f.seen(s, id), v.seen(s, id));
        }
    }
    assert!(f.regiment(il_core::RegimentId(9_999)).is_none());
    assert_eq!(f.setup(), w.setup());
    assert_eq!(
        f.spatial_grid().map(|g| g.cell()),
        Some(v.spatial_grid().cell())
    );
}

#[test]
fn the_overlay_parts_are_captured_only_when_asked() {
    let w = skirmish_at(50);
    let f = BattleFrame::capture(&w, FrameDetail::default());
    let id = w.view().regiments().next().unwrap().id;
    assert!(f.formation_state(id).is_none());
    assert!(f.path(id).is_none());
    assert!(f.spatial_grid().is_none());
    assert!(f.ai_plan(0).is_none());
    // The rest is there regardless.
    assert_eq!(f.regiment_units(id), w.view().regiment_units(id));
}

#[test]
fn capture_into_reuses_its_buffers_and_leaves_the_hash_alone() {
    let mut w = skirmish_at(100);
    let statics = Arc::new(FrameStatics::of(&w));
    let mut f = BattleFrame::new(w.registries().clone(), statics.clone());
    f.capture_into(&w.view(), &statics, FrameDetail::ALL);
    let first = f.soldiers_unordered().next().map(|s| s.id);
    let before = w.recompute_hash();
    for _ in 0..5 {
        w.step(&[]);
        f.capture_into(&w.view(), &statics, FrameDetail::ALL);
    }
    assert_eq!(f.tick(), w.tick());
    assert!(f.soldiers_unordered().eq(w.view().soldiers_unordered()));
    assert!(first.is_some());
    // Capturing again changes nothing the hash sees.
    let h = w.recompute_hash();
    f.capture_into(&w.view(), &statics, FrameDetail::ALL);
    assert_eq!(w.recompute_hash(), h);
    assert_ne!(before, h, "the world moved on");
    assert!(Arc::ptr_eq(f.statics(), &statics));
}
