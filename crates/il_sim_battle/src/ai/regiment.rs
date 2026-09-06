//! Regiment-level decisions (SIM-AI-021, SIM-AI-022; T2-081): one pass per
//! due regiment over the movement, formation, fire and ability channels of
//! its side's regiment action set (plan decision 5), each winner turned
//! into a Command only when it changes something (plan I6) so the outbox
//! never carries a command the sim would reject or that re-issues an order
//! the regiment already follows.

use il_ai::{Channel, select};
use il_core::{Angle, RngStream, S, Scalar, V2};
use il_data::{ActionKind, AiActionSet, AiProfile, Registries, Targeting};

use crate::ai::inputs::{RegRow, RegimentContext, SideSnapshot};
use crate::ai::{ArmyPlan, Decisions, Role};
use crate::command::{AbilityTarget, CommandKind, FireMode, SpeedMode};
use crate::components::OrderKind;
use crate::movement::regiment::deg_to_rad;

/// The reserve position of a regiment: `reserve_offset` behind the line
/// at its own lateral position (plan decision 13).
pub fn reserve_position(plan: &ArmyPlan, anchor: V2, reserve_offset: S) -> V2 {
    let forward = plan.line_facing.direction();
    let right = V2::new(forward.y, -forward.x);
    let lateral = (anchor - plan.line_anchor).dot(right);
    plan.line_anchor + right * lateral - forward * reserve_offset
}

/// SIM-AI-022: the `1/morale`-weighted centroid of the side's other
/// standing regiments, pushed to `reserve_offset` behind the line when a
/// plan exists (plan decision 14).
pub fn general_position(
    snap: &SideSnapshot,
    me: &RegRow,
    plan: Option<&ArmyPlan>,
    reserve_offset: S,
) -> Option<V2> {
    let mut sum = V2::ZERO;
    let mut total = S::ZERO;
    for r in snap.own.iter().filter(|r| r.id != me.id) {
        let w = S::ONE / r.morale.max(S::ONE);
        sum += r.anchor * w;
        total = total + w;
    }
    if total <= S::ZERO {
        return None;
    }
    let mut target = sum * (S::ONE / total);
    if let Some(plan) = plan {
        let forward = plan.line_facing.direction();
        let ahead = (target - plan.line_anchor).dot(forward);
        if ahead > -reserve_offset {
            target -= forward * (ahead + reserve_offset);
        }
    }
    Some(target)
}

/// Whether a `Move` to `target` is already the regiment's order.
fn already_moving_to(me: &RegRow, target: V2, tolerance: S) -> bool {
    me.order == OrderKind::Move && me.order_target.distance(target) <= tolerance
}

/// A `Move` toward `target` facing `facing` (at walk, or at run for a
/// flank group, plan decision 24), unless the regiment is there or already
/// going there.
fn move_to(
    me: &RegRow,
    target: V2,
    facing: Option<Angle<S>>,
    tolerance: S,
    reform_angle_deg: S,
    run: bool,
    out: &mut Decisions,
) {
    if me.anchor.distance(target) > tolerance {
        // No facing on the move: a formation that must keep facing the
        // enemy while stepping sideways crabs at a fraction of walk speed;
        // the facing is dressed on arrival below (SIM-MOVE-013).
        if !already_moving_to(me, target, S::ONE) {
            // The approach marches (walk pace, half the fatigue of walking
            // in formation, SIM-FAT-002); flank groups and the charge run.
            out.push(CommandKind::Move {
                regiments: vec![me.id],
                target,
                facing: None,
                speed: if run {
                    SpeedMode::Run
                } else {
                    SpeedMode::March
                },
            });
        }
    } else if me.order.moves() {
        out.push(CommandKind::Halt {
            regiments: vec![me.id],
        });
    } else if let Some(f) = facing
        && me.order == OrderKind::Idle
        && me.facing.delta(f).abs() > deg_to_rad(reform_angle_deg)
    {
        out.push(CommandKind::SetFacing {
            regiments: vec![me.id],
            facing: f,
        });
    }
}

/// `AttackRegiment` on `target` unless the regiment already chases it;
/// flank groups run (plan decision 24).
fn attack(me: &RegRow, target: il_core::RegimentId, run: bool, out: &mut Decisions) {
    if run && me.order_speed != SpeedMode::Run {
        out.push(CommandKind::SetSpeedMode {
            regiments: vec![me.id],
            mode: SpeedMode::Run,
        });
    }
    if !(me.order == OrderKind::AttackRegiment && me.order_target_regiment == Some(target)) {
        out.push(CommandKind::AttackRegiment {
            regiments: vec![me.id],
            target,
        });
    }
}

/// Decides for one own regiment (`me` indexes `snap.own`).
#[allow(clippy::too_many_arguments)]
pub fn decide(
    regs: &Registries,
    snap: &SideSnapshot,
    me: &RegRow,
    profile: &AiProfile,
    set: &AiActionSet,
    plan: Option<&ArmyPlan>,
    rng: &mut RngStream,
    out: &mut Decisions,
) {
    let role = plan.and_then(|p| p.role_of(me.id));
    let slot = role.and_then(|r| r.slot());
    let ctx = RegimentContext::new(snap, me, slot, plan.is_some_and(|p| p.charging), regs);
    // A slot within twice the waypoint radius is held (the line steps by
    // `advance_step`, which must exceed this for the line to move).
    let tolerance = (regs.rules.movement.waypoint_radius * S::from_i32(2)).max(S::ONE);
    let reform_angle = regs.rules.formation.reform_angle;
    let line_facing = plan.map(|p| p.line_facing);
    let is_bodyguard = snap.is_bodyguard(me.id);
    // Flank groups always run; the line runs the last stretch once the plan
    // is charging (SIM-AI-011, T2-082 tuning).
    let charging = plan.is_some_and(|p| p.charging);
    let run = matches!(role, Some(Role::Flank { .. }))
        || (charging && matches!(role, Some(Role::Line { .. }) | Some(Role::Reserve { .. })));

    // ---- movement (plan I13: some roles decide it themselves) ----------
    let overridden = match role {
        Some(Role::Flank {
            charge: Some(t), ..
        }) => {
            attack(me, t, true, out);
            true
        }
        Some(Role::Committed { target }) => {
            attack(me, target, false, out);
            true
        }
        _ => false,
    };
    if !overridden {
        let nearest = ctx.nearest;
        let enemy_share = S::ONE - ctx.strength_ratio();
        let winner = select(
            set,
            Channel::Movement,
            &mut |_, a| match a.kind {
                // Only the line (and a regiment without a plan) picks its own
                // fights; skirmishers, reserves, flank groups, counters and
                // screens hold their slots until the plan commits them.
                ActionKind::EngageNearest => {
                    nearest.is_some()
                        && (!is_bodyguard || profile.general_aggression > enemy_share)
                        && matches!(role, None | Some(Role::Line { .. }) | Some(Role::Bodyguard))
                }
                ActionKind::FollowCentroid => is_bodyguard,
                ActionKind::FallBack => plan.is_some(),
                ActionKind::HoldPosition => true,
                _ => false,
            },
            &ctx,
            Some(rng),
        );
        match winner.map(|c| &c.action.kind) {
            Some(ActionKind::EngageNearest) => {
                if let Some(e) = nearest {
                    attack(me, e.id, run, out);
                }
            }
            Some(ActionKind::HoldPosition) => {
                // An engaged regiment is never pulled out of its fight for a
                // slot (SIM-MOR-025 would charge the disengage shock).
                if !me.engaged
                    && let Some(s) = slot
                {
                    move_to(me, s, line_facing, tolerance, reform_angle, run, out);
                }
            }
            Some(ActionKind::FallBack) => {
                if let Some(p) = plan {
                    let target = reserve_position(p, me.anchor, profile.reserve_offset);
                    move_to(me, target, line_facing, tolerance, reform_angle, false, out);
                }
            }
            Some(ActionKind::FollowCentroid) => {
                if !me.engaged
                    && let Some(target) = general_position(snap, me, plan, profile.reserve_offset)
                {
                    move_to(me, target, line_facing, tolerance, reform_angle, false, out);
                }
            }
            _ => {}
        }
    }

    // ---- formation ------------------------------------------------------
    let unit = regs.units.get(me.unit);
    let winner = select(
        set,
        Channel::Formation,
        &mut |_, a| match &a.kind {
            ActionKind::SwitchFormation { layout } => {
                *layout != me.layout
                    && !me.morphing
                    && unit
                        .formations
                        .iter()
                        .any(|h| regs.formations.get(*h).layout == *layout)
            }
            _ => false,
        },
        &ctx,
        Some(rng),
    );
    if let Some(ActionKind::SwitchFormation { layout }) = winner.map(|c| &c.action.kind)
        && let Some(h) = unit
            .formations
            .iter()
            .find(|h| regs.formations.get(**h).layout == *layout)
    {
        out.push(CommandKind::SetFormation {
            regiments: vec![me.id],
            template: regs.formations.id_of(*h).clone(),
            ranks: None,
        });
    }

    // ---- fire -----------------------------------------------------------
    if let Some(mode) = me.fire_mode {
        let winner = select(
            set,
            Channel::Fire,
            &mut |_, a| matches!(a.kind, ActionKind::FireAtWill | ActionKind::HoldFire),
            &ctx,
            Some(rng),
        );
        let wanted = match winner.map(|c| &c.action.kind) {
            Some(ActionKind::FireAtWill) => Some(FireMode::FireAtWill),
            Some(ActionKind::HoldFire) => Some(FireMode::Hold),
            _ => None,
        };
        if let Some(w) = wanted
            && w != mode
            && !(w == FireMode::FireAtWill && matches!(mode, FireMode::Target(_)))
        {
            out.push(CommandKind::FireMode {
                regiments: vec![me.id],
                mode: w,
            });
        }
    }

    // ---- abilities: one channel per slot --------------------------------
    for (handle, cooldown) in &me.slots {
        if *cooldown > 0 {
            continue;
        }
        let ability = regs.abilities.get(*handle);
        if me.energy < ability.energy_cost
            || (ability.requires_not_engaged && me.engaged)
            || (ability.requires_not_moving && me.moving)
        {
            continue;
        }
        let in_range = |p: V2| ability.range <= S::ZERO || me.anchor.distance(p) <= ability.range;
        let target = match ability.targeting {
            Targeting::SelfTarget | Targeting::Area => Some(AbilityTarget::SelfTarget),
            Targeting::RegimentEnemy => snap
                .nearest_enemy_where(me.anchor, |e| in_range(e.anchor))
                .map(|e| AbilityTarget::Regiment(e.id)),
            Targeting::RegimentAlly => {
                let mut best: Option<(S, &RegRow)> = None;
                for r in snap
                    .own
                    .iter()
                    .filter(|r| r.id != me.id && in_range(r.anchor))
                {
                    let d = r.anchor.distance_sq(me.anchor);
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, r));
                    }
                }
                best.map(|(_, r)| AbilityTarget::Regiment(r.id))
            }
            Targeting::Point => Some(AbilityTarget::Point(me.anchor)),
        };
        let Some(target) = target else {
            continue;
        };
        let id = regs.abilities.id_of(*handle);
        let winner = select(
            set,
            Channel::Ability,
            &mut |_, a| matches!(&a.kind, ActionKind::UseAbility { ability } if ability == id),
            &ctx,
            Some(rng),
        );
        if winner.is_some() {
            out.push(CommandKind::UseAbility {
                regiment: me.id,
                ability: id.clone(),
                target,
            });
        }
    }
}
