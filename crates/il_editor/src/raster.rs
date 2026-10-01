//! The zone brush's raster turned into map polygons (T3-061, TDD §16).
//!
//! A painted zone is exact: each 4-connected component of one raster value
//! becomes one polygon along the cells' edges, collinear runs merged, and
//! every hole joined to the loop around it by a horizontal slit (a doubled
//! zero-width edge on a grid line). The sim fills polygons even-odd at cell
//! centres (`LoadedMap::from_def`), which never sit on a grid line, so the
//! slits are invisible and the polygon fills exactly the component's cells.

use std::collections::BTreeMap;

/// A grid vertex `(x, y)` in cells.
pub type GridPoint = (i32, i32);

/// One traced polygon: the raster value it carries and its vertices in
/// grid units, counter-clockwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TracedPolygon {
    pub value: u8,
    pub points: Vec<GridPoint>,
}

/// Unit steps of the four edge directions: +x, +y, -x, -y.
const STEP: [GridPoint; 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

/// Every component of every non-zero value, as polygons: values in
/// ascending order, components in raster order (row-major from `y = 0`),
/// a component split at a diagonal pinch giving one polygon per outer loop.
pub fn trace_polygons(raster: &[u8], cols: u32, rows: u32) -> Vec<TracedPolygon> {
    let (c, r) = (cols as usize, rows as usize);
    assert_eq!(raster.len(), c * r, "raster size");
    // Component label per cell (0 = none) and each component's value.
    let mut label = vec![0u32; raster.len()];
    let mut comps: Vec<(u8, Vec<usize>)> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..raster.len() {
        let v = raster[start];
        if v == 0 || label[start] != 0 {
            continue;
        }
        let id = comps.len() as u32 + 1;
        let mut cells = Vec::new();
        label[start] = id;
        stack.push(start);
        while let Some(k) = stack.pop() {
            cells.push(k);
            let (i, j) = (k % c, k / c);
            let mut visit = |n: usize| {
                if raster[n] == v && label[n] == 0 {
                    label[n] = id;
                    stack.push(n);
                }
            };
            if i > 0 {
                visit(k - 1);
            }
            if i + 1 < c {
                visit(k + 1);
            }
            if j > 0 {
                visit(k - c);
            }
            if j + 1 < r {
                visit(k + c);
            }
        }
        comps.push((v, cells));
    }
    let mut order: Vec<usize> = (0..comps.len()).collect();
    order.sort_by_key(|&n| comps[n].0);
    let mut out = Vec::new();
    for n in order {
        let (value, cells) = &comps[n];
        let id = n as u32 + 1;
        let inside = |i: i64, j: i64| -> bool {
            i >= 0
                && j >= 0
                && (i as usize) < c
                && (j as usize) < r
                && label[j as usize * c + i as usize] == id
        };
        for points in component_polygons(cells, c, inside) {
            out.push(TracedPolygon {
                value: *value,
                points,
            });
        }
    }
    out
}

/// The boundary loops of one component joined into polygons: one per
/// outer (counter-clockwise) loop, each hole slit into the loop it sits in.
fn component_polygons(
    cells: &[usize],
    cols: usize,
    inside: impl Fn(i64, i64) -> bool,
) -> Vec<Vec<GridPoint>> {
    // Directed boundary edges with the component on their left, keyed by
    // their start vertex (at most two leave a vertex: a diagonal pinch).
    let mut edges: BTreeMap<GridPoint, Vec<u8>> = BTreeMap::new();
    for &k in cells {
        let (i, j) = ((k % cols) as i64, (k / cols) as i64);
        let (x, y) = (i as i32, j as i32);
        if !inside(i, j - 1) {
            edges.entry((x, y)).or_default().push(0);
        }
        if !inside(i + 1, j) {
            edges.entry((x + 1, y)).or_default().push(1);
        }
        if !inside(i, j + 1) {
            edges.entry((x + 1, y + 1)).or_default().push(2);
        }
        if !inside(i - 1, j) {
            edges.entry((x, y + 1)).or_default().push(3);
        }
    }
    // Walk the loops, turning left at a pinch so diagonal cells stay apart.
    let mut loops: Vec<Vec<GridPoint>> = Vec::new();
    while let Some((&start, _)) = edges.iter().find(|(_, d)| !d.is_empty()) {
        let mut lp = Vec::new();
        let mut at = start;
        let mut dir: Option<u8> = None;
        loop {
            let outs = edges
                .get_mut(&at)
                .expect("an edge leaves every loop vertex");
            let d = match dir {
                None => outs[0],
                Some(dir) => [(dir + 1) % 4, dir, (dir + 3) % 4]
                    .into_iter()
                    .find(|d| outs.contains(d))
                    .expect("in- and out-degree match at every vertex"),
            };
            outs.retain(|x| *x != d);
            lp.push(at);
            dir = Some(d);
            let s = STEP[usize::from(d)];
            at = (at.0 + s.0, at.1 + s.1);
            if at == start {
                break;
            }
        }
        loops.push(simplify(lp));
    }
    // Outer loops start their polygons; holes join in order of their
    // leftmost vertex, so the edge a hole's slit meets is already placed.
    let mut polys: Vec<Vec<GridPoint>> = Vec::new();
    let mut holes: Vec<Vec<GridPoint>> = Vec::new();
    for lp in loops {
        if area2(&lp) > 0 {
            polys.push(lp);
        } else {
            holes.push(lp);
        }
    }
    holes.sort_by_key(|h| leftmost(h));
    for hole in holes {
        let (hi, v) = leftmost_index(&hole);
        // The nearest vertical edge left of `v` that spans its row.
        let mut best: Option<(i32, usize, usize)> = None;
        for (pn, poly) in polys.iter().enumerate() {
            let m = poly.len();
            for e in 0..m {
                let (a, b) = (poly[e], poly[(e + 1) % m]);
                if a.0 != b.0 || a.0 >= v.0 {
                    continue;
                }
                let (lo, hi_y) = (a.1.min(b.1), a.1.max(b.1));
                if v.1 < lo || v.1 > hi_y {
                    continue;
                }
                if best.is_none_or(|(x, _, _)| a.0 > x) {
                    best = Some((a.0, pn, e));
                }
            }
        }
        let (x, pn, e) = best.expect("a hole lies inside an outer loop");
        let poly = &mut polys[pn];
        let p = (x, v.1);
        // ... a, P, v, (hole from v round to v), v, P, b ...
        let mut bridge = vec![p];
        bridge.extend(hole[hi..].iter().copied());
        bridge.extend(hole[..hi].iter().copied());
        bridge.push(v);
        bridge.push(p);
        let at = e + 1;
        poly.splice(at..at, bridge);
        dedup_ring(poly);
    }
    polys
}

/// Twice the signed area (positive counter-clockwise).
fn area2(p: &[GridPoint]) -> i64 {
    let n = p.len();
    (0..n)
        .map(|k| {
            let (a, b) = (p[k], p[(k + 1) % n]);
            i64::from(a.0) * i64::from(b.1) - i64::from(b.0) * i64::from(a.1)
        })
        .sum()
}

/// The leftmost vertex, the lowest among equals.
fn leftmost(p: &[GridPoint]) -> GridPoint {
    leftmost_index(p).1
}

fn leftmost_index(p: &[GridPoint]) -> (usize, GridPoint) {
    p.iter()
        .copied()
        .enumerate()
        .min_by_key(|&(_, v)| v)
        .expect("a loop has vertices")
}

/// Drops the vertices inside straight runs.
fn simplify(lp: Vec<GridPoint>) -> Vec<GridPoint> {
    let n = lp.len();
    let dir = |a: GridPoint, b: GridPoint| ((b.0 - a.0).signum(), (b.1 - a.1).signum());
    (0..n)
        .filter(|&k| dir(lp[(k + n - 1) % n], lp[k]) != dir(lp[k], lp[(k + 1) % n]))
        .map(|k| lp[k])
        .collect()
}

/// Removes repeated consecutive vertices (a slit meeting a vertex).
fn dedup_ring(p: &mut Vec<GridPoint>) {
    p.dedup();
    while p.len() > 1 && p.first() == p.last() {
        p.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Even-odd at cell centres, the sim's rule, in grid units.
    fn fill(points: &[GridPoint], cols: u32, rows: u32) -> Vec<bool> {
        let mut out = vec![false; (cols * rows) as usize];
        for j in 0..rows {
            for i in 0..cols {
                let (px, py) = (f64::from(i) + 0.5, f64::from(j) + 0.5);
                let n = points.len();
                let mut inside = false;
                for k in 0..n {
                    let (a, b) = (points[k], points[(k + n - 1) % n]);
                    let (ay, by) = (f64::from(a.1), f64::from(b.1));
                    if (ay <= py) != (by <= py) {
                        let x = f64::from(a.0) + (py - ay) * f64::from(b.0 - a.0) / (by - ay);
                        if px < x {
                            inside = !inside;
                        }
                    }
                }
                out[(j * cols + i) as usize] = inside;
            }
        }
        out
    }

    fn check(raster: &[u8], cols: u32, rows: u32) -> Vec<TracedPolygon> {
        let polys = trace_polygons(raster, cols, rows);
        let mut got = vec![0u8; raster.len()];
        for p in &polys {
            for (k, on) in fill(&p.points, cols, rows).into_iter().enumerate() {
                if on {
                    assert_eq!(got[k], 0, "polygons of one raster never overlap");
                    got[k] = p.value;
                }
            }
        }
        assert_eq!(got, raster, "the polygons fill exactly the painted cells");
        polys
    }

    #[test]
    fn a_square_is_four_corners() {
        let mut r = vec![0u8; 36];
        for j in 1..4 {
            for i in 1..5 {
                r[j * 6 + i] = 1;
            }
        }
        let p = check(&r, 6, 6);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].points, vec![(1, 1), (5, 1), (5, 4), (1, 4)]);
    }

    #[test]
    fn a_ring_keeps_its_hole() {
        let mut r = vec![0u8; 36];
        for j in 0..6 {
            for i in 0..6 {
                let ring = (1..5).contains(&i) && (1..5).contains(&j);
                let hole = (2..4).contains(&i) && (2..4).contains(&j);
                if ring && !hole {
                    r[j * 6 + i] = 1;
                }
            }
        }
        let p = check(&r, 6, 6);
        assert_eq!(p.len(), 1, "the hole is slit into the ring");
    }

    #[test]
    fn nested_holes_islands_pinches_and_values() {
        // A 12 × 12 frame of 1 around a hole holding an island of 2, a
        // diagonal pinch of 1s, a 3 touching the frame, the map edge used.
        let (c, r) = (12u32, 12u32);
        let mut g = vec![0u8; 144];
        let set = |g: &mut Vec<u8>, i: usize, j: usize, v: u8| g[j * 12 + i] = v;
        for j in 0..10 {
            for i in 0..10 {
                set(&mut g, i, j, 1);
            }
        }
        for j in 2..8 {
            for i in 2..8 {
                set(&mut g, i, j, 0);
            }
        }
        for j in 4..6 {
            for i in 4..6 {
                set(&mut g, i, j, 2);
            }
        }
        set(&mut g, 10, 10, 1);
        set(&mut g, 11, 11, 1);
        set(&mut g, 2, 2, 1);
        set(&mut g, 3, 3, 1);
        for i in 0..12 {
            set(&mut g, i, 11, 3);
        }
        set(&mut g, 11, 11, 1);
        check(&g, c, r);
    }

    #[test]
    fn random_rasters_round_trip() {
        // A fixed LCG: the patterns are arbitrary but repeatable.
        let mut s: u64 = 0x1234_5678;
        for _ in 0..200 {
            let (c, r) = (9u32, 7u32);
            let g: Vec<u8> = (0..c * r)
                .map(|_| {
                    s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                    ((s >> 33) % 4) as u8
                })
                .collect();
            check(&g, c, r);
        }
    }
}
