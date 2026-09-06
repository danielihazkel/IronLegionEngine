//! What one AI side knows this tick (SIM-AI-003; T2-081): its own
//! regiments and the enemy regiments its visibility mask shows, read once
//! per tick into plain rows, and the regiment-level inputs of SIM-AI-021
//! computed over those rows (`RegimentContext: InputProvider`).
//!
//! Everything here comes from state or from derived data a restore
//! rebuilds, never from the per-tick targeting gates (plan I8), so a
//! restored battle decides exactly as the uninterrupted one.

use bevy_ecs::prelude::*;
use il_ai::InputProvider;
use il_core::{Angle, RegimentId, S, Scalar, V2};
use il_data::{Ability, Handle, InputId, Layout, Registries, UnitCategory, UnitType};

use crate::combat::formulas::{Arc, attack_arc};
use crate::command::FireMode;
use crate::components::{
    Anchor, Combat, Cooldowns, Energy, Fire, FormationState, Morale, MoraleState, Order, OrderKind,
    Path, RangedState, Regiment, RegimentFatigue,
};
use crate::movement::regiment::{anchor_moves, formation_width};
use crate::resources::{Ids, MapRes, Regs, Sides};
use crate::visibility::Visibility;

/// A distance standing for "nobody there": far beyond any `scale`, so a
/// normalised distance input reads 1.
pub fn far() -> S {
    S::from_i32(100_000)
}

/// One regiment as the AI reads it.
#[derive(Clone, Debug)]
pub struct RegRow {
    pub id: RegimentId,
    pub entity: Entity,
    pub side: u8,
    pub anchor: V2,
    pub facing: Angle<S>,
    pub count: u16,
    pub unit: Handle<UnitType>,
    pub category: UnitCategory,
    pub order: OrderKind,
    pub order_target: V2,
    pub order_target_regiment: Option<RegimentId>,
    pub order_speed: crate::command::SpeedMode,
    pub engaged: bool,
    pub moving: bool,
    pub morale: S,
    pub morale_state: MoraleState,
    pub fatigue: S,
    /// `unit.ranged.range` when the unit shoots.
    pub range: Option<S>,
    pub direct_fire: bool,
    pub fire_mode: Option<FireMode>,
    /// Half the current formation width, metres.
    pub half_width: S,
    /// `count × unit.cost` (plan I12).
    pub weight: S,
    pub layout: Layout,
    pub template: Handle<il_data::FormationTemplate>,
    pub morphing: bool,
    /// Soldiers at spawn (`Morale.initial`).
    pub initial: u16,
    /// Ability slots with their cooldowns (own rows only).
    pub slots: Vec<(Handle<Ability>, u16)>,
    pub energy: S,
    /// Best remaining ammo over the soldiers, as a fraction of the unit's.
    pub ammo: S,
}

/// One side's view of the field this tick.
#[derive(Clone, Debug)]
pub struct SideSnapshot {
    pub side: u8,
    /// Own regiments with soldiers, ascending id.
    pub own: Vec<RegRow>,
    /// Visible enemy regiments with soldiers, ascending id.
    pub enemies: Vec<RegRow>,
    /// The mean of the other sides' deployment zone centres (plan decision 9).
    pub enemy_zone: V2,
    pub bodyguard: Option<RegimentId>,
    pub general_alive: bool,
    /// Soldiers the side started with, every regiment (dead ones too).
    pub own_initial: u32,
    /// Soldiers the side has on the field now.
    pub own_alive: u32,
}

fn row(
    world: &World,
    regs: &Registries,
    id: RegimentId,
    entity: Entity,
    own: bool,
) -> Option<RegRow> {
    let r = world.get::<Regiment>(entity)?;
    let a = world.get::<Anchor>(entity)?;
    let o = world.get::<Order>(entity)?;
    let c = world.get::<Combat>(entity)?;
    let m = world.get::<Morale>(entity)?;
    let f = world.get::<FormationState>(entity)?;
    let p = world.get::<Path>(entity)?;
    let unit = regs.units.get(r.unit);
    let template = regs.formations.get(f.template);
    let half_width = formation_width(template, f.files.max(1), unit.soldier_radius) * S::HALF;
    let fire = world.get::<Fire>(entity);
    let (slots, ammo) = if own {
        let cooldowns = world.get::<Cooldowns>(entity);
        let slots = crate::abilities::slots(world, entity)
            .into_iter()
            .enumerate()
            .map(|(i, h)| (h, cooldowns.and_then(|c| c.0.get(i).copied()).unwrap_or(0)))
            .collect();
        let ammo = match &unit.ranged {
            Some(ranged) if ranged.ammo > 0 => {
                let ids = world.resource::<Ids>();
                let best = r
                    .soldiers
                    .iter()
                    .filter_map(|sid| ids.soldier_entity(*sid))
                    .filter_map(|e| world.get::<RangedState>(e))
                    .map(|s| s.ammo)
                    .max()
                    .unwrap_or(0);
                S::from_i32(i32::from(best)) / S::from_i32(i32::from(ranged.ammo))
            }
            _ => S::ZERO,
        };
        (slots, ammo)
    } else {
        (Vec::new(), S::ZERO)
    };
    Some(RegRow {
        id,
        entity,
        side: r.side,
        anchor: a.pos,
        facing: a.facing,
        count: r.soldiers.len() as u16,
        unit: r.unit,
        category: unit.category,
        order: o.kind,
        order_target: o.target,
        order_target_regiment: o.target_regiment,
        order_speed: o.speed,
        engaged: c.engaged,
        moving: anchor_moves(o, p, c),
        morale: m.m,
        morale_state: m.state,
        fatigue: world
            .get::<RegimentFatigue>(entity)
            .map_or(S::ZERO, |f| f.mean),
        range: unit.ranged.as_ref().map(|rg| rg.range),
        direct_fire: unit
            .ranged
            .as_ref()
            .is_some_and(|rg| rg.arc == il_data::ProjectileArc::Direct),
        fire_mode: fire.map(|f| f.mode),
        half_width,
        weight: S::from_i32(r.soldiers.len() as i32) * S::from_i32(unit.cost.min(1 << 20) as i32),
        layout: template.layout,
        template: f.template,
        morphing: f.prior_template.is_some(),
        initial: m.initial,
        slots,
        energy: world.get::<Energy>(entity).map_or(S::ZERO, |e| e.e),
        ammo,
    })
}

impl SideSnapshot {
    /// Reads the side's regiments and the visible enemies (SIM-VIS-004).
    pub fn build(world: &World, side: u8) -> Self {
        let regs = world.resource::<Regs>().0.clone();
        let map = world.resource::<MapRes>().0.clone();
        let sides = world.resource::<Sides>().0.clone();
        let vis = world.resource::<Visibility>();
        let entities: Vec<(RegimentId, Entity)> = world.resource::<Ids>().regiment_entities.clone();
        let mut own = Vec::new();
        let mut enemies = Vec::new();
        let mut own_initial = 0u32;
        let mut own_alive = 0u32;
        for (i, (id, entity)) in entities.iter().enumerate() {
            let Some(r) = world.get::<Regiment>(*entity) else {
                continue;
            };
            if r.side == side {
                own_initial += world
                    .get::<Morale>(*entity)
                    .map_or(0, |m| u32::from(m.initial));
                own_alive += r.soldiers.len() as u32;
            }
            if r.soldiers.is_empty() {
                continue;
            }
            if r.side == side {
                if let Some(row) = row(world, &regs, *id, *entity, true) {
                    own.push(row);
                }
            } else if vis.sees(side, i)
                && let Some(row) = row(world, &regs, *id, *entity, false)
            {
                enemies.push(row);
            }
        }
        let others: Vec<V2> = sides
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != usize::from(side))
            .map(|(_, s)| crate::flow_battle::zone_centre(&map, s.deployment_zone))
            .collect();
        let enemy_zone = if others.is_empty() {
            V2::new(map.width * S::HALF, map.height * S::HALF)
        } else {
            others.iter().fold(V2::ZERO, |acc, p| acc + *p)
                * (S::ONE / S::from_i32(others.len() as i32))
        };
        let (bodyguard, general_alive) = sides
            .get(usize::from(side))
            .map_or((None, false), |s| (s.general_regiment, !s.general_dead));
        Self {
            side,
            own,
            enemies,
            enemy_zone,
            bodyguard,
            general_alive,
            own_initial,
            own_alive,
        }
    }

    /// SIM-AI-022 applies while the side has another standing regiment to
    /// guard; the last regiment fights like any other (T2-082).
    pub fn is_bodyguard(&self, id: RegimentId) -> bool {
        self.bodyguard == Some(id) && self.general_alive && self.standing().any(|r| r.id != id)
    }

    /// Own regiments that stand (not Routing or Shattered).
    pub fn standing(&self) -> impl Iterator<Item = &RegRow> {
        self.own.iter().filter(|r| {
            !matches!(
                r.morale_state,
                MoraleState::Routing | MoraleState::Shattered
            )
        })
    }

    /// The visible enemy nearest to `p` (ties by ascending id).
    pub fn nearest_enemy(&self, p: V2) -> Option<&RegRow> {
        let mut best: Option<(S, &RegRow)> = None;
        for e in &self.enemies {
            let d = e.anchor.distance_sq(p);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, e));
            }
        }
        best.map(|(_, e)| e)
    }

    /// The visible enemy nearest to `p` among those `keep` admits.
    pub fn nearest_enemy_where(&self, p: V2, keep: impl Fn(&RegRow) -> bool) -> Option<&RegRow> {
        let mut best: Option<(S, &RegRow)> = None;
        for e in self.enemies.iter().filter(|e| keep(e)) {
            let d = e.anchor.distance_sq(p);
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, e));
            }
        }
        best.map(|(_, e)| e)
    }

    /// The centroid of the visible enemies weighted by soldier count, or
    /// the enemy zones' mean when nothing is visible (plan decision 9).
    pub fn enemy_centroid(&self) -> V2 {
        let mut sum = V2::ZERO;
        let mut n = S::ZERO;
        for e in &self.enemies {
            let w = S::from_i32(i32::from(e.count));
            sum += e.anchor * w;
            n = n + w;
        }
        if n > S::ZERO {
            sum * (S::ONE / n)
        } else {
            self.enemy_zone
        }
    }
}

/// Distance from `p` to the segment `a → b`.
pub fn point_segment_distance(p: V2, a: V2, b: V2) -> S {
    let ab = b - a;
    let len_sq = ab.length_sq();
    if len_sq <= S::ZERO {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(S::ZERO, S::ONE);
    p.distance(a + ab * t)
}

/// The regiment-level inputs of one own regiment (SIM-AI-021).
pub struct RegimentContext<'a> {
    pub snap: &'a SideSnapshot,
    pub me: &'a RegRow,
    /// The plan slot the regiment holds, if any (T2-082).
    pub slot: Option<V2>,
    /// The side's plan is charging (SIM-AI-011).
    pub charging: bool,
    pub nearest: Option<&'a RegRow>,
    pub friendly_block_dist: S,
    pub outnumber_radius: S,
    pub frontal_arc_deg: S,
}

impl<'a> RegimentContext<'a> {
    pub fn new(
        snap: &'a SideSnapshot,
        me: &'a RegRow,
        slot: Option<V2>,
        charging: bool,
        regs: &Registries,
    ) -> Self {
        Self {
            snap,
            me,
            slot,
            charging,
            nearest: snap.nearest_enemy(me.anchor),
            friendly_block_dist: regs.rules.combat.friendly_block_dist,
            outnumber_radius: regs.rules.morale.outnumber_radius,
            frontal_arc_deg: regs.units.get(me.unit).frontal_arc_deg,
        }
    }

    fn bool(b: bool) -> S {
        if b { S::ONE } else { S::ZERO }
    }

    /// `own / (own + enemy)` against the nearest enemy; 1 with none.
    pub fn strength_ratio(&self) -> S {
        match self.nearest {
            Some(e) if self.me.weight + e.weight > S::ZERO => {
                self.me.weight / (self.me.weight + e.weight)
            }
            _ => S::ONE,
        }
    }

    /// Whether the nearest enemy sees us outside its frontal arc.
    fn enemy_flank_open(&self) -> bool {
        self.nearest.is_some_and(|e| {
            let arc = attack_arc(
                e.facing,
                self.me.anchor - e.anchor,
                self.snap_frontal_arc(e),
            );
            arc != Arc::Front
        })
    }

    fn snap_frontal_arc(&self, _e: &RegRow) -> S {
        // Enemy unit stats are content the AI may read (a player sees the
        // unit type too); the frontal arc is the same for every flagship
        // unit, so the own value stands in until unit rows carry it.
        self.frontal_arc_deg
    }

    fn engaged_frontal(&self) -> bool {
        self.me.engaged
            && self.nearest.is_some_and(|e| {
                attack_arc(
                    self.me.facing,
                    e.anchor - self.me.anchor,
                    self.frontal_arc_deg,
                ) == Arc::Front
            })
    }

    /// Distance to the nearest other own regiment.
    fn nearest_friend_distance(&self) -> S {
        self.snap
            .own
            .iter()
            .filter(|r| r.id != self.me.id)
            .map(|r| r.anchor.distance(self.me.anchor))
            .fold(far(), |acc, d| acc.min(d))
    }

    fn nearest_enemy_distance_where(&self, keep: impl Fn(&RegRow) -> bool) -> S {
        self.snap
            .nearest_enemy_where(self.me.anchor, keep)
            .map_or(far(), |e| e.anchor.distance(self.me.anchor))
    }

    fn enemy_ranged_in_range(&self) -> bool {
        self.snap.enemies.iter().any(|e| {
            e.range
                .is_some_and(|r| e.anchor.distance(self.me.anchor) <= r + self.me.half_width)
        })
    }

    fn enemy_in_own_range(&self) -> bool {
        self.me.range.is_some_and(|r| {
            self.snap
                .enemies
                .iter()
                .any(|e| e.anchor.distance(self.me.anchor) <= r)
        })
    }

    /// SIM-PROJ-009 as the AI predicts it: a direct-fire unit with an own
    /// regiment standing within `friendly_block_dist` plus its half width
    /// of the line to the nearest enemy.
    fn friendly_in_line_of_fire(&self) -> bool {
        if !self.me.direct_fire {
            return false;
        }
        let Some(e) = self.nearest else {
            return false;
        };
        self.snap.own.iter().any(|f| {
            f.id != self.me.id
                && point_segment_distance(f.anchor, self.me.anchor, e.anchor)
                    <= self.friendly_block_dist + f.half_width
        })
    }

    fn outnumbered(&self) -> S {
        let near: i32 = self
            .snap
            .enemies
            .iter()
            .filter(|e| e.anchor.distance(self.me.anchor) <= self.outnumber_radius)
            .map(|e| i32::from(e.count))
            .sum();
        let own = i32::from(self.me.count);
        if near + own > 0 {
            S::from_i32(near) / S::from_i32(near + own)
        } else {
            S::ZERO
        }
    }
}

impl InputProvider for RegimentContext<'_> {
    fn input(&self, id: InputId) -> S {
        match id {
            InputId::Constant => S::ONE,
            InputId::DistanceToNearestEnemy => self
                .nearest
                .map_or(far(), |e| e.anchor.distance(self.me.anchor)),
            InputId::StrengthRatio => self.strength_ratio(),
            InputId::EnemyShare => S::ONE - self.strength_ratio(),
            InputId::OwnMorale => self.me.morale / S::from_i32(100),
            InputId::OwnFatigue => self.me.fatigue,
            InputId::Engaged => Self::bool(self.me.engaged),
            InputId::EngagedFrontal => Self::bool(self.engaged_frontal()),
            InputId::IsFlankExposed => self.nearest_friend_distance(),
            InputId::EnemyFlankOpen => Self::bool(self.enemy_flank_open()),
            InputId::CavalryApproaching => self.nearest_enemy_distance_where(|e| {
                e.category == UnitCategory::Cavalry && e.order.moves()
            }),
            InputId::InfantryThreat => {
                self.nearest_enemy_distance_where(|e| e.category != UnitCategory::Cavalry)
            }
            InputId::EnemyRangedInRange => Self::bool(self.enemy_ranged_in_range()),
            InputId::EnemyInOwnRange => Self::bool(self.enemy_in_own_range()),
            InputId::FriendlyInLineOfFire => Self::bool(self.friendly_in_line_of_fire()),
            InputId::Outnumbered => self.outnumbered(),
            InputId::Charging => Self::bool(self.charging),
            InputId::SlotError => self.slot.map_or(S::ZERO, |s| s.distance(self.me.anchor)),
            // Missiles of a missile unit: infantry pila do not make a
            // regiment a skirmisher.
            InputId::Ammo => {
                if matches!(
                    self.me.category,
                    UnitCategory::Ranged | UnitCategory::Skirmisher
                ) {
                    self.me.ammo
                } else {
                    S::ZERO
                }
            }
            // Army inputs never reach a regiment set (checked at load).
            InputId::ArmyStrengthRatio
            | InputId::ArmyMoraleMean
            | InputId::ArmyFatigueMean
            | InputId::TerrainAdvantage
            | InputId::TimeRemaining
            | InputId::Aggression
            | InputId::CasualtyFraction
            | InputId::EnemyVisible => S::ZERO,
        }
    }
}
