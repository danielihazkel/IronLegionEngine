//! `MapDef`: a battle map definition (Modding SDK §6.1, `map-def.schema.json`).
//! Geometry is parsed here; the heightmap sidecar and rasterisation happen in
//! `il_sim_battle` when a battle loads the map (T1-030).

use il_core::{S, StateHasher, V2};
use serde::{Deserialize, Serialize};

use crate::content_id::ContentId;
use crate::de::{de_points, de_s, s};
use crate::handle::Handle;
use crate::registry::{ContentKind, Lookup, ResolveError};
use crate::schema::KindTag;
use crate::zone::ZoneType;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct MapSize {
    #[serde(deserialize_with = "de_s")]
    pub w: S,
    #[serde(deserialize_with = "de_s")]
    pub h: S,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct HeightmapRef {
    /// Metres per sample.
    #[serde(deserialize_with = "de_s")]
    pub cell: S,
    /// `.hgt` path under the mod's assets root.
    pub path: String,
    /// Metres per raw 16-bit unit.
    #[serde(deserialize_with = "de_s", default = "d_scale")]
    pub scale: S,
}

fn d_scale() -> S {
    s(0.01)
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ZonePolygon {
    #[serde(rename = "type")]
    pub type_id: ContentId,
    #[serde(skip)]
    pub zone: Option<Handle<ZoneType>>,
    #[serde(deserialize_with = "de_points")]
    pub polygon: Vec<V2>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct River {
    #[serde(deserialize_with = "de_s")]
    pub width: S,
    #[serde(deserialize_with = "de_points")]
    pub points: Vec<V2>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct DeploymentZone {
    pub side: u8,
    #[serde(deserialize_with = "de_points")]
    pub polygon: Vec<V2>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapEdge {
    North,
    South,
    East,
    West,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ReinforcementEdge {
    pub side: u8,
    pub edge: MapEdge,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct MapDef {
    pub id: ContentId,
    pub name_key: String,
    pub size: MapSize,
    #[serde(default)]
    pub campaign_terrain_tags: Vec<String>,
    #[serde(default = "d_weather")]
    pub weather_allowed: Vec<String>,
    pub heightmap: HeightmapRef,
    #[serde(default)]
    pub zones: Vec<ZonePolygon>,
    #[serde(default)]
    pub rivers: Vec<River>,
    pub deployment: Vec<DeploymentZone>,
    #[serde(default)]
    pub reinforcement_edges: Vec<ReinforcementEdge>,
    /// Reserved for sieges (REQ-SIM-045); stored inert.
    #[serde(default)]
    pub structures: Vec<serde_json::Value>,
    #[serde(default)]
    pub siege_points: Vec<serde_json::Value>,
    #[serde(default)]
    pub deprecated: Option<String>,
}

fn d_weather() -> Vec<String> {
    vec!["clear".to_string()]
}

impl ContentKind for MapDef {
    const DIR: &'static str = "maps";
    const TAG: KindTag = KindTag::Map;

    fn id(&self) -> &ContentId {
        &self.id
    }

    fn resolve(&mut self, lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        for (i, z) in self.zones.iter_mut().enumerate() {
            match lookup.handle::<ZoneType>(&z.type_id) {
                Some(h) => z.zone = Some(h),
                None => errors.push(ResolveError::new(
                    format!("zones[{i}].type"),
                    z.type_id.clone(),
                    KindTag::Zone,
                )),
            }
        }
    }

    fn hash_content(&self, h: &mut StateHasher) {
        h.write(&self.id);
        h.write(&self.size.w);
        h.write(&self.size.h);
        h.write(&self.heightmap.cell);
        h.write(&self.heightmap.scale);
        h.write_bytes(self.heightmap.path.as_bytes());
        h.write_u8(0);
        h.write_u32(self.zones.len() as u32);
        for z in &self.zones {
            h.write(&z.type_id);
            h.write(&z.polygon);
        }
        h.write_u32(self.rivers.len() as u32);
        for r in &self.rivers {
            h.write(&r.width);
            h.write(&r.points);
        }
        h.write_u32(self.deployment.len() as u32);
        for d in &self.deployment {
            h.write_u8(d.side);
            h.write(&d.polygon);
        }
        h.write_u32(self.reinforcement_edges.len() as u32);
        for e in &self.reinforcement_edges {
            h.write_u8(e.side);
            h.write_u8(e.edge as u8);
        }
    }
}
