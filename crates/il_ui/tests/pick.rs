//! Hit testing (T1-061) against a real `BattleWorld` built from the
//! flagship content: only the local player's regiments are pickable, a
//! click lands on the nearest soldier, a box takes every regiment with a
//! soldier inside it, and double-click-by-type follows the unit type.
//! `RegimentId`s are assigned in spawn order from 0 (the setup `id` is
//! only the file's label).

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use glam::Vec2;
use il_core::{PlayerId, RegimentId, Scalar, V2};
use il_sim_battle::{BattleSetup, BattleView, BattleWorld};
use il_ui::{own_regiments, owned, pick_regiment, regiments_in_box, regiments_of_type_on_screen};

const SCREEN: Vec2 = Vec2::new(1280.0, 800.0);
/// A top-down test projection: 10 px per metre, world y up, screen y down.
const PPM: f32 = 10.0;

fn project(w: V2) -> Vec2 {
    Vec2::new(
        w.x.to_f32_render() * PPM,
        SCREEN.y - w.y.to_f32_render() * PPM,
    )
}

fn world() -> BattleWorld {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../game");
    let regs = Arc::new(il_data::load_roots(&[root]).unwrap_or_else(|e| panic!("{e}")));
    let setup: BattleSetup = json5::from_str(
        r#"{
          map_id: "rome:test_field",
          seed: 7,
          sides: [
            { faction: "rome:rome", player: 0, deployment_zone: 0,
              general: { unit_type: "rome:hastati", name_key: "g0" },
              regiments: [
                { id: 1, unit_type: "rome:hastati", count: 12, position: [20, 20], facing_deg: 90 },
                { id: 2, unit_type: "rome:hastati", count: 12, position: [60, 20], facing_deg: 90 },
                { id: 3, unit_type: "rome:velites", count: 12, position: [100, 20], facing_deg: 90 },
              ] },
            { faction: "rome:rome", player: 1, deployment_zone: 1,
              general: { unit_type: "rome:hastati", name_key: "g1" },
              regiments: [
                { id: 4, unit_type: "rome:hastati", count: 12, position: [20, 60], facing_deg: 270 },
              ] },
          ],
        }"#,
    )
    .expect("setup parses");
    BattleWorld::new(&setup, regs).expect("world builds")
}

fn ids(set: &BTreeSet<RegimentId>) -> Vec<u32> {
    set.iter().map(|r| r.0).collect()
}

/// Screen position of the first soldier of `regiment`.
fn a_soldier_of(view: &BattleView, regiment: u32) -> Vec2 {
    let s = view
        .soldiers()
        .find(|s| s.regiment == RegimentId(regiment))
        .expect("regiment has soldiers");
    project(s.pos)
}

#[test]
fn only_the_local_players_regiments_are_pickable() {
    let world = world();
    let view = world.view();
    assert_eq!(ids(&own_regiments(&view, PlayerId(0))), [0, 1, 2]);
    assert_eq!(ids(&own_regiments(&view, PlayerId(1))), [3]);
    assert!(owned(&view, RegimentId(3), PlayerId(1)));
    assert!(!owned(&view, RegimentId(3), PlayerId(0)));

    let on_enemy = a_soldier_of(&view, 3);
    assert_eq!(
        pick_regiment(&view, &project, PPM, PlayerId(0), on_enemy),
        None
    );
    assert_eq!(
        pick_regiment(&view, &project, PPM, PlayerId(1), on_enemy),
        Some(RegimentId(3))
    );
}

#[test]
fn click_hits_the_nearest_soldier_within_its_circle_and_misses_open_ground() {
    let world = world();
    let view = world.view();
    let p = a_soldier_of(&view, 1);
    assert_eq!(
        pick_regiment(&view, &project, PPM, PlayerId(0), p),
        Some(RegimentId(1))
    );
    // Sprites stand on their ground point: a little above still hits.
    assert_eq!(
        pick_regiment(&view, &project, PPM, PlayerId(0), p - Vec2::new(0.0, 5.0)),
        Some(RegimentId(1))
    );
    // Far from everyone.
    assert_eq!(
        pick_regiment(&view, &project, PPM, PlayerId(0), Vec2::new(1200.0, 20.0)),
        None
    );
}

#[test]
fn box_select_takes_every_regiment_with_a_soldier_inside() {
    let world = world();
    let view = world.view();
    // Regiments 0 and 1 stand at x = 20 m and 60 m; a box over 0..80 m and
    // 0..40 m spans both but not the velites at 100 m nor the enemy at y = 60 m.
    let a = project(V2::new(S(0.0), S(40.0)));
    let b = project(V2::new(S(80.0), S(0.0)));
    assert_eq!(
        ids(&regiments_in_box(&view, &project, PlayerId(0), a, b)),
        [0, 1]
    );
    // Corner order does not matter.
    assert_eq!(
        ids(&regiments_in_box(&view, &project, PlayerId(0), b, a)),
        [0, 1]
    );
    // A taller box also covers the enemy at y = 60 m, which only player 1 gets.
    let c = project(V2::new(S(0.0), S(80.0)));
    assert_eq!(
        ids(&regiments_in_box(&view, &project, PlayerId(0), c, b)),
        [0, 1]
    );
    assert_eq!(
        ids(&regiments_in_box(&view, &project, PlayerId(1), c, b)),
        [3]
    );
}

#[test]
fn double_click_selects_the_type_on_screen() {
    let world = world();
    let view = world.view();
    let hastati = regiments_of_type_on_screen(&view, &project, PlayerId(0), RegimentId(0), SCREEN);
    assert_eq!(ids(&hastati), [0, 1]);
    let velites = regiments_of_type_on_screen(&view, &project, PlayerId(0), RegimentId(2), SCREEN);
    assert_eq!(ids(&velites), [2]);
    // A screen that ends before regiment 1 (x = 60 m = 600 px) leaves it out.
    let narrow = Vec2::new(400.0, SCREEN.y);
    let near = regiments_of_type_on_screen(&view, &project, PlayerId(0), RegimentId(0), narrow);
    assert_eq!(ids(&near), [0]);
    // Someone else's regiment yields nothing.
    assert!(
        regiments_of_type_on_screen(&view, &project, PlayerId(0), RegimentId(3), SCREEN).is_empty()
    );
}

#[allow(non_snake_case)]
fn S(v: f32) -> il_core::S {
    il_core::S::from_f32_data(v)
}
