//! `il_cli genart`: deterministic placeholder sprite sheets (T1-051).
//!
//! One 8-facing sheet per unit category, drawn from simple shapes so the
//! renderer, the frame-table format and the faction tint path are exercised
//! long before real art exists. Output is committed under `game/assets/` and
//! `game/content/sprites/`; the renderer never depends on this generator.

// A tiny rasteriser: plain f32 math is fine here, nothing reaches the sim.
#![allow(clippy::float_arithmetic)]

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// Unit categories in the order the sheets are generated.
pub const CATEGORIES: [&str; 6] = [
    "infantry",
    "cavalry",
    "ranged",
    "skirmisher",
    "general",
    "siege",
];

pub const FRAME_W: u32 = 64;
pub const FRAME_H: u32 = 64;
pub const FACINGS: u32 = 8;
/// Columns: one idle frame followed by four walk frames.
pub const COLUMNS: u32 = 5;
/// Pixel of a frame that sits on the soldier's ground position.
pub const ORIGIN: (u32, u32) = (32, 52);

const SUPERSAMPLE: u32 = 3;

#[derive(Clone, Copy)]
struct Rgba(u8, u8, u8, u8);

/// A filled shape in frame pixel space (x right, y down).
enum Shape {
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        colour: Rgba,
    },
    Ring {
        cx: f32,
        cy: f32,
        r_in: f32,
        r_out: f32,
        colour: Rgba,
    },
    Rect {
        cx: f32,
        cy: f32,
        hw: f32,
        hh: f32,
        colour: Rgba,
    },
    /// Isosceles triangle from `(cx, cy)` pointing along `angle` (radians,
    /// counter-clockwise on screen) with `len` and half base `hw`.
    Wedge {
        cx: f32,
        cy: f32,
        angle: f32,
        len: f32,
        hw: f32,
        colour: Rgba,
    },
}

impl Shape {
    fn hit(&self, x: f32, y: f32) -> Option<Rgba> {
        match *self {
            Shape::Ellipse {
                cx,
                cy,
                rx,
                ry,
                colour,
            } => {
                let dx = (x - cx) / rx;
                let dy = (y - cy) / ry;
                (dx * dx + dy * dy <= 1.0).then_some(colour)
            }
            Shape::Ring {
                cx,
                cy,
                r_in,
                r_out,
                colour,
            } => {
                let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                (d >= r_in && d <= r_out).then_some(colour)
            }
            Shape::Rect {
                cx,
                cy,
                hw,
                hh,
                colour,
            } => ((x - cx).abs() <= hw && (y - cy).abs() <= hh).then_some(colour),
            Shape::Wedge {
                cx,
                cy,
                angle,
                len,
                hw,
                colour,
            } => {
                // Screen y points down, so a counter-clockwise angle flips y.
                let (s, c) = angle.sin_cos();
                let dx = x - cx;
                let dy = y - cy;
                let along = dx * c - dy * s;
                let across = dx * s + dy * c;
                (along >= 0.0 && along <= len && across.abs() <= hw * (1.0 - along / len))
                    .then_some(colour)
            }
        }
    }
}

/// Shapes of one frame, back to front.
fn frame_shapes(category: &str, facing: u32, column: u32) -> Vec<Shape> {
    let angle = facing as f32 * std::f32::consts::FRAC_PI_4;
    // Walk cycle: column 0 is idle; 1..=4 bob the body.
    let bob = match column {
        2 => -1.0,
        4 => 1.0,
        _ => 0.0,
    };
    let stride = match column {
        1 | 3 => 1.5,
        _ => 0.0,
    };
    let (ox, oy) = (ORIGIN.0 as f32, ORIGIN.1 as f32);
    let body_y = oy - 12.0 + bob;
    let body = Rgba(170, 170, 170, 255);
    let outline = Rgba(28, 28, 28, 255);
    let mark = Rgba(255, 255, 255, 255);
    let dark = Rgba(70, 70, 70, 255);
    let shadow = Rgba(0, 0, 0, 96);

    let mut shapes = vec![Shape::Ellipse {
        cx: ox,
        cy: oy,
        rx: 13.0,
        ry: 5.0,
        colour: shadow,
    }];
    let (rx, ry) = if category == "cavalry" {
        (17.0, 11.0)
    } else if category == "siege" {
        (15.0, 12.0)
    } else {
        (12.0, 12.0)
    };
    shapes.push(Shape::Ellipse {
        cx: ox,
        cy: body_y,
        rx: rx + 1.5,
        ry: ry + 1.5,
        colour: outline,
    });
    shapes.push(Shape::Ellipse {
        cx: ox,
        cy: body_y,
        rx,
        ry,
        colour: body,
    });
    shapes.push(Shape::Wedge {
        cx: ox,
        cy: body_y,
        angle,
        len: rx + 4.0 + stride,
        hw: 5.0,
        colour: mark,
    });
    match category {
        "infantry" => shapes.push(Shape::Rect {
            cx: ox,
            cy: body_y,
            hw: 3.5,
            hh: 3.5,
            colour: dark,
        }),
        "cavalry" => shapes.push(Shape::Rect {
            cx: ox,
            cy: body_y,
            hw: 6.0,
            hh: 2.0,
            colour: dark,
        }),
        "ranged" => shapes.push(Shape::Ring {
            cx: ox,
            cy: body_y,
            r_in: 3.0,
            r_out: 5.0,
            colour: dark,
        }),
        "skirmisher" => shapes.push(Shape::Ellipse {
            cx: ox,
            cy: body_y,
            rx: 2.5,
            ry: 2.5,
            colour: dark,
        }),
        "general" => shapes.push(Shape::Ring {
            cx: ox,
            cy: body_y,
            r_in: 14.5,
            r_out: 16.5,
            colour: Rgba(255, 220, 120, 255),
        }),
        "siege" => shapes.push(Shape::Rect {
            cx: ox,
            cy: body_y,
            hw: 8.0,
            hh: 4.0,
            colour: dark,
        }),
        _ => {}
    }
    shapes
}

/// Renders one category sheet as RGBA8 rows (`COLUMNS * FRAME_W` by
/// `FACINGS * FRAME_H`).
pub fn render_sheet(category: &str) -> Vec<u8> {
    let width = COLUMNS * FRAME_W;
    let height = FACINGS * FRAME_H;
    let mut out = vec![0u8; (width * height * 4) as usize];
    let ss = SUPERSAMPLE as f32;
    let samples = (SUPERSAMPLE * SUPERSAMPLE) as f32;
    for facing in 0..FACINGS {
        for column in 0..COLUMNS {
            let shapes = frame_shapes(category, facing, column);
            for py in 0..FRAME_H {
                for px in 0..FRAME_W {
                    let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
                    for sy in 0..SUPERSAMPLE {
                        for sx in 0..SUPERSAMPLE {
                            let x = px as f32 + (sx as f32 + 0.5) / ss;
                            let y = py as f32 + (sy as f32 + 0.5) / ss;
                            let mut hit: Option<Rgba> = None;
                            for shape in &shapes {
                                if let Some(c) = shape.hit(x, y) {
                                    hit = Some(c);
                                }
                            }
                            if let Some(Rgba(cr, cg, cb, ca)) = hit {
                                let alpha = ca as f32 / 255.0;
                                r += cr as f32 * alpha;
                                g += cg as f32 * alpha;
                                b += cb as f32 * alpha;
                                a += alpha;
                            }
                        }
                    }
                    let idx =
                        (((facing * FRAME_H + py) * width) + column * FRAME_W + px) as usize * 4;
                    if a > 0.0 {
                        // Un-premultiply so the texture is straight alpha.
                        out[idx] = (r / a).round() as u8;
                        out[idx + 1] = (g / a).round() as u8;
                        out[idx + 2] = (b / a).round() as u8;
                        out[idx + 3] = (a / samples * 255.0).round() as u8;
                    }
                }
            }
        }
    }
    out
}

/// Encodes RGBA8 rows as a PNG.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().context("writing the PNG header")?;
        writer
            .write_image_data(rgba)
            .context("writing the PNG image data")?;
    }
    Ok(bytes)
}

/// The frame table (a `SpriteSet` content file) for one category.
pub fn frame_table(category: &str) -> String {
    format!(
        "// Generated by `il_cli genart`; placeholder art (T1-051). Regenerate rather than edit.\n\
{{\n\
  id: \"rome:sprites_{category}\",\n\
  atlas: \"sprites/units/{category}.png\",\n\
  frame_w: {FRAME_W},\n\
  frame_h: {FRAME_H},\n\
  facings: {FACINGS},\n\
  columns: {COLUMNS},\n\
  origin: [{ox}, {oy}],\n\
  anims: {{\n\
    idle: {{ first: 0, count: 1, fps: 1 }},\n\
    walk: {{ first: 1, count: 4, fps: 8 }},\n\
  }},\n\
}}\n",
        ox = ORIGIN.0,
        oy = ORIGIN.1,
    )
}

/// Files `generate` would write: `(relative path, bytes)` pairs, sorted by
/// path, without touching the filesystem.
pub fn artifacts() -> anyhow::Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut out = Vec::new();
    for category in CATEGORIES {
        let rgba = render_sheet(category);
        let png = encode_png(&rgba, COLUMNS * FRAME_W, FACINGS * FRAME_H)?;
        out.push((
            PathBuf::from(format!("assets/sprites/units/{category}.png")),
            png,
        ));
        out.push((
            PathBuf::from(format!("content/sprites/{category}.json5")),
            frame_table(category).into_bytes(),
        ));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Writes every artifact under `mod_root` (`game/` by default).
pub fn generate(mod_root: &Path, out: &mut dyn Write) -> anyhow::Result<()> {
    for (rel, bytes) in artifacts()? {
        let path = mod_root.join(&rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
        writeln!(out, "wrote {} ({} bytes)", path.display(), bytes.len())?;
    }
    Ok(())
}
