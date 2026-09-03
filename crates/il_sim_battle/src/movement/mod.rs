//! Movement (TDD §6.2): regiment path following (T1-042), soldier steering
//! and integration (T1-043), collision (T1-044).

pub mod collision;
pub mod integrate;
pub mod regiment;
pub mod steer;

pub use collision::{Disc, accumulate_pushes, collision_resolve, pair_push};
pub use integrate::{integrate, push_out};
pub use regiment::{
    deg_to_rad, formation_width, mode_speed, regiment_follow_path, slope_mult, tick_dt,
    zone_move_mult,
};
pub use steer::{seek_velocity, soldier_steer};
