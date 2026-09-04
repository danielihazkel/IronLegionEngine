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

use crate::components::{
    Anchor, Combat, Fire, FormationState, Morale, MoraleState, Order, OrderKind, Path, Pos,
    Regiment, SlotRef,
};
use crate::events::BattleEvent;
use crate::formation::{
    RegimentInfo, arrange_group, effective_ranks, set_facing, slot_world, spacing,
};
use crate::resources::{
    BattlePhase, Clock, CommandInbox, Events, Ids, MapRes, PathRequests, Phase, Regs, Rejected,
    Sides,
};

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
    /// A content id the command names is not in the registries.
    UnknownContent(ContentId),
    /// The regiment's unit type does not list this formation template.
    FormationNotAllowed {
        regiment: RegimentId,
        template: ContentId,
    },
    /// `AttackRegiment` or `FireMode::Target` names an own-side or empty
    /// regiment (T2-020, T2-030).
    InvalidTarget(RegimentId),
    /// `FireMode` addressed a regiment whose unit has no `ranged` block
    /// (T2-030).
    NotRanged(RegimentId),
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
        // SIM-CMD-004: routing and shattered regiments take no orders.
        let routing = world
            .get::<Morale>(entity)
            .is_some_and(|m| matches!(m.state, MoraleState::Routing | MoraleState::Shattered));
        if routing && !matches!(command.kind, CommandKind::Withdraw { .. }) {
            return Err(RejectReason::Routing(rid));
        }
        entities.push(entity);
    }

    match &command.kind {
        // SIM-DET-008: app-level behaviour, recorded only.
        CommandKind::Pause | CommandKind::SetSpeed { .. } => Ok(()),
        CommandKind::Halt { .. } => {
            for entity in entities {
                halt(world, entity);
            }
            Ok(())
        }
        CommandKind::Move {
            target,
            facing,
            speed,
            ..
        } => {
            for entity in entities {
                // SIM-CMBT-003: an engaged regiment obeys the move but not
                // the facing (the morale penalty arrives with T2-041).
                let engaged = world.get::<Combat>(entity).is_some_and(|c| c.engaged);
                issue_move(
                    world,
                    entity,
                    OrderKind::Move,
                    *target,
                    if engaged { None } else { *facing },
                    *speed,
                    current,
                );
            }
            Ok(())
        }
        // SIM-CMBT-004: chase a regiment. Every addressed regiment must be
        // an enemy of the target, and the target must still have soldiers.
        CommandKind::AttackRegiment { target, .. } => {
            let target_entity = world
                .resource::<Ids>()
                .regiment_entity(*target)
                .ok_or(RejectReason::UnknownRegiment(*target))?;
            let (target_side, alive, target_pos) = {
                let r = world
                    .get::<Regiment>(target_entity)
                    .ok_or(RejectReason::UnknownRegiment(*target))?;
                let a = world.get::<Anchor>(target_entity).expect("anchor");
                (r.side, !r.soldiers.is_empty(), a.pos)
            };
            if !alive {
                return Err(RejectReason::InvalidTarget(*target));
            }
            for entity in &entities {
                let side = world.get::<Regiment>(*entity).expect("validated").side;
                if side == target_side {
                    return Err(RejectReason::InvalidTarget(*target));
                }
            }
            for entity in entities {
                let speed = world
                    .get::<Order>(entity)
                    .map_or(SpeedMode::Walk, |o| o.speed);
                issue_move(
                    world,
                    entity,
                    OrderKind::AttackRegiment,
                    target_pos,
                    None,
                    speed,
                    current,
                );
                if let Some(mut order) = world.get_mut::<Order>(entity) {
                    order.target_regiment = Some(*target);
                }
            }
            Ok(())
        }
        // SIM-CMBT-005: an attack-move acquires its target on the way
        // (`combat::pursue_update`).
        CommandKind::AttackMove { target, .. } => {
            for entity in entities {
                let speed = world
                    .get::<Order>(entity)
                    .map_or(SpeedMode::Walk, |o| o.speed);
                issue_move(
                    world,
                    entity,
                    OrderKind::AttackMove,
                    *target,
                    None,
                    speed,
                    current,
                );
            }
            Ok(())
        }
        CommandKind::SetFormation {
            template, ranks, ..
        } => {
            let handle = world
                .resource::<Regs>()
                .0
                .formations
                .lookup(template)
                .ok_or_else(|| RejectReason::UnknownContent(template.clone()))?;
            // Every regiment must be allowed the template before any changes.
            for entity in &entities {
                let regiment = world.get::<Regiment>(*entity).expect("validated");
                let allowed = world
                    .resource::<Regs>()
                    .0
                    .units
                    .get(regiment.unit)
                    .formations
                    .contains(&handle);
                if !allowed {
                    return Err(RejectReason::FormationNotAllowed {
                        regiment: regiment.id,
                        template: template.clone(),
                    });
                }
            }
            for entity in entities {
                set_formation(world, entity, handle, *ranks, current);
            }
            Ok(())
        }
        CommandKind::SetFacing { facing, .. } => {
            for entity in entities {
                apply_facing(world, entity, *facing);
            }
            Ok(())
        }
        CommandKind::SetSpeedMode { mode, .. } => {
            for entity in entities {
                if let Some(mut order) = world.get_mut::<Order>(entity) {
                    order.speed = *mode;
                }
            }
            Ok(())
        }
        CommandKind::GroupFormation {
            template,
            anchor,
            facing,
            width,
            ..
        } => {
            let regs = world.resource::<Regs>().0.clone();
            let group = regs
                .group_formations
                .lookup(template)
                .ok_or_else(|| RejectReason::UnknownContent(template.clone()))?;
            let infos: Vec<RegimentInfo> = entities
                .iter()
                .map(|&e| {
                    let r = world.get::<Regiment>(e).expect("validated");
                    let a = world.get::<Anchor>(e).expect("anchor");
                    let f = world.get::<FormationState>(e).expect("formation");
                    let unit = regs.units.get(r.unit);
                    RegimentInfo {
                        id: r.id,
                        pos: a.pos,
                        category: unit.category,
                        count: r.soldiers.len() as u16,
                        template: f.template,
                        radius: unit.soldier_radius,
                    }
                })
                .collect();
            let placements = arrange_group(
                regs.group_formations.get(group),
                &infos,
                *anchor,
                *facing,
                *width,
                &regs.rules.formation,
                &regs,
            );
            for placement in placements {
                let Some(entity) = world.resource::<Ids>().regiment_entity(placement.id) else {
                    continue;
                };
                let (template, count) = {
                    let r = world.get::<Regiment>(entity).expect("validated");
                    let f = world.get::<FormationState>(entity).expect("formation");
                    (f.template, r.soldiers.len() as u16)
                };
                let ranks =
                    effective_ranks(regs.formations.get(template), count, Some(placement.ranks));
                if let Some(mut state) = world.get_mut::<FormationState>(entity) {
                    state.ranks = ranks;
                    state.needs_reform = true;
                }
                let speed = world
                    .get::<Order>(entity)
                    .map_or(SpeedMode::Walk, |o| o.speed);
                issue_move(
                    world,
                    entity,
                    OrderKind::Move,
                    placement.anchor,
                    Some(placement.facing),
                    speed,
                    current,
                );
            }
            Ok(())
        }
        CommandKind::Deploy {
            position,
            facing,
            template,
            ..
        } => {
            if world.resource::<Phase>().0 != BattlePhase::Deployment {
                return Err(RejectReason::WrongPhase);
            }
            let entity = entities[0];
            if let Some(id) = template {
                let handle = world
                    .resource::<Regs>()
                    .0
                    .formations
                    .lookup(id)
                    .ok_or_else(|| RejectReason::UnknownContent(id.clone()))?;
                set_formation(world, entity, handle, None, current);
            }
            deploy(world, entity, *position, *facing);
            Ok(())
        }
        // SIM-PROJ-001: every addressed regiment must shoot; a `Target` must
        // be an enemy with soldiers left. The target is re-acquired at the
        // next Stage 9 under the new mode.
        CommandKind::FireMode { mode, .. } => {
            for entity in &entities {
                if world.get::<Fire>(*entity).is_none() {
                    let id = world.get::<Regiment>(*entity).expect("validated").id;
                    return Err(RejectReason::NotRanged(id));
                }
            }
            if let FireMode::Target(target) = mode {
                let target_entity = world
                    .resource::<Ids>()
                    .regiment_entity(*target)
                    .ok_or(RejectReason::UnknownRegiment(*target))?;
                let (target_side, alive) = {
                    let r = world
                        .get::<Regiment>(target_entity)
                        .ok_or(RejectReason::UnknownRegiment(*target))?;
                    (r.side, !r.soldiers.is_empty())
                };
                if !alive {
                    return Err(RejectReason::InvalidTarget(*target));
                }
                for entity in &entities {
                    let side = world.get::<Regiment>(*entity).expect("validated").side;
                    if side == target_side {
                        return Err(RejectReason::InvalidTarget(*target));
                    }
                }
            }
            for entity in entities {
                if let Some(mut fire) = world.get_mut::<Fire>(entity) {
                    fire.mode = *mode;
                    fire.target = None;
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

/// `Halt`: the order ends where the regiment stands; a pending path
/// request is dropped and no wheel target remains.
pub(crate) fn halt(world: &mut World, entity: Entity) {
    let id = world.get::<Regiment>(entity).map(|r| r.id);
    if let Some(mut order) = world.get_mut::<Order>(entity) {
        order.kind = OrderKind::Idle;
        order.facing = None;
        order.target_regiment = None;
    }
    if let Some(mut path) = world.get_mut::<Path>(entity) {
        *path = Path::default();
    }
    if let Some(id) = id {
        world.resource_mut::<PathRequests>().0.remove(&id);
    }
}

/// `Move` / `AttackMove` / a group placement: a fresh order and path
/// request; the target is clamped to the map; a reform is requested
/// (SIM-FORM-020).
fn issue_move(
    world: &mut World,
    entity: Entity,
    kind: OrderKind,
    target: V2,
    facing: Option<Angle<S>>,
    speed: SpeedMode,
    tick: Tick,
) {
    let target = world.resource::<MapRes>().0.clamp(target);
    let id = world.get::<Regiment>(entity).map(|r| r.id);
    if let Some(mut order) = world.get_mut::<Order>(entity) {
        *order = Order {
            kind,
            target,
            target_regiment: None,
            facing,
            speed,
            since: tick,
        };
    }
    if let Some(mut path) = world.get_mut::<Path>(entity) {
        *path = Path {
            waypoints: Vec::new(),
            next: 0,
            requested: true,
        };
    }
    if let Some(mut state) = world.get_mut::<FormationState>(entity) {
        state.needs_reform = true;
    }
    if let Some(id) = id {
        world.resource_mut::<PathRequests>().0.insert(id);
    }
}

/// `SetFormation` (SIM-FORM-032): a morph of `morph_ticks` to the new
/// template; an explicit order cancels any automatic corridor morph.
fn set_formation(
    world: &mut World,
    entity: Entity,
    handle: il_data::Handle<il_data::FormationTemplate>,
    ranks: Option<u8>,
    tick: Tick,
) {
    let (count, morph_ticks, new_ranks) = {
        let regs = &world.resource::<Regs>().0;
        let t = regs.formations.get(handle);
        let count = world
            .get::<Regiment>(entity)
            .map_or(0, |r| r.soldiers.len() as u16);
        (count, t.morph_ticks, effective_ranks(t, count, ranks))
    };
    let _ = count;
    if let Some(mut state) = world.get_mut::<FormationState>(entity) {
        if state.template != handle {
            state.template = handle;
            state.morph_until = Tick(tick.0 + u32::from(morph_ticks));
        }
        state.prior_template = None;
        state.ranks = new_ranks;
        state.needs_reform = true;
    }
}

/// `SetFacing` through `formation::set_facing` (wheel or about-face).
fn apply_facing(world: &mut World, entity: Entity, facing: Angle<S>) {
    let sr = {
        let regs = &world.resource::<Regs>().0;
        let r = world.get::<Regiment>(entity).expect("validated");
        let f = world.get::<FormationState>(entity).expect("formation");
        spacing(
            regs.formations.get(f.template),
            regs.units.get(r.unit).soldier_radius,
        )
        .1
    };
    let rules = world.resource::<Regs>().0.rules.formation.clone();
    let mut anchor = *world.get::<Anchor>(entity).expect("anchor");
    let mut order = *world.get::<Order>(entity).expect("order");
    let mut state = world
        .get::<FormationState>(entity)
        .expect("formation")
        .clone();
    set_facing(&mut anchor, &mut order, &mut state, &rules, sr, facing);
    *world.get_mut::<Anchor>(entity).expect("anchor") = anchor;
    *world.get_mut::<Order>(entity).expect("order") = order;
    *world.get_mut::<FormationState>(entity).expect("formation") = state;
}

/// `Deploy` (position only in Phase 1): the anchor moves and every soldier
/// is placed on its current slot around it.
fn deploy(world: &mut World, entity: Entity, position: V2, facing: Angle<S>) {
    let position = world.resource::<MapRes>().0.clamp(position);
    let anchor = Anchor {
        pos: position,
        facing,
    };
    *world.get_mut::<Anchor>(entity).expect("anchor") = anchor;
    halt(world, entity);
    let (soldiers, slots) = {
        let r = world.get::<Regiment>(entity).expect("validated");
        let f = world.get::<FormationState>(entity).expect("formation");
        (r.soldiers.clone(), f.slots.clone())
    };
    if let Some(mut state) = world.get_mut::<FormationState>(entity) {
        state.laid_out_facing = facing;
        state.needs_reform = true;
    }
    for sid in soldiers {
        let Some(e) = world.resource::<Ids>().soldier_entity(sid) else {
            continue;
        };
        let Some(slot) = world.get::<SlotRef>(e).and_then(|s| s.slot) else {
            continue;
        };
        if let Some(target) = slots.get(usize::from(slot)).map(|s| slot_world(&anchor, s))
            && let Some(mut pos) = world.get_mut::<Pos>(e)
        {
            pos.p = target;
        }
    }
}
