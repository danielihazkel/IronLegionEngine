//! Integration (T1-043; TDD §6.2 `integrate`, SIM-MOVE-042).
//!
//! Stage 5, parallel over soldiers: `p += v × dt`, clamped to the map and
//! pushed out of impassable nav cells (Phase 1 plan S12: try the full move,
//! then x only, then y only, else stay).

use bevy_ecs::prelude::*;
use il_core::V2;

use crate::components::{Pos, Vel};
use crate::map::LoadedMap;
use crate::movement::regiment::tick_dt;
use crate::nav::NavGrid;
use crate::resources::{MapRes, NavGridRes};

/// The position a soldier at `p` moving by `delta` ends up in: the full
/// move if its cell is passable, else the x-only or y-only move, else `p`.
pub fn push_out(map: &LoadedMap, nav: &NavGrid, p: V2, delta: V2) -> V2 {
    let full = map.clamp(p + delta);
    if nav.is_passable_at(full) {
        return full;
    }
    let x_only = map.clamp(V2::new(p.x + delta.x, p.y));
    if nav.is_passable_at(x_only) {
        return x_only;
    }
    let y_only = map.clamp(V2::new(p.x, p.y + delta.y));
    if nav.is_passable_at(y_only) {
        return y_only;
    }
    p
}

/// Stage 5 `integrate`.
pub fn integrate(mut soldiers: Query<(&mut Pos, &Vel)>, map: Res<MapRes>, nav: Res<NavGridRes>) {
    let dt = tick_dt();
    let map = &map.0;
    let nav = &nav.0;
    let run = |(mut pos, vel): (Mut<Pos>, &Vel)| {
        if vel.v != V2::ZERO {
            pos.p = push_out(map, nav, pos.p, vel.v * dt);
        }
    };
    let parallel = bevy_tasks::ComputeTaskPool::try_get().is_some_and(|p| p.thread_num() > 1);
    if parallel {
        soldiers.par_iter_mut().for_each(run);
    } else {
        soldiers.iter_mut().for_each(run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_core::{S, Scalar};

    #[test]
    fn push_out_tries_full_then_axes_then_stays() {
        let map = LoadedMap::flat(S::from_i32(8), S::from_i32(8));
        // 4 x 4 cells of 2 m; the column x in [4, 6) is rock.
        let mut cost = vec![100u16; 16];
        for y in 0..4 {
            cost[y * 4 + 2] = 0;
        }
        let nav = NavGrid::from_costs(S::from_i32(2), 4, 4, cost);
        let p = V2::new(S::from_f32_data(3.0), S::from_f32_data(3.0));
        // Into the wall diagonally: y-only move survives.
        let d = V2::new(S::from_f32_data(1.5), S::from_f32_data(0.5));
        assert_eq!(
            push_out(&map, &nav, p, d),
            V2::new(p.x, S::from_f32_data(3.5))
        );
        // Straight into the wall: stay.
        let d = V2::new(S::from_f32_data(1.5), S::ZERO);
        assert_eq!(push_out(&map, &nav, p, d), p);
        // Free move.
        let d = V2::new(-S::ONE, S::ONE);
        assert_eq!(push_out(&map, &nav, p, d), p + d);
        // The map edge clamps.
        let d = V2::new(-S::from_i32(10), S::ZERO);
        assert_eq!(push_out(&map, &nav, p, d), V2::new(S::ZERO, p.y));
    }
}
