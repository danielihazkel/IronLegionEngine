//! `MapDef`: a battle map definition (Modding SDK §6.1, `map-def.schema.json`).
//! Geometry is parsed here and the 16-bit heightmap sidecar is read by the
//! pipeline into `HeightmapRef::samples` (il_data is the only crate that
//! touches the filesystem); rasterisation happens in `il_sim_battle::map`
//! when a battle loads the map (T1-030).

use il_core::{S, Scalar, StateHasher, V2};
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
    /// Raw samples of the sidecar, row-major from `y = 0`, `cols × rows`
    /// (see [`MapDef::heightmap_dims`]); filled by the load pipeline.
    #[serde(skip)]
    pub samples: Vec<u16>,
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
    /// Zone type of the ground outside every polygon.
    pub base_zone: ContentId,
    #[serde(skip)]
    pub base_zone_handle: Option<Handle<ZoneType>>,
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

/// Smallest integer not below `v` (`v >= 0`).
fn ceil_u32(v: S) -> u32 {
    let floor = v.floor_i32();
    let up = if S::from_i32(floor) < v {
        floor + 1
    } else {
        floor
    };
    up.max(0) as u32
}

impl MapDef {
    /// `(cols, rows)` of the heightmap: `ceil(w / cell) + 1` by
    /// `ceil(h / cell) + 1` samples, so the last sample sits on or past the
    /// far edge.
    pub fn heightmap_dims(&self) -> (u32, u32) {
        let cell = self.heightmap.cell;
        (
            ceil_u32(self.size.w / cell) + 1,
            ceil_u32(self.size.h / cell) + 1,
        )
    }

    /// The resolved base zone; `resolve` always fills it on a valid map.
    pub fn base_zone(&self) -> Handle<ZoneType> {
        self.base_zone_handle
            .expect("MapDef::resolve fills base_zone_handle")
    }
}

impl ContentKind for MapDef {
    const DIR: &'static str = "maps";
    const TAG: KindTag = KindTag::Map;

    fn id(&self) -> &ContentId {
        &self.id
    }

    fn resolve(&mut self, lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        match lookup.handle::<ZoneType>(&self.base_zone) {
            Some(h) => self.base_zone_handle = Some(h),
            None => errors.push(ResolveError::new(
                "base_zone",
                self.base_zone.clone(),
                KindTag::Zone,
            )),
        }
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
        h.write_u32(self.heightmap.samples.len() as u32);
        for s in &self.heightmap.samples {
            h.write_u16(*s);
        }
        h.write(&self.base_zone);
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
