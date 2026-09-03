//! Commands: the sim's only input (TDD §4.2, SIM-CMD-001..005, ADR-005).
//!
//! Content references inside commands are `ContentId`s rather than handles
//! so a command stream is self-describing in replays and on the wire; Stage 0
//! resolves them against the registries (recorded in TDD §4.2 by T0-052).

use bevy_ecs::prelude::*;
use il_core::hash::{Hashable, StateHasher};
use il_core::{Angle, PlayerId, RegimentId, S, Tick, V2, impl_hashable_fieldless_enum};
use il_data::ContentId;
use serde::{Deserialize, Serialize};

use crate::components::{Order, OrderKind, Regiment};
use crate::events::BattleEvent;
use crate::resources::{Clock, CommandInbox, Events, Ids, Rejected, Sides};

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum SpeedMode {
    #[default]
    Walk = 0,
    Run = 1,
    March = 2,
}
impl_hashable_fieldless_enum!(SpeedMode);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum FireMode {
    FireAtWill,
    Hold,
    Target(RegimentId),
}

impl Hashable for FireMode {
    fn hash_state(&self, h: &mut StateHasher) {
        match self {
            FireMode::FireAtWill => h.write_u8(0),
            FireMode::Hold => h.write_u8(1),
            FireMode::Target(r) => {
                h.write_u8(2);
                r.hash_state(h);
            }
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AbilityTarget {
    SelfTarget,
    Point(V2),
    Regiment(RegimentId),
}

impl Hashable for AbilityTarget {
    fn hash_state(&self, h: &mut StateHasher) {
        match self {
            AbilityTarget::SelfTarget => h.write_u8(0),
            AbilityTarget::Point(p) => {
                h.write_u8(1);
                p.hash_state(h);
            }
            AbilityTarget::Regiment(r) => {
                h.write_u8(2);
                r.hash_state(h);
            }
        }
    }
}

/// Every battle command kind (SIM-CMD-002). All variants exist from Phase 0;
/// the ones without an implementation are rejected with
/// [`RejectReason::NotImplemented`] so nothing silently disappears.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CommandKind {
    Move {
        regiments: Vec<RegimentId>,
        target: V2,
        facing: Option<Angle<S>>,
        speed: SpeedMode,
    },
    AttackRegiment {
        regiments: Vec<RegimentId>,
        target: RegimentId,
    },
    AttackMove {
        regiments: Vec<RegimentId>,
        target: V2,
    },
    Halt {
        regiments: Vec<RegimentId>,
    },
    SetFormation {
        regiments: Vec<RegimentId>,
        template: ContentId,
        ranks: Option<u8>,
    },
    SetFacing {
        regiments: Vec<RegimentId>,
        facing: Angle<S>,
    },
    SetSpeedMode {
        regiments: Vec<RegimentId>,
        mode: SpeedMode,
    },
    GroupFormation {
        regiments: Vec<RegimentId>,
        template: ContentId,
        anchor: V2,
        facing: Angle<S>,
        width: S,
    },
    FireMode {
        regiments: Vec<RegimentId>,
        mode: FireMode,
    },
    UseAbility {
        regiment: RegimentId,
        ability: ContentId,
        target: AbilityTarget,
    },
    Withdraw {
        regiments: Vec<RegimentId>,
    },
    Deploy {
        regiment: RegimentId,
        position: V2,
        facing: Angle<S>,
        template: Option<ContentId>,
    },
    ConfirmDeployment,
    /// Recorded for replays and peers; a no-op inside the sim (SIM-DET-008).
    Pause,
    /// Recorded for replays and peers; a no-op inside the sim (SIM-DET-008).
    SetSpeed {
        mult_x100: u16,
    },
    Surrender,
    /// Hands every regiment of `from` to `to`; `to = 255` is the engine AI
    /// (SIM-CMD-002, Networking Spec §9).
    TransferControl {
        from: PlayerId,
        to: PlayerId,
    },
}

impl CommandKind {
    /// The regiments a command addresses, for ownership validation.
    pub fn regiments(&self) -> &[RegimentId] {
        match self {
            CommandKind::Move { regiments, .. }
            | CommandKind::AttackRegiment { regiments, .. }
            | CommandKind::AttackMove { regiments, .. }
            | CommandKind::Halt { regiments }
            | CommandKind::SetFormation { regiments, .. }
            | CommandKind::SetFacing { regiments, .. }
            | CommandKind::SetSpeedMode { regiments, .. }
            | CommandKind::GroupFormation { regiments, .. }
            | CommandKind::FireMode { regiments, .. }
            | CommandKind::Withdraw { regiments } => regiments,
            CommandKind::UseAbility { regiment, .. } | CommandKind::Deploy { regiment, .. } => {
                core::slice::from_ref(regiment)
            }
            CommandKind::ConfirmDeployment
            | CommandKind::Pause
            | CommandKind::SetSpeed { .. }
            | CommandKind::Surrender
            | CommandKind::TransferControl { .. } => &[],
        }
    }

    fn discriminant(&self) -> u8 {
        match self {
            CommandKind::Move { .. } => 0,
            CommandKind::AttackRegiment { .. } => 1,
            CommandKind::AttackMove { .. } => 2,
            CommandKind::Halt { .. } => 3,
            CommandKind::SetFormation { .. } => 4,
            CommandKind::SetFacing { .. } => 5,
            CommandKind::SetSpeedMode { .. } => 6,
            CommandKind::GroupFormation { .. } => 7,
            CommandKind::FireMode { .. } => 8,
            CommandKind::UseAbility { .. } => 9,
            CommandKind::Withdraw { .. } => 10,
            CommandKind::Deploy { .. } => 11,
            CommandKind::ConfirmDeployment => 12,
            CommandKind::Pause => 13,
            CommandKind::SetSpeed { .. } => 14,
            CommandKind::Surrender => 15,
            CommandKind::TransferControl { .. } => 16,
        }
    }
}

impl Hashable for CommandKind {
    fn hash_state(&self, h: &mut StateHasher) {
        h.write_u8(self.discriminant());
        match self {
            CommandKind::Move {
                regiments,
                target,
                facing,
                speed,
            } => {
                regiments.hash_state(h);
                target.hash_state(h);
                facing.hash_state(h);
                speed.hash_state(h);
            }
            CommandKind::AttackRegiment { regiments, target } => {
                regiments.hash_state(h);
                target.hash_state(h);
            }
            CommandKind::AttackMove { regiments, target } => {
                regiments.hash_state(h);
                target.hash_state(h);
            }
            CommandKind::Halt { regiments } | CommandKind::Withdraw { regiments } => {
                regiments.hash_state(h);
            }
            CommandKind::SetFormation {
                regiments,
                template,
                ranks,
            } => {
                regiments.hash_state(h);
                template.hash_state(h);
                ranks.hash_state(h);
            }
            CommandKind::SetFacing { regiments, facing } => {
                regiments.hash_state(h);
                facing.hash_state(h);
            }
            CommandKind::SetSpeedMode { regiments, mode } => {
                regiments.hash_state(h);
                mode.hash_state(h);
            }
            CommandKind::GroupFormation {
                regiments,
                template,
                anchor,
                facing,
                width,
            } => {
                regiments.hash_state(h);
                template.hash_state(h);
                anchor.hash_state(h);
                facing.hash_state(h);
                width.hash_state(h);
            }
            CommandKind::FireMode { regiments, mode } => {
                regiments.hash_state(h);
                mode.hash_state(h);
            }
            CommandKind::UseAbility {
                regiment,
                ability,
                target,
            } => {
                regiment.hash_state(h);
                ability.hash_state(h);
                target.hash_state(h);
            }
            CommandKind::Deploy {
                regiment,
                position,
                facing,
                template,
            } => {
                regiment.hash_state(h);
                position.hash_state(h);
                facing.hash_state(h);
                template.hash_state(h);
            }
            CommandKind::ConfirmDeployment | CommandKind::Pause | CommandKind::Surrender => {}
            CommandKind::SetSpeed { mult_x100 } => mult_x100.hash_state(h),
            CommandKind::TransferControl { from, to } => {
                from.hash_state(h);
                to.hash_state(h);
            }
        }
    }
}

/// `{ tick, player, seq, kind }` (SIM-CMD-001).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// The tick at which the command executes; must equal the tick being
    /// simulated or it is rejected as stale.
    pub tick: Tick,
    pub player: PlayerId,
    /// Per-player sequence number; commands of one player apply in `seq` order.
    pub seq: u16,
    pub kind: CommandKind,
}

impl Hashable for Command {
    fn hash_state(&self, h: &mut StateHasher) {
        self.tick.hash_state(h);
        self.player.hash_state(h);
        self.seq.hash_state(h);
        self.kind.hash_state(h);
    }
}

/// Why Stage 0 rejected a command; carried by `BattleEvent::CommandRejected`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    /// `command.tick` is not the tick being simulated (SIM-CMD-001).
    StaleTick { command_tick: Tick, current: Tick },
    /// The regiment does not exist.
    UnknownRegiment(RegimentId),
    /// The regiment belongs to another player (SIM-CMD-003).
    NotOwner(RegimentId),
    /// Routing or shattered regiments cannot be ordered (SIM-CMD-004).
    Routing(RegimentId),
    /// Not allowed in the current battle phase.
    WrongPhase,
    /// The variant has no implementation yet; never silently dropped.
    NotImplemented,
}

/// Stage 0 (TDD §4.5 `apply_commands`). Exclusive so ownership checks and
/// mutations happen in one defined order.
pub fn apply_commands(world: &mut World) {
    let current = world.resource::<Clock>().tick;
    let mut inbox = core::mem::take(&mut world.resource_mut::<CommandInbox>().0);
    // SIM-CMD-001: apply in (player, seq) order regardless of arrival order.
    inbox.sort_by_key(|c| (c.player, c.seq));

    for command in inbox {
        let outcome = validate_and_apply(world, &command, current);
        if let Err(reason) = outcome {
            world.resource_mut::<Events>().0.push(
                current,
                BattleEvent::CommandRejected {
                    command_seq: command.seq,
                    player: command.player,
                    reason: reason.clone(),
                },
            );
            world.resource_mut::<Rejected>().0.push((command, reason));
        }
    }
}

fn validate_and_apply(
    world: &mut World,
    command: &Command,
    current: Tick,
) -> Result<(), RejectReason> {
    if command.tick != current {
        return Err(RejectReason::StaleTick {
            command_tick: command.tick,
            current,
        });
    }

    // SIM-CMD-003: every addressed regiment must exist and belong to the player.
    let mut entities = Vec::with_capacity(command.kind.regiments().len());
    for &rid in command.kind.regiments() {
        let entity = world
            .resource::<Ids>()
            .regiment_entity(rid)
            .ok_or(RejectReason::UnknownRegiment(rid))?;
        let side = world
            .get::<Regiment>(entity)
            .ok_or(RejectReason::UnknownRegiment(rid))?
            .side;
        let owner = world.resource::<Sides>().0[side as usize].player;
        if owner != command.player {
            return Err(RejectReason::NotOwner(rid));
        }
        entities.push(entity);
    }

    match &command.kind {
        // SIM-DET-008: app-level behaviour, recorded only.
        CommandKind::Pause | CommandKind::SetSpeed { .. } => Ok(()),
        CommandKind::Halt { .. } => {
            for entity in entities {
                if let Some(mut order) = world.get_mut::<Order>(entity) {
                    order.kind = OrderKind::Idle;
                }
            }
            Ok(())
        }
        CommandKind::TransferControl { from, to } => {
            // Only the current owner (or the engine) may hand over control.
            if command.player != *from && command.player != PlayerId::ENGINE_AI {
                return Err(RejectReason::WrongPhase);
            }
            let mut sides = world.resource_mut::<Sides>();
            for side in &mut sides.0 {
                if side.player == *from {
                    side.player = *to;
                }
            }
            world.resource_mut::<Events>().0.push(
                current,
                BattleEvent::ControlTransferred {
                    from: *from,
                    to: *to,
                },
            );
            Ok(())
        }
        _ => Err(RejectReason::NotImplemented),
    }
}
