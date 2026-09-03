//! `Registries`: every content kind after a load (TDD §3.2), plus the two
//! hashes peers and saves compare (Networking Spec §4.2, SAD §5.3).

use il_core::StateHasher;

use crate::faction::Faction;
use crate::formation::{FormationTemplate, GroupFormationTemplate};
use crate::map_def::MapDef;
use crate::registry::{ContentKind, Registry};
use crate::rules::{InputBindings, Rules};
use crate::sprite_set::SpriteSet;
use crate::unit_type::UnitType;
use crate::zone::ZoneType;

/// One loaded mod, for save headers and the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModInfo {
    pub id: String,
    pub version: String,
}

#[derive(Debug)]
pub struct Registries {
    pub units: Registry<UnitType>,
    pub formations: Registry<FormationTemplate>,
    pub group_formations: Registry<GroupFormationTemplate>,
    pub factions: Registry<Faction>,
    pub zones: Registry<ZoneType>,
    pub maps: Registry<MapDef>,
    pub sprite_sets: Registry<SpriteSet>,
    pub rules: Rules,
    pub input: InputBindings,
    /// Mods in load order.
    pub mods: Vec<ModInfo>,
    /// xxh3 over `(id, version)` in load order.
    pub mod_list_hash: u64,
    /// xxh3 over every sim-relevant content field, kinds in a fixed order and
    /// items in ContentId order; independent of file order, whitespace, key
    /// order and registry layout.
    pub content_registry_hash: u64,
}

impl Default for Registries {
    /// Empty registries with zeroed rules: for tests and tools that build a
    /// `BattleWorld` without content. Real loads go through the pipeline.
    fn default() -> Self {
        Self {
            units: Registry::new(),
            formations: Registry::new(),
            group_formations: Registry::new(),
            factions: Registry::new(),
            zones: Registry::new(),
            maps: Registry::new(),
            sprite_sets: Registry::new(),
            rules: Rules::zeroed(),
            input: InputBindings::default(),
            mods: Vec::new(),
            mod_list_hash: 0,
            content_registry_hash: 0,
        }
    }
}

fn hash_registry<T: ContentKind>(reg: &Registry<T>, h: &mut StateHasher) {
    h.write_u32(reg.len() as u32);
    for (_, item) in reg.iter() {
        item.hash_content(h);
    }
}

impl Registries {
    /// Recomputes `content_registry_hash` from the typed content.
    pub fn compute_content_hash(&self) -> u64 {
        let mut h = StateHasher::new();
        hash_registry(&self.units, &mut h);
        hash_registry(&self.formations, &mut h);
        hash_registry(&self.group_formations, &mut h);
        hash_registry(&self.factions, &mut h);
        hash_registry(&self.zones, &mut h);
        hash_registry(&self.maps, &mut h);
        self.rules.hash_content(&mut h);
        h.finish().0
    }
}
