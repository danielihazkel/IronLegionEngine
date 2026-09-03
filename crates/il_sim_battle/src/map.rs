//! `LoadedMap`: the battle terrain as the sim reads it (T1-030; SIM-CORE-001,
//! SIM-MOVE-031..033, TDD §6.2).
//!
//! Built once per battle from an `il_data::MapDef` whose heightmap sidecar
//! the data pipeline already read. Heights are bilinear on a grid of
//! `height_cell`; zone polygons and rivers are rasterised at the centres of
//! `zone_cell` cells (later polygons override earlier ones; a river cell is
//! impassable unless a crossing zone covers it). Walls, gates and siege
//! points are parsed and stored inert until Phase 5.

use il_core::{S, Scalar, V2};
use il_data::{ContentId, DeploymentZone, Handle, MapDef, ReinforcementEdge, River, ZoneType};

/// Id of the placeholder map of `BattleWorld::empty` (flat, no zones).
pub const FLAT_MAP_ID: &str = "engine:flat";

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MapError {
    #[error("map {id}: heightmap holds {found} samples, {expected} expected")]
    HeightmapSize {
        id: ContentId,
        expected: usize,
        found: usize,
    },
    #[error("map {id}: {count} zone polygons exceed the limit of 255")]
    TooManyZones { id: ContentId, count: usize },
    #[error("map {id}: the base zone is unresolved")]
    UnresolvedZone { id: ContentId },
}

#[derive(Clone, Debug)]
pub struct LoadedMap {
    pub id: ContentId,
    pub width: S,
    pub height: S,
    /// Metres per height sample (SIM-CORE-001).
    pub height_cell: S,
    pub height_cols: u32,
    pub height_rows: u32,
    /// Row-major from `y = 0`, `height_cols × height_rows`, metres.
    pub heights: Vec<S>,
    /// Metres per zone raster cell (`movement.zone_cell`).
    pub zone_cell: S,
    pub zone_cols: u32,
    pub zone_rows: u32,
    /// Per zone cell: index into `zone_handles` (`0` = base zone).
    pub zones: Vec<u8>,
    /// Per zone cell: covered by a river.
    pub river: Vec<bool>,
    /// `[0]` is the base zone, then one entry per polygon in map order.
    /// Empty only for the flat placeholder map.
    pub zone_handles: Vec<Handle<ZoneType>>,
    pub rivers: Vec<River>,
    pub deployment: Vec<DeploymentZone>,
    pub reinforcement_edges: Vec<ReinforcementEdge>,
    /// Reserved (REQ-SIM-045), inert until Phase 5.
    pub structures: Vec<serde_json::Value>,
    pub siege_points: Vec<serde_json::Value>,
}

/// Smallest integer not below `v`.
fn ceil_i32(v: S) -> i32 {
    -((-v).floor_i32())
}

/// Even-odd point-in-polygon (crossing number).
pub fn polygon_contains(poly: &[V2], p: V2) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let a = poly[i];
        let b = poly[j];
        if (a.y <= p.y) != (b.y <= p.y) {
            let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if p.x < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Distance from `p` to the segment `a b`.
fn dist_point_segment(p: V2, a: V2, b: V2) -> S {
    let ab = b - a;
    let ap = p - a;
    let len_sq = ab.length_sq();
    let t = if len_sq > S::ZERO {
        (ap.dot(ab) / len_sq).clamp(S::ZERO, S::ONE)
    } else {
        S::ZERO
    };
    (ap - ab * t).length()
}

/// Calls `f(i, j)` for every cell of a `cols × rows` raster of `cell` whose
/// centre lies inside `poly` (scanline, even-odd, half-open on the right).
fn rasterise_polygon(poly: &[V2], cell: S, cols: u32, rows: u32, mut f: impl FnMut(u32, u32)) {
    if poly.len() < 3 {
        return;
    }
    let (mut y_min, mut y_max) = (poly[0].y, poly[0].y);
    for v in poly {
        y_min = y_min.min(v.y);
        y_max = y_max.max(v.y);
    }
    // Rows whose centre `(j + 0.5) * cell` lies in `[y_min, y_max]`.
    let j0 = ceil_i32(y_min / cell - S::HALF).max(0);
    let j1 = (y_max / cell - S::HALF).floor_i32().min(rows as i32 - 1);
    let mut xs: Vec<S> = Vec::new();
    for j in j0..=j1 {
        let y = (S::from_i32(j) + S::HALF) * cell;
        xs.clear();
        let n = poly.len();
        let mut prev = poly[n - 1];
        for &cur in poly {
            if (prev.y <= y) != (cur.y <= y) {
                xs.push(prev.x + (y - prev.y) * (cur.x - prev.x) / (cur.y - prev.y));
            }
            prev = cur;
        }
        xs.sort_by(|a, b| a.partial_cmp(b).expect("finite crossings"));
        for pair in xs.as_chunks::<2>().0 {
            // Cells whose centre `(i + 0.5) * cell` lies in `[x0, x1)`.
            let i0 = ceil_i32(pair[0] / cell - S::HALF).max(0);
            let i1 = ceil_i32(pair[1] / cell - S::HALF).min(cols as i32);
            for i in i0..i1 {
                f(i as u32, j as u32);
            }
        }
    }
}

/// Calls `f(i, j)` for every cell whose centre is within `half` of the
/// polyline (a capsule per segment).
fn rasterise_polyline(
    points: &[V2],
    half: S,
    cell: S,
    cols: u32,
    rows: u32,
    mut f: impl FnMut(u32, u32),
) {
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let x_min = a.x.min(b.x) - half;
        let x_max = a.x.max(b.x) + half;
        let y_min = a.y.min(b.y) - half;
        let y_max = a.y.max(b.y) + half;
        let i0 = (x_min / cell).floor_i32().max(0);
        let i1 = (x_max / cell).floor_i32().min(cols as i32 - 1);
        let j0 = (y_min / cell).floor_i32().max(0);
        let j1 = (y_max / cell).floor_i32().min(rows as i32 - 1);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let c = V2::new(
                    (S::from_i32(i) + S::HALF) * cell,
                    (S::from_i32(j) + S::HALF) * cell,
                );
                if dist_point_segment(c, a, b) <= half {
                    f(i as u32, j as u32);
                }
            }
        }
    }
}

impl LoadedMap {
    /// Builds the sim view of a map definition. `zone_cell` is
    /// `movement.zone_cell`; the height cell comes from the map.
    pub fn from_def(def: &MapDef, zone_cell: S) -> Result<Self, MapError> {
        let (height_cols, height_rows) = def.heightmap_dims();
        let expected = height_cols as usize * height_rows as usize;
        if def.heightmap.samples.len() != expected {
            return Err(MapError::HeightmapSize {
                id: def.id.clone(),
                expected,
                found: def.heightmap.samples.len(),
            });
        }
        if def.zones.len() >= usize::from(u8::MAX) {
            return Err(MapError::TooManyZones {
                id: def.id.clone(),
                count: def.zones.len(),
            });
        }
        let scale = def.heightmap.scale;
        let heights = def
            .heightmap
            .samples
            .iter()
            .map(|&raw| S::from_i32(i32::from(raw)) * scale)
            .collect();

        let mut zone_handles = Vec::with_capacity(def.zones.len() + 1);
        zone_handles.push(
            def.base_zone_handle
                .ok_or_else(|| MapError::UnresolvedZone { id: def.id.clone() })?,
        );
        for z in &def.zones {
            zone_handles.push(
                z.zone
                    .ok_or_else(|| MapError::UnresolvedZone { id: def.id.clone() })?,
            );
        }

        let zone_cols = ceil_i32(def.size.w / zone_cell).max(1) as u32;
        let zone_rows = ceil_i32(def.size.h / zone_cell).max(1) as u32;
        let cells = zone_cols as usize * zone_rows as usize;
        let mut zones = vec![0u8; cells];
        for (k, z) in def.zones.iter().enumerate() {
            let index = (k + 1) as u8;
            rasterise_polygon(&z.polygon, zone_cell, zone_cols, zone_rows, |i, j| {
                zones[j as usize * zone_cols as usize + i as usize] = index;
            });
        }
        let mut river = vec![false; cells];
        for r in &def.rivers {
            rasterise_polyline(
                &r.points,
                r.width * S::HALF,
                zone_cell,
                zone_cols,
                zone_rows,
                |i, j| river[j as usize * zone_cols as usize + i as usize] = true,
            );
        }

        Ok(Self {
            id: def.id.clone(),
            width: def.size.w,
            height: def.size.h,
            height_cell: def.heightmap.cell,
            height_cols,
            height_rows,
            heights,
            zone_cell,
            zone_cols,
            zone_rows,
            zones,
            river,
            zone_handles,
            rivers: def.rivers.clone(),
            deployment: def.deployment.clone(),
            reinforcement_edges: def.reinforcement_edges.clone(),
            structures: def.structures.clone(),
            siege_points: def.siege_points.clone(),
        })
    }

    /// A flat `width × height` map with no zones, rivers or deployment
    /// areas: the placeholder of `BattleWorld::empty`. `zone_at` is `None`
    /// everywhere.
    pub fn flat(width: S, height: S) -> Self {
        let cell = width.max(height);
        Self {
            id: ContentId::new(FLAT_MAP_ID).expect("valid id"),
            width,
            height,
            height_cell: cell,
            height_cols: 2,
            height_rows: 2,
            heights: vec![S::ZERO; 4],
            zone_cell: cell,
            zone_cols: 1,
            zone_rows: 1,
            zones: vec![0],
            river: vec![false],
            zone_handles: Vec::new(),
            rivers: Vec::new(),
            deployment: Vec::new(),
            reinforcement_edges: Vec::new(),
            structures: Vec::new(),
            siege_points: Vec::new(),
        }
    }

    /// Inside `[0, width] × [0, height]` (SIM-CORE-001).
    pub fn in_bounds(&self, p: V2) -> bool {
        p.x >= S::ZERO && p.x <= self.width && p.y >= S::ZERO && p.y <= self.height
    }

    /// The nearest point of the map rectangle (SIM-MOVE-042).
    pub fn clamp(&self, p: V2) -> V2 {
        V2::new(
            p.x.clamp(S::ZERO, self.width),
            p.y.clamp(S::ZERO, self.height),
        )
    }

    /// Bilinear terrain height in metres; positions outside the map read
    /// the nearest edge.
    pub fn height_at(&self, p: V2) -> S {
        let p = self.clamp(p);
        let gx = p.x / self.height_cell;
        let gy = p.y / self.height_cell;
        let cols = self.height_cols as i32;
        let rows = self.height_rows as i32;
        let i = gx.floor_i32().clamp(0, cols - 2);
        let j = gy.floor_i32().clamp(0, rows - 2);
        let fx = (gx - S::from_i32(i)).clamp(S::ZERO, S::ONE);
        let fy = (gy - S::from_i32(j)).clamp(S::ZERO, S::ONE);
        let at = |i: i32, j: i32| self.heights[(j * cols + i) as usize];
        let top = at(i, j) + (at(i + 1, j) - at(i, j)) * fx;
        let bottom = at(i, j + 1) + (at(i + 1, j + 1) - at(i, j + 1)) * fx;
        top + (bottom - top) * fy
    }

    /// Zone raster cell containing `p` (clamped to the map).
    pub fn zone_cell_of(&self, p: V2) -> (u32, u32) {
        let p = self.clamp(p);
        let i = (p.x / self.zone_cell)
            .floor_i32()
            .clamp(0, self.zone_cols as i32 - 1);
        let j = (p.y / self.zone_cell)
            .floor_i32()
            .clamp(0, self.zone_rows as i32 - 1);
        (i as u32, j as u32)
    }

    fn zone_slot(&self, p: V2) -> usize {
        let (i, j) = self.zone_cell_of(p);
        j as usize * self.zone_cols as usize + i as usize
    }

    /// Index into [`zone_handles`](Self::zone_handles) at `p`.
    pub fn zone_index_at(&self, p: V2) -> u8 {
        self.zones[self.zone_slot(p)]
    }

    /// Zone type at `p`; `None` only on the flat placeholder map.
    pub fn zone_at(&self, p: V2) -> Option<Handle<ZoneType>> {
        self.zone_handles
            .get(usize::from(self.zone_index_at(p)))
            .copied()
    }

    /// Whether a river covers the zone cell at `p` (a crossing zone may
    /// still make it passable; SIM-MOVE-032).
    pub fn river_at(&self, p: V2) -> bool {
        self.river[self.zone_slot(p)]
    }

    /// The deployment polygon of side index `side`, if the map defines one.
    pub fn deployment_polygon(&self, side: u8) -> Option<&[V2]> {
        self.deployment
            .iter()
            .find(|d| d.side == side)
            .map(|d| d.polygon.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: i32, y: i32) -> V2 {
        V2::new(S::from_i32(x), S::from_i32(y))
    }

    fn cells(poly: &[V2], cell: i32, cols: u32, rows: u32) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        rasterise_polygon(poly, S::from_i32(cell), cols, rows, |i, j| out.push((i, j)));
        out.sort_unstable();
        out
    }

    #[test]
    fn scanline_matches_point_in_polygon_at_cell_centres() {
        // A concave "L" and a triangle, both on a 2 m raster of 6 x 6 cells.
        let shapes = [
            vec![v(1, 1), v(9, 1), v(9, 5), v(5, 5), v(5, 11), v(1, 11)],
            vec![v(0, 0), v(12, 0), v(12, 12)],
            vec![v(3, 2), v(7, 9), v(2, 6)],
        ];
        for poly in &shapes {
            let mut expected: Vec<(u32, u32)> = (0..6)
                .flat_map(|j| (0..6).map(move |i| (i, j)))
                .filter(|&(i, j)| {
                    let c = V2::new(
                        (S::from_i32(i as i32) + S::HALF) * S::from_i32(2),
                        (S::from_i32(j as i32) + S::HALF) * S::from_i32(2),
                    );
                    polygon_contains(poly, c)
                })
                .collect();
            expected.sort_unstable();
            assert_eq!(cells(poly, 2, 6, 6), expected, "{poly:?}");
            assert!(!expected.is_empty());
        }
        // Clipping: a polygon far outside the raster touches nothing.
        assert!(cells(&[v(-20, -20), v(-10, -20), v(-10, -10)], 2, 6, 6).is_empty());
    }

    #[test]
    fn river_capsule_covers_cells_within_half_width() {
        let mut hit = Vec::new();
        rasterise_polyline(
            &[v(0, 5), v(12, 5)],
            S::from_f32_data(1.25),
            S::from_i32(2),
            6,
            6,
            |i, j| hit.push((i, j)),
        );
        hit.sort_unstable();
        // Rows with centres 5 (distance 0) and 7 (distance 2 > 1.25) and
        // 3 (distance 2): only row 2 (centre y = 5).
        assert_eq!(hit, (0..6).map(|i| (i, 2)).collect::<Vec<_>>());
    }

    #[test]
    fn flat_map_is_zero_everywhere_and_clamps() {
        let m = LoadedMap::flat(S::from_i32(100), S::from_i32(50));
        assert_eq!(m.height_at(v(30, 20)), S::ZERO);
        assert_eq!(m.height_at(v(-5, 500)), S::ZERO);
        assert!(m.in_bounds(v(100, 50)));
        assert!(!m.in_bounds(v(101, 50)));
        assert_eq!(m.clamp(v(-1, 60)), v(0, 50));
        assert_eq!(m.zone_at(v(10, 10)), None);
        assert!(!m.river_at(v(10, 10)));
        assert_eq!(m.zone_cell_of(v(99, 49)), (0, 0));
    }
}
