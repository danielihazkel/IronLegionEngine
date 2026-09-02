//! T0-020 / T0-021 done-criterion: loading the flagship game's units gives
//! handles that `Registry::get` resolves, with the §15.2 values.

use std::path::Path;

use il_core::{S, Scalar};
use il_data::{ContentId, Registries, UnitCategory};

fn game_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game")
}

#[test]
fn game_units_load_and_resolve() {
    let regs = Registries::load_root(&game_root()).unwrap_or_else(|d| panic!("{d}"));
    let id = ContentId::new("rome:hastati").unwrap();
    let h = regs.units.lookup(&id).expect("rome:hastati registered");
    let u = regs.units.get(h);
    assert_eq!(u.id, id);
    assert_eq!(regs.units.id_of(h), &id);
    assert_eq!(u.category, UnitCategory::Infantry);
    assert_eq!(u.soldier_radius, S::from_f32_data(0.4));
    assert_eq!(u.mass, S::from_f32_data(80.0));
    assert_eq!(u.hp, S::from_f32_data(100.0));
    assert_eq!(u.speed_walk, S::from_f32_data(1.6));
    assert_eq!(u.speed_run, S::from_f32_data(4.0));
    assert_eq!(u.speed_march, S::from_f32_data(1.6));
    assert_eq!(u.morale_base, S::from_f32_data(60.0));
    assert_eq!(u.los_radius, S::from_f32_data(200.0));
    assert_eq!(u.name_key, "rome.units.hastati.name");
}
