//! `il_cli gensound`: deterministic placeholder battle sounds (T2-100,
//! plan decision 7).
//!
//! Synthesises the small set the flagship `rome:battle` sound set names
//! (clashes, charges, volleys, deaths, morale events, a gong for the
//! general, drums for the phases, victory and defeat, and a seamless
//! battle-roar loop) as 22,050 Hz mono 16-bit WAV files under
//! `assets/sounds/`, and writes the set itself to `content/sounds/`.
//! Output is committed; `il_audio` never depends on this generator. Every
//! sample comes from a tiny synth seeded per sound name, so two runs give
//! identical bytes.

// Plain f32 DSP on the tool side; nothing here reaches the sim.
#![allow(clippy::float_arithmetic)]

use std::f32::consts::{PI, TAU};
use std::io::Write;
use std::path::Path;

use anyhow::Context;

pub const SAMPLE_RATE: u32 = 22_050;
/// Peak amplitude every sample is normalised to.
const PEAK: f32 = 0.8;
/// The roar loop's length and its seam crossfade.
const ROAR_SECONDS: f32 = 2.0;
const ROAR_SEAM_SECONDS: f32 = 0.1;

/// The sounds written, in output order (file stem, seed).
pub const SOUNDS: [&str; 18] = [
    "clash_1",
    "clash_2",
    "charge",
    "cavalry_charge",
    "volley",
    "arrow_hit",
    "death_1",
    "death_2",
    "cavalry_death",
    "rout",
    "rally",
    "shatter",
    "general_died",
    "ability",
    "phase",
    "victory",
    "defeat",
    "roar_loop",
];

/// xorshift64: the only randomness, seeded from the sound's name.
struct Synth {
    state: u64,
}

impl Synth {
    fn new(name: &str) -> Self {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        for b in name.bytes() {
            state = (state ^ u64::from(b)).wrapping_mul(0x100_0000_01B3);
        }
        Self {
            state: state.max(1),
        }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// White noise in [-1, 1].
    fn noise(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }
}

fn samples(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32) as usize
}

/// Time of sample `i` in seconds.
fn t(i: usize) -> f32 {
    i as f32 / SAMPLE_RATE as f32
}

/// Attack-then-exponential-decay envelope.
fn env(time: f32, attack: f32, decay: f32) -> f32 {
    if time < attack {
        time / attack.max(1e-4)
    } else {
        (-(time - attack) / decay.max(1e-4)).exp()
    }
}

/// One-pole low-pass with a per-sample cutoff.
struct LowPass {
    y: f32,
}

impl LowPass {
    fn new() -> Self {
        Self { y: 0.0 }
    }

    fn step(&mut self, x: f32, cutoff_hz: f32) -> f32 {
        let a = 1.0 - (-TAU * cutoff_hz / SAMPLE_RATE as f32).exp();
        self.y += a * (x - self.y);
        self.y
    }
}

/// A phase accumulator for a sine or triangle at a per-sample frequency.
struct Osc {
    phase: f32,
}

impl Osc {
    fn new() -> Self {
        Self { phase: 0.0 }
    }

    fn advance(&mut self, hz: f32) -> f32 {
        self.phase = (self.phase + hz / SAMPLE_RATE as f32).fract();
        self.phase
    }

    fn sine(&mut self, hz: f32) -> f32 {
        (self.advance(hz) * TAU).sin()
    }

    fn triangle(&mut self, hz: f32) -> f32 {
        let p = self.advance(hz);
        4.0 * (p - 0.5).abs() - 1.0
    }
}

fn lerp(a: f32, b: f32, x: f32) -> f32 {
    a + (b - a) * x.clamp(0.0, 1.0)
}

/// Normalises to [`PEAK`].
fn normalise(buf: &mut [f32]) {
    let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        let k = PEAK / peak;
        for s in buf.iter_mut() {
            *s *= k;
        }
    }
}

/// A short metallic clash: two decaying partials under a noise burst.
fn clash(s: &mut Synth, seconds: f32, base_hz: f32) -> Vec<f32> {
    let n = samples(seconds);
    let (mut o1, mut o2) = (Osc::new(), Osc::new());
    (0..n)
        .map(|i| {
            let time = t(i);
            let ring = 0.5 * o1.sine(base_hz) + 0.3 * o2.sine(base_hz * 1.53);
            ring * env(time, 0.002, 0.06) + s.noise() * env(time, 0.001, 0.025) * 0.9
        })
        .collect()
}

/// A shout: low-passed noise with a swept cutoff over a vibrato tone.
fn shout(s: &mut Synth, seconds: f32, tone_hz: f32, cut_from: f32, cut_to: f32) -> Vec<f32> {
    let n = samples(seconds);
    let mut lp = LowPass::new();
    let (mut osc, mut lfo) = (Osc::new(), Osc::new());
    (0..n)
        .map(|i| {
            let time = t(i);
            let x = time / seconds;
            let cutoff = lerp(cut_from, cut_to, (x * 2.0).min(1.0));
            let vib = 1.0 + 0.03 * lfo.sine(6.0);
            let voice = lp.step(s.noise(), cutoff) * 1.6 + 0.35 * osc.triangle(tone_hz * vib);
            voice * env(time, 0.05, seconds * 0.45)
        })
        .collect()
}

/// Hooves: a run of low thuds.
fn hooves(s: &mut Synth, seconds: f32, beats: usize) -> Vec<f32> {
    let n = samples(seconds);
    let mut lp = LowPass::new();
    let mut osc = Osc::new();
    let period = seconds / beats as f32;
    (0..n)
        .map(|i| {
            let time = t(i);
            let local = time % period;
            let thump = osc.sine(lerp(90.0, 45.0, local / 0.08)) * env(local, 0.002, 0.05);
            let dust = lp.step(s.noise(), 600.0) * env(local, 0.001, 0.03) * 2.0;
            (thump + dust) * env(time, 0.05, seconds)
        })
        .collect()
}

/// A whoosh: noise through a cutoff that rises then falls.
fn whoosh(s: &mut Synth, seconds: f32) -> Vec<f32> {
    let n = samples(seconds);
    let mut lp = LowPass::new();
    (0..n)
        .map(|i| {
            let time = t(i);
            let x = time / seconds;
            let cutoff = 300.0 + 2700.0 * (PI * x).sin();
            lp.step(s.noise(), cutoff) * 2.0 * (PI * x).sin()
        })
        .collect()
}

/// A thud: a low tone and a noise click.
fn thud(s: &mut Synth, seconds: f32, hz: f32) -> Vec<f32> {
    let n = samples(seconds);
    let mut osc = Osc::new();
    let mut lp = LowPass::new();
    (0..n)
        .map(|i| {
            let time = t(i);
            osc.sine(lerp(hz, hz * 0.6, time / seconds)) * env(time, 0.002, seconds * 0.35)
                + lp.step(s.noise(), 1500.0) * env(time, 0.001, 0.02) * 1.5
        })
        .collect()
}

/// A grunt or cry: a tone gliding from `from_hz` to `to_hz` under noise.
fn cry(s: &mut Synth, seconds: f32, from_hz: f32, to_hz: f32, vibrato: f32) -> Vec<f32> {
    let n = samples(seconds);
    let (mut osc, mut lfo) = (Osc::new(), Osc::new());
    let mut lp = LowPass::new();
    (0..n)
        .map(|i| {
            let time = t(i);
            let hz = lerp(from_hz, to_hz, time / seconds) * (1.0 + vibrato * lfo.sine(7.0));
            (0.6 * osc.triangle(hz) + lp.step(s.noise(), 900.0) * 0.8)
                * env(time, 0.03, seconds * 0.5)
        })
        .collect()
}

/// A glide of one triangle tone.
fn glide(seconds: f32, from_hz: f32, to_hz: f32) -> Vec<f32> {
    let n = samples(seconds);
    let mut osc = Osc::new();
    (0..n)
        .map(|i| {
            let time = t(i);
            osc.triangle(lerp(from_hz, to_hz, time / seconds)) * env(time, 0.02, seconds * 0.6)
        })
        .collect()
}

/// A gong: inharmonic partials with a slow decay.
fn gong(seconds: f32) -> Vec<f32> {
    let n = samples(seconds);
    let partials = [220.0f32, 331.0, 498.0, 720.0, 1010.0];
    let mut oscs: Vec<Osc> = partials.iter().map(|_| Osc::new()).collect();
    (0..n)
        .map(|i| {
            let time = t(i);
            partials
                .iter()
                .zip(oscs.iter_mut())
                .enumerate()
                .map(|(k, (hz, o))| {
                    o.sine(*hz) * env(time, 0.005, seconds * 0.4 / (k as f32 + 1.0))
                })
                .sum::<f32>()
        })
        .collect()
}

/// A chime: two sines.
fn chime(seconds: f32, hz: f32) -> Vec<f32> {
    let n = samples(seconds);
    let (mut a, mut b) = (Osc::new(), Osc::new());
    (0..n)
        .map(|i| {
            let time = t(i);
            (a.sine(hz) + 0.5 * b.sine(hz * 1.5)) * env(time, 0.005, seconds * 0.4)
        })
        .collect()
}

/// A sequence of notes, each a sine with its own envelope.
fn notes(seconds_each: f32, hzs: &[f32]) -> Vec<f32> {
    let per = samples(seconds_each);
    let mut out = Vec::with_capacity(per * hzs.len());
    for hz in hzs {
        let mut osc = Osc::new();
        for i in 0..per {
            let time = t(i);
            out.push(osc.sine(*hz) * env(time, 0.01, seconds_each * 0.5));
        }
    }
    out
}

/// The battle roar: low-passed noise under two slow amplitude LFOs, its
/// tail crossfaded into its head so it loops without a click.
fn roar(s: &mut Synth) -> Vec<f32> {
    let n = samples(ROAR_SECONDS);
    let mut lp = LowPass::new();
    let (mut a, mut b) = (Osc::new(), Osc::new());
    let mut raw: Vec<f32> = (0..n)
        .map(|_| {
            let mod_ = 0.7 + 0.2 * a.sine(0.9) + 0.1 * b.sine(2.3);
            lp.step(s.noise(), 900.0) * mod_
        })
        .collect();
    let seam = samples(ROAR_SEAM_SECONDS);
    for k in 0..seam {
        let x = k as f32 / seam as f32;
        let head = raw[k];
        let tail = raw[n - seam + k];
        raw[k] = head * x + tail * (1.0 - x);
    }
    raw.truncate(n - seam);
    raw
}

/// Renders one named sound.
pub fn render(name: &str) -> Vec<f32> {
    let s = &mut Synth::new(name);
    let mut buf = match name {
        "clash_1" => clash(s, 0.25, 2400.0),
        "clash_2" => clash(s, 0.22, 3100.0),
        "charge" => shout(s, 0.6, 180.0, 400.0, 1200.0),
        "cavalry_charge" => hooves(s, 0.8, 6),
        "volley" => whoosh(s, 0.5),
        "arrow_hit" => thud(s, 0.12, 150.0),
        "death_1" => cry(s, 0.35, 140.0, 90.0, 0.0),
        "death_2" => cry(s, 0.3, 170.0, 100.0, 0.0),
        "cavalry_death" => cry(s, 0.5, 500.0, 250.0, 0.04),
        "rout" => glide(0.6, 600.0, 200.0),
        "rally" => glide(0.5, 300.0, 700.0),
        "shatter" => {
            let mut v = glide(0.35, 500.0, 300.0);
            v.extend(cry(s, 0.35, 300.0, 120.0, 0.02));
            v
        }
        "general_died" => gong(1.5),
        "ability" => chime(0.4, 880.0),
        "phase" => thud(s, 0.4, 80.0),
        "victory" => notes(0.3, &[523.25, 659.25, 783.99]),
        "defeat" => notes(0.3, &[392.0, 311.13, 261.63]),
        "roar_loop" => roar(s),
        other => panic!("unknown sound {other}"),
    };
    normalise(&mut buf);
    buf
}

/// A 16-bit mono RIFF/WAVE file (44-byte header).
pub fn wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// The sound set the generator writes (`content/sounds/battle.json5`).
pub const SOUND_SET_JSON5: &str = r#"// The flagship battle sound set (schema docs/schemas/sound-set.schema.json,
// TDD 12, T2-100). Every sample is a placeholder written by
// `il_cli gensound`; the factions name this set through `sound_set`.
{
  id: "rome:battle",
  events: {
    charge:         { samples: ["sounds/charge.wav"], min_interval_ms: 400, max_voices: 2, gain: 0.9 },
    cavalry_charge: { samples: ["sounds/cavalry_charge.wav"], min_interval_ms: 400, max_voices: 2, gain: 1.0 },
    clash:          { samples: ["sounds/clash_1.wav", "sounds/clash_2.wav"], min_interval_ms: 150, max_voices: 6, gain: 0.8 },
    volley:         { samples: ["sounds/volley.wav"], min_interval_ms: 300, max_voices: 3, gain: 0.9 },
    arrow_hit:      { samples: ["sounds/arrow_hit.wav"], min_interval_ms: 60, max_voices: 8, gain: 0.5 },
    death:          { samples: ["sounds/death_1.wav", "sounds/death_2.wav"], min_interval_ms: 80, max_voices: 8, gain: 0.7 },
    cavalry_death:  { samples: ["sounds/cavalry_death.wav"], min_interval_ms: 200, max_voices: 3, gain: 0.8 },
    rout:           { samples: ["sounds/rout.wav"], min_interval_ms: 500, max_voices: 2, gain: 0.9 },
    rally:          { samples: ["sounds/rally.wav"], min_interval_ms: 500, max_voices: 2, gain: 0.8 },
    shatter:        { samples: ["sounds/shatter.wav"], min_interval_ms: 500, max_voices: 2, gain: 0.9 },
    general_died:   { samples: ["sounds/general_died.wav"], min_interval_ms: 1000, max_voices: 1, gain: 1.0 },
    ability:        { samples: ["sounds/ability.wav"], min_interval_ms: 300, max_voices: 2, gain: 0.8 },
    phase:          { samples: ["sounds/phase.wav"], min_interval_ms: 500, max_voices: 1, gain: 1.0 },
    victory:        { samples: ["sounds/victory.wav"], min_interval_ms: 1000, max_voices: 1, gain: 1.0 },
    defeat:         { samples: ["sounds/defeat.wav"], min_interval_ms: 1000, max_voices: 1, gain: 1.0 },
  },
  // The far-zoom battle roar: full gain once this many soldiers fight.
  roar: { sample: "sounds/roar_loop.wav", ref_engaged: 2000 },
  // Camera pixels per metre: only the roar at or below `far`, the
  // individual effects at full gain at or above `near` (the default zoom
  // is 12, the strategic limit 2, the tactical limit 96).
  zoom: { far: 3.0, near: 24.0 },
  max_voices: 24,
  cull_pad_m: 40,
}
"#;

/// Every file the generator writes: `(path relative to the mod root, bytes)`.
pub fn artifacts() -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = SOUNDS
        .iter()
        .map(|name| {
            (
                format!("assets/sounds/{name}.wav"),
                wav_bytes(&render(name), SAMPLE_RATE),
            )
        })
        .collect();
    out.push((
        "content/sounds/battle.json5".to_owned(),
        SOUND_SET_JSON5.as_bytes().to_vec(),
    ));
    out
}

pub fn generate(mod_root: &Path, out: &mut dyn Write) -> anyhow::Result<()> {
    for (rel, bytes) in artifacts() {
        let path = mod_root.join(&rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
        writeln!(out, "wrote {} ({} bytes)", path.display(), bytes.len())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
    }

    #[test]
    fn every_sound_is_a_valid_normalised_wav() {
        for name in SOUNDS {
            let samples = render(name);
            assert!(!samples.is_empty(), "{name}");
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!((peak - PEAK).abs() < 1e-3, "{name}: peak {peak}");
            let bytes = wav_bytes(&samples, SAMPLE_RATE);
            assert_eq!(&bytes[0..4], b"RIFF");
            assert_eq!(&bytes[8..12], b"WAVE");
            assert_eq!(u32_at(&bytes, 24), SAMPLE_RATE);
            assert_eq!(u32_at(&bytes, 40) as usize, samples.len() * 2);
            assert_eq!(bytes.len(), 44 + samples.len() * 2);
        }
    }

    #[test]
    fn output_is_deterministic_and_small() {
        let a = artifacts();
        let b = artifacts();
        assert_eq!(a, b);
        let total: usize = a.iter().map(|(_, bytes)| bytes.len()).sum();
        assert!(total < 1_000_000, "{total} bytes");
        assert_eq!(a.len(), SOUNDS.len() + 1);
        assert!(a.last().unwrap().0.ends_with("battle.json5"));
    }

    #[test]
    fn the_roar_loops_without_a_seam() {
        let r = render("roar_loop");
        // The crossfaded head continues the tail: the jump across the seam
        // is no bigger than a typical neighbouring-sample jump.
        let seam_jump = (r[0] - r[r.len() - 1]).abs();
        let typical: f32 = r.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / r.len() as f32;
        assert!(
            seam_jump < typical * 8.0,
            "seam {seam_jump} vs typical {typical}"
        );
    }

    #[test]
    fn the_sound_set_names_every_written_sample() {
        for name in SOUNDS {
            assert!(
                SOUND_SET_JSON5.contains(&format!("sounds/{name}.wav")),
                "{name} missing from the set"
            );
        }
    }
}
