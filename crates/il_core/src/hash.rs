//! Deterministic state hashing (TDD §2.2 `hash.rs`, REQ-SIM-005, SIM-DET-004).
//!
//! `StateHasher` wraps xxh3-64. `Hashable` feeds a value's state into it in a
//! fixed field order; floats are hashed by bit pattern. Phase 0 decision
//! (T0-012): no proc-macro derive; structs use [`impl_hashable_struct!`] and
//! field-less enums use [`impl_hashable_fieldless_enum!`], recorded in TDD §2.

use core::fmt;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::Xxh3;

use crate::angle::Angle;
use crate::ids::{ArmyId, FactionId, PlayerId, ProjectileId, ProvinceId, RegimentId, SoldierId};
use crate::scalar::Scalar;
use crate::time::{Tick, Turn};
use crate::vec::Vec2;

/// A 64-bit state hash (REQ-SIM-005).
#[derive(
    Copy, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct StateHash(pub u64);

impl fmt::Display for StateHash {
    /// Sixteen lower-case hex digits, the format `il_cli` prints.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Streaming xxh3-64 hasher over simulation state.
pub struct StateHasher {
    inner: Xxh3,
}

impl Default for StateHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StateHasher {
    pub fn new() -> Self {
        Self { inner: Xxh3::new() }
    }

    #[inline]
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.inner.update(bytes);
    }
    #[inline]
    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }
    #[inline]
    pub fn write_u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn write_i32(&mut self, v: i32) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn write_i64(&mut self, v: i64) {
        self.write_bytes(&v.to_le_bytes());
    }
    #[inline]
    pub fn write<T: Hashable + ?Sized>(&mut self, v: &T) {
        v.hash_state(self);
    }

    /// The hash of everything written so far. The hasher can keep going.
    #[inline]
    pub fn current(&self) -> StateHash {
        StateHash(self.inner.digest())
    }

    #[inline]
    pub fn finish(self) -> StateHash {
        StateHash(self.inner.digest())
    }
}

/// Feeds a value's state into a [`StateHasher`] in a fixed order.
pub trait Hashable {
    fn hash_state(&self, h: &mut StateHasher);
}

/// Hash of a single value.
pub fn hash_of<T: Hashable + ?Sized>(v: &T) -> StateHash {
    let mut h = StateHasher::new();
    v.hash_state(&mut h);
    h.finish()
}

macro_rules! hashable_int {
    ($($ty:ty => $method:ident),* $(,)?) => {$(
        impl Hashable for $ty {
            #[inline]
            fn hash_state(&self, h: &mut StateHasher) {
                h.$method(*self as _);
            }
        }
    )*};
}
hashable_int!(
    u8 => write_u8, u16 => write_u16, u32 => write_u32, u64 => write_u64,
    i8 => write_u8, i16 => write_u16, i32 => write_i32, i64 => write_i64,
);

impl Hashable for usize {
    /// Always hashed as 64 bits so the hash does not depend on pointer width.
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u64(*self as u64);
    }
}

impl Hashable for bool {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u8(u8::from(*self));
    }
}

impl Hashable for f32 {
    /// By bit pattern (SIM-DET-004): `-0.0` and `0.0` hash differently on purpose.
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u32(self.to_bits());
    }
}

macro_rules! hashable_newtype_u32 {
    ($($ty:ty),* $(,)?) => {$(
        impl Hashable for $ty {
            #[inline]
            fn hash_state(&self, h: &mut StateHasher) { h.write_u32(self.0); }
        }
    )*};
}
hashable_newtype_u32!(Tick, Turn, SoldierId, RegimentId, ProjectileId);

impl Hashable for ArmyId {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u16(self.0);
    }
}
impl Hashable for ProvinceId {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u16(self.0);
    }
}
impl Hashable for FactionId {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u8(self.0);
    }
}
impl Hashable for PlayerId {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u8(self.0);
    }
}

impl<T: Scalar> Hashable for Vec2<T> {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        self.x.hash_state(h);
        self.y.hash_state(h);
    }
}

impl<T: Scalar> Hashable for Angle<T> {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        self.radians().hash_state(h);
    }
}

impl<T: Hashable> Hashable for Option<T> {
    /// Tag byte then the payload, so `None` and `Some(0)` differ.
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        match self {
            None => h.write_u8(0),
            Some(v) => {
                h.write_u8(1);
                v.hash_state(h);
            }
        }
    }
}

impl<T: Hashable> Hashable for [T] {
    /// Length-prefixed so `[a][b]` and `[a, b]` differ.
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u32(self.len() as u32);
        for v in self {
            v.hash_state(h);
        }
    }
}

impl<T: Hashable> Hashable for Vec<T> {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        self.as_slice().hash_state(h);
    }
}

impl<T: Hashable, const N: usize> Hashable for [T; N] {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        self.as_slice().hash_state(h);
    }
}

impl<T: Hashable + ?Sized> Hashable for &T {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        (**self).hash_state(h);
    }
}

impl<T: Hashable + ?Sized> Hashable for Box<T> {
    #[inline]
    fn hash_state(&self, h: &mut StateHasher) {
        (**self).hash_state(h);
    }
}

impl Hashable for () {
    #[inline]
    fn hash_state(&self, _h: &mut StateHasher) {}
}

macro_rules! hashable_tuple {
    ($($name:ident),+) => {
        impl<$($name: Hashable),+> Hashable for ($($name,)+) {
            #[allow(non_snake_case)]
            #[inline]
            fn hash_state(&self, h: &mut StateHasher) {
                let ($($name,)+) = self;
                $($name.hash_state(h);)+
            }
        }
    };
}
hashable_tuple!(A);
hashable_tuple!(A, B);
hashable_tuple!(A, B, C);
hashable_tuple!(A, B, C, D);

/// Implements [`Hashable`] for a struct by hashing the listed fields in order.
///
/// ```
/// use il_core::{impl_hashable_struct, hash::hash_of};
/// struct Health { hp: f32, armour: u16 }
/// impl_hashable_struct!(Health { hp, armour });
/// let _ = hash_of(&Health { hp: 1.0, armour: 2 });
/// ```
///
/// Generic form: `impl_hashable_struct!(impl<T: Scalar> Body<T> { r, m });`
#[macro_export]
macro_rules! impl_hashable_struct {
    ($ty:ty { $($field:ident),* $(,)? }) => {
        impl $crate::hash::Hashable for $ty {
            #[inline]
            fn hash_state(&self, h: &mut $crate::hash::StateHasher) {
                $( $crate::hash::Hashable::hash_state(&self.$field, h); )*
            }
        }
    };
    (impl<$($g:ident : $b:path),* $(,)?> $ty:ty { $($field:ident),* $(,)? }) => {
        impl<$($g: $b),*> $crate::hash::Hashable for $ty {
            #[inline]
            fn hash_state(&self, h: &mut $crate::hash::StateHasher) {
                $( $crate::hash::Hashable::hash_state(&self.$field, h); )*
            }
        }
    };
}

/// Implements [`Hashable`] for a field-less enum by its discriminant as `u8`.
///
/// ```
/// use il_core::impl_hashable_fieldless_enum;
/// #[derive(Copy, Clone)] enum Mode { Walk, Run }
/// impl_hashable_fieldless_enum!(Mode);
/// ```
#[macro_export]
macro_rules! impl_hashable_fieldless_enum {
    ($($ty:ty),* $(,)?) => {$(
        impl $crate::hash::Hashable for $ty {
            #[inline]
            fn hash_state(&self, h: &mut $crate::hash::StateHasher) {
                h.write_u8(*self as u8);
            }
        }
    )*};
}

#[cfg(test)]
mod tests {
    use super::*;

    type V2 = crate::vec::Vec2<f32>;

    #[derive(Copy, Clone)]
    enum Mode {
        Walk,
        Run,
    }
    impl_hashable_fieldless_enum!(Mode);

    struct Fixed {
        tick: Tick,
        id: SoldierId,
        pos: V2,
        facing: Angle<f32>,
        hp: f32,
        slot: Option<u16>,
        mode: Mode,
        list: Vec<u8>,
        flag: bool,
    }
    impl_hashable_struct!(Fixed {
        tick,
        id,
        pos,
        facing,
        hp,
        slot,
        mode,
        list,
        flag
    });

    fn fixed() -> Fixed {
        Fixed {
            tick: Tick(1234),
            id: SoldierId(7),
            pos: V2::new(1.5, -2.25),
            facing: Angle::new(0.75),
            hp: 100.0,
            slot: Some(3),
            mode: Mode::Run,
            list: vec![1, 2, 3],
            flag: true,
        }
    }

    /// Golden value: any change to the hasher, the field order, or an
    /// encoding changes this constant, which is the point (T0-012).
    const GOLDEN: u64 = 0x5da1_987d_eab0_9e40;

    #[test]
    fn golden_hash_of_fixed_struct() {
        let h = hash_of(&fixed());
        assert_eq!(h, StateHash(GOLDEN), "got {h}");
        assert_eq!(format!("{h}"), format!("{GOLDEN:016x}"));
    }

    #[test]
    fn every_field_influences_the_hash() {
        let base = hash_of(&fixed());
        let mut f = fixed();
        f.tick = Tick(1235);
        assert_ne!(hash_of(&f), base);
        let mut f = fixed();
        f.pos.y = -2.0;
        assert_ne!(hash_of(&f), base);
        let mut f = fixed();
        f.slot = None;
        assert_ne!(hash_of(&f), base);
        let mut f = fixed();
        f.mode = Mode::Walk;
        assert_ne!(hash_of(&f), base);
        let mut f = fixed();
        f.list.push(4);
        assert_ne!(hash_of(&f), base);
        let mut f = fixed();
        f.flag = false;
        assert_ne!(hash_of(&f), base);
    }

    #[test]
    fn encodings_distinguish_shapes() {
        assert_ne!(hash_of(&0.0f32), hash_of(&-0.0f32));
        assert_ne!(hash_of(&None::<u32>), hash_of(&Some(0u32)));
        assert_ne!(hash_of(&vec![1u8, 2]), hash_of(&vec![1u8]));
        assert_eq!(hash_of(&[1u8, 2]), hash_of(&vec![1u8, 2]));
        assert_eq!(hash_of(&(1u8, 2u16)), {
            let mut h = StateHasher::new();
            h.write_u8(1);
            h.write_u16(2);
            h.finish()
        });
        let mut h = StateHasher::new();
        h.write(&7u32);
        let mid = h.current();
        h.write(&8u32);
        assert_ne!(mid, h.finish());
    }
}
