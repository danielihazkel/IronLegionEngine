//! The `Scalar` abstraction over simulation arithmetic (TDD §2.2 `scalar.rs`,
//! REQ-TECH-009, ADR-003).
//!
//! This is the only module in the workspace where float arithmetic is
//! permitted; everything else goes through the trait so that a fixed-point
//! implementation (`Fixed32`, Phase 7) can be substituted by changing `S`.
//!
//! `S` is the newtype [`F32`] rather than a bare `f32` alias: clippy's
//! `float_arithmetic` lint looks through type aliases, so an alias would
//! either fire on every sim expression or force the lint off. The newtype
//! keeps the lint on and makes mixing a raw `f32` into sim maths a type
//! error. (Deviation from the TDD text, recorded in TDD §2 by T0-052.)
//!
//! Determinism note (REQ-PLAT-003): `f32::sin`, `cos`, `atan2` and `sqrt`
//! call the platform maths library. Results are stable across machines that
//! run the same build on the same OS, which is the MVP contract.
#![allow(clippy::float_arithmetic)]

use core::fmt::Debug;
use core::ops::{Add, Div, Mul, Neg, Sub};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::hash::{Hashable, StateHasher};

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
    + Hashable
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
    /// method of the same name. `f32::mul_add` is banned in sim crates by
    /// clippy (`disallowed-methods`).
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
        // Truncate, then step down for negative fractions. Same result as
        // `f32::floor(self) as i32` for every finite input (saturating like
        // the cast beyond the i32 range) without the libm `floorf` call the
        // baseline x86-64 target makes, which dominated grid rebuilds.
        let t = self as i32;
        // Branch-free so bucketing loops vectorise; saturating at i32::MIN.
        t.saturating_sub(i32::from((t as f32) > self))
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

/// The MVP simulation scalar: a transparent newtype over `f32` (see the
/// module docs for why it is not a bare alias). Serialises as a plain number.
#[derive(Copy, Clone, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct F32(f32);

impl F32 {
    /// Bit pattern, for hashing and golden tests.
    #[inline]
    pub const fn to_bits(self) -> u32 {
        self.0.to_bits()
    }

    #[inline]
    pub const fn from_bits(bits: u32) -> Self {
        Self(f32::from_bits(bits))
    }
}

impl Debug for F32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Debug::fmt(&self.0, f)
    }
}

impl core::fmt::Display for F32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Display::fmt(&self.0, f)
    }
}

impl Add for F32 {
    type Output = Self;
    #[inline]
    fn add(self, o: Self) -> Self {
        Self(self.0 + o.0)
    }
}
impl Sub for F32 {
    type Output = Self;
    #[inline]
    fn sub(self, o: Self) -> Self {
        Self(self.0 - o.0)
    }
}
impl Mul for F32 {
    type Output = Self;
    #[inline]
    fn mul(self, o: Self) -> Self {
        Self(self.0 * o.0)
    }
}
impl Div for F32 {
    type Output = Self;
    #[inline]
    fn div(self, o: Self) -> Self {
        Self(self.0 / o.0)
    }
}
impl Neg for F32 {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self {
        Self(-self.0)
    }
}

impl Hashable for F32 {
    /// By bit pattern (SIM-DET-004).
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u32(self.0.to_bits());
    }
}

impl Scalar for F32 {
    const ZERO: Self = Self(0.0);
    const ONE: Self = Self(1.0);
    const HALF: Self = Self(0.5);
    const PI: Self = Self(core::f32::consts::PI);
    const TAU: Self = Self(core::f32::consts::TAU);

    #[inline]
    fn from_i32(v: i32) -> Self {
        Self(v as f32)
    }
    #[inline]
    fn from_f32_data(v: f32) -> Self {
        Self(v)
    }
    #[inline]
    fn to_f32_render(self) -> f32 {
        self.0
    }
    #[inline]
    fn sqrt(self) -> Self {
        Self(Scalar::sqrt(self.0))
    }
    #[inline]
    fn sin(self) -> Self {
        Self(Scalar::sin(self.0))
    }
    #[inline]
    fn cos(self) -> Self {
        Self(Scalar::cos(self.0))
    }
    #[inline]
    fn atan2(y: Self, x: Self) -> Self {
        Self(<f32 as Scalar>::atan2(y.0, x.0))
    }
    #[inline]
    fn abs(self) -> Self {
        Self(Scalar::abs(self.0))
    }
    #[inline]
    fn min(self, o: Self) -> Self {
        Self(Scalar::min(self.0, o.0))
    }
    #[inline]
    fn max(self, o: Self) -> Self {
        Self(Scalar::max(self.0, o.0))
    }
    #[inline]
    fn clamp(self, lo: Self, hi: Self) -> Self {
        Self(Scalar::clamp(self.0, lo.0, hi.0))
    }
    #[inline]
    fn floor_i32(self) -> i32 {
        Scalar::floor_i32(self.0)
    }
    #[inline]
    fn mul_add_rounded(self, a: Self, b: Self) -> Self {
        Self(Scalar::mul_add_rounded(self.0, a.0, b.0))
    }
    #[inline]
    fn is_finite(self) -> bool {
        Scalar::is_finite(self.0)
    }
}

/// The simulation scalar. `F32` for MVP; `Fixed32` behind the `fixed`
/// feature in Phase 7 (TDD §1.1).
pub type S = F32;

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
            let s = F32(c).mul_add_rounded(F32(a), F32(b));
            assert_eq!(s.to_bits(), expected.to_bits());
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
        assert_eq!((-2.0f32).floor_i32(), -2);
        assert_eq!((-0.5f32).floor_i32(), -1);
        assert_eq!(0.0f32.floor_i32(), 0);
        assert_eq!((-0.0f32).floor_i32(), 0);
        assert_eq!(3e9f32.floor_i32(), i32::MAX);
        assert_eq!((-3e9f32).floor_i32(), i32::MIN);
        for k in -2000..2000 {
            let v = k as f32 * 0.37;
            assert_eq!(v.floor_i32(), f32::floor(v) as i32, "{v}");
        }
        assert_eq!(Scalar::clamp(5.0f32, 0.0, 1.0), 1.0);
        assert_eq!(Scalar::clamp(-5.0f32, 0.0, 1.0), 0.0);
        assert_eq!(Scalar::min(1.0f32, 2.0), 1.0);
        assert_eq!(Scalar::max(1.0f32, 2.0), 2.0);
        assert_eq!(Scalar::sqrt(16.0f32), 4.0);
    }

    #[test]
    fn newtype_matches_f32_bit_for_bit() {
        let a = S::from_f32_data(1.7);
        let b = S::from_i32(3);
        assert_eq!((a * b + S::HALF).to_bits(), (1.7f32 * 3.0 + 0.5).to_bits());
        assert_eq!((a / b - S::ONE).to_bits(), (1.7f32 / 3.0 - 1.0).to_bits());
        assert_eq!((-a).to_bits(), (-1.7f32).to_bits());
        assert_eq!(S::from_i32(16).sqrt(), S::from_i32(4));
        assert_eq!(S::from_f32_data(2.7).floor_i32(), 2);
        assert_eq!(Scalar::clamp(S::from_i32(5), S::ZERO, S::ONE), S::ONE);
        assert_eq!(S::PI.to_f32_render(), core::f32::consts::PI);
        assert!(S::from_i32(1) < S::from_i32(2));
        assert_eq!(
            serde_json::to_string(&S::from_f32_data(1.5)).unwrap(),
            "1.5"
        );
        assert_eq!(serde_json::from_str::<S>("2").unwrap(), S::from_i32(2));
        assert_eq!(format!("{:?}", S::from_f32_data(0.25)), "0.25");
    }
}
