//! Stage 2 formation systems (T1-041; SIM-FORM-020..022, TDD §7).
//!
//! `formation_layout` runs once per regiment that needs a reform: it lays
//! the slots out again and computes a fresh assignment into the regiment's
//! own `FormationState` (parallel over regiments when a task pool exists,
//! serial otherwise; either way every write is to the regiment's own
//! component). `formation_apply` then writes `SlotRef` and `Rank` to the
//! soldiers in regiment id order, the single-threaded apply step of
//! SAD §8 rule 2.

use bevy_ecs::prelude::*;
use il_core::{Angle, S, Scalar, SoldierId, V2};
use il_data::FormationRules;

use crate::components::{Anchor, FormationState, Order, Pos, Rank, Regiment, SlotRef, Soldier};
use crate::formation::assign::{
    AssignScratch, AssignSoldier, assign_slots, local_to_world, slot_world,
};
use crate::formation::layout::{effective_ranks, files_used, layout_slots, spacing};
use crate::resources::{Clock, Ids, Regs};

/// SIM-FORM-030: the fraction of a regiment's soldiers within `radius` of
/// their slot (`1` for an empty regiment).
pub fn integrity(
    regiment: &Regiment,
    anchor: &Anchor,
    state: &FormationState,
    soldiers: &SoldierRead,
    ids: &Ids,
    radius: S,
) -> S {
    if regiment.soldiers.is_empty() {
        return S::ONE;
    }
    let r_sq = radius * radius;
    let mut inside = 0;
    for &sid in &regiment.soldiers {
        let Some(entity) = ids.soldier_entity(sid) else {
            continue;
        };
        let Ok((_, pos, slot)) = soldiers.get(entity) else {
            continue;
        };
        if let Some(slot) = slot.slot.and_then(|s| state.slots.get(usize::from(s)))
            && slot_world(anchor, slot).distance_sq(pos.p) <= r_sq
        {
            inside += 1;
        }
    }
    S::from_i32(inside) / S::from_i32(regiment.soldiers.len() as i32)
}

/// Stage 2, last: `formation_integrity` every `integrity_period_ticks`
/// (SIM-FORM-030), `integrity_radius` in file spacings.
pub fn formation_integrity(
    mut regiments: Query<(&Regiment, &Anchor, &mut FormationState)>,
    soldiers: SoldierRead,
    ids: Res<Ids>,
    regs: Res<Regs>,
    clock: Res<Clock>,
) {
    let period = u32::from(regs.0.rules.formation.integrity_period_ticks.max(1));
    if !clock.tick.0.is_multiple_of(period) {
        return;
    }
    let parallel = bevy_tasks::ComputeTaskPool::try_get().is_some_and(|p| p.thread_num() > 1);
    let soldiers = &soldiers;
    let ids = &ids;
    let regs = &regs;
    let run = |(r, anchor, mut state): (&Regiment, &Anchor, Mut<FormationState>)| {
        let radius = regs.0.units.get(r.unit).soldier_radius;
        let (sf, _) = spacing(regs.0.formations.get(state.template), radius);
        state.integrity = integrity(
            r,
            anchor,
            &state,
            soldiers,
            ids,
            regs.0.rules.formation.integrity_radius * sf,
        );
    };
    if parallel {
        regiments.par_iter_mut().for_each(run);
    } else {
        regiments.iter_mut().for_each(run);
    }
}

/// SIM-FORM-024 (Phase 1 plan S10): a facing order for a halted regiment.
/// Beyond `turn_in_place_angle` the regiment about-faces: the anchor moves
/// to the rear rank's centre (`a − R(θ_a)·(0, (ranks − 1)·sr)`), the facing
/// flips and a reform makes the rear rank the front. Otherwise the regiment
/// wheels: `order.facing` becomes the target that `regiment_follow_path`
/// turns toward at `wheel_rate`. Returns whether it about-faced.
pub fn set_facing(
    anchor: &mut Anchor,
    order: &mut Order,
    state: &mut FormationState,
    rules: &FormationRules,
    sr: S,
    facing: Angle<S>,
) -> bool {
    order.facing = Some(facing);
    let delta = anchor.facing.delta(facing).abs();
    let threshold = rules.turn_in_place_angle * S::PI / S::from_i32(180);
    if !order.kind.moves() && delta > threshold {
        let depth = S::from_i32(i32::from(state.ranks.max(1)) - 1) * sr;
        anchor.pos = local_to_world(anchor, V2::new(S::ZERO, -depth));
        anchor.facing = Angle::new(anchor.facing.radians() + S::PI);
        state.needs_reform = true;
        // Whatever remains of the turn is wheeled from the new facing.
        return true;
    }
    false
}

/// SIM-FORM-020: whether the regiment's formation must be laid out again.
fn wants_reform(
    state: &FormationState,
    regiment: &Regiment,
    anchor: &Anchor,
    reform_angle: S,
) -> bool {
    state.needs_reform
        || state.slots.len() != regiment.soldiers.len()
        || state.laid_out_facing.delta(anchor.facing).abs() > reform_angle
}

type SoldierRead<'w, 's> = Query<'w, 's, (&'static Soldier, &'static Pos, &'static SlotRef)>;

fn reform_one(
    regiment: &Regiment,
    anchor: &Anchor,
    state: &mut FormationState,
    soldiers: &SoldierRead,
    ids: &Ids,
    regs: &Regs,
    scratch: &mut AssignScratch,
) {
    let rules = &regs.0.rules.formation;
    let template = regs.0.formations.get(state.template);
    let radius = regs.0.units.get(regiment.unit).soldier_radius;
    let n = regiment.soldiers.len() as u16;
    state.ranks = effective_ranks(template, n, Some(state.ranks));
    layout_slots(template, n, state.ranks, radius, &mut state.slots);
    state.files = files_used(&state.slots);

    let mut members: Vec<AssignSoldier> = Vec::with_capacity(regiment.soldiers.len());
    let mut prev: Vec<Option<u16>> = Vec::with_capacity(regiment.soldiers.len());
    for &sid in &regiment.soldiers {
        let Some(entity) = ids.soldier_entity(sid) else {
            continue;
        };
        let Ok((soldier, pos, slot)) = soldiers.get(entity) else {
            continue;
        };
        members.push(AssignSoldier {
            id: soldier.id,
            pos: pos.p,
            category: soldier.category,
        });
        prev.push(slot.slot);
    }
    assign_slots(
        &members,
        &state.slots,
        anchor,
        rules,
        &prev,
        &mut state.assignment,
        scratch,
    );
    state.laid_out_facing = anchor.facing;
    state.needs_reform = false;
    state.dirty = true;
}

/// Stage 2, first: lay out and assign every regiment that needs it.
pub fn formation_layout(
    mut regiments: Query<(&Regiment, &Anchor, &mut FormationState)>,
    soldiers: SoldierRead,
    ids: Res<Ids>,
    regs: Res<Regs>,
) {
    let reform_angle = regs.0.rules.formation.reform_angle * S::PI / S::from_i32(180);
    let parallel = bevy_tasks::ComputeTaskPool::try_get().is_some_and(|p| p.thread_num() > 1);
    let soldiers = &soldiers;
    let ids = &ids;
    let regs = &regs;
    if parallel {
        regiments.par_iter_mut().for_each_init(
            AssignScratch::default,
            |scratch, (r, anchor, mut state)| {
                if wants_reform(&state, r, anchor, reform_angle) {
                    reform_one(r, anchor, &mut state, soldiers, ids, regs, scratch);
                }
            },
        );
    } else {
        let mut scratch = AssignScratch::default();
        for (r, anchor, mut state) in regiments.iter_mut() {
            if wants_reform(&state, r, anchor, reform_angle) {
                reform_one(r, anchor, &mut state, soldiers, ids, regs, &mut scratch);
            }
        }
    }
}

/// Stage 2, second: write the fresh assignments to the soldiers in
/// regiment id order.
pub fn formation_apply(world: &mut World) {
    let regiment_entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    let mut updates: Vec<(SoldierId, Option<u16>, Rank)> = Vec::new();
    for entity in regiment_entities {
        let Some(mut state) = world.get_mut::<FormationState>(entity) else {
            continue;
        };
        if !state.dirty {
            continue;
        }
        state.dirty = false;
        let state = world.get::<FormationState>(entity).expect("just read");
        let regiment = world.get::<Regiment>(entity).expect("regiment");
        updates.clear();
        for (k, &sid) in regiment.soldiers.iter().enumerate() {
            let slot = state.assignment.get(k).copied().flatten();
            let rank = slot.map_or(Rank::default(), |s| {
                let slot = &state.slots[usize::from(s)];
                Rank {
                    rank: slot.rank,
                    file: slot.file,
                }
            });
            updates.push((sid, slot, rank));
        }
        for (sid, slot, rank) in &updates {
            let Some(soldier) = world.resource::<Ids>().soldier_entity(*sid) else {
                continue;
            };
            if let Some(mut s) = world.get_mut::<SlotRef>(soldier) {
                s.slot = *slot;
            }
            if let Some(mut r) = world.get_mut::<Rank>(soldier) {
                *r = *rank;
            }
        }
    }
}

/// Rebuilds every regiment's slots from its template, count and ranks, and
/// every soldier's `Rank` from its `SlotRef` (restore; TDD §4.6).
pub fn rebuild_formation_derived(world: &mut World) {
    let regiment_entities: Vec<Entity> = world
        .resource::<Ids>()
        .regiment_entities
        .iter()
        .map(|(_, e)| *e)
        .collect();
    for entity in regiment_entities {
        let (template, unit, n, ranks) = {
            let regiment = world.get::<Regiment>(entity).expect("regiment");
            let state = world
                .get::<FormationState>(entity)
                .expect("formation state");
            (
                state.template,
                regiment.unit,
                regiment.soldiers.len() as u16,
                state.ranks,
            )
        };
        let (slots, files) = {
            let regs = &world.resource::<Regs>().0;
            let mut slots = Vec::new();
            layout_slots(
                regs.formations.get(template),
                n,
                ranks,
                regs.units.get(unit).soldier_radius,
                &mut slots,
            );
            let files = files_used(&slots);
            (slots, files)
        };
        let soldier_ids = world
            .get::<Regiment>(entity)
            .expect("regiment")
            .soldiers
            .clone();
        let mut assignment = Vec::with_capacity(soldier_ids.len());
        for sid in soldier_ids {
            let slot = world
                .resource::<Ids>()
                .soldier_entity(sid)
                .and_then(|e| world.get::<SlotRef>(e).map(|s| s.slot));
            let slot = slot.flatten().filter(|s| usize::from(*s) < slots.len());
            assignment.push(slot);
            if let Some(e) = world.resource::<Ids>().soldier_entity(sid)
                && let Some(mut rank) = world.get_mut::<Rank>(e)
            {
                *rank = slot.map_or(Rank::default(), |s| Rank {
                    rank: slots[usize::from(s)].rank,
                    file: slots[usize::from(s)].file,
                });
            }
        }
        let mut state = world
            .get_mut::<FormationState>(entity)
            .expect("formation state");
        state.slots = slots;
        state.files = files;
        state.assignment = assignment;
        state.dirty = false;
    }
}
