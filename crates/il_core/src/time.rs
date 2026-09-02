//! Tick and turn counters (TDD §2.2 `time.rs`). The sim tick is fixed at
//! 20 Hz (REQ-SIM-021); `TICK_SECONDS` is the only `f32` constant allowed
//! outside `scalar.rs` and exists for app-side accumulator arithmetic only.

use serde::{Deserialize, Serialize};

/// Sim ticks per second (REQ-SIM-021).
pub const TICKS_PER_SECOND: u32 = 20;

/// Seconds of sim time per tick. App-side only; sim code derives `dt` through
/// `Scalar::from_i32(1) / Scalar::from_i32(TICKS_PER_SECOND as i32)`.
pub const TICK_SECONDS: f32 = 0.05;

/// A battle tick. `u32` never wraps in practice (2^32 ticks is 6.8 years).
#[derive(
    Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Tick(pub u32);

impl Tick {
    pub const ZERO: Tick = Tick(0);

    /// The following tick.
    #[inline]
    #[must_use]
    pub const fn next(self) -> Tick {
        Tick(self.0 + 1)
    }

    /// Ticks elapsed since `earlier`, saturating at zero.
    #[inline]
    pub const fn since(self, earlier: Tick) -> u32 {
        self.0.saturating_sub(earlier.0)
    }
}

/// A campaign turn.
#[derive(
    Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Turn(pub u32);

impl Turn {
    pub const ZERO: Turn = Turn(0);

    #[inline]
    #[must_use]
    pub const fn next(self) -> Turn {
        Turn(self.0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_arithmetic() {
        assert_eq!(Tick::ZERO.next(), Tick(1));
        assert_eq!(Tick(10).since(Tick(4)), 6);
        assert_eq!(Tick(4).since(Tick(10)), 0);
        assert_eq!(TICKS_PER_SECOND, 20);
        assert_eq!(Turn(1).next(), Turn(2));
    }
}
