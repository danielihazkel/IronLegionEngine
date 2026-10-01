//! The height and zone brushes (T3-061, TDD §16). A dab edits the samples
//! or raster cells within `radius` of a world point and returns the block
//! it touched, so the session can patch the terrain view and the sim view
//! of that block only.

use glam::Vec2;

/// What the height brush does to the ground under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeightOp {
    Raise,
    Lower,
    /// Towards the mean of the four neighbours.
    Smooth,
    /// Towards the height under the cursor when the stroke began.
    Flatten,
}

/// How the brush's weight falls from the centre (1) to the rim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    /// `1 − t`.
    Linear,
    /// `1 − (3t² − 2t³)`, flat at the centre and the rim.
    Smooth,
    /// 1 everywhere inside the radius.
    Constant,
}

impl Falloff {
    /// The weight at `t` = distance / radius in `[0, 1]`.
    pub fn weight(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Falloff::Linear => 1.0 - t,
            Falloff::Smooth => 1.0 - t * t * (3.0 - 2.0 * t),
            Falloff::Constant => 1.0,
        }
    }
}

/// A half-open block of grid indices `[i0, i1) × [j0, j1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub i0: u32,
    pub j0: u32,
    pub i1: u32,
    pub j1: u32,
}

impl Block {
    pub fn is_empty(&self) -> bool {
        self.i0 >= self.i1 || self.j0 >= self.j1
    }

    /// Grown by `n` on every side, clipped to `cols × rows`.
    pub fn padded(&self, n: u32, cols: u32, rows: u32) -> Block {
        Block {
            i0: self.i0.saturating_sub(n),
            j0: self.j0.saturating_sub(n),
            i1: (self.i1 + n).min(cols),
            j1: (self.j1 + n).min(rows),
        }
    }

    pub fn union(self, o: Block) -> Block {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        Block {
            i0: self.i0.min(o.i0),
            j0: self.j0.min(o.j0),
            i1: self.i1.max(o.i1),
            j1: self.j1.max(o.j1),
        }
    }
}

/// The block of points `(i × step + offset, j × step + offset)` within
/// `radius` of `centre`'s bounding square, clipped to `cols × rows`.
fn block_around(centre: Vec2, radius: f32, step: f32, offset: f32, cols: u32, rows: u32) -> Block {
    let lo = |v: f32| (((v - radius - offset) / step).ceil().max(0.0)) as u32;
    let hi =
        |v: f32, n: u32| ((((v + radius - offset) / step).floor() + 1.0).max(0.0) as u32).min(n);
    Block {
        i0: lo(centre.x).min(cols),
        j0: lo(centre.y).min(rows),
        i1: hi(centre.x, cols),
        j1: hi(centre.y, rows),
    }
}

/// One height dab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightDab {
    pub op: HeightOp,
    pub falloff: Falloff,
    pub centre: Vec2,
    pub radius: f32,
    /// Metres per second for Raise and Lower; the approach rate per second
    /// for Smooth and Flatten.
    pub strength: f32,
    pub dt: f32,
    /// Flatten's height.
    pub target: f32,
}

/// Applies `dab` to `heights` (`cols × rows` samples `cell` apart), clamped
/// to `[0, max_height]`; returns the samples it may have changed.
pub fn height_dab(
    heights: &mut [f32],
    cols: u32,
    rows: u32,
    cell: f32,
    max_height: f32,
    dab: &HeightDab,
) -> Block {
    let block = block_around(dab.centre, dab.radius, cell, 0.0, cols, rows);
    if block.is_empty() || dab.radius <= 0.0 {
        return Block {
            i0: 0,
            j0: 0,
            i1: 0,
            j1: 0,
        };
    }
    let at = |h: &[f32], i: i64, j: i64| -> f32 {
        let i = i.clamp(0, i64::from(cols) - 1);
        let j = j.clamp(0, i64::from(rows) - 1);
        h[(j * i64::from(cols) + i) as usize]
    };
    // Smooth reads the heights before this dab, never its own writes.
    let before: Option<Vec<f32>> = (dab.op == HeightOp::Smooth).then(|| heights.to_vec());
    for j in block.j0..block.j1 {
        for i in block.i0..block.i1 {
            let p = Vec2::new(i as f32 * cell, j as f32 * cell);
            let d = p.distance(dab.centre);
            if d > dab.radius {
                continue;
            }
            let w = dab.falloff.weight(d / dab.radius);
            let k = (j * cols + i) as usize;
            let h = heights[k];
            let rate = (dab.strength * w * dab.dt).min(1.0);
            let new = match dab.op {
                HeightOp::Raise => h + dab.strength * w * dab.dt,
                HeightOp::Lower => h - dab.strength * w * dab.dt,
                HeightOp::Smooth => {
                    let b = before.as_deref().expect("copied for smooth");
                    let (ii, jj) = (i64::from(i), i64::from(j));
                    let mean = (at(b, ii - 1, jj)
                        + at(b, ii + 1, jj)
                        + at(b, ii, jj - 1)
                        + at(b, ii, jj + 1))
                        * 0.25;
                    h + (mean - h) * rate
                }
                HeightOp::Flatten => h + (dab.target - h) * rate,
            };
            heights[k] = new.clamp(0.0, max_height);
        }
    }
    block
}

/// Sets every raster cell (`cols × rows` of `cell`) whose centre lies
/// within `radius` of `centre` to `value`; returns the cells it may have
/// changed.
pub fn zone_dab(
    raster: &mut [u8],
    cols: u32,
    rows: u32,
    cell: f32,
    centre: Vec2,
    radius: f32,
    value: u8,
) -> Block {
    let block = block_around(centre, radius, cell, cell * 0.5, cols, rows);
    if block.is_empty() || radius <= 0.0 {
        return Block {
            i0: 0,
            j0: 0,
            i1: 0,
            j1: 0,
        };
    }
    for j in block.j0..block.j1 {
        for i in block.i0..block.i1 {
            let c = Vec2::new((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell);
            if c.distance(centre) <= radius {
                raster[(j * cols + i) as usize] = value;
            }
        }
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dab(op: HeightOp, centre: Vec2) -> HeightDab {
        HeightDab {
            op,
            falloff: Falloff::Smooth,
            centre,
            radius: 20.0,
            strength: 10.0,
            dt: 0.1,
            target: 3.0,
        }
    }

    #[test]
    fn a_raised_hill_is_symmetric_and_clamped() {
        let (c, r, cell) = (21u32, 21u32, 2.0);
        let mut h = vec![0.0f32; (c * r) as usize];
        for _ in 0..50 {
            height_dab(
                &mut h,
                c,
                r,
                cell,
                30.0,
                &dab(HeightOp::Raise, Vec2::new(20.0, 20.0)),
            );
        }
        let at = |i: u32, j: u32| h[(j * c + i) as usize];
        assert_eq!(at(10, 10), 30.0, "clamped to the maximum");
        for (i, j) in [(4, 10), (16, 10), (10, 4), (10, 16)] {
            assert_eq!(at(i, j), at(4, 10), "symmetric");
        }
        assert!(at(4, 10) > 0.0 && at(4, 10) < 30.0);
        assert_eq!(at(0, 0), 0.0, "outside the radius");
        for _ in 0..200 {
            height_dab(
                &mut h,
                c,
                r,
                cell,
                30.0,
                &dab(HeightOp::Lower, Vec2::new(20.0, 20.0)),
            );
        }
        assert!(h.iter().all(|v| *v == 0.0), "clamped at zero");
    }

    #[test]
    fn flatten_converges_and_smooth_evens_out() {
        let (c, r, cell) = (21u32, 21u32, 2.0);
        let mut h: Vec<f32> = (0..c * r).map(|k| (k % 7) as f32).collect();
        for _ in 0..200 {
            height_dab(
                &mut h,
                c,
                r,
                cell,
                100.0,
                &dab(HeightOp::Flatten, Vec2::new(20.0, 20.0)),
            );
        }
        assert!((h[(10 * c + 10) as usize] - 3.0).abs() < 1e-3);
        let mut s: Vec<f32> = (0..c * r)
            .map(|k| if k % 2 == 0 { 0.0 } else { 4.0 })
            .collect();
        let spread = |v: &[f32]| {
            v[(10 * c + 10) as usize].max(v[(10 * c + 11) as usize])
                - v[(10 * c + 10) as usize].min(v[(10 * c + 11) as usize])
        };
        let before = spread(&s);
        for _ in 0..20 {
            height_dab(
                &mut s,
                c,
                r,
                cell,
                100.0,
                &dab(HeightOp::Smooth, Vec2::new(20.0, 20.0)),
            );
        }
        assert!(spread(&s) < before * 0.1);
    }

    #[test]
    fn the_zone_dab_is_the_disc() {
        let (c, r, cell) = (30u32, 20u32, 2.0);
        let mut g = vec![0u8; (c * r) as usize];
        let centre = Vec2::new(31.0, 17.0);
        let b = zone_dab(&mut g, c, r, cell, centre, 9.0, 3);
        for j in 0..r {
            for i in 0..c {
                let inside = Vec2::new((i as f32 + 0.5) * cell, (j as f32 + 0.5) * cell)
                    .distance(centre)
                    <= 9.0;
                assert_eq!(g[(j * c + i) as usize] == 3, inside, "cell {i},{j}");
                if inside {
                    assert!(i >= b.i0 && i < b.i1 && j >= b.j0 && j < b.j1);
                }
            }
        }
        // At the map edge the block clips.
        let b = zone_dab(&mut g, c, r, cell, Vec2::new(0.0, 0.0), 5.0, 1);
        assert_eq!((b.i0, b.j0), (0, 0));
    }
}
