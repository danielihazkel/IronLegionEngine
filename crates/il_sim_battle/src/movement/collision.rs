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
//!
//! T3-022 kept every result bit for bit (`S` is an `f32`, so a soldier's
//! push is the same floating sum only if the same additions happen in the
//! same order) while cutting the cost:
//!
//! - the buffers live in [`CollisionScratch`] across ticks and the radii and
//!   masses come from the [`SoldierBodies`] table the Stage 6 rebuild fills;
//! - a squared-distance pre-check skips the pairs [`pair_push`] would reject
//!   before their square root ([`pre_check_slack`]);
//! - a pass that moved nobody ends the loop (the next pass would recompute
//!   the same zero pushes);
//! - the pair set is *narrowed* at enumeration to the pairs closer than
//!   `2 r_max + M` ([`narrow_margin`]) at the start of the tick, with a
//!   runtime guard: a dropped pair sits more than `M` beyond touching, so
//!   while the two soldiers' travel so far (each one's sum of steps) stays
//!   under `M` the pair cannot overlap and contributes no addition; the
//!   kept pairs keep their `(row, i, j)` order, so every soldier's fold is
//!   the T1-044 fold. The guard is local: per grid cell the largest travel
//!   sum, and a row whose cells (with their half-neighbourhood) could hold
//!   a dropped pair past the bound is re-enumerated in full before the next
//!   pass, as T1-044 enumerated every row; the other rows stay narrowed.
//!   [`CollisionScratch::widened`] counts the ticks with a widened row.

use bevy_ecs::prelude::*;
use il_core::{S, Scalar, SoldierId, V2};

use crate::components::Pos;
use crate::map::LoadedMap;
use crate::movement::integrate::push_out;
use crate::nav::NavGrid;
use crate::resources::{MapRes, NavGridRes, Regs, SpatialGridRes};
use crate::spatial::{Entry, SoldierBodies, SpatialGrid};

/// Radius and mass per grid entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Disc {
    pub r: S,
    pub m: S,
}

/// Buffers `collision_resolve` reuses across ticks (derived scratch, never
/// hashed or snapshotted; T3-022), plus two counters for the evidence.
#[derive(Resource, Default)]
pub struct CollisionScratch {
    rows: Vec<Vec<(u32, u32)>>,
    pos: Vec<V2>,
    push: Vec<V2>,
    moved: Vec<bool>,
    updated: Vec<Entry<SoldierId>>,
    /// Guard state: travel bound per entry, its maximum per cell, the cells
    /// with any travel, the rows widened this tick, the rows to widen now.
    travel: Vec<S>,
    cell_max: Vec<S>,
    hot_cells: Vec<u32>,
    row_full: Vec<bool>,
    to_widen: Vec<u32>,
    /// Ticks resolved so far.
    pub ticks: u64,
    /// Ticks in which at least one row's pair set had to be widened to the
    /// full neighbourhood for a later pass.
    pub widened: u64,
    /// Rows enumerated (narrowed) and rows widened, summed over the ticks.
    pub rows_total: u64,
    pub rows_widened: u64,
}

/// Relative slack of the squared pre-check in [`accumulate_pushes`]: a pair
/// is skipped only when `d² > reach² × (1 + 2⁻¹⁶)`. Then `d² > reach² ×
/// (1 + 2⁻¹⁷)` holds for the exact values even after the two roundings of
/// `reach × reach` and of the product, so `√d² > reach × (1 + 2⁻¹⁸)`, which
/// is more than an ulp above the representable `reach`, and the rounded
/// square root [`pair_push`] compares is at least `reach`: the pair would
/// have returned `None`. Pairs inside the slack take the exact path.
fn pre_check_slack() -> S {
    S::ONE / S::from_i32(65_536)
}

/// `M` of the narrowed pair set: a pair is enumerated only when its
/// start-of-tick distance is at most `2 r_max + M` (1 m). Not a rule: any
/// value gives the same positions; it trades pairs against widenings.
fn narrow_margin() -> S {
    S::ONE
}

/// The guard widens a row once two travel bounds in its neighbourhood sum
/// to `M × (1 − 2⁻¹⁰)`: the 2⁻¹⁰ absorbs the roundings of the enumeration's
/// squared distances and of the travel sums.
fn guard_factor() -> S {
    S::ONE - S::ONE / S::from_i32(1024)
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
/// Pairs the squared pre-check rejects are skipped before [`pair_push`];
/// every skipped pair is one `pair_push` returns `None` for (see
/// [`pre_check_slack`]), so each soldier sees exactly the additions of the
/// plain fold, in the same order.
pub fn accumulate_pushes(pos: &[V2], discs: &[Disc], rows: &[Vec<(u32, u32)>], push: &mut Vec<V2>) {
    push.clear();
    push.resize(pos.len(), V2::ZERO);
    let slack = pre_check_slack();
    for row in rows {
        for &(i, j) in row {
            let (i, j) = (i as usize, j as usize);
            let reach = discs[i].r + discs[j].r;
            let reach_sq = reach * reach;
            if (pos[j] - pos[i]).length_sq() > reach_sq + reach_sq * slack {
                continue;
            }
            if let Some((a, b)) = pair_push(pos[i], discs[i], pos[j], discs[j]) {
                push[i] += a;
                push[j] += b;
            }
        }
    }
}

/// The pairs of row `cy`, sorted `(i, j)`; with `keep_sq`, only the pairs
/// whose squared distance in the grid is at most it.
fn fill_row(grid: &SpatialGrid<SoldierId>, cy: u32, out: &mut Vec<(u32, u32)>, keep_sq: Option<S>) {
    let entries = grid.entries();
    out.clear();
    match keep_sq {
        Some(keep_sq) => grid.for_each_pair_in_row(cy, |i, j| {
            if entries[i].pos.distance_sq(entries[j].pos) <= keep_sq {
                out.push((i as u32, j as u32));
            }
        }),
        None => grid.for_each_pair_in_row(cy, |i, j| out.push((i as u32, j as u32))),
    }
    out.sort_unstable();
}

/// Enumerates every row (rows in parallel when a task pool exists).
fn enumerate_rows(
    grid: &SpatialGrid<SoldierId>,
    rows: &mut Vec<Vec<(u32, u32)>>,
    keep_sq: Option<S>,
) {
    let n = grid.rows() as usize;
    rows.iter_mut().for_each(Vec::clear);
    rows.resize_with(n, Vec::new);
    match bevy_tasks::ComputeTaskPool::try_get() {
        Some(pool) if pool.thread_num() > 1 && n > 1 => {
            pool.scope(|scope| {
                for (cy, out) in rows.iter_mut().enumerate() {
                    scope.spawn(async move { fill_row(grid, cy as u32, out, keep_sq) });
                }
            });
        }
        _ => {
            for (cy, out) in rows.iter_mut().enumerate() {
                fill_row(grid, cy as u32, out, keep_sq);
            }
        }
    }
}

/// Re-enumerates the listed rows with the full neighbourhood.
fn widen_rows(grid: &SpatialGrid<SoldierId>, rows: &mut [Vec<(u32, u32)>], which: &[u32]) {
    match bevy_tasks::ComputeTaskPool::try_get() {
        Some(pool) if pool.thread_num() > 1 && which.len() > 1 => {
            // Distinct rows, so each buffer is handed to one task.
            let mut taken: Vec<(u32, &mut Vec<(u32, u32)>)> = Vec::with_capacity(which.len());
            let mut rest: &mut [Vec<(u32, u32)>] = rows;
            let mut offset = 0u32;
            for &cy in which {
                let (head, tail) = std::mem::take(&mut rest).split_at_mut((cy - offset) as usize);
                let _ = head;
                let (row, tail) = tail.split_first_mut().expect("row index in range");
                taken.push((cy, row));
                rest = tail;
                offset = cy + 1;
            }
            pool.scope(|scope| {
                for (cy, out) in taken {
                    scope.spawn(async move { fill_row(grid, cy, out, None) });
                }
            });
        }
        _ => {
            for &cy in which {
                fill_row(grid, cy, &mut rows[cy as usize], None);
            }
        }
    }
}

/// The passes of one tick over `scratch.pos` (the grid's positions, in
/// entry order) with the discs of the entries: SIM-MOVE-040/041 with the
/// narrowed pair set when `margin` is given (see the module docs), the full
/// set otherwise. Marks `scratch.moved` and returns whether any row's pair
/// set had to be widened.
fn resolve_passes(
    map: &LoadedMap,
    nav: &NavGrid,
    grid: &SpatialGrid<SoldierId>,
    discs: &[Disc],
    iterations: u16,
    margin: Option<S>,
    scratch: &mut CollisionScratch,
) -> bool {
    let n = scratch.pos.len();
    scratch.moved.clear();
    scratch.moved.resize(n, false);
    let keep_sq = margin.map(|m| {
        let r_max = discs.iter().fold(S::ZERO, |a, d| a.max(d.r));
        let t = r_max + r_max + m;
        t * t
    });
    enumerate_rows(grid, &mut scratch.rows, keep_sq);
    scratch.rows_total += grid.rows() as u64;
    let guard = margin.map(|m| m * guard_factor());
    // Per soldier, the sum of its steps so far (a bound on its travel); per
    // cell, the largest such sum; the cells with any travel; the rows
    // already widened.
    scratch.travel.clear();
    scratch.travel.resize(n, S::ZERO);
    scratch.cell_max.clear();
    scratch.cell_max.resize(grid.cell_count(), S::ZERO);
    scratch.hot_cells.clear();
    scratch.row_full.clear();
    scratch.row_full.resize(grid.rows() as usize, false);
    let mut widened = false;
    for pass in 0..iterations {
        if let Some(guard) = guard
            && pass > 0
        {
            // A dropped pair (cells a, b in the half-neighbourhood) can
            // overlap only if the two soldiers' travel sums reach M; the
            // row that emits the pair is the lower cell row.
            scratch.to_widen.clear();
            for &c in &scratch.hot_cells {
                let (cx, cy) = grid.cell_coords(c);
                let own = scratch.cell_max[c as usize];
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (i64::from(cx) + dx, i64::from(cy) + dy);
                        if nx < 0
                            || ny < 0
                            || nx >= i64::from(grid.cols())
                            || ny >= i64::from(grid.rows())
                        {
                            continue;
                        }
                        let nc = grid.cell_index_of(nx as u32, ny as u32);
                        if own + scratch.cell_max[nc] >= guard {
                            let row = cy.min(ny as u32);
                            if !scratch.row_full[row as usize] {
                                scratch.row_full[row as usize] = true;
                                scratch.to_widen.push(row);
                            }
                        }
                    }
                }
            }
            if !scratch.to_widen.is_empty() {
                scratch.to_widen.sort_unstable();
                widen_rows(grid, &mut scratch.rows, &scratch.to_widen);
                scratch.rows_widened += scratch.to_widen.len() as u64;
                widened = true;
            }
        }
        accumulate_pushes(&scratch.pos, discs, &scratch.rows, &mut scratch.push);
        let mut changed = false;
        for (k, p) in scratch.push.iter().enumerate() {
            if *p != V2::ZERO {
                let next = push_out(map, nav, scratch.pos[k], *p);
                if next != scratch.pos[k] {
                    changed = true;
                    if guard.is_some() {
                        let travelled = scratch.travel[k] + next.distance(scratch.pos[k]);
                        scratch.travel[k] = travelled;
                        let c = grid.entry_cell(k);
                        if scratch.cell_max[c as usize] == S::ZERO {
                            scratch.hot_cells.push(c);
                        }
                        scratch.cell_max[c as usize] = scratch.cell_max[c as usize].max(travelled);
                    }
                    scratch.pos[k] = next;
                }
                scratch.moved[k] = true;
            }
        }
        if !changed {
            break;
        }
    }
    widened
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
        world.resource_scope(|world, mut scratch: Mut<CollisionScratch>| {
            let grid = &mut grid.0;
            let scratch = &mut *scratch;
            let entries = grid.entries();
            if entries.len() < 2 {
                return;
            }
            let discs: &[Disc] = &world.resource::<SoldierBodies>().discs;
            debug_assert_eq!(
                discs.len(),
                entries.len(),
                "body table aligned with the grid"
            );
            scratch.pos.clear();
            scratch.pos.extend(entries.iter().map(|e| e.pos));
            let map: &LoadedMap = &world.resource::<MapRes>().0;
            let nav: &NavGrid = &world.resource::<NavGridRes>().0;
            let widened = resolve_passes(
                map,
                nav,
                grid,
                discs,
                iterations,
                Some(narrow_margin()),
                scratch,
            );
            scratch.ticks += 1;
            scratch.widened += u64::from(widened);
            if !scratch.moved.iter().any(|m| *m) {
                return;
            }
            for (k, e) in entries.iter().enumerate() {
                if scratch.moved[k]
                    && let Some(mut p) = world.get_mut::<Pos>(e.entity)
                {
                    p.p = scratch.pos[k];
                }
            }
            scratch.updated.clear();
            scratch
                .updated
                .extend(entries.iter().zip(&scratch.pos).map(|(e, p)| Entry {
                    id: e.id,
                    entity: e.entity,
                    pos: *p,
                }));
            // Same ids in the same order, so `SoldierBodies` stays aligned.
            grid.rebuild(scratch.updated.drain(..));
        });
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

    /// The T1-044 fold without the pre-check: the reference the optimised
    /// fold must match bit for bit.
    fn accumulate_plain(pos: &[V2], discs: &[Disc], rows: &[Vec<(u32, u32)>]) -> Vec<V2> {
        let mut push = vec![V2::ZERO; pos.len()];
        for row in rows {
            for &(i, j) in row {
                let (i, j) = (i as usize, j as usize);
                if let Some((a, b)) = pair_push(pos[i], discs[i], pos[j], discs[j]) {
                    push[i] += a;
                    push[j] += b;
                }
            }
        }
        push
    }

    /// Small deterministic generator (no `rand` in sim crates).
    struct Lcg(u64);
    impl Lcg {
        fn next_unit(&mut self) -> S {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            S::from_i32(((self.0 >> 40) & 0xffff) as i32) / S::from_i32(0x1_0000)
        }
    }

    /// A jittered lattice of `n` discs at roughly touching distance, one in
    /// five a horse, starting at `origin`.
    fn crowd(lcg: &mut Lcg, n: usize, origin: V2, pitch: f32) -> (Vec<V2>, Vec<Disc>) {
        let mut pos = Vec::with_capacity(n);
        let mut discs = Vec::with_capacity(n);
        let (pitch, jitter) = (S::from_f32_data(pitch), S::from_f32_data(0.3));
        for k in 0..n {
            let (gx, gy) = (S::from_i32((k % 8) as i32), S::from_i32((k / 8) as i32));
            pos.push(
                origin
                    + V2::new(
                        gx * pitch + (lcg.next_unit() - S::HALF) * jitter,
                        gy * pitch + (lcg.next_unit() - S::HALF) * jitter,
                    ),
            );
            discs.push(if lcg.next_unit() < S::from_f32_data(0.2) {
                disc(0.7, 400.0)
            } else {
                disc(0.4, 80.0)
            });
        }
        (pos, discs)
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

    /// T3-022: the squared pre-check changes no push, bit for bit, on dense
    /// random crowds of mixed radii (many pairs sit right at the touching
    /// distance, where the slack matters).
    #[test]
    fn the_pre_check_matches_the_plain_fold_bit_for_bit() {
        let mut lcg = Lcg(0x5eed);
        for round in 0..40 {
            let n = 60;
            let (pos, discs) = crowd(&mut lcg, n, V2::ZERO, 0.8);
            // Every pair, one row, sorted like the grid's lists.
            let mut row = Vec::new();
            for i in 0..n as u32 {
                for j in i + 1..n as u32 {
                    row.push((i, j));
                }
            }
            let rows = vec![row];
            let mut push = Vec::new();
            accumulate_pushes(&pos, &discs, &rows, &mut push);
            let plain = accumulate_plain(&pos, &discs, &rows);
            assert_eq!(push, plain, "round {round}");
            assert!(
                push.iter().any(|p| *p != V2::ZERO),
                "round {round}: no overlap"
            );
        }
        // Exactly at the touching distance: rejected by both paths.
        let pos = vec![V2::ZERO, V2::from_f32_data(0.8, 0.0)];
        let discs = vec![disc(0.4, 80.0); 2];
        let mut push = Vec::new();
        accumulate_pushes(&pos, &discs, &[vec![(0, 1)]], &mut push);
        assert_eq!(push, vec![V2::ZERO; 2]);
    }

    /// Runs the passes over a crowd on a flat 40 × 40 m map with a 4 m grid.
    fn run_passes(
        pos: &[V2],
        discs: &[Disc],
        iterations: u16,
        margin: Option<S>,
    ) -> (Vec<V2>, Vec<bool>, bool) {
        let size = S::from_i32(40);
        let map = LoadedMap::flat(size, size);
        let nav = NavGrid::from_costs(S::from_i32(4), 10, 10, vec![100u16; 100]);
        let mut grid = SpatialGrid::new(size, size, S::from_i32(4));
        grid.rebuild(pos.iter().enumerate().map(|(k, p)| Entry {
            id: SoldierId(k as u32),
            entity: Entity::PLACEHOLDER,
            pos: *p,
        }));
        let mut scratch = CollisionScratch {
            pos: pos.to_vec(),
            ..Default::default()
        };
        let widened = resolve_passes(&map, &nav, &grid, discs, iterations, margin, &mut scratch);
        (scratch.pos, scratch.moved, widened)
    }

    /// T3-022: the narrowed pair set gives the T1-044 positions bit for bit,
    /// with and without a widening, over several pass counts.
    #[test]
    fn the_narrowed_pair_set_matches_the_full_set_bit_for_bit() {
        let mut lcg = Lcg(0xc011);
        for round in 0..30 {
            // Two crowds, one packed tight (large pushes), on the same map.
            let (mut pos, mut discs) = crowd(&mut lcg, 64, V2::from_f32_data(6.0, 6.0), 0.9);
            let (p2, d2) = crowd(&mut lcg, 48, V2::from_f32_data(20.0, 18.0), 0.55);
            pos.extend(p2);
            discs.extend(d2);
            for iterations in [1, 2, 4] {
                let (full, full_moved, w) = run_passes(&pos, &discs, iterations, None);
                assert!(!w);
                let (narrow, narrow_moved, _) =
                    run_passes(&pos, &discs, iterations, Some(narrow_margin()));
                assert_eq!(narrow, full, "round {round}, {iterations} passes");
                assert_eq!(narrow_moved, full_moved);
                // A margin so small the guard must widen before pass 2.
                let (tiny, _, widened) =
                    run_passes(&pos, &discs, iterations, Some(S::from_f32_data(0.001)));
                assert_eq!(
                    tiny, full,
                    "round {round}, {iterations} passes, tiny margin"
                );
                assert_eq!(widened, iterations > 1, "round {round}");
            }
        }
    }
}
