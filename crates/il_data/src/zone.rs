//! `ZoneType`: terrain zone types (SIM-MOVE-031..032, `zone-type.schema.json`).

use il_core::{S, StateHasher};
use serde::Deserialize;

use crate::content_id::ContentId;
use crate::de::{Rgb, d_one, d_true, de_s};
use crate::registry::ContentKind;
use crate::schema::KindTag;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ZoneType {
    pub id: ContentId,
    pub name_key: String,
    /// Soldier speed multiplier inside the zone.
    #[serde(deserialize_with = "de_s")]
    pub move_mult: S,
    /// Pathfinding cost multiplier per nav cell (at least 1).
    #[serde(deserialize_with = "de_s")]
    pub move_cost: S,
    #[serde(deserialize_with = "de_s", default = "d_one")]
    pub los_mult: S,
    #[serde(default)]
    pub conceal: bool,
    #[serde(deserialize_with = "de_s", default = "d_one")]
    pub fatigue_mult: S,
    #[serde(deserialize_with = "de_s", default = "d_one")]
    pub formation_integrity_mult: S,
    #[serde(default = "d_true")]
    pub passable: bool,
    pub colour: Rgb,
    #[serde(default)]
    pub deprecated: Option<String>,
}

impl ContentKind for ZoneType {
    const DIR: &'static str = "zones";
    const TAG: KindTag = KindTag::Zone;

    fn id(&self) -> &ContentId {
        &self.id
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        h.write(&self.move_mult);
        h.write(&self.move_cost);
        h.write(&self.los_mult);
        h.write(&self.conceal);
        h.write(&self.fatigue_mult);
        h.write(&self.formation_integrity_mult);
        h.write(&self.passable);
    }
}
