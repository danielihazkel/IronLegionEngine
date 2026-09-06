//! `Ability` and `Effect` (Simulation Spec §10 SIM-ABIL-001..002, TDD §8.3,
//! Modding SDK §4.4, `ability.schema.json`). Every effect kind parses; only
//! `buff` and `debuff` are executable before the Phase 5 fantasy layer, and
//! an ability using another kind is rejected at load with a diagnostic
//! naming it (SIM-ABIL-002).

use il_core::{S, StateHasher, impl_hashable_fieldless_enum};
use serde::{Deserialize, Serialize};

use crate::content_id::ContentId;
use crate::de::{d_one, d_zero, de_s};
use crate::registry::{ContentKind, Lookup, ResolveError};
use crate::schema::KindTag;

/// Who an ability may be used on (SIM-ABIL-001).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Targeting {
    /// The using regiment itself.
    #[serde(rename = "self")]
    SelfTarget = 0,
    /// One same-side regiment within `range`.
    RegimentAlly = 1,
    /// One enemy regiment within `range` (visible to the user's side).
    RegimentEnemy = 2,
    /// A point within `range`; regiments whose anchor lies within `radius`
    /// of it are affected.
    Point = 3,
    /// Regiments whose anchor lies within `radius` of the user's anchor.
    Area = 4,
}
impl_hashable_fieldless_enum!(Targeting);

/// The stat a buff or debuff touches (SIM-ABIL-002).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Stat {
    Attack = 0,
    Defence = 1,
    Armour = 2,
    Damage = 3,
    Speed = 4,
    AttackInterval = 5,
    MoralePerS = 6,
    FatigueRate = 7,
    LosRadius = 8,
    Accuracy = 9,
}
impl_hashable_fieldless_enum!(Stat);

impl Stat {
    pub const ALL: [Stat; 10] = [
        Stat::Attack,
        Stat::Defence,
        Stat::Armour,
        Stat::Damage,
        Stat::Speed,
        Stat::AttackInterval,
        Stat::MoralePerS,
        Stat::FatigueRate,
        Stat::LosRadius,
        Stat::Accuracy,
    ];
}

/// How a second application of the same ability combines with an active
/// status (SIM-ABIL-004).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum Stacking {
    /// Reset the remaining duration.
    Refresh = 0,
    /// Increment the stack count up to `max_stacks` (additive parts scale
    /// with the count) and reset the duration.
    Stack = 1,
    /// Keep the longer remaining duration and the higher stack count.
    Highest = 2,
}
impl_hashable_fieldless_enum!(Stacking);

/// One effect of an ability (SIM-ABIL-002). Tagged by `type`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    Buff {
        stat: Stat,
        #[serde(deserialize_with = "de_s", default = "d_one")]
        mult: S,
        #[serde(deserialize_with = "de_s", default = "d_zero")]
        add: S,
    },
    Debuff {
        stat: Stat,
        #[serde(deserialize_with = "de_s", default = "d_one")]
        mult: S,
        #[serde(deserialize_with = "de_s", default = "d_zero")]
        add: S,
    },
    Damage {
        #[serde(deserialize_with = "de_s")]
        amount: S,
        #[serde(deserialize_with = "de_s", default = "d_zero")]
        armour_penetration: S,
        #[serde(default)]
        per_tick: bool,
    },
    Heal {
        #[serde(deserialize_with = "de_s")]
        amount: S,
        #[serde(default)]
        per_tick: bool,
    },
    Summon {
        unit_type: ContentId,
        count: u16,
        formation: ContentId,
    },
    Fear,
    Area {
        effects: Vec<Effect>,
        #[serde(deserialize_with = "de_s")]
        radius: S,
        #[serde(default)]
        duration_ticks: u16,
    },
    Teleport {
        #[serde(deserialize_with = "de_s")]
        max_distance: S,
    },
}

impl Effect {
    /// The `type` name as written in content.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Effect::Buff { .. } => "buff",
            Effect::Debuff { .. } => "debuff",
            Effect::Damage { .. } => "damage",
            Effect::Heal { .. } => "heal",
            Effect::Summon { .. } => "summon",
            Effect::Fear => "fear",
            Effect::Area { .. } => "area",
            Effect::Teleport { .. } => "teleport",
        }
    }

    /// Whether the engine can execute this effect before Phase 5.
    pub fn is_executable(&self) -> bool {
        matches!(self, Effect::Buff { .. } | Effect::Debuff { .. })
    }

    fn hash_content(&self, h: &mut StateHasher) {
        match self {
            Effect::Buff { stat, mult, add } => {
                h.write_u8(0);
                h.write(stat);
                h.write(mult);
                h.write(add);
            }
            Effect::Debuff { stat, mult, add } => {
                h.write_u8(1);
                h.write(stat);
                h.write(mult);
                h.write(add);
            }
            Effect::Damage {
                amount,
                armour_penetration,
                per_tick,
            } => {
                h.write_u8(2);
                h.write(amount);
                h.write(armour_penetration);
                h.write(per_tick);
            }
            Effect::Heal { amount, per_tick } => {
                h.write_u8(3);
                h.write(amount);
                h.write(per_tick);
            }
            Effect::Summon {
                unit_type,
                count,
                formation,
            } => {
                h.write_u8(4);
                h.write(unit_type);
                h.write_u16(*count);
                h.write(formation);
            }
            Effect::Fear => h.write_u8(5),
            Effect::Area {
                effects,
                radius,
                duration_ticks,
            } => {
                h.write_u8(6);
                h.write_u32(effects.len() as u32);
                for e in effects {
                    e.hash_content(h);
                }
                h.write(radius);
                h.write_u16(*duration_ticks);
            }
            Effect::Teleport { max_distance } => {
                h.write_u8(7);
                h.write(max_distance);
            }
        }
    }
}

/// An ability (SIM-ABIL-001): data only; the engine reads it at `UseAbility`
/// and at Stage 12.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Ability {
    pub id: ContentId,
    pub name_key: String,
    /// Tooltip key (render only).
    #[serde(default)]
    pub description_key: Option<String>,
    /// Icon path under `assets_root` (render only).
    #[serde(default)]
    pub icon: Option<String>,
    pub targeting: Targeting,
    /// Metres, for `point` and `area` targeting.
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub radius: S,
    /// Metres from the user's anchor to the target anchor or point.
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub range: S,
    pub cooldown_ticks: u16,
    /// Status duration; buffs and debuffs need at least 1.
    #[serde(default)]
    pub duration_ticks: u16,
    /// Phase 5 resource (SIM-ABIL-006); antiquity content uses 0.
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub energy_cost: S,
    pub effects: Vec<Effect>,
    #[serde(default = "d_refresh")]
    pub stacking: Stacking,
    #[serde(default = "d_one_u8")]
    pub max_stacks: u8,
    #[serde(default)]
    pub requires_not_engaged: bool,
    #[serde(default)]
    pub requires_not_moving: bool,
    #[serde(default)]
    pub deprecated: Option<String>,
}

fn d_refresh() -> Stacking {
    Stacking::Refresh
}
fn d_one_u8() -> u8 {
    1
}

impl Ability {
    /// Whether any effect is a buff or debuff (the only executable kinds).
    pub fn has_status_effect(&self) -> bool {
        self.effects.iter().any(Effect::is_executable)
    }
}

impl ContentKind for Ability {
    const DIR: &'static str = "abilities";
    const TAG: KindTag = KindTag::Ability;

    fn id(&self) -> &ContentId {
        &self.id
    }

    /// SIM-ABIL-002: every effect kind other than buff and debuff is
    /// rejected until Phase 5; a buff or debuff needs a duration.
    fn resolve(&mut self, _lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        for (i, e) in self.effects.iter().enumerate() {
            if !e.is_executable() {
                errors.push(
                    ResolveError::new(format!("effects[{i}]"), self.id.clone(), KindTag::Ability)
                        .with_message(format!(
                            "effect kind {:?} is not executable before Phase 5",
                            e.kind_name()
                        ))
                        .with_expected("buff or debuff"),
                );
            }
        }
        if self.has_status_effect() && self.duration_ticks == 0 {
            errors.push(
                ResolveError::new("duration_ticks", self.id.clone(), KindTag::Ability)
                    .with_message("a buff or debuff needs duration_ticks of at least 1")
                    .with_expected(">= 1"),
            );
        }
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        h.write(&self.targeting);
        h.write(&self.radius);
        h.write(&self.range);
        h.write_u16(self.cooldown_ticks);
        h.write_u16(self.duration_ticks);
        h.write(&self.energy_cost);
        h.write_u32(self.effects.len() as u32);
        for e in &self.effects {
            e.hash_content(h);
        }
        h.write(&self.stacking);
        h.write_u8(self.max_stacks);
        h.write(&self.requires_not_engaged);
        h.write(&self.requires_not_moving);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json5::{FileId, parse_json5};
    use il_core::Scalar;

    fn from_json5(src: &str) -> Result<Ability, serde_json::Error> {
        serde_json::from_value(parse_json5(src, FileId(0)).unwrap().to_json())
    }

    #[test]
    fn every_effect_kind_parses_and_defaults_apply() {
        let a = from_json5(
            r#"{ id: "m:x", name_key: "m.x", targeting: "self", cooldown_ticks: 100, duration_ticks: 10,
                 effects: [
                   { type: "buff", stat: "armour", mult: 2 },
                   { type: "debuff", stat: "morale_per_s", add: -2 },
                   { type: "damage", amount: 5 },
                   { type: "heal", amount: 5, per_tick: true },
                   { type: "summon", unit_type: "m:u", count: 3, formation: "m:line" },
                   { type: "fear" },
                   { type: "area", effects: [{ type: "fear" }], radius: 20 },
                   { type: "teleport", max_distance: 50 },
                 ] }"#,
        )
        .unwrap();
        assert_eq!(a.targeting, Targeting::SelfTarget);
        assert_eq!(a.stacking, Stacking::Refresh);
        assert_eq!(a.max_stacks, 1);
        assert_eq!(a.effects.len(), 8);
        match &a.effects[0] {
            Effect::Buff { stat, mult, add } => {
                assert_eq!(*stat, Stat::Armour);
                assert_eq!(*mult, S::from_f32_data(2.0));
                assert_eq!(*add, S::ZERO);
            }
            other => panic!("{other:?}"),
        }
        match &a.effects[1] {
            Effect::Debuff { stat, mult, add } => {
                assert_eq!(*stat, Stat::MoralePerS);
                assert_eq!(*mult, S::ONE);
                assert_eq!(*add, S::from_f32_data(-2.0));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(a.effects[5], Effect::Fear);
        assert!(!a.effects[6].is_executable());
    }

    #[test]
    fn targeting_names_follow_the_spec() {
        for (name, want) in [
            ("self", Targeting::SelfTarget),
            ("regiment_ally", Targeting::RegimentAlly),
            ("regiment_enemy", Targeting::RegimentEnemy),
            ("point", Targeting::Point),
            ("area", Targeting::Area),
        ] {
            let a = from_json5(&format!(
                r#"{{ id: "m:x", name_key: "m.x", targeting: "{name}", cooldown_ticks: 1,
                     duration_ticks: 1, effects: [{{ type: "fear" }}] }}"#
            ))
            .unwrap();
            assert_eq!(a.targeting, want, "{name}");
        }
    }

    #[test]
    fn unsupported_effects_and_zero_durations_are_load_errors() {
        let mut a = from_json5(
            r#"{ id: "m:x", name_key: "m.x", targeting: "self", cooldown_ticks: 1,
                 effects: [{ type: "buff", stat: "attack", mult: 1.1 }, { type: "summon", unit_type: "m:u", count: 1, formation: "m:l" }] }"#,
        )
        .unwrap();
        let mut errors = Vec::new();
        a.resolve(&Lookup::new(), &mut errors);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(errors[0].field, "effects[1]");
        assert_eq!(
            errors[0].message.as_deref(),
            Some("effect kind \"summon\" is not executable before Phase 5")
        );
        assert_eq!(errors[0].expected.as_deref(), Some("buff or debuff"));
        assert_eq!(errors[1].field, "duration_ticks");
    }

    #[test]
    fn hash_covers_effects() {
        let base = r#"{ id: "m:x", name_key: "m.x", targeting: "self", cooldown_ticks: 1, duration_ticks: 5,
                        effects: [{ type: "buff", stat: "attack", mult: 1.1 }] }"#;
        let a = from_json5(base).unwrap();
        let b = from_json5(&base.replace("1.1", "1.2")).unwrap();
        let hash = |x: &Ability| {
            let mut h = StateHasher::new();
            x.hash_content(&mut h);
            h.finish().0
        };
        assert_ne!(hash(&a), hash(&b));
        assert_eq!(hash(&a), hash(&a.clone()));
    }
}
