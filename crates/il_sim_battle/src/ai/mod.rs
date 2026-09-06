//! Battle AI inside the sim (SAD §6.2 Stage 1, TDD §8.5, SIM-AI-001..003,
//! SIM-CMD-005, Networking Spec §2.7; T2-080).
//!
//! Sides owned by [`PlayerId::ENGINE_AI`] are driven by `ai_decide`, an
//! exclusive Stage 1 system: every tick it reads the world (state and
//! derived data that `rebuild_derived` recreates, never the per-tick
//! gates), lets the army AI (T2-082) and the due regiments (T2-081) decide,
//! and queues their commands for `tick + 1` in [`AiState::outbox`]. `step`
//! moves the outbox into the inbox before Stage 0, so the commands pass
//! through the same validation as a player's. The outbox and the army
//! plans cross the tick boundary and are therefore hashed and snapshotted
//! (plan decision 23).

pub mod deploy;
pub mod inputs;
pub mod regiment;

use bevy_ecs::prelude::*;
use il_core::hash::{Hashable, StateHasher};
use il_core::{
    Angle, PlayerId, RegimentId, RngStream, S, StreamId, Tick, V2, impl_hashable_fieldless_enum,
};
use il_data::{AiProfile, Handle};
use serde::{Deserialize, Serialize};

use crate::command::{Command, CommandKind};
use crate::components::MoraleState;
use crate::resources::{BattlePhase, Clock, Phase, Regs, Rng, SetupRes, Sides};

/// SIM-AI-010: the army's posture.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Stance {
    Attack = 0,
    Defend = 1,
    #[default]
    Hold = 2,
    Retreat = 3,
}
impl_hashable_fieldless_enum!(Stance);

/// A regiment's part in the plan (plan I13).
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Role {
    /// Holds a slot on the main line.
    Line { slot: V2 },
    /// Ranged or skirmisher: ahead of the line, then behind (SIM-AI-011).
    Skirmish { slot: V2 },
    /// Held behind the line until committed (SIM-AI-014).
    Reserve { slot: V2 },
    /// Cavalry heading for the enemy flank; charges `charge` when set.
    Flank {
        slot: V2,
        charge: Option<RegimentId>,
    },
    /// Cavalry behind a line end, ready to counter-charge (SIM-AI-012).
    Counter { slot: V2 },
    /// Cavalry screening a retreat (SIM-AI-013).
    Screen { slot: V2 },
    /// A committed reserve chasing `target`.
    Committed { target: RegimentId },
    /// The general's bodyguard (SIM-AI-022).
    Bodyguard,
}

impl Role {
    fn discriminant(&self) -> u8 {
        match self {
            Role::Line { .. } => 0,
            Role::Skirmish { .. } => 1,
            Role::Reserve { .. } => 2,
            Role::Flank { .. } => 3,
            Role::Counter { .. } => 4,
            Role::Screen { .. } => 5,
            Role::Committed { .. } => 6,
            Role::Bodyguard => 7,
        }
    }

    /// The slot the role holds, if it is a place.
    pub fn slot(&self) -> Option<V2> {
        match self {
            Role::Line { slot }
            | Role::Skirmish { slot }
            | Role::Reserve { slot }
            | Role::Flank { slot, .. }
            | Role::Counter { slot }
            | Role::Screen { slot } => Some(*slot),
            Role::Committed { .. } | Role::Bodyguard => None,
        }
    }
}

impl Hashable for Role {
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u8(self.discriminant());
        match self {
            Role::Line { slot }
            | Role::Skirmish { slot }
            | Role::Reserve { slot }
            | Role::Counter { slot }
            | Role::Screen { slot } => slot.hash_state(h),
            Role::Flank { slot, charge } => {
                slot.hash_state(h);
                charge.hash_state(h);
            }
            Role::Committed { target } => target.hash_state(h),
            Role::Bodyguard => {}
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub regiment: RegimentId,
    pub role: Role,
}
il_core::impl_hashable_struct!(Assignment { regiment, role });

/// SIM-AI-010: one side's plan, rebuilt every army period (T2-082) and
/// read by its regiments in between. Hashed and snapshotted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArmyPlan {
    pub stance: Stance,
    /// The winning stance's score, for the hysteresis margin.
    pub stance_score: S,
    pub decided_at: Tick,
    /// The enemy centroid aimed at (or the enemy zones', plan decision 9).
    pub target: V2,
    pub line_anchor: V2,
    pub line_facing: Angle<S>,
    pub line_width: S,
    /// Mean line slot error under `line_tolerance` (plan decision 22).
    pub formed: bool,
    /// Ascending regiment id.
    pub assignments: Vec<Assignment>,
    /// The flank groups have been sent in (SIM-AI-011).
    pub charging: bool,
}

il_core::impl_hashable_struct!(ArmyPlan {
    stance,
    stance_score,
    decided_at,
    target,
    line_anchor,
    line_facing,
    line_width,
    formed,
    assignments,
    charging
});

impl ArmyPlan {
    pub fn role_of(&self, regiment: RegimentId) -> Option<Role> {
        self.assignments
            .binary_search_by_key(&regiment, |a| a.regiment)
            .ok()
            .map(|i| self.assignments[i].role)
    }
}

/// The AI's state across ticks (plan decision 23): the commands queued for
/// the next tick and one plan per side (`Some` once the side has been
/// decided for as the engine's). Hashed after the visibility masks,
/// snapshotted verbatim.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct AiState {
    pub outbox: Vec<Command>,
    pub plans: Vec<Option<ArmyPlan>>,
}

impl AiState {
    pub fn hash_state(&self, h: &mut StateHasher) {
        h.write(&self.outbox);
        h.write_u32(self.plans.len() as u32);
        for p in &self.plans {
            match p {
                None => h.write_u8(0),
                Some(p) => {
                    h.write_u8(1);
                    p.hash_state(h);
                }
            }
        }
    }

    pub fn plan(&self, side: u8) -> Option<&ArmyPlan> {
        self.plans.get(usize::from(side)).and_then(|p| p.as_ref())
    }
}

/// Whether Stage 1 runs (plan decision 15): a replay that feeds the logged
/// AI commands switches it off. Not state: never hashed or snapshotted.
#[derive(Resource, Clone, Copy, Debug)]
pub struct AiEnabled(pub bool);

impl Default for AiEnabled {
    fn default() -> Self {
        Self(true)
    }
}

/// The profile a side decides with: the setup's override, else its
/// faction's (plan decision 12). `None` for a world without content.
pub fn side_profile(world: &World, side: u8) -> Option<Handle<AiProfile>> {
    let regs = &world.resource::<Regs>().0;
    if let Some(setup) = world.resource::<SetupRes>().0.as_ref()
        && let Some(s) = setup.sides.get(usize::from(side))
        && let Some(id) = &s.ai_profile
    {
        return regs.ai_profiles.lookup(id);
    }
    let faction = &world.resource::<Sides>().0.get(usize::from(side))?.faction;
    regs.factions
        .lookup(faction)
        .and_then(|h| regs.factions.get(h).ai_profile_handle)
}

/// Commands one AI side decided on this tick, in emission order.
#[derive(Default)]
pub struct Decisions(pub Vec<CommandKind>);

impl Decisions {
    pub fn push(&mut self, kind: CommandKind) {
        self.0.push(kind);
    }
}

/// Stage 1 `ai_decide` (SIM-AI-002, SIM-CMD-005): for every side the engine
/// owns, in ascending side order, the Deployment placement (T2-081), the
/// army decision when due (T2-082) and the due regiments (T2-081); every
/// decision becomes a `Command` for the next tick under `PlayerId(255)`,
/// `seq` counting from 0 in emission order (plan I9).
pub fn ai_decide(world: &mut World) {
    if !world.resource::<AiEnabled>().0 {
        return;
    }
    let tick = world.resource::<Clock>().tick;
    let phase = world.resource::<Phase>().0;
    let n_sides = world.resource::<Sides>().0.len();
    if world.resource::<AiState>().plans.len() < n_sides {
        world.resource_mut::<AiState>().plans.resize(n_sides, None);
    }
    let mut out: Vec<Command> = Vec::new();
    for side in 0..n_sides {
        let (owned, defeated, confirmed) = {
            let s = &world.resource::<Sides>().0[side];
            (
                s.player == PlayerId::ENGINE_AI,
                s.defeated,
                s.deployment_confirmed,
            )
        };
        if !owned || defeated {
            continue;
        }
        let Some(profile) = side_profile(world, side as u8) else {
            continue;
        };
        let mut decisions = Decisions::default();
        match phase {
            BattlePhase::Deployment => {
                if !confirmed {
                    deploy_side(world, side as u8, profile, &mut decisions);
                }
            }
            BattlePhase::Battle | BattlePhase::Pursuit => {
                decide_side(world, side as u8, profile, tick, &mut decisions);
            }
            BattlePhase::Ended => {}
        }
        for kind in decisions.0 {
            out.push(Command {
                tick: tick.next(),
                player: PlayerId::ENGINE_AI,
                seq: out.len() as u16,
                kind,
            });
        }
    }
    world.resource_mut::<AiState>().outbox = out;
}

/// SIM-AI-020 (T2-081): places the side and confirms.
fn deploy_side(world: &mut World, side: u8, profile: Handle<AiProfile>, out: &mut Decisions) {
    deploy::commands(world, side, profile, out);
}

/// SIM-AI-010..014 and SIM-AI-021 (T2-081/082): the army plan when due,
/// then the due regiments, each drawing its noise from the regiment stream
/// in decision order.
fn decide_side(
    world: &mut World,
    side: u8,
    profile: Handle<AiProfile>,
    tick: Tick,
    out: &mut Decisions,
) {
    let regs = world.resource::<Regs>().0.clone();
    let profile = regs.ai_profiles.get(profile);
    let Some(set) = profile.regiment_set.map(|h| regs.ai_action_sets.get(h)) else {
        return;
    };
    let snap = inputs::SideSnapshot::build(world, side);
    let plan = world.resource::<AiState>().plan(side).cloned();
    let mut rng: RngStream = world.resource::<Rng>().streams[StreamId::AiRegiment.index()].clone();
    for me in &snap.own {
        if matches!(
            me.morale_state,
            MoraleState::Routing | MoraleState::Shattered
        ) || !il_ai::due(tick, profile.regiment_period_ticks, me.id.0)
        {
            continue;
        }
        regiment::decide(&regs, &snap, me, profile, set, plan.as_ref(), &mut rng, out);
    }
    world.resource_mut::<Rng>().streams[StreamId::AiRegiment.index()] = rng;
}
