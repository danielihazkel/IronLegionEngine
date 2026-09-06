//! The battle screen's panels (T2-090, REQ-UI-001, REQ-UI-003): builds the
//! `il_ui` models from the session, the camera and the selection, and turns
//! the panels' clicks back into the same intents the keys give
//! (REQ-INP-006). `app.rs` owns the frame; this module owns what the frame
//! shows in a battle.

use std::collections::BTreeSet;

use glam::Vec2;
use il_core::{RegimentId, Scalar};
use il_data::Registries;
use il_render::{Camera, side_tint};
use il_sim_battle::components::{MoraleState, OrderKind};
use il_sim_battle::morale::{FatigueState, fatigue_state};
use il_sim_battle::{BattlePhase, BattleResult, BattleView, GeneralFate};
use il_ui::{
    AbilitySlot, CommandCardModel, MiniBlock, Minimap, RegimentCard, ResultRow, ResultSide,
    SelectedRegiment, Selection, SettingsState, SideTally,
};

use crate::session::BattleSession;

/// A cursor armed by a command that needs a target click (plan I4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Armed {
    AttackMove,
}

/// The battle screen's own state, reset per battle.
pub struct BattleUi {
    pub armed: Option<Armed>,
    /// The pause menu is open (decision 10).
    pub pause_open: bool,
    /// Whether the session was paused before the menu opened.
    pub pause_was_paused: bool,
    pub minimap: Minimap,
    /// The settings screen opened from the pause menu (T2-091).
    pub settings: Option<Box<SettingsState>>,
}

impl Default for BattleUi {
    fn default() -> Self {
        Self {
            armed: None,
            pause_open: false,
            pause_was_paused: false,
            minimap: Minimap::new(),
            settings: None,
        }
    }
}

/// The result screen's rows (T2-091, decision 15): per side the faction's
/// name, the general's fate, the loot and a row per regiment, named through
/// the setup's rosters (`RegimentResult.id` is the setup id).
pub fn result_sides(session: &BattleSession, result: &BattleResult) -> Vec<ResultSide> {
    let view = session.world.view();
    let regs = view.regs();
    let l = &regs.locale;
    let setup = session.setup();
    result
        .sides
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let side_setup = setup.and_then(|st| st.sides.get(i));
            let name = side_setup
                .and_then(|ss| regs.factions.lookup(&ss.faction))
                .map(|h| l.get(&regs.factions.get(h).name_key).to_string())
                .unwrap_or_else(|| l.fmt("il.result.side", &[("side", &i)]));
            let unit_name = |id: u32| -> String {
                side_setup
                    .and_then(|ss| {
                        ss.regiments
                            .iter()
                            .chain(ss.reinforcements.iter().flat_map(|g| g.regiments.iter()))
                            .find(|r| r.id == id)
                    })
                    .and_then(|r| regs.units.lookup(&r.unit_type))
                    .map(|h| l.get(&regs.units.get(h).name_key).to_string())
                    .unwrap_or_else(|| format!("#{id}"))
            };
            ResultSide {
                name,
                tint: side_tint(i as u8),
                fate: l
                    .get(match s.general_fate {
                        GeneralFate::Alive => "il.fate.alive",
                        GeneralFate::Wounded => "il.fate.wounded",
                        GeneralFate::Dead => "il.fate.dead",
                        GeneralFate::Captured => "il.fate.captured",
                    })
                    .to_string(),
                loot: s.loot,
                rows: s
                    .regiments
                    .iter()
                    .map(|r| ResultRow {
                        unit: unit_name(r.id),
                        initial: r.initial,
                        survivors: r.survivors,
                        killed: r.killed,
                        fled: r.fled,
                        experience: r.experience_gain,
                        ammo: r.ammo_left,
                        arrived: r.arrived,
                    })
                    .collect(),
            }
        })
        .collect()
}

/// The owned half of a `MinimapInput` (the map and the slices borrow it).
pub struct MinimapData {
    pub zone_colours: Vec<[u8; 3]>,
    pub zone_crossing: Vec<bool>,
    pub discs: Vec<(Vec2, f32)>,
    pub blocks: Vec<MiniBlock>,
    pub viewport: [Vec2; 4],
}

fn v(p: il_core::V2) -> Vec2 {
    Vec2::new(p.x.to_f32_render(), p.y.to_f32_render())
}

pub fn ticks_to_seconds(ticks: u16) -> f32 {
    f32::from(ticks) * il_core::TICK_SECONDS
}

pub fn morale_key(state: MoraleState) -> &'static str {
    match state {
        MoraleState::Steady => "il.morale.steady",
        MoraleState::Unsettled => "il.morale.unsettled",
        MoraleState::Shaken => "il.morale.shaken",
        MoraleState::Broken => "il.morale.broken",
        MoraleState::Routing => "il.morale.routing",
        MoraleState::Shattered => "il.morale.shattered",
    }
}

pub fn fatigue_key(state: FatigueState) -> &'static str {
    match state {
        FatigueState::Fresh => "il.fatigue.fresh",
        FatigueState::Active => "il.fatigue.active",
        FatigueState::Tired => "il.fatigue.tired",
        FatigueState::Exhausted => "il.fatigue.exhausted",
    }
}

pub fn order_key(order: OrderKind) -> &'static str {
    match order {
        OrderKind::Idle => "il.order.idle",
        OrderKind::Move => "il.order.move",
        OrderKind::AttackMove => "il.order.attack_move",
        OrderKind::AttackRegiment => "il.order.attack_regiment",
        OrderKind::Withdraw => "il.order.withdraw",
    }
}

/// The command card's rows for the selection (T1-070's selection card).
pub fn selection_rows(session: &BattleSession, selection: &Selection) -> Vec<SelectedRegiment> {
    let view = session.world.view();
    let regs = view.regs();
    let l = &regs.locale;
    selection
        .regiments
        .iter()
        .filter_map(|id| view.regiment(*id))
        .map(|r| SelectedRegiment {
            id: r.id,
            unit: l.get(&regs.units.get(r.unit).name_key).to_string(),
            soldiers: r.soldier_count,
            formation: l
                .get(&regs.formations.get(r.formation).name_key)
                .to_string(),
            ranks: r.ranks,
            order: l.get(order_key(r.order)).to_string(),
            morale: l.fmt(
                "il.battle.morale",
                &[
                    ("value", &format!("{:.0}", r.morale.to_f32_render())),
                    ("state", &l.get(morale_key(r.morale_state))),
                ],
            ),
            fatigue: l
                .get(fatigue_key(fatigue_state(
                    r.fatigue_mean,
                    &regs.rules.fatigue,
                )))
                .to_string(),
            abilities: ability_slots(&view, r.id)
                .into_iter()
                .map(|a| {
                    l.fmt(
                        "il.battle.ability",
                        &[
                            ("key", &a.key as &dyn std::fmt::Display),
                            ("name", &a.name),
                            ("state", &a.state),
                        ],
                    )
                })
                .collect(),
            statuses: view
                .statuses(r.id)
                .iter()
                .map(|s| {
                    l.fmt(
                        "il.battle.status",
                        &[
                            (
                                "name",
                                &l.get(&regs.abilities.get(s.ability).name_key)
                                    as &dyn std::fmt::Display,
                            ),
                            ("seconds", &format!("{:.0}", ticks_to_seconds(s.remaining))),
                        ],
                    )
                })
                .collect(),
        })
        .collect()
}

/// The regiment's ability slots as the card shows them (T2-050 lines).
pub fn ability_slots(view: &BattleView, id: RegimentId) -> Vec<AbilitySlot> {
    let regs = view.regs();
    let l = &regs.locale;
    view.abilities(id)
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let ready = a.cooldown == 0;
            AbilitySlot {
                key: (i + 1) as u8,
                name: l.get(&regs.abilities.get(a.ability).name_key).to_string(),
                state: if ready {
                    l.get("il.battle.ready").to_string()
                } else {
                    l.fmt(
                        "il.battle.seconds",
                        &[("seconds", &format!("{:.0}", ticks_to_seconds(a.cooldown)))],
                    )
                },
                ready,
            }
        })
        .collect()
}

/// One card per own regiment, ascending id (decision 8).
pub fn card_models(session: &BattleSession, selection: &Selection) -> Vec<RegimentCard> {
    let view = session.world.view();
    let regs = view.regs();
    let l = &regs.locale;
    let player = session.local_player();
    let sides = view.sides();
    view.regiments()
        .filter(|r| {
            sides
                .get(usize::from(r.side))
                .is_some_and(|s| s.player == player)
        })
        .map(|r| RegimentCard {
            id: r.id,
            unit: l.get(&regs.units.get(r.unit).name_key).to_string(),
            soldiers: r.soldier_count,
            initial: u32::from(r.initial),
            morale_state: r.morale_state,
            fatigue: l
                .get(fatigue_key(fatigue_state(
                    r.fatigue_mean,
                    &regs.rules.fatigue,
                )))
                .to_string(),
            // `ammo` is the soldiers' sum; a volley spends one per soldier.
            volleys: r.fire.map(|_| {
                let n = r.soldier_count.max(1);
                (u32::from(view.ammo(r.id)) / n) as u16
            }),
            engaged: r.engaged,
            selected: selection.regiments.contains(&r.id),
            group: selection
                .groups
                .iter()
                .position(|g| g.contains(&r.id))
                .map(|n| n as u8),
            tint: side_tint(r.side),
        })
        .collect()
}

/// The command card's model for the selection (decision 9).
pub fn command_model<'a>(
    session: &BattleSession,
    selection: &Selection,
    rows: &'a [SelectedRegiment],
    regs: &'a Registries,
    run: bool,
    armed: Option<Armed>,
) -> CommandCardModel<'a> {
    let view = session.world.view();
    let l = &regs.locale;
    let first = selection.regiments.iter().find_map(|id| view.regiment(*id));
    let fire = selection
        .regiments
        .iter()
        .filter_map(|id| view.regiment(*id))
        .find_map(|r| r.fire);
    let formations = first.map_or_else(Vec::new, |r| {
        regs.units
            .get(r.unit)
            .formation_ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let h = regs.formations.lookup(id)?;
                Some((
                    (i + 1) as u8,
                    l.get(&regs.formations.get(h).name_key).to_string(),
                ))
            })
            .collect()
    });
    let abilities = first.map_or_else(Vec::new, |r| ability_slots(&view, r.id));
    let presets = regs
        .group_formations
        .iter()
        .map(|(_, t)| (t.id.clone(), l.get(&t.name_key).to_string()))
        .collect();
    CommandCardModel {
        selection: rows,
        fire,
        run,
        armed_attack_move: armed == Some(Armed::AttackMove),
        formations,
        abilities,
        presets,
        deploying: view.phase() == BattlePhase::Deployment,
        locale: l,
    }
}

/// The casualties line's rows (decision 11), from the regiment rows: alive
/// is the soldier count, fled the fled plus the withdrawn, killed what is
/// left of the initial strength (as `result::compute` counts it). Read
/// from the rows rather than tallied from events so a loaded battle
/// (T2-101) shows the same numbers.
pub fn tallies(session: &BattleSession) -> Vec<SideTally> {
    let view = session.world.view();
    let regs = view.regs();
    let l = &regs.locale;
    let mut totals = vec![(0u32, 0u32, 0u32); view.sides().len()];
    for r in view.regiments() {
        if let Some(t) = totals.get_mut(usize::from(r.side)) {
            let fled = u32::from(r.fled) + u32::from(r.withdrawn);
            t.0 += r.soldier_count;
            t.1 += u32::from(r.initial)
                .saturating_sub(r.soldier_count)
                .saturating_sub(fled);
            t.2 += fled;
        }
    }
    view.sides()
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let name = regs
                .factions
                .lookup(&s.faction)
                .map(|h| l.get(&regs.factions.get(h).name_key).to_string())
                .unwrap_or_else(|| l.fmt("il.result.side", &[("side", &i)]));
            SideTally {
                name,
                tint: side_tint(i as u8),
                alive: totals[i].0,
                killed: totals[i].1,
                fled: totals[i].2,
            }
        })
        .collect()
}

/// The minimap's data: the observer side's discs and what it sees
/// (decisions 4 and 11).
pub fn minimap_data(
    session: &BattleSession,
    selection: &BTreeSet<RegimentId>,
    camera: &Camera,
    screen: Vec2,
) -> MinimapData {
    let view = session.world.view();
    let regs = view.regs();
    let map = view.map();
    let zone_colours = map
        .zone_handles
        .iter()
        .map(|h| regs.zones.get(*h).colour.0)
        .collect();
    let zone_crossing = map
        .zone_handles
        .iter()
        .map(|h| regs.zones.get(*h).crossing)
        .collect();
    let observer = session.observer_side();
    let mut discs = Vec::new();
    let mut blocks = Vec::new();
    for r in view.regiments() {
        if r.soldier_count == 0 {
            continue;
        }
        let own = observer == Some(r.side);
        if own {
            discs.push((v(r.anchor_pos), view.los_radius(r.id).to_f32_render()));
        }
        let visible = match observer {
            Some(side) => view.visible(side, r.id),
            None => true,
        };
        if own || visible {
            blocks.push(MiniBlock {
                pos: v(r.anchor_pos),
                tint: side_tint(r.side),
                ghost: false,
                selected: selection.contains(&r.id),
            });
        } else if let Some(seen) = observer.and_then(|side| view.seen(side, r.id)) {
            blocks.push(MiniBlock {
                pos: v(seen.pos),
                tint: side_tint(r.side),
                ghost: true,
                selected: false,
            });
        }
    }
    let viewport = [
        Vec2::ZERO,
        Vec2::new(screen.x, 0.0),
        screen,
        Vec2::new(0.0, screen.y),
    ]
    .map(|c| camera.screen_to_world(c, screen));
    MinimapData {
        zone_colours,
        zone_crossing,
        discs,
        blocks,
        viewport,
    }
}

/// The zoom factor for a window height (decision 7): 1080 rows are scale 1.
pub fn zoom_factor(window_height_px: f32, user_scale: f32) -> f32 {
    (window_height_px / 1080.0 * user_scale).clamp(0.5, 3.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_factor_follows_the_window_height_and_the_user_scale() {
        assert!((zoom_factor(1080.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((zoom_factor(1440.0, 1.0) - 1.3333).abs() < 1e-3);
        assert!((zoom_factor(1080.0, 1.5) - 1.5).abs() < 1e-6);
        assert!((zoom_factor(200.0, 1.0) - 0.5).abs() < 1e-6, "clamped");
    }
}
