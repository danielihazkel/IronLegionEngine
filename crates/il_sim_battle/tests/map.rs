//! T1-030: `LoadedMap` against a hand-written map (`tests/maps/tiny`):
//! `height_at` and `zone_at` match hand-computed samples, rivers and
//! crossings rasterise as specified, and the flagship test map loads.

use std::path::Path;

use il_core::{S, Scalar, V2};
use il_data::{ContentId, Registries};
use il_sim_battle::LoadedMap;

fn regs_with_tiny() -> Registries {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    Registries::load_roots(&[root.join("game"), root.join("tests/maps")])
        .unwrap_or_else(|d| panic!("{d}"))
}

fn load(regs: &Registries, id: &str) -> LoadedMap {
    let h = regs
        .maps
        .lookup(&ContentId::new(id).unwrap())
        .unwrap_or_else(|| panic!("{id} registered"));
    LoadedMap::from_def(regs.maps.get(h), regs.rules.movement.zone_cell).unwrap()
}

fn v(x: f32, y: f32) -> V2 {
    V2::from_f32_data(x, y)
}

fn s(x: f32) -> S {
    S::from_f32_data(x)
}

#[test]
fn heights_are_bilinear_and_clamped() {
    let regs = regs_with_tiny();
    let m = load(&regs, "tiny:tiny");
    assert_eq!((m.height_cols, m.height_rows), (3, 3));
    assert_eq!(m.height_cell, s(4.0));
    // The sidecar encodes h(x, y) = x / 4 + y.
    assert_eq!(m.height_at(v(0.0, 0.0)), s(0.0));
    assert_eq!(m.height_at(v(4.0, 4.0)), s(5.0));
    assert_eq!(m.height_at(v(8.0, 8.0)), s(10.0));
    assert_eq!(m.height_at(v(1.0, 2.0)), s(2.25));
    assert_eq!(m.height_at(v(6.0, 2.0)), s(3.5));
    assert_eq!(m.height_at(v(3.0, 7.0)), s(7.75));
    // Outside the map: the nearest edge.
    assert_eq!(m.height_at(v(-3.0, 20.0)), s(8.0));
    assert!(m.in_bounds(v(8.0, 8.0)));
    assert!(!m.in_bounds(v(8.1, 8.0)));
}

#[test]
fn zones_rivers_and_crossings_rasterise_at_cell_centres() {
    let regs = regs_with_tiny();
    let m = load(&regs, "tiny:tiny");
    assert_eq!(m.zone_cell, s(2.0));
    assert_eq!((m.zone_cols, m.zone_rows), (4, 4));
    let name = |p: V2| -> String {
        regs.zones
            .id_of(m.zone_at(p).expect("real maps always have a zone"))
            .as_str()
            .to_string()
    };
    // Forest quarter, open ground, rock below the diagonal, ford on top of
    // the rock and the river, open river bank.
    assert_eq!(name(v(1.0, 1.0)), "rome:forest");
    assert_eq!(name(v(3.0, 3.0)), "rome:forest");
    assert_eq!(name(v(1.0, 3.0)), "rome:forest");
    assert_eq!(name(v(5.0, 1.0)), "rome:open");
    assert_eq!(name(v(5.0, 3.0)), "rome:open");
    assert_eq!(name(v(1.0, 5.0)), "rome:open");
    assert_eq!(name(v(5.0, 5.0)), "rome:rock");
    assert_eq!(name(v(7.0, 5.0)), "rome:ford");
    assert_eq!(name(v(7.0, 7.0)), "rome:ford");
    assert_eq!(name(v(5.0, 7.0)), "rome:open");
    // River y in [4.75, 7.25]: rows with centres 5 and 7, not 1 and 3.
    assert!(m.river_at(v(1.0, 5.0)));
    assert!(m.river_at(v(5.0, 5.0)));
    assert!(m.river_at(v(7.0, 7.0)));
    assert!(m.river_at(v(5.0, 7.0)));
    assert!(!m.river_at(v(1.0, 3.0)));
    assert!(!m.river_at(v(1.0, 1.0)));
    // Crossing flags come from the zone types.
    assert!(regs.zones.get(m.zone_at(v(7.0, 7.0)).unwrap()).crossing);
    assert!(!regs.zones.get(m.zone_at(v(5.0, 7.0)).unwrap()).crossing);
    assert!(!regs.zones.get(m.zone_at(v(5.0, 5.0)).unwrap()).passable);
    assert_eq!(m.zone_handles.len(), 4);
    assert_eq!(m.deployment_polygon(0).map(<[V2]>::len), Some(4));
    assert_eq!(m.deployment_polygon(1), None);
    assert!(m.structures.is_empty() && m.siege_points.is_empty());
}

#[test]
fn the_flagship_test_map_loads_with_its_features() {
    let regs = regs_with_tiny();
    let m = load(&regs, "rome:test_field");
    assert_eq!((m.width, m.height), (s(800.0), s(600.0)));
    assert_eq!((m.height_cols, m.height_rows), (201, 151));
    assert_eq!((m.zone_cols, m.zone_rows), (400, 300));
    let name = |p: V2| regs.zones.id_of(m.zone_at(p).unwrap()).as_str().to_string();
    assert_eq!(name(v(300.0, 150.0)), "rome:open");
    assert_eq!(name(v(150.0, 450.0)), "rome:forest");
    assert_eq!(name(v(600.0, 500.0)), "rome:rock");
    assert_eq!(name(v(399.0, 100.0)), "rome:road");
    // The river is impassable except at the bridge and the ford.
    assert!(m.river_at(v(100.0, 295.0)));
    assert_eq!(name(v(100.0, 295.0)), "rome:open");
    assert!(m.river_at(v(399.0, 310.0)));
    assert_eq!(name(v(399.0, 310.0)), "rome:bridge");
    assert!(m.river_at(v(650.0, 297.0)));
    assert_eq!(name(v(650.0, 297.0)), "rome:ford");
    assert!(!m.river_at(v(300.0, 150.0)));
    // The river bed is (nearly) flat and the hill rises above it.
    assert!(m.height_at(v(400.0, 310.0)) < s(0.1));
    assert!(m.height_at(v(600.0, 460.0)) > s(15.0));
    assert!(m.deployment_polygon(0).is_some() && m.deployment_polygon(1).is_some());
}
