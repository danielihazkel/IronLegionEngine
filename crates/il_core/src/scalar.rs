//! The `Scalar` abstraction over simulation arithmetic (TDD §2.2 `scalar.rs`,
//! REQ-TECH-009, ADR-003).
//!
//! This is the only module in the workspace where float arithmetic is
//! permitted; everything else goes through the trait so that a fixed-point
//! implementation (`Fixed32`, Phase 7) can be substituted by changing `S`.
//!
//! Determinism note (REQ-PLAT-003): `f32::sin`, `cos`, `atan2` and `sqrt`
//! call the platform maths library. Results are stable across machines that
//! run the same build on the same OS, which is the MVP contract.
#![allow(clippy::float_arithmetic)]

use core::fmt::Debug;
use core::ops::{Add, Div, Mul, Neg, Sub};

use serde::{Serialize, de::DeserializeOwned};

/// Simulation number type. See the module docs.
pub trait Scalar:
    Copy
    + PartialOrd
    + PartialEq
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
    + Default
    + Debug
    + Serialize
    + DeserializeOwned
    + crate::hash::Hashable
    + Send
    + Sync
    + 'static
{
    const ZERO: Self;
    const ONE: Self;
    const HALF: Self;
    const PI: Self;
    const TAU: Self;

    /// Exact conversion from a small integer. The sanctioned way to write a
    /// numeric constant in sim code.
    fn from_i32(v: i32) -> Self;

    /// Conversion from content data. Data loading only; never in tick code.
    fn from_f32_data(v: f32) -> Self;

    /// Conversion for the renderer and UI. Never feeds back into the sim.
    fn to_f32_render(self) -> f32;

    fn sqrt(self) -> Self;
    fn sin(self) -> Self;
    fn cos(self) -> Self;
    fn atan2(y: Self, x: Self) -> Self;
    fn abs(self) -> Self;
    fn min(self, o: Self) -> Self;
    fn max(self, o: Self) -> Self;
    fn clamp(self, lo: Self, hi: Self) -> Self;
    /// Largest integer not greater than `self`.
    fn floor_i32(self) -> i32;

    /// `a * b + self`, computed as two rounded operations. Never a fused
    /// multiply-add: the fused form rounds once and would differ between
    /// CPUs that do and do not have FMA (REQ-TECH-010).
    ///
    /// Named `mul_add_rounded` rather than the TDD's `mul_add` because the
    /// inherent `f32::mul_add` is the fused form and would shadow a trait
    /// method of the same name on the concrete `S = f32`. `f32::mul_add` is
    /// banned in sim crates by clippy (`disallowed-methods`).
    fn mul_add_rounded(self, a: Self, b: Self) -> Self;

    /// `true` for finite values; used by debug assertions only.
    fn is_finite(self) -> bool;
}

impl Scalar for f32 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    const HALF: Self = 0.5;
    const PI: Self = core::f32::consts::PI;
    const TAU: Self = core::f32::consts::TAU;

    #[inline]
    fn from_i32(v: i32) -> Self {
        v as f32
    }
    #[inline]
    fn from_f32_data(v: f32) -> Self {
        v
    }
    #[inline]
    fn to_f32_render(self) -> f32 {
        self
    }
    #[inline]
    fn sqrt(self) -> Self {
        f32::sqrt(self)
    }
    #[inline]
    fn sin(self) -> Self {
        f32::sin(self)
    }
    #[inline]
    fn cos(self) -> Self {
        f32::cos(self)
    }
    #[inline]
    fn atan2(y: Self, x: Self) -> Self {
        f32::atan2(y, x)
    }
    #[inline]
    fn abs(self) -> Self {
        f32::abs(self)
    }
    #[inline]
    fn min(self, o: Self) -> Self {
        // Explicit comparison rather than `f32::min` so NaN handling is ours.
        if o < self { o } else { self }
    }
    #[inline]
    fn max(self, o: Self) -> Self {
        if o > self { o } else { self }
    }
    #[inline]
    fn clamp(self, lo: Self, hi: Self) -> Self {
        Scalar::min(Scalar::max(self, lo), hi)
    }
    #[inline]
    fn floor_i32(self) -> i32 {
        f32::floor(self) as i32
    }
    #[inline]
    fn mul_add_rounded(self, a: Self, b: Self) -> Self {
        // Two separate operations on purpose; see the trait docs.
        let product = a * b;
        product + self
    }
    #[inline]
    fn is_finite(self) -> bool {
        f32::is_finite(self)
    }
}

/// The simulation scalar. `f32` for MVP; `Fixed32` behind the `fixed`
/// feature in Phase 7 (TDD §1.1).
pub type S = f32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_add_is_two_rounded_operations() {
        // Values chosen so that a fused multiply-add would round differently.
        let cases: [(f32, f32, f32); 4] = [
            (1.000_000_1, 3.0, 1.0e-8),
            (0.1, 0.2, 0.3),
            (1.0e7, 1.0e-7, 1.0),
            (-2.5, 4.0, 10.000_001),
        ];
        for (a, b, c) in cases {
            let expected = {
                let p = a * b;
                p + c
            };
            assert_eq!(c.mul_add_rounded(a, b).to_bits(), expected.to_bits());
        }
    }

    #[test]
    fn constants_and_conversions() {
        assert_eq!(f32::from_i32(-7), -7.0);
        assert_eq!(f32::from_f32_data(1.5), 1.5);
        assert_eq!(1.5f32.to_f32_render(), 1.5);
        assert_eq!(f32::TAU, 2.0 * f32::PI);
        assert_eq!(f32::HALF + f32::HALF, f32::ONE);
        assert_eq!(2.7f32.floor_i32(), 2);
        assert_eq!((-2.3f32).floor_i32(), -3);
        assert_eq!(Scalar::clamp(5.0f32, 0.0, 1.0), 1.0);
        assert_eq!(Scalar::clamp(-5.0f32, 0.0, 1.0), 0.0);
        assert_eq!(Scalar::min(1.0f32, 2.0), 1.0);
        assert_eq!(Scalar::max(1.0f32, 2.0), 2.0);
        assert_eq!(Scalar::sqrt(16.0f32), 4.0);
    }
}
