//! `FormationTemplate` and `GroupFormationTemplate` (TDD §7, Modding SDK §4.2,
//! `formation-template.schema.json`, `group-formation.schema.json`).

use il_core::{S, StateHasher, V2, impl_hashable_fieldless_enum};
use serde::{Deserialize, Serialize};

use crate::content_id::ContentId;
use crate::de::{d_one, d_true, d_zero, de_s, de_xy_points, s};
use crate::registry::ContentKind;
use crate::schema::KindTag;
use crate::unit_type::UnitCategory;

/// Names the engine layout function (SIM-FORM-003..009).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Layout {
    Line = 0,
    Column = 1,
    Square = 2,
    Wedge = 3,
    Phalanx = 4,
    Loose = 5,
    Custom = 6,
}
impl_hashable_fieldless_enum!(Layout);

/// Ranks (1-based, inclusive, from the front) reserved for a category in a
/// mixed regiment (SIM-FORM-011, Phase 3).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct RoleZone {
    pub unit_category: UnitCategory,
    pub ranks_from: u8,
    pub ranks_to: u8,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct FormationTemplate {
    pub id: ContentId,
    pub name_key: String,
    pub layout: Layout,
    pub default_ranks: u8,
    #[serde(default = "d_one_u8")]
    pub min_ranks: u8,
    #[serde(default = "d_sixteen")]
    pub max_ranks: u8,
    /// Multiples of the soldier diameter (SIM-FORM-002).
    #[serde(deserialize_with = "de_s", default = "d_one")]
    pub spacing_file: S,
    #[serde(deserialize_with = "de_s", default = "d_spacing_rank")]
    pub spacing_rank: S,
    #[serde(default)]
    pub role_zones: Vec<RoleZone>,
    #[serde(default = "d_morph_ticks")]
    pub morph_ticks: u16,
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub integrity_bonus_attack: S,
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub integrity_bonus_defence: S,
    #[serde(deserialize_with = "de_s", default = "d_one")]
    pub speed_mult: S,
    /// Offsets in soldier diameters, x right, y forward (SIM-FORM-009).
    #[serde(deserialize_with = "de_xy_points", default)]
    pub custom_slots: Vec<V2>,
    #[serde(default = "d_two")]
    pub min_files: u8,
    #[serde(deserialize_with = "de_s", default = "d_loose_mult")]
    pub loose_mult: S,
    #[serde(default = "d_four")]
    pub default_files_column: u8,
    #[serde(default)]
    pub deprecated: Option<String>,
}

fn d_one_u8() -> u8 {
    1
}
fn d_two() -> u8 {
    2
}
fn d_four() -> u8 {
    4
}
fn d_sixteen() -> u8 {
    16
}
fn d_morph_ticks() -> u16 {
    60
}
fn d_spacing_rank() -> S {
    s(1.2)
}
fn d_loose_mult() -> S {
    s(2.0)
}

impl ContentKind for FormationTemplate {
    const DIR: &'static str = "formations";
    const TAG: KindTag = KindTag::Formation;

    fn id(&self) -> &ContentId {
        &self.id
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        h.write(&self.layout);
        h.write_u8(self.default_ranks);
        h.write_u8(self.min_ranks);
        h.write_u8(self.max_ranks);
        h.write(&self.spacing_file);
        h.write(&self.spacing_rank);
        h.write_u32(self.role_zones.len() as u32);
        for z in &self.role_zones {
            h.write(&z.unit_category);
            h.write_u8(z.ranks_from);
            h.write_u8(z.ranks_to);
        }
        h.write_u16(self.morph_ticks);
        h.write(&self.integrity_bonus_attack);
        h.write(&self.integrity_bonus_defence);
        h.write(&self.speed_mult);
        h.write(&self.custom_slots);
        h.write_u8(self.min_files);
        h.write(&self.loose_mult);
        h.write_u8(self.default_files_column);
    }
}

/// Multi-regiment arrangements (SIM-FORM-040).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum GroupKind {
    BattleLine = 0,
    DoubleLine = 1,
    EchelonLeft = 2,
    EchelonRight = 3,
    RefusedLeft = 4,
    RefusedRight = 5,
    Custom = 6,
}
impl_hashable_fieldless_enum!(GroupKind);

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct GroupFormationTemplate {
    pub id: ContentId,
    pub name_key: String,
    pub kind: GroupKind,
    /// Metres between regiments; zero means `formation.group_gap`.
    #[serde(deserialize_with = "de_s", default = "d_zero")]
    pub gap: S,
    #[serde(default = "d_true")]
    pub skirmishers_forward: bool,
    #[serde(default = "d_true")]
    pub cavalry_flanks: bool,
    #[serde(default = "d_one_u8")]
    pub lines: u8,
    #[serde(default)]
    pub deprecated: Option<String>,
}

impl ContentKind for GroupFormationTemplate {
    const DIR: &'static str = "group_formations";
    const TAG: KindTag = KindTag::GroupFormation;

    fn id(&self) -> &ContentId {
        &self.id
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        h.write(&self.kind);
        h.write(&self.gap);
        h.write(&self.skirmishers_forward);
        h.write(&self.cavalry_flanks);
        h.write_u8(self.lines);
    }
}
