//! Army-level decisions (SIM-AI-010..014; T2-082): every army period the
//! side's stance is scored over its army action set (with the hysteresis
//! margin of plan decision 10) and the plan is rebuilt from it: the battle
//! line and its slots, the skirmishers' places, the reserves and their
//! commitment, the cavalry's flank, counter-charge or screen roles, and the
//! `Withdraw`s of a retreat. Regiments read their role between periods.

use il_ai::{Channel, InputProvider, score, select};
use il_core::{Angle, RegimentId, RngStream, S, Scalar, Tick, V2};
use il_data::{
    ActionKind, AiActionSet, AiProfile, ContentId, GroupFormationTemplate, GroupKind, Registries,
    UnitCategory,
};

use crate::ai::inputs::{RegRow, SideSnapshot};
use crate::ai::{ArmyPlan, Assignment, Decisions, Role, Stance};
use crate::command::CommandKind;
use crate::components::OrderKind;
use crate::formation::{RegimentInfo, arrange_group, arranged_width};
use crate::map::LoadedMap;

/// The army inputs of SIM-AI-010 (`ArmyContext: InputProvider`).
pub struct ArmyContext<'a> {
    pub snap: &'a SideSnapshot,
    pub profile: &'a AiProfile,
    pub map: &'a LoadedMap,
    pub height_ref: S,
    pub tick: Tick,
    pub battle_start: Tick,
    pub time_limit: u32,
}

impl ArmyContext<'_> {
    fn weights(&self) -> (S, S) {
        let own = self.snap.standing().fold(S::ZERO, |acc, r| acc + r.weight);
        let enemy = self
            .snap
            .enemies
            .iter()
            .fold(S::ZERO, |acc, r| acc + r.weight);
        (own, enemy)
    }

    /// `own / (own + visible enemy)` by cost (plan I12); 1 with nobody visible.
    pub fn strength_ratio(&self) -> S {
        let (own, enemy) = self.weights();
        if own + enemy > S::ZERO {
            own / (own + enemy)
        } else {
            S::ONE
        }
    }

    fn mean(&self, f: impl Fn(&RegRow) -> S) -> S {
        let mut sum = S::ZERO;
        let mut n = 0;
        for r in self.snap.standing() {
            sum = sum + f(r);
            n += 1;
        }
        if n > 0 { sum / S::from_i32(n) } else { S::ZERO }
    }

    /// The mean anchor of the standing regiments (the whole side when none
    /// stands), or the enemy zones' mean for an empty side.
    pub fn centroid(&self) -> V2 {
        let mut sum = V2::ZERO;
        let mut n = 0;
        for r in self.snap.standing() {
            sum += r.anchor;
            n += 1;
        }
        if n == 0 {
            for r in &self.snap.own {
                sum += r.anchor;
                n += 1;
            }
        }
        if n > 0 {
            sum * (S::ONE / S::from_i32(n))
        } else {
            self.snap.enemy_zone
        }
    }
}

impl InputProvider for ArmyContext<'_> {
    fn input(&self, id: il_data::InputId) -> S {
        use il_data::InputId;
        match id {
            InputId::Constant => S::ONE,
            InputId::ArmyStrengthRatio => self.strength_ratio(),
            InputId::ArmyMoraleMean => self.mean(|r| r.morale / S::from_i32(100)),
            InputId::ArmyFatigueMean => self.mean(|r| r.fatigue),
            InputId::TerrainAdvantage => {
                // Plan I22: 0.5 + 0.5 · sat((h_own − h_target) / height_ref).
                let own = self.map.height_at(self.centroid());
                let target = self.map.height_at(self.snap.enemy_centroid());
                let d = if self.height_ref > S::ZERO {
                    ((own - target) / self.height_ref).clamp(-S::ONE, S::ONE)
                } else {
                    S::ZERO
                };
                S::HALF + S::HALF * d
            }
            InputId::TimeRemaining => {
                let elapsed = self.tick.0.saturating_sub(self.battle_start.0);
                if self.time_limit == 0 {
                    S::ONE
                } else {
                    let left = self.time_limit.saturating_sub(elapsed);
                    S::from_i32(left.min(1 << 30) as i32)
                        / S::from_i32(self.time_limit.min(1 << 30) as i32)
                }
            }
            InputId::Aggression => self.profile.aggression,
            InputId::CasualtyFraction => {
                if self.snap.own_initial == 0 {
                    S::ZERO
                } else {
                    S::ONE
                        - S::from_i32(self.snap.own_alive.min(1 << 30) as i32)
                            / S::from_i32(self.snap.own_initial.min(1 << 30) as i32)
                }
            }
            InputId::EnemyVisible => {
                if self.snap.enemies.is_empty() {
                    S::ZERO
                } else {
                    S::ONE
                }
            }
            _ => S::ZERO,
        }
    }
}

fn stance_of(kind: &ActionKind) -> Option<Stance> {
    match kind {
        ActionKind::Attack => Some(Stance::Attack),
        ActionKind::Defend => Some(Stance::Defend),
        ActionKind::Hold => Some(Stance::Hold),
        ActionKind::Retreat => Some(Stance::Retreat),
        _ => None,
    }
}

/// SIM-AI-010 with plan decision 10: the highest-scoring stance, unless
/// the current stance's fresh score is within `stance_margin` of it
/// (`retreat` always takes over).
pub fn choose_stance(
    set: &AiActionSet,
    ctx: &ArmyContext<'_>,
    current: Option<(Stance, S)>,
    margin: S,
    rng: &mut RngStream,
) -> (Stance, S) {
    let winner = select(
        set,
        Channel::Stance,
        &mut |_, a| stance_of(&a.kind).is_some(),
        ctx,
        Some(rng),
    );
    let Some(choice) = winner else {
        return current.unwrap_or((Stance::Hold, S::ZERO));
    };
    let stance = stance_of(&choice.action.kind).expect("stance action");
    let Some((cur, _)) = current else {
        return (stance, choice.score);
    };
    if stance == cur || stance == Stance::Retreat {
        return (stance, choice.score);
    }
    // The current stance's score now (0 when the set no longer lists it).
    let cur_score = set
        .channel_actions(Channel::Stance)
        .find(|(_, a)| stance_of(&a.kind) == Some(cur))
        .map_or(S::ZERO, |(_, a)| score(a, ctx, None));
    if choice.score >= cur_score + margin {
        (stance, choice.score)
    } else {
        (cur, cur_score)
    }
}

/// The battle line the plan lays its line regiments on (plan I14).
fn line_template(gap: S) -> GroupFormationTemplate {
    GroupFormationTemplate {
        id: ContentId::new("il:ai_line").expect("valid id"),
        name_key: String::new(),
        kind: GroupKind::BattleLine,
        gap,
        skirmishers_forward: false,
        cavalry_flanks: false,
        lines: 1,
        deprecated: None,
    }
}

fn info(r: &RegRow, regs: &Registries) -> RegimentInfo {
    RegimentInfo {
        id: r.id,
        pos: r.anchor,
        category: r.category,
        count: r.count,
        template: r.template,
        radius: regs.units.get(r.unit).soldier_radius,
    }
}

/// SIM-AI-012 (plan I15): the highest heightmap sample within `radius` of
/// `centre`, in raster order (ties keep the first), or `centre`.
pub fn highest_ground(map: &LoadedMap, centre: V2, radius: S) -> V2 {
    let cell = map.height_cell;
    if cell <= S::ZERO || map.height_cols == 0 || map.height_rows == 0 {
        return centre;
    }
    let r_cells = (radius / cell).floor_i32().max(0);
    let cx = (centre.x / cell).floor_i32();
    let cy = (centre.y / cell).floor_i32();
    let mut best: Option<(S, V2)> = None;
    for j in (cy - r_cells)..=(cy + r_cells) {
        if j < 0 || j >= map.height_rows as i32 {
            continue;
        }
        for i in (cx - r_cells)..=(cx + r_cells) {
            if i < 0 || i >= map.height_cols as i32 {
                continue;
            }
            let p = V2::new(S::from_i32(i) * cell, S::from_i32(j) * cell);
            if p.distance(centre) > radius {
                continue;
            }
            let h = map.heights[j as usize * map.height_cols as usize + i as usize];
            if best.is_none_or(|(bh, _)| h > bh) {
                best = Some((h, p));
            }
        }
    }
    best.map_or(centre, |(_, p)| p)
}

/// The nearest point to `wanted` on the segment from `from` that the nav
/// grid can stand on, stepping back 8 m at a time (a flank slot in a river
/// or on rock would otherwise draw a `PathNotFound` every period).
pub fn passable_toward(nav: &crate::nav::NavGrid, from: V2, wanted: V2) -> V2 {
    if nav.is_passable_at(wanted) {
        return wanted;
    }
    let d = wanted - from;
    let len = d.length();
    let step = S::from_i32(8);
    let mut back = step;
    while back < len {
        let p = wanted - d * (back / len);
        if nav.is_passable_at(p) {
            return p;
        }
        back = back + step;
    }
    from
}

/// Mean forward lag of the line regiments behind their slots in `prev`.
fn line_lag(line: &[&RegRow], prev: &ArmyPlan, forward: V2) -> S {
    let mut sum = S::ZERO;
    let mut n = 0;
    for r in line {
        if let Some(Role::Line { slot }) = prev.role_of(r.id) {
            sum = sum + (slot - r.anchor).dot(forward).max(S::ZERO);
            n += 1;
        }
    }
    if n > 0 { sum / S::from_i32(n) } else { S::ZERO }
}

/// Whether the enemy `e` sits beyond a line end and within reach of it.
fn threatens_flank(e: &RegRow, anchor: V2, right: V2, half_width: S, reach: S) -> bool {
    let lateral = (e.anchor - anchor).dot(right);
    lateral.abs() > half_width && lateral.abs() - half_width <= reach
}

/// Builds this period's plan and queues the army-level commands (the
/// `Withdraw`s of a retreat).
#[allow(clippy::too_many_arguments)]
pub fn build_plan(
    regs: &Registries,
    map: &LoadedMap,
    nav: &crate::nav::NavGrid,
    snap: &SideSnapshot,
    profile: &AiProfile,
    stance: Stance,
    stance_score: S,
    prev: Option<&ArmyPlan>,
    tick: Tick,
    out: &mut Decisions,
) -> ArmyPlan {
    let same_stance = prev.is_some_and(|p| p.stance == stance);

    // ---- partition (plan I13, I20) --------------------------------------
    let standing: Vec<&RegRow> = snap.standing().collect();
    let is_bodyguard = |r: &RegRow| snap.is_bodyguard(r.id);
    let cavalry: Vec<&RegRow> = standing
        .iter()
        .copied()
        .filter(|r| !is_bodyguard(r) && r.category == UnitCategory::Cavalry)
        .collect();
    let ranged: Vec<&RegRow> = standing
        .iter()
        .copied()
        .filter(|r| {
            !is_bodyguard(r)
                && matches!(r.category, UnitCategory::Ranged | UnitCategory::Skirmisher)
        })
        .collect();
    let infantry: Vec<&RegRow> = standing
        .iter()
        .copied()
        .filter(|r| {
            !is_bodyguard(r)
                && !matches!(
                    r.category,
                    UnitCategory::Cavalry | UnitCategory::Ranged | UnitCategory::Skirmisher
                )
        })
        .collect();
    // Reserves: the smallest infantry by cost up to `reserve_fraction` of
    // the infantry cost (ascending weight, ties by id); a committed reserve
    // stays committed while its target is visible and alive.
    let infantry_weight = infantry.iter().fold(S::ZERO, |acc, r| acc + r.weight);
    let mut by_weight: Vec<&RegRow> = infantry.clone();
    by_weight.sort_by(|a, b| {
        a.weight
            .partial_cmp(&b.weight)
            .expect("finite weights")
            .then(a.id.cmp(&b.id))
    });
    let mut reserves: Vec<RegimentId> = Vec::new();
    let mut reserved = S::ZERO;
    for r in &by_weight {
        if reserved + r.weight > profile.reserve_fraction * infantry_weight {
            break;
        }
        reserved = reserved + r.weight;
        reserves.push(r.id);
    }
    let committed_before = |id: RegimentId| -> Option<RegimentId> {
        match prev.and_then(|p| p.role_of(id)) {
            Some(Role::Committed { target }) if snap.enemies.iter().any(|e| e.id == target) => {
                Some(target)
            }
            _ => None,
        }
    };
    // The line: the infantry that is not held back, plus the missile units
    // on defend or once their ammo is spent; with no infantry left every
    // standing foot regiment (the bodyguard included) is the line.
    let mut line: Vec<&RegRow> = infantry
        .iter()
        .copied()
        .filter(|r| !reserves.contains(&r.id))
        .chain(
            ranged
                .iter()
                .copied()
                .filter(|r| stance == Stance::Defend || r.ammo <= S::ZERO),
        )
        .collect();
    let bodyguard_in_line = line.is_empty();
    if bodyguard_in_line {
        line = standing
            .iter()
            .copied()
            .filter(|r| r.category != UnitCategory::Cavalry)
            .collect();
    }
    let ranged: Vec<&RegRow> = ranged
        .into_iter()
        .filter(|r| !line.iter().any(|l| l.id == r.id))
        .collect();
    line.sort_by_key(|r| r.id);

    // The line's own centroid: the line regiments, else whoever stands.
    let ctx_centroid = {
        let mut sum = V2::ZERO;
        let mut n = 0;
        let rows: Vec<&RegRow> = if line.is_empty() {
            standing.clone()
        } else {
            line.clone()
        };
        for r in &rows {
            sum += r.anchor;
            n += 1;
        }
        if n > 0 {
            sum * (S::ONE / S::from_i32(n))
        } else {
            snap.enemy_zone
        }
    };
    let target = snap.enemy_centroid();
    let to_target = target - ctx_centroid;
    let line_facing = if to_target.length_sq() > S::ZERO {
        Angle::from_direction(to_target.normalized_or_zero())
    } else {
        prev.map_or_else(
            || Angle::from_direction((snap.enemy_zone - ctx_centroid).normalized_or_zero()),
            |p| p.line_facing,
        )
    };
    let forward = line_facing.direction();
    let right = V2::new(forward.y, -forward.x);
    // The visible enemy's lateral half extent about its centroid, and the
    // point the attack aims at: the enemy's nearer wing rather than its
    // centre, so the whole line meets a part of theirs (T2-082 tuning; a
    // line that hits a prepared line head-on breaks on SIM-MOR-020).
    let enemy_half = snap.enemies.iter().fold(S::ZERO, |acc, e| {
        acc.max((e.anchor - target).dot(right).abs() + e.half_width)
    });
    let strike = if snap.enemies.is_empty() {
        target
    } else {
        let own_lateral = (ctx_centroid - target).dot(right);
        let sign = if own_lateral < S::ZERO {
            -S::ONE
        } else {
            S::ONE
        };
        target + right * (sign * enemy_half * S::from_f32_data(0.6))
    };
    let mut charging = prev.is_some_and(|p| p.charging && same_stance);
    if stance == Stance::Attack
        && !charging
        && line.iter().any(|r| {
            snap.enemies
                .iter()
                .any(|e| e.anchor.distance(r.anchor) <= profile.charge_trigger_dist)
        })
    {
        charging = true;
    }
    // ---- the line -------------------------------------------------------
    let line_anchor = match stance {
        Stance::Attack => match prev.filter(|_| same_stance) {
            // Form up where the line stands (decision 22), then step.
            None => ctx_centroid,
            // Decision 22 as tuned in T2-082: the line keeps stepping while
            // its regiments lag it by less than twice `line_tolerance`
            // (`formed` is the stricter reading kept for the overlay). It
            // forms at `approach_distance` and holds there while its
            // missile units stand off and shoot (SIM-AI-011), then closes.
            Some(p) => {
                let d = p.line_anchor.distance(strike);
                let lag = line_lag(&line, p, forward);
                let skirmishing = !snap.enemies.is_empty()
                    && ranged.iter().any(|r| {
                        r.ammo > S::ZERO
                            && matches!(p.role_of(r.id), Some(Role::Skirmish { slot })
                                if (slot - p.line_anchor).dot(p.line_facing.direction()) > S::ZERO)
                    });
                // Rest at the approach distance until the line is fresh
                // (SIM-FAT-004: a tired line fights and holds worse) before
                // closing; the missile units' stand-off covers the wait.
                let fresh = regs.rules.fatigue.thresholds[0];
                let tired = {
                    let mut sum = S::ZERO;
                    let mut n = 0;
                    for r in &line {
                        sum = sum + r.fatigue;
                        n += 1;
                    }
                    n > 0 && sum / S::from_i32(n) > fresh
                };
                let goal = if !charging && (skirmishing || tired) {
                    profile.approach_distance
                } else {
                    S::from_i32(10)
                };
                if d > goal && lag <= profile.line_tolerance * S::from_i32(2) {
                    let step = profile.advance_step.min(d - goal);
                    let dir = (strike - p.line_anchor).normalized_or_zero();
                    p.line_anchor + dir * step
                } else {
                    p.line_anchor
                }
            }
        },
        Stance::Defend => match prev.filter(|_| same_stance) {
            Some(p) => p.line_anchor,
            None => highest_ground(map, ctx_centroid, profile.defend_search_radius),
        },
        Stance::Hold => prev
            .filter(|_| same_stance)
            .map_or(ctx_centroid, |p| p.line_anchor),
        Stance::Retreat => ctx_centroid,
    };
    let infos: Vec<RegimentInfo> = line.iter().map(|r| info(r, regs)).collect();
    // The line deepens to the visible enemy's frontage (SIM-FORM-042 rank
    // selection): three regiments at their default ranks would stand far
    // wider than an enemy block and only the centre would meet it. With
    // nothing visible the templates' own ranks stand.
    let width = if enemy_half > S::ZERO {
        enemy_half * S::from_i32(2)
    } else {
        S::from_i32(100_000)
    };
    let placements = if stance == Stance::Retreat || infos.is_empty() {
        Vec::new()
    } else {
        arrange_group(
            &line_template(regs.rules.formation.group_gap),
            &infos,
            line_anchor,
            line_facing,
            width,
            &regs.rules.formation,
            regs,
        )
    };
    let line_width = arranged_width(&placements, &infos, regs, right);
    let half_width = line_width * S::HALF;
    // Formed: the line regiments lag their slots by less than
    // `line_tolerance` on average along the line's forward axis (lateral
    // dressing happens on the march).
    let (mut err, mut n_err) = (S::ZERO, 0);
    let mut assignments: Vec<Assignment> = Vec::new();
    for p in &placements {
        // A slot in a river or on rock is pulled back along the regiment's
        // approach to passable ground: the line anchor keeps stepping
        // across, and once the slots emerge on the far bank the regiments
        // path over the crossing (T2-082).
        let slot = line
            .iter()
            .find(|r| r.id == p.id)
            .map_or(p.anchor, |r| passable_toward(nav, r.anchor, p.anchor));
        if let Some(r) = line.iter().find(|r| r.id == p.id) {
            err = err + (slot - r.anchor).dot(forward).abs();
            n_err += 1;
        }
        assignments.push(Assignment {
            regiment: p.id,
            role: Role::Line { slot },
        });
    }
    let formed = n_err > 0 && err / S::from_i32(n_err) < profile.line_tolerance;
    let gap = regs.rules.formation.group_gap;
    let behind = line_anchor - forward * profile.reserve_offset;

    // ---- skirmishers (I16) -------------------------------------------------
    if stance != Stance::Defend {
        for r in &ranged {
            let lateral = (r.anchor - line_anchor).dot(right);
            let nearest = snap.nearest_enemy(r.anchor);
            // A missile unit with ammo stands off at `skirmish_range_frac`
            // of its reach from the nearest enemy and shoots, unless an
            // enemy missile unit reaches it there (standing in the arrow
            // storm loses the regiment and shakes its friends) and the
            // charge is not on; spent or under fire it falls behind the
            // line (bows half a `reserve_offset` back, shooting over it).
            let stand_off = match (nearest, r.range) {
                (Some(e), Some(range)) => {
                    let toward = (r.anchor - e.anchor).normalized_or_zero();
                    Some(e.anchor + toward * (profile.skirmish_range_frac * range))
                }
                _ => None,
            };
            let under_fire = |p: V2| {
                snap.enemies.iter().any(|e| {
                    e.range
                        .is_some_and(|er| e.anchor.distance(p) <= er + S::from_i32(10))
                })
            };
            let bows = r.category == UnitCategory::Ranged;
            let behind_offset = if bows {
                profile.reserve_offset * S::HALF
            } else {
                profile.reserve_offset
            };
            let slot = match stand_off {
                _ if stance == Stance::Retreat => behind,
                Some(p)
                    if r.ammo > S::ZERO
                        && (!under_fire(p) || (charging && !bows))
                        && (p - line_anchor).dot(forward) > -behind_offset =>
                {
                    p
                }
                _ => line_anchor + right * lateral - forward * behind_offset,
            };
            let slot = passable_toward(nav, r.anchor, slot);
            assignments.push(Assignment {
                regiment: r.id,
                role: Role::Skirmish { slot },
            });
        }
    }

    // ---- reserves (I20) ----------------------------------------------------
    let n_res = reserves.len() as i32;
    let mut weakest: Option<(S, &RegRow)> = None;
    for r in &line {
        if r.morale < profile.commit_morale && weakest.is_none_or(|(m, _)| r.morale < m) {
            weakest = Some((r.morale, r));
        }
    }
    let mut commit_targets: Vec<RegimentId> = Vec::new();
    if let Some((_, w)) = weakest
        && let Some(e) = snap.nearest_enemy(w.anchor)
    {
        commit_targets.push(e.id);
    }
    for e in &snap.enemies {
        if threatens_flank(e, line_anchor, right, half_width, profile.flank_offset)
            && !commit_targets.contains(&e.id)
        {
            commit_targets.push(e.id);
        }
    }
    let mut next_target = commit_targets.into_iter();
    for (k, id) in reserves.iter().enumerate() {
        let role = if let Some(t) = committed_before(*id) {
            Role::Committed { target: t }
        } else if stance != Stance::Retreat
            && let Some(t) = next_target.next()
        {
            Role::Committed { target: t }
        } else {
            let r = infantry
                .iter()
                .find(|r| r.id == *id)
                .expect("reserve is infantry");
            let pitch = r.half_width * S::from_i32(2) + gap;
            let offset = (S::from_i32(k as i32) - S::from_i32(n_res - 1) * S::HALF) * pitch;
            Role::Reserve {
                slot: passable_toward(nav, r.anchor, behind + right * offset),
            }
        };
        assignments.push(Assignment {
            regiment: *id,
            role,
        });
    }

    // ---- cavalry (I17, I18, I19) -------------------------------------------
    let rearmost = snap
        .enemies
        .iter()
        .max_by(|a, b| {
            (a.anchor - target)
                .dot(forward)
                .partial_cmp(&(b.anchor - target).dot(forward))
                .expect("finite")
                .then(b.id.cmp(&a.id))
        })
        .map(|e| e.id);
    let infantry_centroid = {
        let mut sum = V2::ZERO;
        let mut n = 0;
        for r in standing
            .iter()
            .filter(|r| r.category != UnitCategory::Cavalry)
        {
            sum += r.anchor;
            n += 1;
        }
        if n > 0 {
            sum * (S::ONE / S::from_i32(n))
        } else {
            ctx_centroid
        }
    };
    // (An early flank charge on the enemy's missile units was tried in
    // T2-082 and lost the cavalry to the skirmish screen every time; the
    // groups wait for the line's charge.)
    for r in &cavalry {
        let lateral = (r.anchor - line_anchor).dot(right);
        let sign = if lateral < S::ZERO { -S::ONE } else { S::ONE };
        let role = match stance {
            Stance::Attack => Role::Flank {
                slot: passable_toward(
                    nav,
                    r.anchor,
                    target + right * (sign * (enemy_half + profile.flank_offset)),
                ),
                charge: if charging { rearmost } else { None },
            },
            Stance::Defend | Stance::Hold => {
                let end = line_anchor + right * (sign * (half_width + gap + r.half_width));
                let threat = snap
                    .enemies
                    .iter()
                    .filter(|e| e.anchor.distance(end) <= profile.counter_charge_dist)
                    .min_by(|a, b| {
                        a.anchor
                            .distance_sq(end)
                            .partial_cmp(&b.anchor.distance_sq(end))
                            .expect("finite")
                            .then(a.id.cmp(&b.id))
                    });
                match committed_before(r.id).or(threat.map(|e| e.id)) {
                    Some(t) => Role::Committed { target: t },
                    None => Role::Counter {
                        slot: passable_toward(
                            nav,
                            r.anchor,
                            end - forward * profile.reserve_offset,
                        ),
                    },
                }
            }
            Stance::Retreat => {
                // The screen holds where the retreat began (SIM-AI-013).
                let slot = match prev.and_then(|p| p.role_of(r.id)) {
                    Some(Role::Screen { slot }) if same_stance => slot,
                    _ => infantry_centroid + forward * profile.screen_offset,
                };
                let nearest_foot = standing
                    .iter()
                    .filter(|f| f.category != UnitCategory::Cavalry)
                    .map(|f| f.anchor.distance(r.anchor))
                    .fold(S::from_i32(100_000), |acc, d| acc.min(d));
                if nearest_foot > profile.screen_gap && r.order != OrderKind::Withdraw {
                    out.push(CommandKind::Withdraw {
                        regiments: vec![r.id],
                    });
                }
                Role::Screen { slot }
            }
        };
        assignments.push(Assignment {
            regiment: r.id,
            role,
        });
    }

    // ---- the bodyguard and the retreat ---------------------------------
    if let Some(b) = snap
        .bodyguard
        .filter(|b| snap.is_bodyguard(*b) && !bodyguard_in_line)
    {
        assignments.push(Assignment {
            regiment: b,
            role: Role::Bodyguard,
        });
    }
    if stance == Stance::Retreat {
        for r in standing
            .iter()
            .filter(|r| r.category != UnitCategory::Cavalry && r.order != OrderKind::Withdraw)
        {
            out.push(CommandKind::Withdraw {
                regiments: vec![r.id],
            });
        }
    }

    assignments.sort_by_key(|a| a.regiment);
    assignments.dedup_by_key(|a| a.regiment);
    ArmyPlan {
        stance,
        stance_score,
        decided_at: tick,
        target,
        line_anchor,
        line_facing,
        line_width,
        formed,
        assignments,
        charging,
    }
}

/// SIM-AI-010 (T2-082): the side's army decision this period.
pub fn decide(
    world: &bevy_ecs::world::World,
    snap: &SideSnapshot,
    side: u8,
    profile: &AiProfile,
    tick: Tick,
    rng: &mut RngStream,
    out: &mut Decisions,
) -> Option<ArmyPlan> {
    use crate::resources::{BattleFlow, MapRes, NavGridRes, Regs, SetupRes};
    let regs = world.resource::<Regs>().0.clone();
    let set = regs.ai_action_sets.get(profile.army_set?);
    let map = world.resource::<MapRes>().0.clone();
    let nav = &world.resource::<NavGridRes>().0;
    let flow = *world.resource::<BattleFlow>();
    let time_limit = world
        .resource::<SetupRes>()
        .0
        .as_ref()
        .map_or(regs.rules.battle_flow.time_limit_ticks, |s| {
            s.time_limit_ticks
        });
    let prev = world.resource::<crate::ai::AiState>().plan(side).cloned();
    let ctx = ArmyContext {
        snap,
        profile,
        map: &map,
        height_ref: regs.rules.combat.height_ref,
        tick,
        battle_start: flow.battle_start,
        time_limit,
    };
    let (stance, stance_score) = choose_stance(
        set,
        &ctx,
        prev.as_ref().map(|p| (p.stance, p.stance_score)),
        profile.stance_margin,
        rng,
    );
    Some(build_plan(
        &regs,
        &map,
        nav,
        snap,
        profile,
        stance,
        stance_score,
        prev.as_ref(),
        tick,
        out,
    ))
}
