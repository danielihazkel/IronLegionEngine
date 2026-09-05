//! Fatigue and morale (TDD §8.3): Stage 13 fatigue accumulation and the
//! ten-tick regiment mean (T2-040); Stage 14 morale factors, shocks and the
//! state machine (T2-041); routing, rally and shatter (T2-042); the general
//! (T2-043). `fatigue` holds the pure rule functions and the systems.

pub mod fatigue;

pub use fatigue::{
    Activity, FATIGUE_MEAN_PERIOD, FatigueState, activity, fatigue_rate, fatigue_state,
    fatigue_tick, regiment_fatigue_mean, weather_fatigue_mult,
};
