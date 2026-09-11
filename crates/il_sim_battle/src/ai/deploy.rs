//! AI deployment (SIM-AI-020; T2-081, plan decision 11): an unconfirmed
//! side owned by the engine places its regiments in a battle line at its
//! zone centre facing the enemy zones (ranged ahead, cavalry on the flanks,
//! the general's bodyguard behind the centre) with one `Deploy` each, then
//! sends `ConfirmDeployment`; the commands apply at the next tick's Stage 0
//! and the side is confirmed at that tick's Stage 16.

use bevy_ecs::prelude::*;
use il_core::{S, Scalar, V2};
use il_data::{AiProfile, ContentId, GroupFormationTemplate, GroupKind, Handle};

use crate::ai::Decisions;
use crate::command::CommandKind;
use crate::components::{Anchor, FormationState, Regiment};
use crate::flow_battle::{facing_toward, zone_centre};
use crate::formation::{RegimentInfo, arrange_group};
use crate::map::polygon_contains;
use crate::resources::{Ids, MapRes, Regs, Sides};

/// The battle line the AI deploys in: skirmishers ahead, cavalry on the
/// flanks (SIM-AI-020).
fn deployment_template(gap: S) -> GroupFormationTemplate {
    GroupFormationTemplate {
        id: ContentId::new("il:ai_deployment").expect("valid id"),
        name_key: String::new(),
        kind: GroupKind::BattleLine,
        gap,
        skirmishers_forward: true,
        cavalry_flanks: true,
        lines: 1,
        deprecated: None,
    }
}

/// Emits the side's `Deploy`s and its `ConfirmDeployment`.
pub fn commands(world: &World, side: u8, profile: Handle<AiProfile>, out: &mut Decisions) {
    let regs = world.resource::<Regs>().0.clone();
    let map = world.resource::<MapRes>().0.clone();
    let profile = regs.ai_profiles.get(profile);
    let sides = world.resource::<Sides>().0.clone();
    let Some(state) = sides.get(usize::from(side)) else {
        return;
    };
    let zone = state.deployment_zone;
    let Some(poly) = map.deployment_polygon(zone) else {
        out.push(CommandKind::ConfirmDeployment);
        return;
    };
    let centre = zone_centre(&map, zone);
    let others: Vec<V2> = sides
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != usize::from(side))
        .map(|(_, s)| zone_centre(&map, s.deployment_zone))
        .collect();
    let facing = facing_toward(&map, centre, &others);
    let forward = facing.direction();

    // The side's regiments with soldiers, ascending id.
    let mut infos: Vec<RegimentInfo> = Vec::new();
    let mut current: Vec<(RegimentInfo, V2)> = Vec::new();
    for (id, entity) in &world.resource::<Ids>().regiment_entities {
        let (Some(r), Some(a), Some(f)) = (
            world.get::<Regiment>(*entity),
            world.get::<Anchor>(*entity),
            world.get::<FormationState>(*entity),
        ) else {
            continue;
        };
        if r.side != side || r.soldiers.is_empty() {
            continue;
        }
        let comp = crate::composition::Composition::of(&regs, &r.units);
        let info = RegimentInfo {
            id: *id,
            pos: a.pos,
            category: comp.category,
            count: r.soldiers.len() as u16,
            template: f.template,
            radius: crate::composition::widest_radius(&regs, &r.units),
        };
        current.push((info, a.pos));
        if state.general_regiment != Some(*id) {
            infos.push(info);
        }
    }
    let template = deployment_template(regs.rules.formation.group_gap);
    let placements = arrange_group(
        &template,
        &infos,
        centre,
        facing,
        S::from_i32(100_000),
        &regs.rules.formation,
        &regs,
    );
    for (info, anchor) in &current {
        let wanted = if state.general_regiment == Some(info.id) {
            centre - forward * profile.reserve_offset
        } else {
            placements
                .iter()
                .find(|p| p.id == info.id)
                .map_or(*anchor, |p| p.anchor)
        };
        // Outside the polygon the regiment keeps its auto-placement, which
        // lies inside by construction (plan I14).
        let position = if polygon_contains(poly, wanted) {
            wanted
        } else {
            *anchor
        };
        out.push(CommandKind::Deploy {
            regiment: info.id,
            position,
            facing,
            template: None,
        });
    }
    out.push(CommandKind::ConfirmDeployment);
}
