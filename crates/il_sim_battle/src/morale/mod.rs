//! Fatigue and morale (TDD §8.3): Stage 13 fatigue accumulation and the
//! ten-tick regiment mean (T2-040); Stage 14 morale factors, shocks and the
//! state machine (T2-041); routing, rally and shatter (T2-042); the general
//! (T2-043). `fatigue` and `factors` hold the pure rule functions, `tick`
//! the Stage 14 system.

pub mod factors;
pub mod fatigue;
pub mod rout;
pub mod tick;

pub use factors::{
    FACTORS, MoraleInputs, morale_delta, morale_factors, morale_state, sat, shock_amount,
};
pub use fatigue::{
    Activity, FATIGUE_MEAN_PERIOD, FatigueState, activity, fatigue_rate, fatigue_state,
    fatigue_tick, regiment_fatigue_mean, weather_fatigue_mult,
};
pub use rout::{enter_routing, follow_centroid, try_rally};
pub use tick::morale_tick;
