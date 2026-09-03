//! `UnitType`: every field of `unit-type.schema.json` (Modding SDK §4.1,
//! Simulation Spec §15.2). Phase 1 reads the movement fields; combat fields
//! wait for Phase 2 but are typed now so content validates end to end.

use il_core::{S, StateHasher, impl_hashable_fieldless_enum};
use serde::{Deserialize, Serialize};

use crate::content_id::ContentId;
use crate::de::{d_one, d_zero, de_s, s};
use crate::formation::FormationTemplate;
use crate::handle::Handle;
use crate::registry::{ContentKind, Lookup, ResolveError};
use crate::schema::KindTag;
use crate::sprite_set::SpriteSet;

/// Soldier category (drives role zones, anti-cavalry, AI considerations).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum UnitCategory {
    Infantry = 0,
    Cavalry = 1,
    Ranged = 2,
    Skirmisher = 3,
    General = 4,
    Siege = 5,
}
impl_hashable_fieldless_enum!(UnitCategory);

impl UnitCategory {
    pub const ALL: [UnitCategory; 6] = [
        UnitCategory::Infantry,
        UnitCategory::Cavalry,
        UnitCategory::Ranged,
        UnitCategory::Skirmisher,
        UnitCategory::General,
        UnitCategory::Siege,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum ProjectileArc {
    Direct = 0,
    Indirect = 1,
}
impl_hashable_fieldless_enum!(ProjectileArc);

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Ranged {
    #[serde(deserialize_with = "de_s")]
    pub range: S,
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub min_range: S,
    #[serde(deserialize_with = "de_s")]
    pub accuracy: S,
    #[serde(deserialize_with = "de_s")]
    pub projectile_speed: S,
    pub reload_ticks: u16,
    pub ammo: u16,
    #[serde(deserialize_with = "de_s")]
    pub damage: S,
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub armour_penetration: S,
    #[serde(default = "default_arc")]
    pub arc: ProjectileArc,
}

fn default_arc() -> ProjectileArc {
    ProjectileArc::Direct
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ExperienceTier {
    pub xp: u32,
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub attack: S,
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub defence: S,
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub morale: S,
}

/// Sound asset paths under the mod's `assets_root` (Phase 2).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct UnitSounds {
    pub select: Option<String>,
    #[serde(rename = "move")]
    pub move_: Option<String>,
    pub attack: Option<String>,
    pub charge: Option<String>,
    pub die: Option<String>,
}

#[derive(Deserialize)]
struct Raw {
    id: ContentId,
    name_key: String,
    category: UnitCategory,
    #[serde(default)]
    soldier_radius: Option<f32>,
    #[serde(default)]
    mass: Option<f32>,
    hp: f32,
    speed_walk: f32,
    speed_run: f32,
    #[serde(default)]
    speed_march: Option<f32>,
    attack: f32,
    defence: f32,
    #[serde(default)]
    armour: f32,
    damage: f32,
    #[serde(default = "d_attack_interval")]
    attack_interval_ticks: u16,
    #[serde(default = "d_reach")]
    reach: f32,
    #[serde(default)]
    charge_bonus: f32,
    #[serde(default)]
    anti_cavalry_bonus: f32,
    #[serde(default)]
    second_rank_attack: bool,
    #[serde(default)]
    shield: bool,
    #[serde(default = "d_frontal_arc")]
    frontal_arc_deg: f32,
    #[serde(default)]
    ranged: Option<Ranged>,
    #[serde(default = "d_morale_base")]
    morale_base: f32,
    #[serde(default = "d_one_f32")]
    fatigue_rate_mult: f32,
    #[serde(default = "d_los")]
    los_radius: f32,
    #[serde(default)]
    abilities: Vec<ContentId>,
    formations: Vec<ContentId>,
    sprite_set: ContentId,
    #[serde(default)]
    sounds: UnitSounds,
    cost: u32,
    upkeep: u32,
    #[serde(default = "d_one_u16")]
    recruit_turns: u16,
    #[serde(default = "d_regiment_size")]
    regiment_size: u16,
    #[serde(default = "d_one_u8")]
    tier: u8,
    #[serde(default)]
    experience_tiers: Vec<ExperienceTier>,
    #[serde(default)]
    deprecated: Option<String>,
}

fn d_attack_interval() -> u16 {
    20
}
fn d_reach() -> f32 {
    1.0
}
fn d_frontal_arc() -> f32 {
    120.0
}
fn d_morale_base() -> f32 {
    60.0
}
fn d_one_f32() -> f32 {
    1.0
}
fn d_los() -> f32 {
    80.0
}
fn d_one_u16() -> u16 {
    1
}
fn d_regiment_size() -> u16 {
    120
}
fn d_one_u8() -> u8 {
    1
}

/// A unit type. Reals are `S`; references are kept as ContentIds and, after
/// `resolve`, as handles.
#[derive(Clone, Debug, PartialEq)]
pub struct UnitType {
    pub id: ContentId,
    pub name_key: String,
    pub category: UnitCategory,
    pub soldier_radius: S,
    pub mass: S,
    pub hp: S,
    pub speed_walk: S,
    pub speed_run: S,
    /// Defaults to `speed_walk`.
    pub speed_march: S,
    pub attack: S,
    pub defence: S,
    pub armour: S,
    pub damage: S,
    pub attack_interval_ticks: u16,
    pub reach: S,
    pub charge_bonus: S,
    pub anti_cavalry_bonus: S,
    pub second_rank_attack: bool,
    pub shield: bool,
    pub frontal_arc_deg: S,
    pub ranged: Option<Ranged>,
    pub morale_base: S,
    pub fatigue_rate_mult: S,
    pub los_radius: S,
    /// Ability ContentIds (kind arrives in Phase 2, so ids only).
    pub abilities: Vec<ContentId>,
    /// Formation templates this unit may use; the first is the default.
    pub formation_ids: Vec<ContentId>,
    pub formations: Vec<Handle<FormationTemplate>>,
    pub sprite_set_id: ContentId,
    sprite_set: Option<Handle<SpriteSet>>,
    pub sounds: UnitSounds,
    pub cost: u32,
    pub upkeep: u32,
    pub recruit_turns: u16,
    pub regiment_size: u16,
    pub tier: u8,
    pub experience_tiers: Vec<ExperienceTier>,
    pub deprecated: Option<String>,
}

impl UnitType {
    /// The sprite set handle; set by `resolve`, which every loaded item
    /// passes through.
    pub fn sprite_set(&self) -> Handle<SpriteSet> {
        self.sprite_set.expect("resolved at load")
    }

    /// The default formation template (the first listed).
    pub fn default_formation(&self) -> Handle<FormationTemplate> {
        self.formations[0]
    }
}

impl<'de> Deserialize<'de> for UnitType {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = Raw::deserialize(d)?;
        Ok(UnitType {
            id: r.id,
            name_key: r.name_key,
            category: r.category,
            soldier_radius: s(r.soldier_radius.unwrap_or(0.4)),
            mass: s(r.mass.unwrap_or(1.0)),
            hp: s(r.hp),
            speed_walk: s(r.speed_walk),
            speed_run: s(r.speed_run),
            speed_march: s(r.speed_march.unwrap_or(r.speed_walk)),
            attack: s(r.attack),
            defence: s(r.defence),
            armour: s(r.armour),
            damage: s(r.damage),
            attack_interval_ticks: r.attack_interval_ticks,
            reach: s(r.reach),
            charge_bonus: s(r.charge_bonus),
            anti_cavalry_bonus: s(r.anti_cavalry_bonus),
            second_rank_attack: r.second_rank_attack,
            shield: r.shield,
            frontal_arc_deg: s(r.frontal_arc_deg),
            ranged: r.ranged,
            morale_base: s(r.morale_base),
            fatigue_rate_mult: s(r.fatigue_rate_mult),
            los_radius: s(r.los_radius),
            abilities: r.abilities,
            formation_ids: r.formations,
            formations: Vec::new(),
            sprite_set_id: r.sprite_set,
            sprite_set: None,
            sounds: r.sounds,
            cost: r.cost,
            upkeep: r.upkeep,
            recruit_turns: r.recruit_turns,
            regiment_size: r.regiment_size,
            tier: r.tier,
            experience_tiers: r.experience_tiers,
            deprecated: r.deprecated,
        })
    }
}

impl ContentKind for UnitType {
    const DIR: &'static str = "units";
    const TAG: KindTag = KindTag::Unit;

    fn id(&self) -> &ContentId {
        &self.id
    }

    fn resolve(&mut self, lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        self.formations.clear();
        for (i, id) in self.formation_ids.iter().enumerate() {
            match lookup.handle::<FormationTemplate>(id) {
                Some(h) => self.formations.push(h),
                None => errors.push(ResolveError::new(
                    format!("formations[{i}]"),
                    id.clone(),
                    KindTag::Formation,
                )),
            }
        }
        match lookup.handle::<SpriteSet>(&self.sprite_set_id) {
            Some(h) => self.sprite_set = Some(h),
            None => errors.push(ResolveError::new(
                "sprite_set",
                self.sprite_set_id.clone(),
                KindTag::SpriteSet,
            )),
        }
        if self.formation_ids.is_empty() {
            errors.push(
                ResolveError::new("formations", self.id.clone(), KindTag::Formation)
                    .with_message("a unit needs at least one formation"),
            );
        }
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        h.write(&self.category);
        h.write(&self.soldier_radius);
        h.write(&self.mass);
        h.write(&self.hp);
        h.write(&self.speed_walk);
        h.write(&self.speed_run);
        h.write(&self.speed_march);
        h.write(&self.attack);
        h.write(&self.defence);
        h.write(&self.armour);
        h.write(&self.damage);
        h.write_u16(self.attack_interval_ticks);
        h.write(&self.reach);
        h.write(&self.charge_bonus);
        h.write(&self.anti_cavalry_bonus);
        h.write(&self.second_rank_attack);
        h.write(&self.shield);
        h.write(&self.frontal_arc_deg);
        match &self.ranged {
            None => h.write_u8(0),
            Some(r) => {
                h.write_u8(1);
                h.write(&r.range);
                h.write(&r.min_range);
                h.write(&r.accuracy);
                h.write(&r.projectile_speed);
                h.write_u16(r.reload_ticks);
                h.write_u16(r.ammo);
                h.write(&r.damage);
                h.write(&r.armour_penetration);
                h.write(&r.arc);
            }
        }
        h.write(&self.morale_base);
        h.write(&self.fatigue_rate_mult);
        h.write(&self.los_radius);
        h.write(&self.abilities);
        h.write(&self.formation_ids);
        h.write_u32(self.cost);
        h.write_u32(self.upkeep);
        h.write_u16(self.recruit_turns);
        h.write_u16(self.regiment_size);
        h.write_u8(self.tier);
        h.write_u32(self.experience_tiers.len() as u32);
        for t in &self.experience_tiers {
            h.write_u32(t.xp);
            h.write(&t.attack);
            h.write(&t.defence);
            h.write(&t.morale);
        }
    }
}

#[allow(dead_code, reason = "d_one is shared with other kinds")]
fn _use_d_one() -> S {
    d_one()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json5::{FileId, parse_json5};
    use il_core::Scalar;

    fn from_json5<T: serde::de::DeserializeOwned>(src: &str) -> Result<T, serde_json::Error> {
        serde_json::from_value(parse_json5(src, FileId(0)).unwrap().to_json())
    }

    #[test]
    fn defaults_and_optional_blocks() {
        let u: UnitType = from_json5(
            r#"{ id: "rome:x", name_key: "u.x", category: "cavalry", hp: 100,
                speed_walk: 1.5, speed_run: 4, attack: 30, defence: 20, damage: 25,
                formations: ["rome:line"], sprite_set: "rome:sprites_cavalry", cost: 1, upkeep: 1,
                ranged: { range: 40, accuracy: 0.5, projectile_speed: 20, reload_ticks: 80, ammo: 8, damage: 30 } }"#,
        )
        .unwrap();
        let s = S::from_f32_data;
        assert_eq!(u.category, UnitCategory::Cavalry);
        assert_eq!(u.soldier_radius, s(0.4));
        assert_eq!(u.mass, s(1.0));
        assert_eq!(u.speed_march, s(1.5), "defaults to speed_walk");
        assert_eq!(u.morale_base, s(60.0));
        assert_eq!(u.los_radius, s(80.0));
        assert_eq!(u.attack_interval_ticks, 20);
        assert_eq!(u.regiment_size, 120);
        let r = u.ranged.as_ref().unwrap();
        assert_eq!(r.arc, ProjectileArc::Direct);
        assert_eq!(r.min_range, s(0.0));
        assert_eq!(u.formation_ids.len(), 1);
        assert!(u.formations.is_empty(), "handles arrive with resolve");
    }

    #[test]
    fn missing_required_field_fails() {
        let err =
            from_json5::<UnitType>(r#"{ id: "rome:x", name_key: "u.x", category: "infantry" }"#)
                .unwrap_err();
        assert!(err.to_string().contains("hp"), "{err}");
    }
}
