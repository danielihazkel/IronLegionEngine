//! `build_snapshot` (T1-052): interpolation, culling and side lookup against
//! a real `BattleWorld` built from the flagship content.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use glam::Vec2;
use il_core::{S, Scalar, V2};
use il_data::Registries;
use il_render::{Camera, RenderSnapshot, SnapshotInput, build_snapshot};
use il_sim_battle::{BattleSetup, BattleWorld};

const SCREEN: Vec2 = Vec2::new(1280.0, 800.0);

fn world() -> BattleWorld {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
    let regs = Arc::new(Registries::load_root(&root).expect("game content loads"));
    let setup: BattleSetup = json5::from_str(
        r#"{
          map_id: "rome:test_field",
          seed: 7,
          sides: [
            { faction: "rome:rome", player: 0, deployment_zone: 0,
              general: { unit_type: "rome:hastati", name_key: "g0" },
              regiments: [ { id: 1, unit_type: "rome:hastati", count: 6, position: [300, 150], facing_deg: 0 } ] },
            { faction: "rome:rome", player: 1, deployment_zone: 1,
              general: { unit_type: "rome:hastati", name_key: "g1" },
              regiments: [ { id: 2, unit_type: "rome:hastati", count: 6, position: [340, 150], facing_deg: 180 } ] },
          ],
        }"#,
    )
    .expect("setup parses");
    BattleWorld::new(&setup, regs).expect("world builds")
}

fn input(
    camera: Camera,
    alpha: f32,
    selected: &BTreeSet<il_core::RegimentId>,
) -> SnapshotInput<'_> {
    SnapshotInput {
        alpha,
        camera,
        screen: SCREEN,
        selected,
        corpses: &[],
    }
}

#[test]
fn snapshot_interpolates_between_prev_and_current_positions() {
    let mut world = world();
    let delta = V2::new(S::from_i32(2), S::from_i32(0));
    world.debug_translate_all(delta, None);
    let selected = BTreeSet::new();
    let camera = Camera::new(Vec2::new(320.0, 150.0));
    let view = world.view();
    let ids: Vec<_> = view.soldiers_unordered().map(|s| s.id).collect();

    let mut at0 = RenderSnapshot::default();
    build_snapshot(&view, &input(camera, 0.0, &selected), &mut at0);
    let mut at1 = RenderSnapshot::default();
    build_snapshot(&view, &input(camera, 1.0, &selected), &mut at1);
    let mut half = RenderSnapshot::default();
    build_snapshot(&view, &input(camera, 0.5, &selected), &mut half);

    assert_eq!(at0.soldiers.len(), ids.len());
    assert_eq!(at0.counts.visible_soldiers, 12);
    assert_eq!(at0.counts.soldiers, 12);
    assert_eq!(at0.counts.regiments, 2);
    for i in 0..ids.len() {
        assert!((at1.soldiers[i].pos[0] - at0.soldiers[i].pos[0] - 2.0).abs() < 1e-4);
        assert!((half.soldiers[i].pos[0] - at0.soldiers[i].pos[0] - 1.0).abs() < 1e-4);
        assert!(at0.soldiers[i].moving);
    }
    let sides: BTreeSet<u8> = at0.soldiers.iter().map(|s| s.side).collect();
    assert_eq!(sides, BTreeSet::from([0, 1]));
    assert_eq!(at0.regiments.len(), 2);
    assert!(!at0.regiments[0].selected);
}

#[test]
fn snapshot_culls_to_the_camera_and_marks_selection() {
    let world = world();
    let view = world.view();
    let mut selected = BTreeSet::new();
    selected.insert(view.regiments().next().unwrap().id);

    let mut far = RenderSnapshot::default();
    let mut camera = Camera::new(Vec2::new(5_000.0, 5_000.0));
    camera.zoom = Camera::MAX_ZOOM;
    build_snapshot(&view, &input(camera, 0.0, &selected), &mut far);
    assert_eq!(far.counts.visible_soldiers, 0);
    assert_eq!(
        far.counts.soldiers, 12,
        "counts still cover the whole battle"
    );
    assert_eq!(far.regiments.len(), 2, "regiment blocks are never culled");

    let mut near = RenderSnapshot::default();
    build_snapshot(
        &view,
        &input(Camera::new(Vec2::new(300.0, 150.0)), 0.0, &selected),
        &mut near,
    );
    assert!(near.counts.visible_soldiers >= 6);
    assert!(near.regiments[0].selected && !near.regiments[1].selected);
    assert!(near.soldiers.iter().any(|s| s.selected));
    assert!(near.soldiers.iter().all(|s| !s.moving));
}
