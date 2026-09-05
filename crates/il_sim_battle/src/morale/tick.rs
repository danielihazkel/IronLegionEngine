//! Stage 14 `morale_tick` (T2-041; SIM-MOR-002..027, TDD §8.3).
//!
//! Exclusive, ascending regiment id, in two phases: gather every
//! regiment's `MoraleInputs` from the state as it stood at the start of
//! the stage (anchor grid for allies and the nearest enemy, soldier grid
//! for `outnumbered`; plan G25), then apply the queued shocks, the factor
//! delta and the hysteresis transition per regiment. Entering Routing is
//! handed to `rout::enter_routing` (T2-042).

use bevy_ecs::prelude::*;
use il_core::{RegimentId, S, Scalar, TICKS_PER_SECOND, Tick, V2};

use crate::components::{
    Anchor, Combat, FormationState, Morale, MoraleState, Regiment, RegimentFatigue, Soldier,
};
use crate::events::BattleEvent;
use crate::morale::factors::{
    MoraleInputs, morale_delta, morale_factors, morale_state, shock_amount,
};
use crate::morale::rout;
use crate::movement::regiment::tick_dt;
use crate::resources::{
    AnchorGridRes, Clock, Events, Ids, MapRes, MoraleShocks, Regs, SpatialGridRes,
};

/// One regiment as the gather phase read it.
struct Row {
    id: RegimentId,
    entity: Entity,
    side: u8,
    anchor: V2,
    m: S,
    state: MoraleState,
    count: u16,
    initial: u16,
    deaths_5s: u32,
    integrity: S,
    fatigue_mean: S,
    engaged: bool,
    engaged_since: Tick,
    arc_hit: [Tick; 3],
}

fn rows(world: &World) -> Vec<Row> {
    let ids = world.resource::<Ids>();
    let mut out = Vec::with_capacity(ids.regiment_entities.len());
    for (id, entity) in &ids.regiment_entities {
        let (Some(r), Some(a), Some(m), Some(f), Some(c), Some(rf)) = (
            world.get::<Regiment>(*entity),
            world.get::<Anchor>(*entity),
            world.get::<Morale>(*entity),
            world.get::<FormationState>(*entity),
            world.get::<Combat>(*entity),
            world.get::<RegimentFatigue>(*entity),
        ) else {
            continue;
        };
        out.push(Row {
            id: *id,
            entity: *entity,
            side: r.side,
            anchor: a.pos,
            m: m.m,
            state: m.state,
            count: r.soldiers.len() as u16,
            initial: m.initial,
            deaths_5s: m.deaths_5s.iter().map(|d| u32::from(*d)).sum(),
            integrity: f.integrity,
            fatigue_mean: rf.mean,
            engaged: c.engaged,
            engaged_since: m.engaged_since,
            arc_hit: m.arc_hit,
        });
    }
    out
}

fn routing(state: MoraleState) -> bool {
    matches!(state, MoraleState::Routing | MoraleState::Shattered)
}

/// SIM-MOR-019: an attack through the arc within the last second.
fn hit_recently(at: Tick, tick: Tick) -> bool {
    at.0 > 0 && tick.0 - at.0 < TICKS_PER_SECOND
}

/// The nearest enemy regiment with soldiers (index into `rows`), ties to
/// the lower id (ascending scan, strict `<`; plan G24).
fn nearest_enemy(rows: &[Row], i: usize) -> Option<(usize, S)> {
    let mut best: Option<(usize, S)> = None;
    for (j, r) in rows.iter().enumerate() {
        if j == i || r.side == rows[i].side || r.count == 0 {
            continue;
        }
        let d = r.anchor.distance(rows[i].anchor);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((j, d));
        }
    }
    best
}

/// Gathers `MoraleInputs` for `rows[i]` (a regiment with soldiers).
fn inputs(
    world: &World,
    rows: &[Row],
    i: usize,
    tick: Tick,
    scratch: &mut Vec<usize>,
) -> MoraleInputs {
    let regs = &world.resource::<Regs>().0;
    let rules = &regs.rules.morale;
    let ids = world.resource::<Ids>();
    let map = &world.resource::<MapRes>().0;
    let row = &rows[i];

    // SIM-MOR-015/016: allies within `ally_radius` by state.
    let anchors = &world.resource::<AnchorGridRes>().0;
    anchors.query_circle_indices(row.anchor, rules.ally_radius, scratch);
    let (mut allies_steady, mut allies_routing) = (0u32, 0u32);
    for &k in scratch.iter() {
        let e = anchors.entries()[k];
        let Some(j) = ids.regiment_index(e.id) else {
            continue;
        };
        let other = &rows[j];
        if j == i || other.side != row.side || other.count == 0 {
            continue;
        }
        if other.state == MoraleState::Steady {
            allies_steady += 1;
        } else if routing(other.state) {
            allies_routing += 1;
        }
    }

    // SIM-MOR-017/023/024: the nearest enemy anchor and the enemies within
    // `safe_radius` (plan G23/G24).
    let nearest = nearest_enemy(rows, i);
    let nearest_enemy = nearest.map(|(j, d)| (rows[j].anchor, d));
    let height_delta =
        nearest.map(|(j, _)| map.height_at(row.anchor) - map.height_at(rows[j].anchor));
    let enemy_within_safe = nearest.is_some_and(|(_, d)| d <= rules.safe_radius);
    let enemy_deaths_5s: u32 = rows
        .iter()
        .filter(|r| {
            r.side != row.side && r.count > 0 && r.anchor.distance(row.anchor) <= rules.safe_radius
        })
        .map(|r| r.deaths_5s)
        .sum();

    // SIM-MOR-020: soldiers of each side within `outnumber_radius`.
    let grid = &world.resource::<SpatialGridRes>().0;
    grid.query_circle_indices(row.anchor, rules.outnumber_radius, scratch);
    let (mut own_near, mut enemies_near) = (0u32, 0u32);
    for &k in scratch.iter() {
        let e = grid.entries()[k];
        let Some(side) = world
            .get::<Soldier>(e.entity)
            .and_then(|s| ids.regiment_index(s.regiment))
            .map(|j| rows[j].side)
        else {
            continue;
        };
        if side == row.side {
            own_near += 1;
        } else {
            enemies_near += 1;
        }
    }

    let engaged_ticks = row.engaged.then(|| {
        if row.engaged_since.0 == 0 {
            0
        } else {
            tick.0 - row.engaged_since.0
        }
    });
    let hit = |k: usize| hit_recently(row.arc_hit[k], tick);
    MoraleInputs {
        count: row.count,
        initial: row.initial,
        own_deaths_5s: row.deaths_5s,
        enemy_deaths_5s,
        fatigue_mean: row.fatigue_mean,
        // SIM-MOR-013: no general exists until T2-043.
        in_aura: false,
        allies_steady,
        allies_routing,
        height_delta,
        // SIM-MOR-018: no status effects until T2-050.
        fear: false,
        hit_flank: hit(1),
        hit_rear: hit(2),
        surrounded: hit(0) && hit(1) && hit(2),
        enemies_near,
        own_near,
        integrity: row.integrity,
        engaged_ticks,
        enemy_within_safe,
        nearest_enemy,
        state: row.state,
    }
}

/// Stage 14 `morale_tick`.
pub fn morale_tick(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let regs = world.resource::<Regs>().0.clone();
    let rules = &regs.rules;
    let rows = rows(world);
    let shocks = core::mem::take(&mut world.resource_mut::<MoraleShocks>().0);
    let dt = tick_dt();
    let hundred = S::from_i32(100);

    // Gather, then apply: every input reads the pre-stage state.
    let mut scratch = Vec::new();
    let gathered: Vec<Option<MoraleInputs>> = (0..rows.len())
        .map(|i| (rows[i].count > 0).then(|| inputs(world, &rows, i, tick, &mut scratch)))
        .collect();

    for (row, inp) in rows.iter().zip(gathered) {
        let Some(inp) = inp else {
            continue;
        };
        let mut m = row.m;
        // Shocks first, in queue order (SIM-MOR-014/025/026/033).
        for s in shocks.iter().filter(|s| s.regiment == row.id) {
            m = m - shock_amount(s.kind, row.state, &rules.morale);
        }
        let x = morale_factors(&inp, &rules.morale, &rules.combat, &rules.formation);
        m = (m + morale_delta(&x, &rules.morale.w, dt)).clamp(S::ZERO, hundred);
        let mut next = morale_state(m, row.state, &rules.morale);
        {
            let mut morale = world.get_mut::<Morale>(row.entity).expect("gathered");
            morale.m = m;
            // SIM-MOR-022: the engagement clock.
            morale.engaged_since = if row.engaged {
                if row.engaged_since.0 == 0 {
                    tick
                } else {
                    row.engaged_since
                }
            } else {
                Tick::ZERO
            };
        }
        // SIM-MOR-030..032 (T2-042): the rout, or the rally out of it.
        if next == MoraleState::Routing && row.state != MoraleState::Routing {
            next = rout::enter_routing(world, row.entity, tick);
        } else if row.state == MoraleState::Routing
            && rout::try_rally(world, row.entity, tick, m, inp.nearest_enemy)
        {
            next = MoraleState::Shaken;
        } else {
            world.get_mut::<Morale>(row.entity).expect("gathered").state = next;
        }
        if next != row.state {
            world.resource_mut::<Events>().0.push(
                tick,
                BattleEvent::MoraleChanged {
                    regiment: row.id,
                    from: row.state,
                    to: next,
                },
            );
        }
    }
    rout::follow_centroid(world);
}
