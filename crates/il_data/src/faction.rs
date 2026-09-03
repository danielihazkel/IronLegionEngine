//! `Faction` (Modding SDK §4.3, `faction.schema.json`). Phase 1 keeps the
//! campaign-side references (`ai_profile`, `tech_tree`) as ContentIds; their
//! kinds arrive in Phase 2 and Phase 4.

use il_core::{S, StateHasher};
use serde::Deserialize;

use crate::content_id::ContentId;
use crate::de::{Rgb, de_s, s};
use crate::handle::Handle;
use crate::registry::{ContentKind, Lookup, ResolveError};
use crate::schema::KindTag;
use crate::unit_type::UnitType;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct DiplomacyPersonality {
    #[serde(deserialize_with = "de_s", default = "d_half")]
    pub aggression: S,
    #[serde(deserialize_with = "de_s", default = "d_half")]
    pub loyalty: S,
    #[serde(deserialize_with = "de_s", default = "d_half")]
    pub greed: S,
    #[serde(deserialize_with = "de_s", default = "d_half")]
    pub expansionism: S,
}

fn d_half() -> S {
    s(0.5)
}

impl Default for DiplomacyPersonality {
    fn default() -> Self {
        Self {
            aggression: d_half(),
            loyalty: d_half(),
            greed: d_half(),
            expansionism: d_half(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Faction {
    pub id: ContentId,
    pub name_key: String,
    pub culture: String,
    pub colour_primary: Rgb,
    pub colour_secondary: Rgb,
    #[serde(rename = "units")]
    pub unit_ids: Vec<ContentId>,
    #[serde(skip)]
    pub units: Vec<Handle<UnitType>>,
    #[serde(default)]
    pub starting_provinces: Vec<String>,
    pub ai_profile: ContentId,
    #[serde(default)]
    pub diplomacy_personality: DiplomacyPersonality,
    pub tech_tree: ContentId,
    #[serde(default)]
    pub deprecated: Option<String>,
}

impl ContentKind for Faction {
    const DIR: &'static str = "factions";
    const TAG: KindTag = KindTag::Faction;

    fn id(&self) -> &ContentId {
        &self.id
    }

    fn resolve(&mut self, lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        self.units.clear();
        for (i, id) in self.unit_ids.iter().enumerate() {
            match lookup.handle::<UnitType>(id) {
                Some(h) => self.units.push(h),
                None => errors.push(ResolveError::new(
                    format!("units[{i}]"),
                    id.clone(),
                    KindTag::Unit,
                )),
            }
        }
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        h.write_bytes(self.culture.as_bytes());
        h.write_u8(0);
        h.write_bytes(&self.colour_primary.0);
        h.write_bytes(&self.colour_secondary.0);
        h.write(&self.unit_ids);
        h.write(&self.ai_profile);
        h.write(&self.diplomacy_personality.aggression);
        h.write(&self.diplomacy_personality.loyalty);
        h.write(&self.diplomacy_personality.greed);
        h.write(&self.diplomacy_personality.expansionism);
        h.write(&self.tech_tree);
    }
}
