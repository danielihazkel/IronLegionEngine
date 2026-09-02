//! Two-dimensional vector over any [`Scalar`] (TDD §2.2 `vec.rs`).

use core::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

use crate::angle::Angle;
use crate::scalar::{S, Scalar};

#[derive(Copy, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(bound(serialize = "", deserialize = ""))]
pub struct Vec2<T: Scalar> {
    pub x: T,
    pub y: T,
}

/// The simulation vector type.
pub type V2 = Vec2<S>;

impl<T: Scalar> Vec2<T> {
    pub const ZERO: Self = Self {
        x: T::ZERO,
        y: T::ZERO,
    };

    #[inline]
    pub const fn new(x: T, y: T) -> Self {
        Self { x, y }
    }

    /// Data-side constructor; never in tick code.
    #[inline]
    pub fn from_f32_data(x: f32, y: f32) -> Self {
        Self::new(T::from_f32_data(x), T::from_f32_data(y))
    }

    #[inline]
    pub fn dot(self, o: Self) -> T {
        self.x * o.x + self.y * o.y
    }

    #[inline]
    pub fn length_sq(self) -> T {
        self.dot(self)
    }

    #[inline]
    pub fn length(self) -> T {
        self.length_sq().sqrt()
    }

    /// Unit vector in the same direction, or zero if the length is zero.
    #[inline]
    pub fn normalized_or_zero(self) -> Self {
        let len = self.length();
        if len > T::ZERO {
            Self::new(self.x / len, self.y / len)
        } else {
            Self::ZERO
        }
    }

    /// Scales the vector down so its length does not exceed `max`.
    #[inline]
    pub fn clamp_length(self, max: T) -> Self {
        let len_sq = self.length_sq();
        if len_sq > max * max {
            let len = len_sq.sqrt();
            Self::new(self.x / len * max, self.y / len * max)
        } else {
            self
        }
    }

    /// Rotates counter-clockwise by `angle` radians. The angle is normalised
    /// to `(-π, π]` first, so rotating by exactly `TAU` is the identity
    /// (`cos 0 = 1`, `sin 0 = 0`) and returns the input bit for bit.
    #[inline]
    pub fn rotate(self, angle: T) -> Self {
        let a = Angle::new(angle).radians();
        let (s, c) = (a.sin(), a.cos());
        Self::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }

    /// The vector rotated 90° counter-clockwise.
    #[inline]
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    #[inline]
    pub fn distance(self, o: Self) -> T {
        (o - self).length()
    }

    #[inline]
    pub fn distance_sq(self, o: Self) -> T {
        (o - self).length_sq()
    }

    #[inline]
    pub fn lerp(self, o: Self, t: T) -> Self {
        self + (o - self) * t
    }
}

impl<T: Scalar> Add for Vec2<T> {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}

impl<T: Scalar> Sub for Vec2<T> {
    type Output = Self;
    #[inline]
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}

impl<T: Scalar> Mul<T> for Vec2<T> {
    type Output = Self;
    #[inline]
    fn mul(self, k: T) -> Self {
        Self::new(self.x * k, self.y * k)
    }
}

impl<T: Scalar> Neg for Vec2<T> {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}

impl<T: Scalar> AddAssign for Vec2<T> {
    #[inline]
    fn add_assign(&mut self, o: Self) {
        *self = *self + o;
    }
}

impl<T: Scalar> SubAssign for Vec2<T> {
    #[inline]
    fn sub_assign(&mut self, o: Self) {
        *self = *self - o;
    }
}

#[cfg(test)]
#[allow(clippy::float_arithmetic)]
mod tests {
    use super::*;

    /// Tests use the bare float so literals stay readable.
    type V2 = Vec2<f32>;

    #[test]
    fn length_and_normalisation() {
        let v = V2::new(3.0, 4.0);
        assert_eq!(v.length(), 5.0);
        assert_eq!(v.length_sq(), 25.0);
        assert_eq!(v.normalized_or_zero(), V2::new(0.6, 0.8));
        assert_eq!(V2::ZERO.normalized_or_zero(), V2::ZERO);
        assert_eq!(v.clamp_length(10.0), v);
        assert_eq!(v.clamp_length(2.5).length_sq(), 6.25);
    }

    #[test]
    fn dot_perp_and_ops() {
        let a = V2::new(1.0, 2.0);
        let b = V2::new(3.0, -1.0);
        assert_eq!(a.dot(b), 1.0);
        assert_eq!(a.perp(), V2::new(-2.0, 1.0));
        assert_eq!(a.perp().dot(a), 0.0);
        assert_eq!(a + b, V2::new(4.0, 1.0));
        assert_eq!(a - b, V2::new(-2.0, 3.0));
        assert_eq!(a * 2.0, V2::new(2.0, 4.0));
        assert_eq!(-a, V2::new(-1.0, -2.0));
        assert_eq!(a.lerp(b, 0.5), V2::new(2.0, 0.5));
        assert_eq!(V2::ZERO.distance(V2::new(0.0, 7.0)), 7.0);
    }

    #[test]
    fn rotate_by_tau_is_bit_exact_identity() {
        let inputs = [
            V2::new(1.0, 0.0),
            V2::new(-3.25, 7.5),
            V2::new(1.0e-3, -2.0e5),
            V2::new(0.1, 0.7),
        ];
        for v in inputs {
            let r = v.rotate(f32::TAU);
            assert_eq!(r.x.to_bits(), v.x.to_bits(), "x of {v:?}");
            assert_eq!(r.y.to_bits(), v.y.to_bits(), "y of {v:?}");
            let r = v.rotate(-f32::TAU);
            assert_eq!(r.x.to_bits(), v.x.to_bits());
            assert_eq!(r.y.to_bits(), v.y.to_bits());
        }
    }

    #[test]
    fn rotate_quarter_turn() {
        let r = V2::new(1.0, 0.0).rotate(f32::PI / 2.0);
        assert!((r.x).abs() < 1.0e-6);
        assert!((r.y - 1.0).abs() < 1.0e-6);
    }
}
