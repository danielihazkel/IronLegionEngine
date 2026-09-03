//! `il_cli genmap`: the deterministic Phase 1 test map (T1-030).
//!
//! Writes one map definition (`content/maps/<item>.json5`) and its 16-bit
//! heightmap sidecar (`assets/maps/<item>.hgt`) into a mod root. Everything
//! is a pure function of the seed, so the committed files can be regenerated
//! bit for bit. The map is 800 × 600 m: a value-noise ground with a hill in
//! the south-east carrying a rock outcrop, a west–east river with an 8 m
//! bridge (the narrowest corridor the 4 m nav grid can represent) and a
//! 30 m ford, a forest, a marsh, a north–south road over the bridge, and one
//! deployment rectangle per side.

// A generator, not the sim: plain f32 math is fine here.
#![allow(clippy::float_arithmetic)]

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

pub const WIDTH: f32 = 800.0;
pub const HEIGHT: f32 = 600.0;
pub const HEIGHT_CELL: f32 = 4.0;
/// Metres per raw 16-bit unit.
pub const SCALE: f32 = 0.01;

#[derive(Clone, Debug)]
pub struct GenmapOptions {
    /// Mod root; files go under `content/maps/` and `assets/maps/`.
    pub mod_root: PathBuf,
    /// ContentId of the map, e.g. `rome:test_field`.
    pub id: String,
    pub seed: u64,
}

/// A polygon zone of the generated map.
struct Zone {
    kind: &'static str,
    polygon: &'static [[f32; 2]],
}

const RIVER_WIDTH: f32 = 12.0;
const RIVER: [[f32; 2]; 5] = [
    [0.0, 300.0],
    [200.0, 290.0],
    [400.0, 310.0],
    [600.0, 295.0],
    [800.0, 305.0],
];
/// Centre and radius of the hill.
const HILL: ([f32; 2], f32, f32) = ([600.0, 460.0], 90.0, 22.0);

const ZONES: [Zone; 6] = [
    Zone {
        kind: "rome:forest",
        polygon: &[
            [60.0, 380.0],
            [220.0, 360.0],
            [260.0, 470.0],
            [180.0, 560.0],
            [70.0, 540.0],
        ],
    },
    Zone {
        kind: "rome:marsh",
        polygon: &[
            [620.0, 330.0],
            [760.0, 320.0],
            [780.0, 400.0],
            [660.0, 410.0],
        ],
    },
    Zone {
        kind: "rome:rock",
        polygon: &[
            [560.0, 470.0],
            [640.0, 460.0],
            [660.0, 520.0],
            [590.0, 540.0],
        ],
    },
    // North–south road, 8 m wide, aligned to the 4 m nav grid.
    Zone {
        kind: "rome:road",
        polygon: &[[396.0, 0.0], [404.0, 0.0], [404.0, 600.0], [396.0, 600.0]],
    },
    // Crossings last so they override the road and the river.
    Zone {
        kind: "rome:bridge",
        polygon: &[
            [396.0, 296.0],
            [404.0, 296.0],
            [404.0, 324.0],
            [396.0, 324.0],
        ],
    },
    Zone {
        kind: "rome:ford",
        polygon: &[
            [635.0, 275.0],
            [665.0, 275.0],
            [665.0, 320.0],
            [635.0, 320.0],
        ],
    },
];

const DEPLOYMENT: [(u8, [[f32; 2]; 4]); 2] = [
    (
        0,
        [[40.0, 40.0], [760.0, 40.0], [760.0, 200.0], [40.0, 200.0]],
    ),
    (
        1,
        [[40.0, 400.0], [760.0, 400.0], [760.0, 560.0], [40.0, 560.0]],
    ),
];

/// Integer hash → `[0, 1)`, the lattice values of the noise.
fn lattice(seed: u64, ix: i32, iy: i32) -> f32 {
    let mut h = seed
        ^ (ix as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (iy as u32 as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value noise in `[0, 1]` with lattice spacing `period` metres.
fn value_noise(seed: u64, x: f32, y: f32, period: f32) -> f32 {
    let gx = x / period;
    let gy = y / period;
    let ix = gx.floor() as i32;
    let iy = gy.floor() as i32;
    let fx = smoothstep(gx - ix as f32);
    let fy = smoothstep(gy - iy as f32);
    let v00 = lattice(seed, ix, iy);
    let v10 = lattice(seed, ix + 1, iy);
    let v01 = lattice(seed, ix, iy + 1);
    let v11 = lattice(seed, ix + 1, iy + 1);
    let a = v00 + (v10 - v00) * fx;
    let b = v01 + (v11 - v01) * fx;
    a + (b - a) * fy
}

fn dist_point_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let abx = b[0] - a[0];
    let aby = b[1] - a[1];
    let apx = p[0] - a[0];
    let apy = p[1] - a[1];
    let len_sq = abx * abx + aby * aby;
    let t = if len_sq > 0.0 {
        ((apx * abx + apy * aby) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let dx = apx - abx * t;
    let dy = apy - aby * t;
    (dx * dx + dy * dy).sqrt()
}

fn dist_to_river(x: f32, y: f32) -> f32 {
    RIVER
        .windows(2)
        .map(|w| dist_point_segment([x, y], w[0], w[1]))
        .fold(f32::INFINITY, f32::min)
}

/// Ground height in metres at `(x, y)`.
pub fn height_at(seed: u64, x: f32, y: f32) -> f32 {
    let rolling = 4.0 * value_noise(seed, x, y, 160.0) + 2.0 * value_noise(seed ^ 0x55, x, y, 50.0);
    let ([hx, hy], radius, peak) = HILL;
    let d2 = (x - hx) * (x - hx) + (y - hy) * (y - hy);
    let hill = peak * (-d2 / (2.0 * radius * radius)).exp();
    // A valley: the river bed is at zero and the banks rise over 40 m.
    let valley = smoothstep((dist_to_river(x, y) / 40.0).clamp(0.0, 1.0));
    (rolling + hill) * valley
}

/// `(cols, rows)` of the sidecar: `ceil(w / cell) + 1` by `ceil(h / cell) + 1`.
pub fn dims() -> (u32, u32) {
    (
        (WIDTH / HEIGHT_CELL).ceil() as u32 + 1,
        (HEIGHT / HEIGHT_CELL).ceil() as u32 + 1,
    )
}

/// The raw samples, row-major from `y = 0`.
pub fn samples(seed: u64) -> Vec<u16> {
    let (cols, rows) = dims();
    let mut out = Vec::with_capacity((cols * rows) as usize);
    for j in 0..rows {
        for i in 0..cols {
            let h = height_at(seed, i as f32 * HEIGHT_CELL, j as f32 * HEIGHT_CELL);
            out.push((h / SCALE).round().clamp(0.0, 65_535.0) as u16);
        }
    }
    out
}

fn fmt_points(points: &[[f32; 2]]) -> String {
    points
        .iter()
        .map(|[x, y]| format!("[{x}, {y}]"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The map definition as JSON5 text.
pub fn map_json5(id: &str, item: &str, seed: u64) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "// Generated by `il_cli genmap --id {id} --seed {seed}`; do not edit by hand.\n\
         // 800 x 600 m: value-noise ground, a hill with a rock outcrop in the\n\
         // south-east, a west-east river with an 8 m bridge and a 30 m ford, a\n\
         // forest, a marsh, a north-south road, one deployment rectangle per side.\n\
         {{\n  id: \"{id}\",\n  name_key: \"rome.maps.{item}.name\",\n  size: {{ w: {WIDTH}, h: {HEIGHT} }},\n\
         \x20 campaign_terrain_tags: [\"plains\", \"river\"],\n  weather_allowed: [\"clear\", \"rain\", \"fog\"],\n\
         \x20 heightmap: {{ cell: {HEIGHT_CELL}, path: \"maps/{item}.hgt\", scale: {SCALE} }},\n\
         \x20 base_zone: \"rome:open\",\n  zones: [\n"
    ));
    for z in &ZONES {
        s.push_str(&format!(
            "    {{ type: \"{}\", polygon: [{}] }},\n",
            z.kind,
            fmt_points(z.polygon)
        ));
    }
    s.push_str("  ],\n  rivers: [\n");
    s.push_str(&format!(
        "    {{ width: {RIVER_WIDTH}, points: [{}] }},\n",
        fmt_points(&RIVER)
    ));
    s.push_str("  ],\n  deployment: [\n");
    for (side, poly) in &DEPLOYMENT {
        s.push_str(&format!(
            "    {{ side: {side}, polygon: [{}] }},\n",
            fmt_points(poly)
        ));
    }
    s.push_str(
        "  ],\n  reinforcement_edges: [\n    { side: 0, edge: \"south\" },\n    { side: 1, edge: \"north\" },\n  ],\n\
         \x20 // Reserved for sieges (REQ-SIM-045).\n  structures: [],\n  siege_points: [],\n}\n",
    );
    s
}

/// Writes the map and its sidecar; prints the two paths.
pub fn generate(opts: &GenmapOptions, out: &mut dyn Write) -> anyhow::Result<()> {
    let item = opts
        .id
        .split_once(':')
        .map(|(_, item)| item)
        .ok_or_else(|| anyhow::anyhow!("map id {:?} is not <namespace>:<item>", opts.id))?;
    let content = opts.mod_root.join("content/maps");
    let assets = opts.mod_root.join("assets/maps");
    std::fs::create_dir_all(&content).with_context(|| format!("creating {}", content.display()))?;
    std::fs::create_dir_all(&assets).with_context(|| format!("creating {}", assets.display()))?;

    let hgt: PathBuf = assets.join(format!("{item}.hgt"));
    let bytes: Vec<u8> = samples(opts.seed)
        .iter()
        .flat_map(|s| s.to_le_bytes())
        .collect();
    std::fs::write(&hgt, bytes).with_context(|| format!("writing {}", hgt.display()))?;
    writeln!(out, "{}", display(&hgt))?;

    let json = content.join(format!("{item}.json5"));
    std::fs::write(&json, map_json5(&opts.id, item, opts.seed))
        .with_context(|| format!("writing {}", json.display()))?;
    writeln!(out, "{}", display(&json))?;
    Ok(())
}

fn display(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_are_deterministic_and_bounded() {
        let a = samples(7);
        let b = samples(7);
        assert_eq!(a, b);
        assert_ne!(a, samples(8));
        let (cols, rows) = dims();
        assert_eq!((cols, rows), (201, 151));
        assert_eq!(a.len(), 201 * 151);
        let max = a.iter().copied().max().unwrap();
        assert!(max as f32 * SCALE < 40.0, "peak {max}");
        // The river bed is flat at zero.
        assert_eq!(height_at(7, 400.0, 310.0), 0.0);
        // The hill is the highest point.
        assert!(height_at(7, 600.0, 460.0) > 15.0);
    }

    #[test]
    fn json5_is_a_valid_map_definition() {
        let text = map_json5("rome:test_field", "test_field", 7);
        let v: serde_json::Value = json5::from_str(&text).unwrap();
        assert_eq!(v["id"], "rome:test_field");
        assert_eq!(v["zones"].as_array().unwrap().len(), ZONES.len());
        assert_eq!(v["deployment"].as_array().unwrap().len(), 2);
        assert_eq!(v["base_zone"], "rome:open");
    }
}
