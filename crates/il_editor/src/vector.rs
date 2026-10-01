//! The vector tools' geometry (T3-062, TDD §16): a road polyline widened
//! into its zone polygon, the map features the select tool edits, vertex
//! and edge picks in screen space, and the height grid resampled when the
//! map's size changes. Editing math is render-side f32.

use glam::Vec2;
use il_core::{Scalar, V2};
use serde_json::Value;

/// A miter longer than this many half-widths becomes a bevel.
const MITER_LIMIT: f32 = 2.0;

/// The outline of `points` widened to `width`: flat ends, mitred joins
/// (bevelled past [`MITER_LIMIT`]), counter-clockwise. A straight road is
/// the rectangle `genmap` writes for its road. Empty for fewer than two
/// distinct points.
pub fn road_outline(points: &[Vec2], width: f32) -> Vec<Vec2> {
    let mut pts: Vec<Vec2> = Vec::with_capacity(points.len());
    for p in points {
        if pts.last().is_none_or(|q: &Vec2| q.distance(*p) > 1e-3) {
            pts.push(*p);
        }
    }
    if pts.len() < 2 || width <= 0.0 {
        return Vec::new();
    }
    let half = width * 0.5;
    let n = pts.len();
    let dir = |k: usize| (pts[k + 1] - pts[k]).normalize();
    let normal = |d: Vec2| Vec2::new(-d.y, d.x);
    // One side's offset points, from the first point to the last.
    let side = |s: f32| -> Vec<Vec2> {
        let mut out = vec![pts[0] + normal(dir(0)) * half * s];
        for (k, p) in pts.iter().enumerate().take(n - 1).skip(1) {
            let p = *p;
            let (a, b) = (normal(dir(k - 1)), normal(dir(k)));
            let m = (a + b).normalize_or_zero();
            let cos = m.dot(a);
            if m == Vec2::ZERO || cos <= 1.0 / MITER_LIMIT {
                out.push(p + a * half * s);
                out.push(p + b * half * s);
            } else {
                out.push(p + m * (half / cos) * s);
            }
        }
        out.push(pts[n - 1] + normal(dir(n - 2)) * half * s);
        out
    };
    // The right side forwards, then the left side backwards.
    let mut outline = side(-1.0);
    let mut left = side(1.0);
    left.reverse();
    outline.extend(left);
    outline
}

/// A map feature the select tool edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feature {
    Zone(usize),
    River(usize),
    Deployment(usize),
    Structure(usize),
    SiegePoint(usize),
}

impl Feature {
    /// The fewest vertices the feature keeps before it is removed whole.
    pub fn min_vertices(self) -> usize {
        match self {
            Feature::Zone(_) | Feature::Deployment(_) => 3,
            Feature::River(_) => 2,
            Feature::Structure(_) | Feature::SiegePoint(_) => 1,
        }
    }

    /// Whether the vertices close into a ring.
    pub fn closed(self) -> bool {
        matches!(self, Feature::Zone(_) | Feature::Deployment(_))
    }
}

pub fn to_vec2(p: V2) -> Vec2 {
    Vec2::new(p.x.to_f32_render(), p.y.to_f32_render())
}

pub fn to_v2(p: Vec2) -> V2 {
    V2::from_f32_data(p.x, p.y)
}

/// A `[x, y]` pair of a structure record.
fn json_point(v: &Value) -> Option<Vec2> {
    let a = v.as_array()?;
    Some(Vec2::new(
        a.first()?.as_f64()? as f32,
        a.get(1)?.as_f64()? as f32,
    ))
}

pub fn json_pair(p: Vec2) -> Value {
    Value::from(vec![
        Value::from(f64::from(p.x)),
        Value::from(f64::from(p.y)),
    ])
}

/// The vertices of a structure or siege point record: its `polyline`, or
/// its `at` as one vertex.
pub fn record_points(v: &Value) -> Vec<Vec2> {
    if let Some(line) = v.get("polyline").and_then(Value::as_array) {
        return line.iter().filter_map(json_point).collect();
    }
    v.get("at").and_then(json_point).into_iter().collect()
}

/// Writes `points` back into the record's `polyline` or `at`.
pub fn set_record_points(v: &mut Value, points: &[Vec2]) {
    let Some(obj) = v.as_object_mut() else {
        return;
    };
    if obj.contains_key("polyline") {
        obj.insert(
            "polyline".into(),
            Value::from(points.iter().map(|p| json_pair(*p)).collect::<Vec<_>>()),
        );
    } else if let Some(p) = points.first() {
        obj.insert("at".into(), json_pair(*p));
    }
}

/// The nearest point to `p` on segment `a b` and its parameter.
fn closest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> (Vec2, f32) {
    let ab = b - a;
    let len = ab.length_squared();
    let t = if len > 0.0 {
        ((p - a).dot(ab) / len).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + ab * t, t)
}

/// The index of the vertex of `screen_pts` nearest `cursor` within
/// `radius` pixels.
pub fn pick_vertex(screen_pts: &[Vec2], cursor: Vec2, radius: f32) -> Option<(usize, f32)> {
    screen_pts
        .iter()
        .enumerate()
        .map(|(k, p)| (k, p.distance(cursor)))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// The edge of `screen_pts` (`k` joins vertex `k` and `k + 1`, wrapping
/// when `closed`) nearest `cursor` within `radius` pixels, with the
/// parameter along it.
pub fn pick_edge(
    screen_pts: &[Vec2],
    closed: bool,
    cursor: Vec2,
    radius: f32,
) -> Option<(usize, f32, f32)> {
    let n = screen_pts.len();
    if n < 2 {
        return None;
    }
    let edges = if closed { n } else { n - 1 };
    (0..edges)
        .map(|k| {
            let (q, t) = closest_on_segment(cursor, screen_pts[k], screen_pts[(k + 1) % n]);
            (k, t, q.distance(cursor))
        })
        .filter(|(_, _, d)| *d <= radius)
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

/// The height grid after a size change: the cell stays, so every new
/// sample inside the old grid keeps its height and samples past it repeat
/// the old edge.
pub fn resample(heights: &[f32], cols: u32, rows: u32, new_cols: u32, new_rows: u32) -> Vec<f32> {
    let at = |i: i64, j: i64| -> f32 {
        let i = i.clamp(0, i64::from(cols) - 1);
        let j = j.clamp(0, i64::from(rows) - 1);
        heights[(j * i64::from(cols) + i) as usize]
    };
    let mut out = Vec::with_capacity(new_cols as usize * new_rows as usize);
    for j in 0..new_rows {
        for i in 0..new_cols {
            // The same cell size: sample (i, j) sits on old sample (i, j).
            out.push(at(i64::from(i), i64::from(j)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_straight_road_is_the_genmap_rectangle() {
        let o = road_outline(&[Vec2::new(400.0, 0.0), Vec2::new(400.0, 600.0)], 8.0);
        assert_eq!(
            o,
            vec![
                Vec2::new(404.0, 0.0),
                Vec2::new(404.0, 600.0),
                Vec2::new(396.0, 600.0),
                Vec2::new(396.0, 0.0),
            ]
        );
    }

    #[test]
    fn a_bent_road_mitres_and_a_hairpin_bevels() {
        let o = road_outline(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(100.0, 0.0),
                Vec2::new(100.0, 100.0),
            ],
            10.0,
        );
        assert_eq!(o.len(), 6);
        let near = |q: Vec2| o.iter().any(|p| p.distance(q) < 1e-3);
        assert!(near(Vec2::new(105.0, -5.0)), "{o:?}");
        assert!(near(Vec2::new(95.0, 5.0)), "{o:?}");
        let h = road_outline(
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(100.0, 0.0),
                Vec2::new(0.0, 1.0),
            ],
            10.0,
        );
        assert_eq!(h.len(), 8, "both sides bevel at the hairpin");
        assert!(road_outline(&[Vec2::ONE, Vec2::ONE], 8.0).is_empty());
    }

    #[test]
    fn picks_find_the_nearest_vertex_and_edge() {
        let pts = [
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
        ];
        assert_eq!(
            pick_vertex(&pts, Vec2::new(97.0, 3.0), 8.0).map(|p| p.0),
            Some(1)
        );
        assert_eq!(pick_vertex(&pts, Vec2::new(50.0, 3.0), 8.0), None);
        let (k, t, _) = pick_edge(&pts, false, Vec2::new(50.0, 3.0), 8.0).unwrap();
        assert_eq!(k, 0);
        assert!((t - 0.5).abs() < 1e-6);
        assert_eq!(pick_edge(&pts, false, Vec2::new(50.0, 50.0), 8.0), None);
        let (k, _, _) = pick_edge(&pts, true, Vec2::new(50.0, 50.0), 8.0).unwrap();
        assert_eq!(k, 2, "the closing edge of a ring");
    }

    #[test]
    fn resampling_keeps_the_overlap_and_extends_the_edge() {
        let h: Vec<f32> = (0..12).map(|k| k as f32).collect(); // 4 × 3
        let g = resample(&h, 4, 3, 5, 2);
        assert_eq!(g, vec![0.0, 1.0, 2.0, 3.0, 3.0, 4.0, 5.0, 6.0, 7.0, 7.0]);
    }

    #[test]
    fn records_keep_their_shape() {
        let mut wall = serde_json::json!({ "kind": "wall", "polyline": [[1, 2], [3, 4]] });
        assert_eq!(
            record_points(&wall),
            vec![Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)]
        );
        set_record_points(&mut wall, &[Vec2::new(5.0, 6.0), Vec2::new(7.0, 8.0)]);
        assert_eq!(wall["polyline"][1][0], 7.0);
        let mut gate = serde_json::json!({ "kind": "gate", "at": [10, 20] });
        set_record_points(&mut gate, &[Vec2::new(11.0, 21.0)]);
        assert_eq!(record_points(&gate), vec![Vec2::new(11.0, 21.0)]);
    }
}
