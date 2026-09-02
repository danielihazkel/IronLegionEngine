//! Seeded randomness (TDD §2.2 `rng.rs`, REQ-SIM-004, SIM-DET-001, SIM-DET-002).
//!
//! Two forms:
//! - [`RngStream`]: a PCG32 sequential generator, one per system, seeded from
//!   the battle seed and a [`StreamId`]. Used only by single-threaded code
//!   whose draw order is fixed.
//! - [`hash_draw`]: a stateless draw keyed by `(seed, tick, entity, index)`,
//!   for per-entity randomness inside parallel systems where iteration order
//!   must not matter.

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

use crate::hash::{Hashable, StateHasher};
use crate::scalar::Scalar;
use crate::time::Tick;

/// One stream per system (SIM-DET-001; `Campaign` added by the TDD).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum StreamId {
    CombatMelee = 0,
    CombatRanged = 1,
    Morale = 2,
    AiRegiment = 3,
    AiArmy = 4,
    Abilities = 5,
    Deployment = 6,
    Weather = 7,
    Campaign = 8,
}

impl StreamId {
    pub const COUNT: usize = 9;
    pub const ALL: [StreamId; Self::COUNT] = [
        StreamId::CombatMelee,
        StreamId::CombatRanged,
        StreamId::Morale,
        StreamId::AiRegiment,
        StreamId::AiArmy,
        StreamId::Abilities,
        StreamId::Deployment,
        StreamId::Weather,
        StreamId::Campaign,
    ];

    #[inline]
    pub const fn index(self) -> usize {
        self as usize
    }
}

const PCG_MULT: u64 = 6_364_136_223_846_793_005;

/// PCG32 (XSH RR) with 64-bit state and a per-stream increment.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RngStream {
    state: u64,
    inc: u64,
}

impl RngStream {
    /// Seeds the stream as `hash(seed, stream_id)` (SIM-DET-001): the initial
    /// state is xxh3 of the stream id keyed by the battle seed, and the PCG
    /// sequence selector is the stream id, so streams never overlap.
    pub fn from_seed(seed: u64, stream: StreamId) -> Self {
        let initstate = xxh3_64_with_seed(&[stream as u8], seed);
        let initseq = stream as u64;
        let mut r = Self {
            state: 0,
            inc: (initseq << 1) | 1,
        };
        r.step();
        r.state = r.state.wrapping_add(initstate);
        r.step();
        r
    }

    /// The seed this stream's `hash_draw` calls should use: its initial
    /// state, which already mixes the battle seed and the stream id.
    pub fn draw_seed(seed: u64, stream: StreamId) -> u64 {
        xxh3_64_with_seed(&[stream as u8], seed)
    }

    #[inline]
    fn step(&mut self) {
        self.state = self.state.wrapping_mul(PCG_MULT).wrapping_add(self.inc);
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.step();
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform in `[0, 1)` with 24 bits of resolution.
    #[inline]
    pub fn unit<T: Scalar>(&mut self) -> T {
        unit_from_u32(self.next_u32())
    }

    /// Uniform integer in `0..n` (`n > 0`), by the multiply-shift method.
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        debug_assert!(n > 0);
        ((u64::from(self.next_u32()) * u64::from(n)) >> 32) as u32
    }
}

impl Hashable for RngStream {
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u64(self.state);
        h.write_u64(self.inc);
    }
}

/// Maps 32 random bits to `[0, 1)` using only exact integer conversions and
/// one division, so the result is identical for any `Scalar` with at least
/// 24 bits of mantissa.
#[inline]
pub fn unit_from_u32<T: Scalar>(x: u32) -> T {
    T::from_i32((x >> 8) as i32) / T::from_i32(1 << 24)
}

/// Stateless draw in `[0, 1)` keyed by `(seed, tick, entity, index)`
/// (SIM-DET-002). `seed` is the stream seed from [`RngStream::draw_seed`].
#[inline]
pub fn hash_draw<T: Scalar>(seed: u64, tick: Tick, entity: u32, index: u32) -> T {
    unit_from_u32(hash_draw_bits(seed, tick, entity, index))
}

/// The raw 32 random bits behind [`hash_draw`].
#[inline]
pub fn hash_draw_bits(seed: u64, tick: Tick, entity: u32, index: u32) -> u32 {
    let mut buf = [0u8; 20];
    buf[0..8].copy_from_slice(&seed.to_le_bytes());
    buf[8..12].copy_from_slice(&tick.0.to_le_bytes());
    buf[12..16].copy_from_slice(&entity.to_le_bytes());
    buf[16..20].copy_from_slice(&index.to_le_bytes());
    (xxh3_64(&buf) >> 32) as u32
}

#[cfg(test)]
#[allow(clippy::float_arithmetic)]
mod tests {
    use super::*;

    /// Golden first four outputs per stream at seed 42. Any change to the
    /// seeding or the generator changes these (T0-013).
    const GOLDEN_SEED: u64 = 42;
    const GOLDEN: [[u32; 4]; StreamId::COUNT] = [
        [0x151a_1f8a, 0x74ea_89c6, 0x7843_aad0, 0xb6c2_413b],
        [0xaeb9_bb83, 0xcf69_62b3, 0x9ff8_137f, 0x2ce9_7595],
        [0x9f8f_ab77, 0xefdf_b7ef, 0xa126_6e1c, 0xba29_69f5],
        [0x69bc_3611, 0xeb8a_0927, 0xa340_1b02, 0xe0bb_a727],
        [0xfdae_476c, 0x9b23_3871, 0xa6fb_f72f, 0x58cf_63a3],
        [0x3bdc_7533, 0x197d_b8ca, 0xc923_9898, 0x6a71_7fed],
        [0x9bb3_f5f1, 0xb37d_29f9, 0xe4b6_d91e, 0x0a2f_6b0f],
        [0xcb0d_f675, 0x2236_e416, 0xddaa_1987, 0x9f4a_b2c1],
        [0x806a_ad35, 0xfbac_871f, 0x176d_8dbf, 0xf481_ceda],
    ];

    #[test]
    fn golden_sequence_per_stream() {
        let mut report = String::new();
        let mut ok = true;
        for (i, id) in StreamId::ALL.iter().enumerate() {
            let mut r = RngStream::from_seed(GOLDEN_SEED, *id);
            let got = [r.next_u32(), r.next_u32(), r.next_u32(), r.next_u32()];
            if got != GOLDEN[i] {
                ok = false;
            }
            report.push_str(&format!(
                "        [0x{:04x}_{:04x}, 0x{:04x}_{:04x}, 0x{:04x}_{:04x}, 0x{:04x}_{:04x}],\n",
                got[0] >> 16,
                got[0] & 0xffff,
                got[1] >> 16,
                got[1] & 0xffff,
                got[2] >> 16,
                got[2] & 0xffff,
                got[3] >> 16,
                got[3] & 0xffff
            ));
        }
        assert!(ok, "golden mismatch; actual table:\n{report}");
    }

    #[test]
    fn streams_are_independent_and_reproducible() {
        let mut a = RngStream::from_seed(1, StreamId::Morale);
        let mut b = RngStream::from_seed(1, StreamId::Morale);
        let mut c = RngStream::from_seed(1, StreamId::Weather);
        let mut d = RngStream::from_seed(2, StreamId::Morale);
        let (xa, xb, xc, xd) = (a.next_u32(), b.next_u32(), c.next_u32(), d.next_u32());
        assert_eq!(xa, xb);
        assert_ne!(xa, xc);
        assert_ne!(xa, xd);
        let json = serde_json::to_string(&a).unwrap();
        let mut e: RngStream = serde_json::from_str(&json).unwrap();
        assert_eq!(a.next_u32(), e.next_u32());
    }

    #[test]
    fn unit_and_below_ranges() {
        let mut r = RngStream::from_seed(7, StreamId::Abilities);
        for _ in 0..10_000 {
            let u: f32 = r.unit();
            assert!((0.0..1.0).contains(&u));
            assert!(r.below(6) < 6);
        }
        assert_eq!(unit_from_u32::<f32>(0), 0.0);
        assert!(unit_from_u32::<f32>(u32::MAX) < 1.0);
    }

    #[test]
    fn hash_draw_is_stable_and_uniform() {
        // Stability: the same key gives the same value; neighbours differ.
        let a: f32 = hash_draw(5, Tick(10), 3, 0);
        assert_eq!(a, hash_draw::<f32>(5, Tick(10), 3, 0));
        assert_ne!(a, hash_draw::<f32>(5, Tick(10), 3, 1));
        assert_ne!(a, hash_draw::<f32>(5, Tick(11), 3, 0));
        assert_ne!(a, hash_draw::<f32>(5, Tick(10), 4, 0));
        assert_ne!(a, hash_draw::<f32>(6, Tick(10), 3, 0));

        // Chi-square over 1e6 draws in 64 bins. 63 degrees of freedom: the
        // 0.1 % critical value is about 103.4.
        const BINS: usize = 64;
        const N: u32 = 1_000_000;
        let mut counts = [0u32; BINS];
        for i in 0..N {
            let u: f32 = hash_draw(0xDEAD_BEEF, Tick(i / 1000), i % 1000, i % 7);
            let bin = (u * BINS as f32) as usize;
            counts[bin.min(BINS - 1)] += 1;
        }
        let expected = f64::from(N) / BINS as f64;
        let chi2: f64 = counts
            .iter()
            .map(|&c| {
                let d = f64::from(c) - expected;
                d * d / expected
            })
            .sum();
        assert!(
            chi2 < 103.4,
            "chi-square {chi2} too high; counts {counts:?}"
        );
    }
}
