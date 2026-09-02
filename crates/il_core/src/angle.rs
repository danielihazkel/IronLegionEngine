//! Angles normalised to `(-π, π]` (TDD §2.2 `vec.rs` `Angle`).
//!
//! Convention: radians, counter-clockwise, `0` along `+x`. Facing index 8
//! (`to_facing8`) splits the circle into eight 45° sectors centred on the
//! multiples of 45°, so `0` is `+x`, `2` is `+y`, `4` is `-x`, `6` is `-y`.

use serde::{Deserialize, Serialize};

use crate::scalar::Scalar;
use crate::vec::Vec2;

#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(bound(serialize = "", deserialize = ""))]
#[serde(transparent)]
pub struct Angle<T: Scalar>(T);

/// Wraps `a` into `(-π, π]`.
fn normalize<T: Scalar>(a: T) -> T {
    // Subtract the whole number of turns, then fix the two boundary cases.
    let turns = ((a + T::PI) / T::TAU).floor_i32();
    let mut r = a - T::TAU * T::from_i32(turns);
    if r > T::PI {
        r = r - T::TAU;
    }
    if r <= -T::PI {
        r = r + T::TAU;
    }
    r
}

impl<T: Scalar> Angle<T> {
    pub const ZERO: Self = Self(T::ZERO);

    /// Builds an angle, normalising `radians` into `(-π, π]`.
    #[inline]
    pub fn new(radians: T) -> Self {
        Self(normalize(radians))
    }

    /// Data-side constructor from degrees; never in tick code.
    pub fn from_degrees_data(deg: f32) -> Self {
        let rad = T::from_f32_data(deg) * T::PI / T::from_i32(180);
        Self::new(rad)
    }

    /// The angle of `v`; `ZERO` for the zero vector.
    #[inline]
    pub fn from_direction(v: Vec2<T>) -> Self {
        if v.x == T::ZERO && v.y == T::ZERO {
            Self::ZERO
        } else {
            Self::new(T::atan2(v.y, v.x))
        }
    }

    #[inline]
    pub fn radians(self) -> T {
        self.0
    }

    /// Unit vector pointing along this angle.
    #[inline]
    pub fn direction(self) -> Vec2<T> {
        Vec2::new(self.0.cos(), self.0.sin())
    }

    /// Signed shortest rotation from `self` to `to`, in `(-π, π]`.
    #[inline]
    pub fn delta(self, to: Self) -> T {
        normalize(to.0 - self.0)
    }

    /// Rotates toward `to` by at most `max` radians. Lands exactly on `to`
    /// when the remaining delta is within `max`, so it never overshoots.
    #[inline]
    pub fn turn_toward(self, to: Self, max: T) -> Self {
        let d = self.delta(to);
        if d.abs() <= max {
            to
        } else {
            Self::new(self.0 + d.clamp(-max, max))
        }
    }

    /// Sector index `0..8`, 45° each, centred on the multiples of 45°.
    #[inline]
    pub fn to_facing8(self) -> u8 {
        let eighth = T::PI / T::from_i32(8);
        let quarter = T::PI / T::from_i32(4);
        let idx = ((self.0 + eighth) / quarter).floor_i32();
        idx.rem_euclid(8) as u8
    }
}

#[cfg(test)]
#[allow(clippy::float_arithmetic)]
mod tests {
    use super::*;

    type A = Angle<f32>;

    fn deg(d: i32) -> A {
        A::new(f32::PI / f32::from_i32(180) * f32::from_i32(d))
    }

    #[test]
    fn normalisation_range_and_boundaries() {
        assert_eq!(A::new(f32::PI).radians(), f32::PI);
        assert_eq!(A::new(-f32::PI).radians(), f32::PI);
        assert_eq!(A::new(f32::TAU).radians(), 0.0);
        assert_eq!(A::new(-f32::TAU).radians(), 0.0);
        assert_eq!(A::new(3.0 * f32::TAU).radians(), 0.0);
        for k in -20..=20 {
            let a = A::new(f32::from_i32(k) * 1.7);
            assert!(a.radians() > -f32::PI && a.radians() <= f32::PI, "{a:?}");
        }
    }

    #[test]
    fn to_facing8_at_every_multiple_of_45_degrees() {
        for k in 0..8 {
            assert_eq!(deg(45 * k).to_facing8(), k as u8, "{k} * 45°");
            assert_eq!(deg(45 * k + 360).to_facing8(), k as u8);
            assert_eq!(deg(45 * k - 360).to_facing8(), k as u8);
            // Interior of the sector on both sides of the centre.
            assert_eq!(deg(45 * k + 20).to_facing8(), k as u8);
            assert_eq!(deg(45 * k - 20).to_facing8(), k as u8);
        }
        assert_eq!(A::ZERO.to_facing8(), 0);
        assert_eq!(A::from_direction(Vec2::new(0.0, 1.0)).to_facing8(), 2);
        assert_eq!(A::from_direction(Vec2::new(-1.0, 0.0)).to_facing8(), 4);
        assert_eq!(A::from_direction(Vec2::new(0.0, -1.0)).to_facing8(), 6);
    }

    #[test]
    fn delta_takes_the_short_way() {
        assert!((deg(170).delta(deg(-170)) - deg(20).radians()).abs() < 1.0e-5);
        assert!((deg(-170).delta(deg(170)) + deg(20).radians()).abs() < 1.0e-5);
        assert_eq!(deg(30).delta(deg(30)), 0.0);
    }

    #[test]
    fn turn_toward_never_overshoots_and_arrives_exactly() {
        let target = deg(150);
        let step = 0.37;
        let mut a = deg(-100);
        let mut prev = a.delta(target).abs();
        for _ in 0..64 {
            a = a.turn_toward(target, step);
            let remaining = a.delta(target).abs();
            assert!(
                remaining <= prev + 1.0e-6,
                "overshoot: {remaining} > {prev}"
            );
            prev = remaining;
            if a == target {
                break;
            }
        }
        assert_eq!(a, target);
        // Turning the long way round is never chosen.
        assert_eq!(deg(170).turn_toward(deg(-170), 1.0), deg(-170));
    }

    #[test]
    fn direction_round_trip() {
        for k in 0..16 {
            let a = deg(k * 22);
            let back = A::from_direction(a.direction());
            assert!(a.delta(back).abs() < 1.0e-5, "{a:?} vs {back:?}");
        }
        assert_eq!(A::from_direction(Vec2::ZERO), A::ZERO);
    }
}
