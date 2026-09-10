//! Battle AI content (Simulation Spec §13, TDD §8.5, Modding SDK §4.8,
//! `ai-action-set.schema.json`, `ai-profile.schema.json`; T2-080).
//!
//! Two kinds: an [`AiActionSet`] (`content/ai/actions/`) lists the candidate
//! actions of one decision scope with their response curves, and an
//! [`AiProfile`] (`content/ai/profiles/`) is a personality plus the army-level
//! distances and cadences, naming one army set and one regiment set. The
//! vocabulary of inputs ([`InputId`]) and behaviours ([`ActionKind`]) is
//! closed here so a load diagnostic, not a runtime surprise, reports a typo;
//! `il_ai` scores and selects, `il_sim_battle::ai` computes the inputs.

use il_core::{S, StateHasher, impl_hashable_fieldless_enum};
use serde::{Deserialize, Serialize};

use crate::content_id::ContentId;
use crate::de::{d_one, d_zero, de_s};
use crate::formation::Layout;
use crate::handle::Handle;
use crate::registry::{ContentKind, Lookup, ResolveError};
use crate::schema::KindTag;
use crate::unit_type::UnitCategory;

/// Which decision maker an action set or input belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum InputScope {
    Regiment = 0,
    Army = 1,
}
impl_hashable_fieldless_enum!(InputScope);

/// The closed input vocabulary (SIM-AI-003, SIM-AI-010, SIM-AI-021). The
/// raw value each one yields is defined in `il_sim_battle::ai::inputs`; a
/// consideration divides it by its `scale` and clamps to `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum InputId {
    /// Always 1 (a base-only action).
    Constant = 0,
    // Regiment scope.
    DistanceToNearestEnemy = 1,
    StrengthRatio = 2,
    EnemyShare = 3,
    OwnMorale = 4,
    OwnFatigue = 5,
    Engaged = 6,
    EngagedFrontal = 7,
    IsFlankExposed = 8,
    EnemyFlankOpen = 9,
    CavalryApproaching = 10,
    InfantryThreat = 11,
    EnemyRangedInRange = 12,
    EnemyInOwnRange = 13,
    FriendlyInLineOfFire = 14,
    Outnumbered = 15,
    SlotError = 16,
    Ammo = 17,
    /// 1 while the side's plan is charging (SIM-AI-011).
    Charging = 26,
    // Army scope.
    ArmyStrengthRatio = 18,
    ArmyMoraleMean = 19,
    ArmyFatigueMean = 20,
    TerrainAdvantage = 21,
    TimeRemaining = 22,
    Aggression = 23,
    CasualtyFraction = 24,
    EnemyVisible = 25,
}
impl_hashable_fieldless_enum!(InputId);

impl InputId {
    /// The scope an input is defined for; `Constant` fits both.
    pub fn scope(self) -> Option<InputScope> {
        match self {
            InputId::Constant => None,
            InputId::DistanceToNearestEnemy
            | InputId::StrengthRatio
            | InputId::EnemyShare
            | InputId::OwnMorale
            | InputId::OwnFatigue
            | InputId::Engaged
            | InputId::EngagedFrontal
            | InputId::IsFlankExposed
            | InputId::EnemyFlankOpen
            | InputId::CavalryApproaching
            | InputId::InfantryThreat
            | InputId::EnemyRangedInRange
            | InputId::EnemyInOwnRange
            | InputId::FriendlyInLineOfFire
            | InputId::Outnumbered
            | InputId::SlotError
            | InputId::Ammo
            | InputId::Charging => Some(InputScope::Regiment),
            InputId::ArmyStrengthRatio
            | InputId::ArmyMoraleMean
            | InputId::ArmyFatigueMean
            | InputId::TerrainAdvantage
            | InputId::TimeRemaining
            | InputId::Aggression
            | InputId::CasualtyFraction
            | InputId::EnemyVisible => Some(InputScope::Army),
        }
    }
}

/// A response curve over a normalised input (SIM-AI-001). Every curve's
/// output is clamped to `[0, 1]` by `il_ai::evaluate`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Curve {
    /// `m · x + b`.
    Linear {
        #[serde(deserialize_with = "de_s")]
        m: S,
        #[serde(deserialize_with = "de_s", default = "d_zero")]
        b: S,
    },
    /// `k · x²`.
    Quadratic {
        #[serde(deserialize_with = "de_s", default = "d_one")]
        k: S,
    },
    /// Algebraic sigmoid `0.5 + 0.5 · t / (1 + |t|)` with `t = k · (x − mid)`
    /// (plan decision 6: no `exp`); a negative `k` decreases.
    Logistic {
        #[serde(deserialize_with = "de_s")]
        k: S,
        #[serde(deserialize_with = "de_s")]
        mid: S,
    },
    /// `1` when `x ≥ threshold`, else `0`.
    Step {
        #[serde(deserialize_with = "de_s")]
        threshold: S,
    },
}

impl Curve {
    fn hash_content(&self, h: &mut StateHasher) {
        match self {
            Curve::Linear { m, b } => {
                h.write_u8(0);
                h.write(m);
                h.write(b);
            }
            Curve::Quadratic { k } => {
                h.write_u8(1);
                h.write(k);
            }
            Curve::Logistic { k, mid } => {
                h.write_u8(2);
                h.write(k);
                h.write(mid);
            }
            Curve::Step { threshold } => {
                h.write_u8(3);
                h.write(threshold);
            }
        }
    }
}

/// One factor of an action's score: `curve(sat(raw(input) / scale))`.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Consideration {
    pub input: InputId,
    /// The raw value that maps to `x = 1` (plan decision 21); inputs that
    /// are already fractions leave it at 1.
    #[serde(deserialize_with = "de_s", default = "d_one")]
    pub scale: S,
    pub curve: Curve,
}

/// The decision channel an action competes in (plan decision 5, I3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Channel {
    Movement = 0,
    Formation = 1,
    Fire = 2,
    Ability = 3,
    Stance = 4,
}
impl_hashable_fieldless_enum!(Channel);

/// What an action does when it wins (plan decision 8). Tagged by `kind`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionKind {
    // Regiment, movement channel (SIM-AI-021, SIM-AI-022).
    EngageNearest,
    HoldPosition,
    FallBack,
    FollowCentroid,
    // Regiment, one channel per ability slot.
    UseAbility { ability: ContentId },
    // Regiment, formation channel.
    SwitchFormation { layout: Layout },
    // Regiment, fire channel.
    FireAtWill,
    HoldFire,
    // Army, stance channel (SIM-AI-010).
    Attack,
    Defend,
    Hold,
    Retreat,
}

impl ActionKind {
    pub fn channel(&self) -> Channel {
        match self {
            ActionKind::EngageNearest
            | ActionKind::HoldPosition
            | ActionKind::FallBack
            | ActionKind::FollowCentroid => Channel::Movement,
            ActionKind::UseAbility { .. } => Channel::Ability,
            ActionKind::SwitchFormation { .. } => Channel::Formation,
            ActionKind::FireAtWill | ActionKind::HoldFire => Channel::Fire,
            ActionKind::Attack | ActionKind::Defend | ActionKind::Hold | ActionKind::Retreat => {
                Channel::Stance
            }
        }
    }

    pub fn scope(&self) -> InputScope {
        match self.channel() {
            Channel::Stance => InputScope::Army,
            _ => InputScope::Regiment,
        }
    }

    /// The `kind` name as written in content.
    pub fn kind_name(&self) -> &'static str {
        match self {
            ActionKind::EngageNearest => "engage_nearest",
            ActionKind::HoldPosition => "hold_position",
            ActionKind::FallBack => "fall_back",
            ActionKind::FollowCentroid => "follow_centroid",
            ActionKind::UseAbility { .. } => "use_ability",
            ActionKind::SwitchFormation { .. } => "switch_formation",
            ActionKind::FireAtWill => "fire_at_will",
            ActionKind::HoldFire => "hold_fire",
            ActionKind::Attack => "attack",
            ActionKind::Defend => "defend",
            ActionKind::Hold => "hold",
            ActionKind::Retreat => "retreat",
        }
    }

    fn discriminant(&self) -> u8 {
        match self {
            ActionKind::EngageNearest => 0,
            ActionKind::HoldPosition => 1,
            ActionKind::FallBack => 2,
            ActionKind::FollowCentroid => 3,
            ActionKind::UseAbility { .. } => 4,
            ActionKind::SwitchFormation { .. } => 5,
            ActionKind::FireAtWill => 6,
            ActionKind::HoldFire => 7,
            ActionKind::Attack => 8,
            ActionKind::Defend => 9,
            ActionKind::Hold => 10,
            ActionKind::Retreat => 11,
        }
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write_u8(self.discriminant());
        match self {
            ActionKind::UseAbility { ability } => h.write(ability),
            ActionKind::SwitchFormation { layout } => h.write(layout),
            _ => {}
        }
    }
}

/// One candidate action: `score = base × Π curve_c(x_c)`, then noise; it
/// wins its channel when it has the highest score of at least `threshold`
/// (ties by list order).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ActionDef {
    /// Unique within the set; shown by the debug overlay.
    pub name: String,
    #[serde(flatten)]
    pub kind: ActionKind,
    #[serde(deserialize_with = "de_s", default = "d_one")]
    pub base: S,
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub threshold: S,
    /// Multiplicative jitter drawn from the AI stream when non-zero
    /// (plan decision 7).
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub noise: S,
    #[serde(default)]
    pub considerations: Vec<Consideration>,
}

/// The candidate actions of one scope (`content/ai/actions/`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AiActionSet {
    pub id: ContentId,
    pub scope: InputScope,
    pub actions: Vec<ActionDef>,
    #[serde(default)]
    pub deprecated: Option<String>,
}

impl AiActionSet {
    /// Indices of the actions in `channel`, in list order.
    pub fn channel_actions(&self, channel: Channel) -> impl Iterator<Item = (usize, &ActionDef)> {
        self.actions
            .iter()
            .enumerate()
            .filter(move |(_, a)| a.kind.channel() == channel)
    }
}

impl ContentKind for AiActionSet {
    const DIR: &'static str = "ai/actions";
    const TAG: KindTag = KindTag::AiActionSet;

    fn id(&self) -> &ContentId {
        &self.id
    }

    /// Every action and input must belong to the set's scope, names are
    /// unique, and a `use_ability` names an ability that exists.
    fn resolve(&mut self, lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        for (i, a) in self.actions.iter().enumerate() {
            if a.kind.scope() != self.scope {
                errors.push(
                    ResolveError::new(format!("actions[{i}].kind"), self.id.clone(), Self::TAG)
                        .with_message(format!(
                            "action kind {:?} is not a {:?} action",
                            a.kind.kind_name(),
                            self.scope
                        ))
                        .with_expected(format!("a {:?}-scope kind", self.scope)),
                );
            }
            if self.actions[..i].iter().any(|b| b.name == a.name) {
                errors.push(
                    ResolveError::new(format!("actions[{i}].name"), self.id.clone(), Self::TAG)
                        .with_message(format!("duplicate action name {:?}", a.name))
                        .with_expected("a name unique within the set"),
                );
            }
            for (c, cons) in a.considerations.iter().enumerate() {
                if cons.input.scope().is_some_and(|s| s != self.scope) {
                    errors.push(
                        ResolveError::new(
                            format!("actions[{i}].considerations[{c}].input"),
                            self.id.clone(),
                            Self::TAG,
                        )
                        .with_message(format!(
                            "input {:?} is not a {:?} input",
                            cons.input, self.scope
                        ))
                        .with_expected(format!("a {:?}-scope input", self.scope)),
                    );
                }
            }
            if let ActionKind::UseAbility { ability } = &a.kind
                && lookup.handle::<crate::ability::Ability>(ability).is_none()
            {
                errors.push(ResolveError::new(
                    format!("actions[{i}].ability"),
                    ability.clone(),
                    KindTag::Ability,
                ));
            }
        }
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        h.write(&self.scope);
        h.write_u32(self.actions.len() as u32);
        for a in &self.actions {
            h.write_bytes(a.name.as_bytes());
            h.write_u8(0);
            a.kind.hash_content(h);
            h.write(&a.base);
            h.write(&a.threshold);
            h.write(&a.noise);
            h.write_u32(a.considerations.len() as u32);
            for c in &a.considerations {
                h.write(&c.input);
                h.write(&c.scale);
                c.curve.hash_content(h);
            }
        }
    }
}

/// Campaign-side personality (TDD §8.5), parsed and hashed now, read in
/// Phase 4 (plan I23).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CampaignProfile {
    #[serde(deserialize_with = "de_s")]
    pub army_strength_target: S,
    #[serde(default)]
    pub composition: Vec<Composition>,
    #[serde(default)]
    pub min_garrison: u8,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Composition {
    pub category: UnitCategory,
    #[serde(deserialize_with = "de_s")]
    pub fraction: S,
}

/// A personality plus the army-level tunables of Simulation Spec §13
/// (`content/ai/profiles/`). Every numeric field is required, like a rules
/// file (§15.4).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct AiProfile {
    pub id: ContentId,
    #[serde(default)]
    pub name_key: Option<String>,
    // Personality (SIM-AI-010, 014, 022; plan decision 10).
    #[serde(deserialize_with = "de_s")]
    pub aggression: S,
    #[serde(deserialize_with = "de_s")]
    pub general_aggression: S,
    #[serde(deserialize_with = "de_s")]
    pub reserve_fraction: S,
    #[serde(deserialize_with = "de_s")]
    pub stance_margin: S,
    // Cadence (SIM-AI-002).
    pub army_period_ticks: u16,
    pub regiment_period_ticks: u16,
    // Distances, metres (SIM-AI-011..014; plan decision 22).
    #[serde(deserialize_with = "de_s")]
    pub approach_distance: S,
    #[serde(deserialize_with = "de_s")]
    pub advance_step: S,
    #[serde(deserialize_with = "de_s")]
    pub line_tolerance: S,
    #[serde(deserialize_with = "de_s")]
    pub skirmish_range_frac: S,
    #[serde(deserialize_with = "de_s")]
    pub flank_offset: S,
    #[serde(deserialize_with = "de_s")]
    pub charge_trigger_dist: S,
    /// SIM-AI-011 (T3-010): the attacking line rests at `approach_distance`
    /// for at most this many ticks, whatever its missile units' ammo.
    pub standoff_max_ticks: u32,
    /// SIM-AI-011 (T3-010): the line runs its charge only while its mean
    /// fatigue is under this; a more tired line walks in.
    #[serde(deserialize_with = "de_s")]
    pub charge_max_fatigue: S,
    #[serde(deserialize_with = "de_s")]
    pub defend_search_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub counter_charge_dist: S,
    #[serde(deserialize_with = "de_s")]
    pub screen_offset: S,
    #[serde(deserialize_with = "de_s")]
    pub screen_gap: S,
    #[serde(deserialize_with = "de_s")]
    pub reserve_offset: S,
    /// Morale below which a line regiment draws a reserve (SIM-AI-014).
    #[serde(deserialize_with = "de_s")]
    pub commit_morale: S,
    /// One army set and one regiment set, in any order.
    #[serde(rename = "action_sets")]
    pub action_set_ids: Vec<ContentId>,
    #[serde(skip)]
    pub army_set: Option<Handle<AiActionSet>>,
    #[serde(skip)]
    pub regiment_set: Option<Handle<AiActionSet>>,
    #[serde(default)]
    pub campaign: Option<CampaignProfile>,
    #[serde(default)]
    pub deprecated: Option<String>,
}

impl ContentKind for AiProfile {
    const DIR: &'static str = "ai/profiles";
    const TAG: KindTag = KindTag::AiProfile;

    fn id(&self) -> &ContentId {
        &self.id
    }

    /// The named sets must exist; the scopes are settled by the sets
    /// themselves, so a profile needs exactly one of each.
    fn resolve(&mut self, lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        self.army_set = None;
        self.regiment_set = None;
        let mut army = 0u8;
        let mut regiment = 0u8;
        for (i, id) in self.action_set_ids.iter().enumerate() {
            match lookup.handle::<AiActionSet>(id) {
                Some(h) => match lookup.scope_of_action_set(id) {
                    Some(InputScope::Army) => {
                        army += 1;
                        self.army_set = Some(h);
                    }
                    Some(InputScope::Regiment) => {
                        regiment += 1;
                        self.regiment_set = Some(h);
                    }
                    None => {}
                },
                None => errors.push(ResolveError::new(
                    format!("action_sets[{i}]"),
                    id.clone(),
                    KindTag::AiActionSet,
                )),
            }
        }
        if army != 1 || regiment != 1 {
            errors.push(
                ResolveError::new("action_sets", self.id.clone(), Self::TAG)
                    .with_message(format!(
                        "found {army} army set(s) and {regiment} regiment set(s)"
                    ))
                    .with_expected("one army set and one regiment set"),
            );
        }
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        for v in [
            self.aggression,
            self.general_aggression,
            self.reserve_fraction,
            self.stance_margin,
        ] {
            h.write(&v);
        }
        h.write_u16(self.army_period_ticks);
        h.write_u16(self.regiment_period_ticks);
        h.write_u32(self.standoff_max_ticks);
        for v in [
            self.approach_distance,
            self.advance_step,
            self.line_tolerance,
            self.skirmish_range_frac,
            self.flank_offset,
            self.charge_trigger_dist,
            self.charge_max_fatigue,
            self.defend_search_radius,
            self.counter_charge_dist,
            self.screen_offset,
            self.screen_gap,
            self.reserve_offset,
            self.commit_morale,
        ] {
            h.write(&v);
        }
        h.write(&self.action_set_ids);
        match &self.campaign {
            None => h.write_u8(0),
            Some(c) => {
                h.write_u8(1);
                h.write(&c.army_strength_target);
                h.write_u32(c.composition.len() as u32);
                for e in &c.composition {
                    h.write(&e.category);
                    h.write(&e.fraction);
                }
                h.write_u8(c.min_garrison);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json5::{FileId, parse_json5};
    use il_core::Scalar;

    fn set(src: &str) -> Result<AiActionSet, serde_json::Error> {
        serde_json::from_value(parse_json5(src, FileId(0)).unwrap().to_json())
    }

    #[test]
    fn every_curve_and_kind_parses() {
        let s = set(r#"{
            id: "rome:r", scope: "regiment",
            actions: [
              { name: "engage", kind: "engage_nearest", base: 1.5, threshold: 0.2, noise: 0.1,
                considerations: [
                  { input: "distance_to_nearest_enemy", scale: 80, curve: { type: "linear", m: -1, b: 1 } },
                  { input: "strength_ratio", curve: { type: "logistic", k: 8, mid: 0.4 } },
                  { input: "engaged", curve: { type: "step", threshold: 0.5 } },
                  { input: "own_morale", curve: { type: "quadratic", k: 2 } } ] },
              { name: "hold", kind: "hold_position" },
              { name: "back", kind: "fall_back" },
              { name: "general", kind: "follow_centroid" },
              { name: "testudo", kind: "use_ability", ability: "rome:testudo" },
              { name: "square", kind: "switch_formation", layout: "square" },
              { name: "fire", kind: "fire_at_will" },
              { name: "hold_fire", kind: "hold_fire" },
            ] }"#)
        .unwrap();
        assert_eq!(s.actions.len(), 8);
        let a = &s.actions[0];
        assert_eq!(a.base, S::from_f32_data(1.5));
        assert_eq!(a.considerations[0].scale, S::from_i32(80));
        assert_eq!(a.considerations[1].scale, S::ONE);
        assert!(matches!(a.considerations[2].curve, Curve::Step { .. }));
        assert_eq!(s.actions[1].base, S::ONE);
        assert_eq!(s.actions[1].threshold, S::ZERO);
        assert!(matches!(
            s.actions[5].kind,
            ActionKind::SwitchFormation {
                layout: Layout::Square
            }
        ));
        assert_eq!(s.channel_actions(Channel::Movement).count(), 4);
        assert_eq!(s.channel_actions(Channel::Fire).count(), 2);
        let army = set(r#"{ id: "rome:a", scope: "army", actions: [
            { name: "attack", kind: "attack" }, { name: "defend", kind: "defend" },
            { name: "hold", kind: "hold" }, { name: "retreat", kind: "retreat" } ] }"#)
        .unwrap();
        assert!(
            army.actions
                .iter()
                .all(|a| a.kind.channel() == Channel::Stance)
        );
    }

    #[test]
    fn unknown_kind_and_input_are_rejected_at_parse() {
        assert!(
            set(
                r#"{ id: "rome:r", scope: "regiment", actions: [ { name: "x", kind: "dance" } ] }"#
            )
            .is_err()
        );
        assert!(
            set(
                r#"{ id: "rome:r", scope: "regiment", actions: [ { name: "x", kind: "hold_position",
            considerations: [ { input: "mood", curve: { type: "step", threshold: 0.5 } } ] } ] }"#
            )
            .is_err()
        );
        assert!(set(r#"{ id: "rome:r", scope: "regiment", actions: [ { name: "x", kind: "use_ability" } ] }"#).is_err());
    }

    #[test]
    fn scope_mismatch_and_duplicates_are_resolve_errors() {
        let mut s = set(r#"{ id: "rome:r", scope: "regiment", actions: [
            { name: "a", kind: "attack" },
            { name: "b", kind: "hold_position", considerations: [ { input: "aggression", curve: { type: "step", threshold: 0.5 } } ] },
            { name: "b", kind: "fall_back" } ] }"#)
        .unwrap();
        let lookup = Lookup::new();
        let mut errors = Vec::new();
        s.resolve(&lookup, &mut errors);
        let fields: Vec<&str> = errors.iter().map(|e| e.field.as_str()).collect();
        assert_eq!(
            fields,
            [
                "actions[0].kind",
                "actions[1].considerations[0].input",
                "actions[2].name"
            ]
        );
        assert!(
            errors[0]
                .message
                .as_deref()
                .unwrap()
                .contains("not a Regiment action")
        );
    }

    #[test]
    fn hash_covers_curves_and_params() {
        let base = set(r#"{ id: "rome:r", scope: "regiment", actions: [
            { name: "x", kind: "switch_formation", layout: "square", considerations: [
              { input: "own_morale", curve: { type: "linear", m: 1 } } ] } ] }"#)
        .unwrap();
        let mut h = StateHasher::new();
        base.hash_content(&mut h);
        let h0 = h.finish();
        for src in [
            r#"{ id: "rome:r", scope: "regiment", actions: [ { name: "x", kind: "switch_formation", layout: "phalanx", considerations: [ { input: "own_morale", curve: { type: "linear", m: 1 } } ] } ] }"#,
            r#"{ id: "rome:r", scope: "regiment", actions: [ { name: "x", kind: "switch_formation", layout: "square", considerations: [ { input: "own_morale", curve: { type: "linear", m: 1, b: 0.5 } } ] } ] }"#,
            r#"{ id: "rome:r", scope: "regiment", actions: [ { name: "x", kind: "switch_formation", layout: "square", threshold: 0.5, considerations: [ { input: "own_morale", curve: { type: "linear", m: 1 } } ] } ] }"#,
            r#"{ id: "rome:r", scope: "regiment", actions: [ { name: "x", kind: "switch_formation", layout: "square", considerations: [ { input: "own_morale", scale: 2, curve: { type: "linear", m: 1 } } ] } ] }"#,
        ] {
            let mut h = StateHasher::new();
            set(src).unwrap().hash_content(&mut h);
            assert_ne!(h.finish(), h0, "{src}");
        }
    }

    #[test]
    fn profile_parses_and_needs_one_set_per_scope() {
        let src = r#"{ id: "rome:p", aggression: 0.6, general_aggression: 0.3, reserve_fraction: 0.2,
            stance_margin: 0.1, army_period_ticks: 40, regiment_period_ticks: 20, approach_distance: 150,
            advance_step: 8, line_tolerance: 12, skirmish_range_frac: 0.9, flank_offset: 80,
            charge_trigger_dist: 40, standoff_max_ticks: 1200, charge_max_fatigue: 0.35,
            defend_search_radius: 200, counter_charge_dist: 30, screen_offset: 40,
            screen_gap: 80, reserve_offset: 60, commit_morale: 45, action_sets: ["rome:a", "rome:r"],
            campaign: { army_strength_target: 1.2, composition: [ { category: "infantry", fraction: 0.6 } ], min_garrison: 1 } }"#;
        let mut p: AiProfile =
            serde_json::from_value(parse_json5(src, FileId(0)).unwrap().to_json()).unwrap();
        assert_eq!(p.army_period_ticks, 40);
        assert_eq!(p.campaign.as_ref().unwrap().composition.len(), 1);
        let mut lookup = Lookup::new();
        lookup.register(
            KindTag::AiActionSet,
            [
                (&ContentId::new("rome:a").unwrap(), 0u32),
                (&ContentId::new("rome:r").unwrap(), 1u32),
            ],
        );
        lookup.register_action_set_scopes([
            (ContentId::new("rome:a").unwrap(), InputScope::Army),
            (ContentId::new("rome:r").unwrap(), InputScope::Regiment),
        ]);
        let mut errors = Vec::new();
        p.resolve(&lookup, &mut errors);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(p.army_set.unwrap().index(), 0);
        assert_eq!(p.regiment_set.unwrap().index(), 1);
        // Two army sets: a diagnostic on `action_sets`.
        lookup.register_action_set_scopes([(ContentId::new("rome:r").unwrap(), InputScope::Army)]);
        p.resolve(&lookup, &mut errors);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "action_sets");
        assert_eq!(
            errors[0].expected.as_deref(),
            Some("one army set and one regiment set")
        );
    }
}
