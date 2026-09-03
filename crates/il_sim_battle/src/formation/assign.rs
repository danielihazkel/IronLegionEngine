//! Slot assignment (T1-041; SIM-FORM-021..023, TDD §7 `assign_slots`).
//!
//! Three passes over a regiment's soldiers (ascending id) and its slots
//! (rank-major): keep the current slot when it still exists and is within
//! `keep_slot_radius`; assign the rest greedily to the nearest free slot,
//! searched through a small grid over the slots in growing rings up to
//! `assign_search_radius` and by brute force beyond; then `swap_passes`
//! passes over the pairs of each rank swapping any two assignments that
//! lower the total squared distance. Every loop runs in id or index order,
//! so the result is a function of the inputs alone.

use bevy_ecs::entity::Entity;
use il_core::{S, Scalar, SoldierId, V2};
use il_data::{FormationRules, UnitCategory};

use crate::components::Anchor;
use crate::formation::layout::Slot;
use crate::spatial::{Entry, SpatialGrid};

/// One soldier as the assignment sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssignSoldier {
    pub id: SoldierId,
    pub pos: V2,
    pub category: UnitCategory,
}

/// The world-space basis of a formation frame: `(right, forward)` for an
/// anchor facing `θ` (local x = right, local y = forward; `θ = 0` faces +x,
/// so forward = (cos θ, sin θ) and right = (sin θ, −cos θ)).
#[inline]
pub fn frame(anchor: &Anchor) -> (V2, V2) {
    let forward = anchor.facing.direction();
    (V2::new(forward.y, -forward.x), forward)
}

/// A local formation offset in world space (SIM-FORM-001, `R(θ_a) · o`
/// with the frame above).
#[inline]
pub fn local_to_world(anchor: &Anchor, offset: V2) -> V2 {
    let (right, forward) = frame(anchor);
    anchor.pos + right * offset.x + forward * offset.y
}

/// World position of `slot` for an anchor (SIM-FORM-001).
#[inline]
pub fn slot_world(anchor: &Anchor, slot: &Slot) -> V2 {
    local_to_world(anchor, slot.offset)
}

/// Reusable buffers so per-tick assignments allocate nothing.
#[derive(Default)]
pub struct AssignScratch {
    world: Vec<V2>,
    taken: Vec<bool>,
    /// Free slots per cell of `index`, so exhausted cells are skipped.
    free_in_cell: Vec<u16>,
    index: Option<SpatialGrid<u16>>,
    by_rank: Vec<Vec<(usize, u16)>>,
    /// Per soldier: distance to the assigned slot (swap pruning).
    dist: Vec<S>,
}

fn accepts(slot: &Slot, category: UnitCategory) -> bool {
    slot.category.is_none_or(|c| c == category)
}

/// The nearest free slot accepting `category`, ties by lower index: cells
/// of the slot grid are visited ring by ring around the soldier's cell and
/// the search stops once the best candidate is closer than any unvisited
/// cell can be (SIM-FORM-022 step 2, exact and O(cells visited)).
#[allow(clippy::too_many_arguments)]
fn nearest_free(
    index: &SpatialGrid<u16>,
    free_in_cell: &[u16],
    world: &[V2],
    taken: &[bool],
    slots: &[Slot],
    local: V2,
    pos: V2,
    category: UnitCategory,
) -> Option<usize> {
    let (cx, cy) = index.cell_of(local);
    let (cols, rows) = (index.cols() as i64, index.rows() as i64);
    let (cx, cy) = (i64::from(cx), i64::from(cy));
    let cell = index.cell();
    let mut best: Option<(S, usize)> = None;
    let consider = |x: i64, y: i64, best: &mut Option<(S, usize)>| {
        if free_in_cell[(y * cols + x) as usize] == 0 {
            return;
        }
        for i in index.cell_entries(x as u32, y as u32) {
            let slot = usize::from(index.entries()[i].id);
            if taken[slot] || !accepts(&slots[slot], category) {
                continue;
            }
            let d = world[slot].distance_sq(pos);
            if best.is_none_or(|(bd, bs)| d < bd || (d == bd && slot < bs)) {
                *best = Some((d, slot));
            }
        }
    };
    let max_ring = cols.max(rows);
    for k in 0..=max_ring {
        if k == 0 {
            consider(cx, cy, &mut best);
        } else {
            // The four edges of the ring, clamped to the grid.
            let (top, bottom, left, right) = (cy - k, cy + k, cx - k, cx + k);
            if top < 0 && bottom >= rows && left < 0 && right >= cols {
                break;
            }
            let x0 = left.max(0);
            let x1 = right.min(cols - 1);
            if top >= 0 {
                for x in x0..=x1 {
                    consider(x, top, &mut best);
                }
            }
            if bottom < rows {
                for x in x0..=x1 {
                    consider(x, bottom, &mut best);
                }
            }
            let y0 = (top + 1).max(0);
            let y1 = (bottom - 1).min(rows - 1);
            if left >= 0 {
                for y in y0..=y1 {
                    consider(left, y, &mut best);
                }
            }
            if right < cols {
                for y in y0..=y1 {
                    consider(right, y, &mut best);
                }
            }
        }
        // Every unvisited cell is at least `k × cell` from the soldier.
        if let Some((bd, _)) = best {
            let reach = S::from_i32(k as i32) * cell;
            if bd <= reach * reach {
                break;
            }
        }
    }
    best.map(|(_, slot)| slot)
}

/// Fills `out[k]` with the slot of `soldiers[k]` (`None` only when there
/// are more soldiers than slots). `prev[k]` is the soldier's current slot.
pub fn assign_slots(
    soldiers: &[AssignSoldier],
    slots: &[Slot],
    anchor: &Anchor,
    rules: &FormationRules,
    prev: &[Option<u16>],
    out: &mut Vec<Option<u16>>,
    scratch: &mut AssignScratch,
) {
    debug_assert_eq!(prev.len(), soldiers.len());
    out.clear();
    out.resize(soldiers.len(), None);
    scratch.world.clear();
    scratch
        .world
        .extend(slots.iter().map(|s| slot_world(anchor, s)));
    scratch.taken.clear();
    scratch.taken.resize(slots.len(), false);
    if slots.is_empty() {
        return;
    }

    // Pass 1: keep.
    let keep_sq = rules.keep_slot_radius * rules.keep_slot_radius;
    for (k, soldier) in soldiers.iter().enumerate() {
        if let Some(slot) = prev.get(k).copied().flatten()
            && let Some(world) = scratch.world.get(usize::from(slot))
            && !scratch.taken[usize::from(slot)]
            && accepts(&slots[usize::from(slot)], soldier.category)
            && world.distance_sq(soldier.pos) <= keep_sq
        {
            out[k] = Some(slot);
            scratch.taken[usize::from(slot)] = true;
        }
    }

    // Pass 2: greedy nearest free slot through a grid over the slots.
    let (mut min, mut max) = (scratch.world[0], scratch.world[0]);
    for p in &scratch.world {
        min = V2::new(min.x.min(p.x), min.y.min(p.y));
        max = V2::new(max.x.max(p.x), max.y.max(p.y));
    }
    let extent = max - min;
    let ring = rules.keep_slot_radius.max(S::ONE);
    let mut index = scratch
        .index
        .take()
        .unwrap_or_else(|| SpatialGrid::new(extent.x, extent.y, ring));
    index.ensure(extent.x.max(S::ONE), extent.y.max(S::ONE), ring);
    index.rebuild(scratch.world.iter().enumerate().map(|(i, p)| Entry {
        id: i as u16,
        entity: Entity::PLACEHOLDER,
        pos: *p - min,
    }));
    let cols = index.cols() as usize;
    scratch.free_in_cell.clear();
    scratch.free_in_cell.resize(cols * index.rows() as usize, 0);
    for (slot, p) in scratch.world.iter().enumerate() {
        if !scratch.taken[slot] {
            let (x, y) = index.cell_of(*p - min);
            scratch.free_in_cell[y as usize * cols + x as usize] += 1;
        }
    }
    for (k, soldier) in soldiers.iter().enumerate() {
        if out[k].is_some() {
            continue;
        }
        let best = nearest_free(
            &index,
            &scratch.free_in_cell,
            &scratch.world,
            &scratch.taken,
            slots,
            soldier.pos - min,
            soldier.pos,
            soldier.category,
        );
        if let Some(slot) = best {
            out[k] = Some(slot as u16);
            scratch.taken[slot] = true;
            let (x, y) = index.cell_of(scratch.world[slot] - min);
            scratch.free_in_cell[y as usize * cols + x as usize] -= 1;
        }
    }
    scratch.index = Some(index);

    // Pass 3: in-rank swaps. Members of a rank are visited in slot-x order;
    // two slots `L` apart cannot profit from a swap when `L` exceeds the
    // two soldiers' distances to their own slots, and `L >= |Δx|`, so the
    // inner loop stops once `Δx` passes `dist[a] + max dist`.
    let ranks = slots
        .iter()
        .map(|s| s.rank)
        .max()
        .map_or(0, |r| usize::from(r) + 1);
    scratch.by_rank.iter_mut().for_each(Vec::clear);
    scratch.by_rank.resize_with(ranks, Vec::new);
    scratch.dist.clear();
    scratch.dist.resize(soldiers.len(), S::ZERO);
    for (k, slot) in out.iter().enumerate() {
        if let Some(slot) = slot {
            scratch.by_rank[usize::from(slots[usize::from(*slot)].rank)].push((k, *slot));
            scratch.dist[k] = soldiers[k].pos.distance(scratch.world[usize::from(*slot)]);
        }
    }
    for members in &mut scratch.by_rank {
        members.sort_by(|a, b| {
            slots[usize::from(a.1)]
                .offset
                .x
                .partial_cmp(&slots[usize::from(b.1)].offset.x)
                .expect("finite offsets")
                .then(a.1.cmp(&b.1))
        });
    }
    for _ in 0..rules.swap_passes {
        let mut swapped = false;
        for members in &mut scratch.by_rank {
            let mut max_dist = S::ZERO;
            for (k, _) in members.iter() {
                max_dist = max_dist.max(scratch.dist[*k]);
            }
            for a in 0..members.len() {
                let (ka, sa) = members[a];
                let reach = scratch.dist[ka] + max_dist;
                for b in a + 1..members.len() {
                    let (kb, sb) = members[b];
                    let dx = slots[usize::from(sb)].offset.x - slots[usize::from(sa)].offset.x;
                    if dx > reach {
                        break;
                    }
                    if !accepts(&slots[usize::from(sb)], soldiers[ka].category)
                        || !accepts(&slots[usize::from(sa)], soldiers[kb].category)
                    {
                        continue;
                    }
                    let pa = soldiers[ka].pos;
                    let pb = soldiers[kb].pos;
                    let wa = scratch.world[usize::from(sa)];
                    let wb = scratch.world[usize::from(sb)];
                    let now = pa.distance_sq(wa) + pb.distance_sq(wb);
                    let then = pa.distance_sq(wb) + pb.distance_sq(wa);
                    if then < now {
                        // The slots stay in place (sorted order holds); the
                        // soldiers trade them.
                        members[a] = (kb, sa);
                        members[b] = (ka, sb);
                        out[ka] = Some(sb);
                        out[kb] = Some(sa);
                        scratch.dist[ka] = pa.distance(wb);
                        scratch.dist[kb] = pb.distance(wa);
                        swapped = true;
                        break;
                    }
                }
            }
        }
        if !swapped {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_core::Angle;
    use il_data::FormationTemplate;

    fn rules() -> FormationRules {
        let mut r = il_data::Rules::zeroed().formation;
        r.keep_slot_radius = S::from_f32_data(1.5);
        r.assign_search_radius = S::from_i32(30);
        r.swap_passes = 2;
        r
    }

    fn template() -> FormationTemplate {
        serde_json::from_str(
            r#"{ "id": "t:line", "name_key": "t.line.name", "layout": "line", "default_ranks": 4 }"#,
        )
        .unwrap()
    }

    fn line(n: u16, ranks: u8) -> Vec<Slot> {
        let mut out = Vec::new();
        crate::formation::layout::layout_slots(
            &template(),
            n,
            ranks,
            S::from_f32_data(0.5),
            &mut out,
        );
        out
    }

    fn soldiers_at(points: &[(f32, f32)]) -> Vec<AssignSoldier> {
        points
            .iter()
            .enumerate()
            .map(|(i, (x, y))| AssignSoldier {
                id: SoldierId(i as u32),
                pos: V2::from_f32_data(*x, *y),
                category: UnitCategory::Infantry,
            })
            .collect()
    }

    #[test]
    fn the_formation_frame_puts_forward_along_the_facing() {
        // Facing +x: right is -y, forward is +x, so a line spans y.
        let east = Anchor {
            pos: V2::ZERO,
            facing: Angle::ZERO,
        };
        let (right, forward) = frame(&east);
        assert_eq!(forward, V2::new(S::ONE, S::ZERO));
        assert_eq!(right, V2::new(S::ZERO, -S::ONE));
        let w = local_to_world(&east, V2::new(S::from_i32(2), -S::ONE));
        assert_eq!(w, V2::new(-S::ONE, -S::from_i32(2)));
        // Facing +y (north): right is +x, forward is +y.
        let north = Anchor {
            pos: V2::new(S::from_i32(10), S::ZERO),
            facing: Angle::from_degrees_data(90.0),
        };
        let w = local_to_world(&north, V2::new(S::from_i32(2), -S::ONE));
        assert!(
            (w.x - S::from_i32(12)).abs() < S::from_f32_data(1e-5),
            "{w:?}"
        );
        assert!((w.y + S::ONE).abs() < S::from_f32_data(1e-5), "{w:?}");
    }

    #[test]
    fn everyone_gets_a_distinct_slot_and_nearby_soldiers_keep_theirs() {
        let slots = line(12, 3);
        let anchor = Anchor {
            pos: V2::from_f32_data(100.0, 50.0),
            facing: Angle::ZERO,
        };
        // Soldiers already on their slots, jittered by 0.3 m.
        let pts: Vec<(f32, f32)> = slots
            .iter()
            .map(|s| {
                let w = slot_world(&anchor, s);
                (w.x.to_f32_render() + 0.3, w.y.to_f32_render() - 0.2)
            })
            .collect();
        let soldiers = soldiers_at(&pts);
        let prev: Vec<Option<u16>> = (0..12).map(|i| Some(i as u16)).collect();
        let mut out = Vec::new();
        let mut scratch = AssignScratch::default();
        assign_slots(
            &soldiers,
            &slots,
            &anchor,
            &rules(),
            &prev,
            &mut out,
            &mut scratch,
        );
        assert_eq!(out, prev, "kept");
        // No previous slots: greedy still finds the same one-to-one mapping.
        let none = vec![None; 12];
        assign_slots(
            &soldiers,
            &slots,
            &anchor,
            &rules(),
            &none,
            &mut out,
            &mut scratch,
        );
        assert_eq!(out, prev, "greedy");
    }

    #[test]
    fn assignment_covers_every_slot_once_and_swaps_reduce_crossing() {
        let slots = line(6, 1);
        let anchor = Anchor {
            pos: V2::ZERO,
            facing: Angle::ZERO,
        };
        // Soldiers in reverse order along the rank, five metres ahead of
        // it, far enough that keep does not apply.
        let pts: Vec<(f32, f32)> = (0..6)
            .map(|i| {
                let w = slot_world(&anchor, &slots[5 - i]) + V2::new(S::from_i32(5), S::ZERO);
                (w.x.to_f32_render(), w.y.to_f32_render())
            })
            .collect();
        let soldiers = soldiers_at(&pts);
        let mut out = Vec::new();
        let mut scratch = AssignScratch::default();
        assign_slots(
            &soldiers,
            &slots,
            &anchor,
            &rules(),
            &[None; 6],
            &mut out,
            &mut scratch,
        );
        let mut used: Vec<u16> = out.iter().map(|s| s.unwrap()).collect();
        used.sort_unstable();
        assert_eq!(used, (0..6).collect::<Vec<u16>>());
        // Total squared distance is minimal: soldier i stands ahead of slot
        // 5 - i and gets it.
        for (k, slot) in out.iter().enumerate() {
            assert_eq!(*slot, Some(5 - k as u16), "soldier {k}");
        }
    }

    #[test]
    fn fewer_slots_than_soldiers_leaves_the_rest_unassigned_and_resize_closes_from_the_rear() {
        let anchor = Anchor {
            pos: V2::ZERO,
            facing: Angle::ZERO,
        };
        let big = line(8, 2);
        let pts: Vec<(f32, f32)> = big
            .iter()
            .map(|s| {
                let w = slot_world(&anchor, s);
                (w.x.to_f32_render(), w.y.to_f32_render())
            })
            .collect();
        let soldiers = soldiers_at(&pts);
        let prev: Vec<Option<u16>> = (0..8).map(|i| Some(i as u16)).collect();
        // Front-rank soldier 1 died: seven soldiers, seven slots.
        let mut alive = soldiers.clone();
        alive.remove(1);
        let mut prev7 = prev.clone();
        prev7.remove(1);
        let small = line(7, 2);
        let mut out = Vec::new();
        let mut scratch = AssignScratch::default();
        assign_slots(
            &alive,
            &small,
            &anchor,
            &rules(),
            &prev7,
            &mut out,
            &mut scratch,
        );
        assert!(out.iter().all(Option::is_some));
        let mut used: Vec<u16> = out.iter().map(|s| s.unwrap()).collect();
        used.sort_unstable();
        assert_eq!(used, (0..7).collect::<Vec<u16>>());
        // The rearmost soldier (id 7, whose slot vanished) ends in the front
        // rank: the gap closes from the rear (a swap may shuffle it along
        // the rank when that shortens the total walk).
        assert_eq!(small[usize::from(out[6].unwrap())].rank, 0);
        assert_eq!(
            out[6],
            Some(2),
            "swapped with soldier 2, which walks half a metre"
        );
        assert_eq!(out[1], Some(1));
        assert_eq!(&out[2..6], &[Some(3), Some(4), Some(5), Some(6)]);
        // More soldiers than slots: the extra one stays unassigned.
        let three = line(3, 1);
        assign_slots(
            &soldiers[..4],
            &three,
            &anchor,
            &rules(),
            &[None; 4],
            &mut out,
            &mut scratch,
        );
        assert_eq!(out.iter().filter(|s| s.is_none()).count(), 1);
    }
}
