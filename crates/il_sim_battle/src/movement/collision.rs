//! Collision resolution (T1-044; SIM-MOVE-040..042, TDD §5 `for_each_pair`,
//! §6.2 `collision_resolve`, SAD §8 rule 2).
//!
//! Stage 7, `collision_iterations` passes over the pairs of this tick's
//! spatial grid: pairs are enumerated per cell row (rows in parallel when a
//! task pool exists) and sorted `(i, j)`, then folded in row order into
//! per-soldier push buffers, which are applied in ascending id with the
//! map clamp and push-out. Positions are written back once at the end and
//! the grid is rebuilt from them, so the grid always indexes end-of-tick
//! positions: the same grid a restore rebuilds (SIM-DET-005).

use bevy_ecs::prelude::*;
use il_core::{S, Scalar, SoldierId, V2};

use crate::components::{Body, Pos};
use crate::map::LoadedMap;
use crate::movement::integrate::push_out;
use crate::nav::NavGrid;
use crate::resources::{MapRes, NavGridRes, Regs, SpatialGridRes};
use crate::spatial::{Entry, SpatialGrid};

/// Radius and mass per grid entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disc {
    pub r: S,
    pub m: S,
}

/// SIM-MOVE-040 for one pair: the pushes on `i` and `j` (in that order), or
/// `None` when the discs do not overlap. Coincident centres separate along
/// `+x`, the lower id moving left.
pub fn pair_push(p_i: V2, d_i: Disc, p_j: V2, d_j: Disc) -> Option<(V2, V2)> {
    let delta = p_j - p_i;
    let dist = delta.length();
    let reach = d_i.r + d_j.r;
    if dist >= reach {
        return None;
    }
    let overlap = reach - dist;
    let n = if dist > S::ZERO {
        delta * (S::ONE / dist)
    } else {
        V2::new(S::ONE, S::ZERO)
    };
    let total = d_i.m + d_j.m;
    Some((
        -n * (overlap * d_j.m / total),
        n * (overlap * d_i.m / total),
    ))
}

/// Folds the sorted pair lists of every row into `push` (cleared first).
pub fn accumulate_pushes(pos: &[V2], discs: &[Disc], rows: &[Vec<(u32, u32)>], push: &mut Vec<V2>) {
    push.clear();
    push.resize(pos.len(), V2::ZERO);
    for row in rows {
        for &(i, j) in row {
            let (i, j) = (i as usize, j as usize);
            if let Some((a, b)) = pair_push(pos[i], discs[i], pos[j], discs[j]) {
                push[i] += a;
                push[j] += b;
            }
        }
    }
}

/// Enumerates the pairs of every row, sorted `(i, j)` within the row.
fn enumerate_rows(grid: &SpatialGrid<SoldierId>, rows: &mut Vec<Vec<(u32, u32)>>) {
    let n = grid.rows() as usize;
    rows.iter_mut().for_each(Vec::clear);
    rows.resize_with(n, Vec::new);
    let fill = |cy: usize, out: &mut Vec<(u32, u32)>| {
        grid.for_each_pair_in_row(cy as u32, |i, j| out.push((i as u32, j as u32)));
        out.sort_unstable();
    };
    match bevy_tasks::ComputeTaskPool::try_get() {
        Some(pool) if pool.thread_num() > 1 && n > 1 => {
            pool.scope(|scope| {
                for (cy, out) in rows.iter_mut().enumerate() {
                    scope.spawn(async move { fill(cy, out) });
                }
            });
        }
        _ => {
            for (cy, out) in rows.iter_mut().enumerate() {
                fill(cy, out);
            }
        }
    }
}

/// Stage 7 `collision_resolve`.
pub fn collision_resolve(world: &mut World) {
    let iterations = world
        .resource::<Regs>()
        .0
        .rules
        .movement
        .collision_iterations;
    if iterations == 0 {
        return;
    }
    world.resource_scope(|world, mut grid: Mut<SpatialGridRes>| {
        let grid = &mut grid.0;
        let entries = grid.entries();
        if entries.len() < 2 {
            return;
        }
        let mut pos: Vec<V2> = entries.iter().map(|e| e.pos).collect();
        let discs: Vec<Disc> = entries
            .iter()
            .map(|e| {
                world.get::<Body>(e.entity).map_or(
                    Disc {
                        r: S::ZERO,
                        m: S::ONE,
                    },
                    |b| Disc { r: b.r, m: b.m },
                )
            })
            .collect();
        let mut rows: Vec<Vec<(u32, u32)>> = Vec::new();
        enumerate_rows(grid, &mut rows);
        let mut push = Vec::new();
        let map: &LoadedMap = &world.resource::<MapRes>().0;
        let nav: &NavGrid = &world.resource::<NavGridRes>().0;
        let mut moved = vec![false; pos.len()];
        for _ in 0..iterations {
            accumulate_pushes(&pos, &discs, &rows, &mut push);
            for (k, p) in push.iter().enumerate() {
                if *p != V2::ZERO {
                    pos[k] = push_out(map, nav, pos[k], *p);
                    moved[k] = true;
                }
            }
        }
        if !moved.iter().any(|m| *m) {
            return;
        }
        for (k, e) in entries.iter().enumerate() {
            if moved[k]
                && let Some(mut p) = world.get_mut::<Pos>(e.entity)
            {
                p.p = pos[k];
            }
        }
        let updated: Vec<Entry<SoldierId>> = entries
            .iter()
            .zip(&pos)
            .map(|(e, p)| Entry {
                id: e.id,
                entity: e.entity,
                pos: *p,
            })
            .collect();
        grid.rebuild(updated);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(r: f32, m: f32) -> Disc {
        Disc {
            r: S::from_f32_data(r),
            m: S::from_f32_data(m),
        }
    }

    #[test]
    fn pushes_separate_to_touching_and_preserve_the_weighted_centre() {
        let a = V2::from_f32_data(0.0, 0.0);
        let b = V2::from_f32_data(0.5, 0.0);
        let (da, db) = (disc(0.4, 80.0), disc(0.4, 400.0));
        let (pa, pb) = pair_push(a, da, b, db).unwrap();
        let (a2, b2) = (a + pa, b + pb);
        assert!((a2.distance(b2) - S::from_f32_data(0.8)).abs() < S::from_f32_data(1e-5));
        // The lighter disc moves five times as far.
        assert!((pa.length() / pb.length() - S::from_i32(5)).abs() < S::from_f32_data(1e-4));
        let before = a * da.m + b * db.m;
        let after = a2 * da.m + b2 * db.m;
        assert!((before - after).length() < S::from_f32_data(1e-4));
        assert!(pair_push(a, da, V2::from_f32_data(0.8, 0.0), da).is_none());
        // Coincident centres still separate.
        let (pa, pb) = pair_push(a, da, a, da).unwrap();
        assert!(pa.x < S::ZERO && pb.x > S::ZERO);
    }

    #[test]
    fn accumulation_is_a_fixed_order_sum() {
        let pos = vec![
            V2::from_f32_data(0.0, 0.0),
            V2::from_f32_data(0.5, 0.0),
            V2::from_f32_data(0.25, 0.4),
        ];
        let discs = vec![disc(0.4, 80.0); 3];
        let rows = vec![vec![(0, 1), (0, 2), (1, 2)]];
        let mut push = Vec::new();
        accumulate_pushes(&pos, &discs, &rows, &mut push);
        let mut again = Vec::new();
        accumulate_pushes(&pos, &discs, &rows, &mut again);
        assert_eq!(push, again);
        // Equal masses: the sum of pushes is zero (centre preserved).
        let total = push.iter().fold(V2::ZERO, |a, p| a + *p);
        assert!(total.length() < S::from_f32_data(1e-5), "{total:?}");
    }
}
