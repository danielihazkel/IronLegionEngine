//! Movement (TDD §6.2): regiment path following (T1-042), soldier steering
//! and integration (T1-043), collision (T1-044).

pub mod regiment;

pub use regiment::{
    deg_to_rad, formation_width, mode_speed, regiment_follow_path, slope_mult, tick_dt,
    zone_move_mult,
};
