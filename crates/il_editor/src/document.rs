//! The map document (T3-060, TDD §16): an `il_data::MapDef` with its
//! heightmap in metres, the source file's header comment, and the save
//! that writes both files into a mod folder through the writer `il_cli
//! genmap` uses (`il_data::write_map`), so a map opened and saved unchanged
//! is byte-identical to its source. `History` is the undo stack of whole
//! documents.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use il_core::{S, Scalar, V2};
use il_data::json5::{FileId, parse_json5};
use il_data::{
    ContentId, ContentKind, DeploymentZone, HeightmapRef, MapDef, MapEdge, MapSize, Registries,
    ReinforcementEdge, heightmap_path_for, read_manifest, write_map,
};
use il_sim_battle::{LoadedMap, MapError};

/// The header a map written by the editor carries when its source had none.
pub const EDITOR_HEADER: &str = "// Written by the Iron Legion map editor.";
/// Metres per raw unit of a new map's heightmap (the `genmap` value).
pub const DEFAULT_SCALE: f32 = 0.01;
/// The blank map's deployment bands: margin from the edge and depth
/// (the `genmap --preset plains` layout).
pub const BAND_MARGIN: f32 = 40.0;
pub const BAND_DEPTH: f32 = 160.0;

/// What the picker's New form asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct BlankMap {
    pub id: ContentId,
    /// Width and height in metres.
    pub size: [f32; 2],
    /// Metres per height sample.
    pub height_cell: f32,
    pub base_zone: ContentId,
}

#[derive(Debug, thiserror::Error)]
pub enum EditorError {
    #[error("map {0} is not in the loaded content")]
    UnknownMap(ContentId),
    #[error("zone type {0} is not in the loaded content")]
    UnknownZone(ContentId),
    #[error("{root} is not a mod folder: {reason}")]
    NotAMod { root: PathBuf, reason: String },
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Map(#[from] MapError),
}

/// The two files a save wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub json5: PathBuf,
    pub hgt: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapDocument {
    /// The definition; zone handles resolved by [`resolve_zones`]
    /// (`Self::resolve_zones`), `heightmap.samples` unused (see `heights`).
    pub def: MapDef,
    /// `height_cols × height_rows` metres, row-major from `y = 0`.
    pub heights: Vec<f32>,
    /// The source file's leading `//` lines, written back verbatim.
    pub header: Vec<String>,
    /// Edited since the last save (or since it was created).
    pub dirty: bool,
}

/// The leading `//` comment lines of a JSON5 file's text.
fn header_of(text: &str) -> Vec<String> {
    text.lines()
        .take_while(|l| l.trim_start().starts_with("//"))
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect()
}

/// Whether the parsed content file `text` defines `id` (an object or an
/// array of objects).
fn defines(text: &str, id: &ContentId) -> bool {
    let Ok(value) = parse_json5(text, FileId(0)) else {
        return false;
    };
    let has = |v: &il_data::json5::SpannedValue| {
        v.get("id").and_then(|i| i.as_str()) == Some(id.as_str())
    };
    match value.as_array() {
        Some(items) => items.iter().any(has),
        None => has(&value),
    }
}

/// The header of the file that last defines `id` under the mod roots'
/// `content/maps/` (the last root wins, the merge order).
pub fn source_header(mod_roots: &[PathBuf], id: &ContentId) -> Option<Vec<String>> {
    let mut found = None;
    for root in mod_roots {
        let content = match read_manifest(root, false) {
            Ok(m) => root.join(&m.manifest.content_root),
            Err(_) => root.join("content"),
        };
        let dir = content.join(MapDef::DIR);
        let mut files = Vec::new();
        if il_data::loader::json5_files(&dir, &mut files).is_err() {
            continue;
        }
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            if defines(&text, id) {
                found = Some(header_of(&text));
            }
        }
    }
    found
}

impl MapDocument {
    /// The registry's map, its heights in metres, and its source header.
    pub fn from_registry(
        regs: &Registries,
        id: &ContentId,
        mod_roots: &[PathBuf],
    ) -> Result<Self, EditorError> {
        let handle = regs
            .maps
            .lookup(id)
            .ok_or_else(|| EditorError::UnknownMap(id.clone()))?;
        let mut def = regs.maps.get(handle).clone();
        let scale = def.heightmap.scale.to_f32_render();
        let heights = def
            .heightmap
            .samples
            .iter()
            .map(|&raw| f32::from(raw) * scale)
            .collect();
        def.heightmap.samples = Vec::new();
        let header =
            source_header(mod_roots, id).unwrap_or_else(|| vec![EDITOR_HEADER.to_string()]);
        Ok(Self {
            def,
            heights,
            header,
            dirty: false,
        })
    }

    /// A flat map of `blank.size` with one deployment band per side along
    /// the south and north edges and their reinforcement edges.
    pub fn blank(blank: &BlankMap, regs: &Registries) -> Result<Self, EditorError> {
        let s = S::from_f32_data;
        let [w, h] = blank.size;
        let (m, d) = (BAND_MARGIN, BAND_DEPTH.min(h * 0.25));
        let band = |y0: f32, y1: f32| -> Vec<V2> {
            vec![
                V2::from_f32_data(m, y0),
                V2::from_f32_data(w - m, y0),
                V2::from_f32_data(w - m, y1),
                V2::from_f32_data(m, y1),
            ]
        };
        let item = blank.id.as_str().split_once(':').map_or("map", |(_, i)| i);
        let namespace = blank.id.as_str().split_once(':').map_or("map", |(n, _)| n);
        let def = MapDef {
            id: blank.id.clone(),
            name_key: format!("{namespace}.maps.{item}.name"),
            size: MapSize { w: s(w), h: s(h) },
            campaign_terrain_tags: Vec::new(),
            weather_allowed: ["clear", "rain", "fog"]
                .into_iter()
                .map(str::to_string)
                .collect(),
            heightmap: HeightmapRef {
                cell: s(blank.height_cell),
                path: heightmap_path_for(item),
                scale: s(DEFAULT_SCALE),
                samples: Vec::new(),
            },
            base_zone: blank.base_zone.clone(),
            base_zone_handle: None,
            zones: Vec::new(),
            rivers: Vec::new(),
            deployment: vec![
                DeploymentZone {
                    side: 0,
                    polygon: band(m, m + d),
                },
                DeploymentZone {
                    side: 1,
                    polygon: band(h - m - d, h - m),
                },
            ],
            reinforcement_edges: vec![
                ReinforcementEdge {
                    side: 0,
                    edge: MapEdge::South,
                },
                ReinforcementEdge {
                    side: 1,
                    edge: MapEdge::North,
                },
            ],
            structures: Vec::new(),
            siege_points: Vec::new(),
            deprecated: None,
        };
        let (cols, rows) = def.heightmap_dims();
        let mut doc = Self {
            def,
            heights: vec![0.0; cols as usize * rows as usize],
            header: vec![EDITOR_HEADER.to_string()],
            dirty: true,
        };
        if let Some(unknown) = doc.resolve_zones(regs).into_iter().next() {
            return Err(EditorError::UnknownZone(unknown));
        }
        Ok(doc)
    }

    /// The item half of the id (`test_field` of `rome:test_field`).
    pub fn item(&self) -> &str {
        self.def
            .id
            .as_str()
            .split_once(':')
            .map_or(self.def.id.as_str(), |(_, i)| i)
    }

    /// `(cols, rows)` of `heights`.
    pub fn height_dims(&self) -> (u32, u32) {
        self.def.heightmap_dims()
    }

    /// Fills the base zone's and every polygon's handle from `regs`;
    /// returns the ids it could not find (their handles stay `None`).
    pub fn resolve_zones(&mut self, regs: &Registries) -> Vec<ContentId> {
        let mut unknown = Vec::new();
        self.def.base_zone_handle = regs.zones.lookup(&self.def.base_zone);
        if self.def.base_zone_handle.is_none() {
            unknown.push(self.def.base_zone.clone());
        }
        for z in &mut self.def.zones {
            z.zone = regs.zones.lookup(&z.type_id);
            if z.zone.is_none() && !unknown.contains(&z.type_id) {
                unknown.push(z.type_id.clone());
            }
        }
        unknown
    }

    /// The heights as the sidecar stores them: `round(h / scale)` clamped
    /// to 16 bits.
    pub fn quantised(&self) -> Vec<u16> {
        let scale = self.def.heightmap.scale.to_f32_render();
        self.heights
            .iter()
            .map(|h| (h / scale).round().clamp(0.0, 65_535.0) as u16)
            .collect()
    }

    /// The definition as the load pipeline would hold it: the quantised
    /// samples in place.
    pub fn def_with_samples(&self) -> MapDef {
        let mut def = self.def.clone();
        def.heightmap.samples = self.quantised();
        def
    }

    /// The document as the sim reads it (the terrain view and the nav
    /// preview); the zone handles must be resolved.
    pub fn to_loaded(&self, regs: &Registries) -> Result<LoadedMap, MapError> {
        let zone_cell = regs.rules.movement.zone_cell;
        let zone_cell = if zone_cell > S::ZERO {
            zone_cell
        } else {
            S::from_i32(2)
        };
        LoadedMap::from_def(&self.def_with_samples(), zone_cell)
    }

    /// The JSON5 text a save writes.
    pub fn text(&self) -> String {
        write_map(&self.def, &self.header)
    }

    /// Writes `<assets>/maps/<item>.hgt` then `<content>/maps/<item>.json5`
    /// under `mod_root` (its manifest names the two roots); the heightmap
    /// path follows the id. Clears `dirty`.
    pub fn save(&mut self, mod_root: &Path) -> Result<Saved, EditorError> {
        let manifest = read_manifest(mod_root, false).map_err(|d| EditorError::NotAMod {
            root: mod_root.to_path_buf(),
            reason: d.to_string().trim().to_string(),
        })?;
        let item = self.item().to_string();
        self.def.heightmap.path = heightmap_path_for(&item);
        let assets = mod_root
            .join(&manifest.manifest.assets_root)
            .join(MapDef::DIR);
        let content = mod_root
            .join(&manifest.manifest.content_root)
            .join(MapDef::DIR);
        let io = |path: &Path, source: std::io::Error| EditorError::Io {
            path: path.to_path_buf(),
            source,
        };
        std::fs::create_dir_all(&assets).map_err(|e| io(&assets, e))?;
        std::fs::create_dir_all(&content).map_err(|e| io(&content, e))?;
        let hgt = assets.join(format!("{item}.hgt"));
        let bytes: Vec<u8> = self
            .quantised()
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        std::fs::write(&hgt, bytes).map_err(|e| io(&hgt, e))?;
        let json5 = content.join(format!("{item}.json5"));
        std::fs::write(&json5, self.text()).map_err(|e| io(&json5, e))?;
        self.dirty = false;
        Ok(Saved { json5, hgt })
    }
}

/// Undo and redo over whole documents (TDD §16): one entry per gesture
/// (a brush stroke, a committed polygon, a metadata edit), never per frame.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: VecDeque<MapDocument>,
    redo: Vec<MapDocument>,
}

impl History {
    /// Steps kept.
    pub const CAP: usize = 64;

    pub fn new() -> Self {
        Self::default()
    }

    /// Records the document as it was before an edit; clears the redo side.
    pub fn push(&mut self, before: MapDocument) {
        if self.undo.len() == Self::CAP {
            self.undo.pop_front();
        }
        self.undo.push_back(before);
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Restores the previous document into `doc`; `false` when there is none.
    pub fn undo(&mut self, doc: &mut MapDocument) -> bool {
        let Some(mut before) = self.undo.pop_back() else {
            return false;
        };
        before.dirty = true;
        std::mem::swap(doc, &mut before);
        self.redo.push(before);
        true
    }

    /// Re-applies the last undone document; `false` when there is none.
    pub fn redo(&mut self, doc: &mut MapDocument) -> bool {
        let Some(mut after) = self.redo.pop() else {
            return false;
        };
        after.dirty = true;
        std::mem::swap(doc, &mut after);
        self.undo.push_back(after);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game")
    }

    fn regs() -> Registries {
        il_data::load_roots(&[game_root()]).unwrap_or_else(|d| panic!("{d}"))
    }

    fn test_field(regs: &Registries) -> MapDocument {
        let id = ContentId::new("rome:test_field").unwrap();
        MapDocument::from_registry(regs, &id, &[game_root()]).unwrap()
    }

    /// T3-060: the registry's test field writes back as the committed file
    /// and sidecar, byte for byte (a checkout may carry CRLF; the writer
    /// writes LF).
    #[test]
    fn the_test_field_round_trips_byte_for_byte() {
        let regs = regs();
        let doc = test_field(&regs);
        assert_eq!(doc.header.len(), 4, "{:?}", doc.header);
        assert!(doc.header[0].starts_with("// Generated by `il_cli genmap"));
        let json = std::fs::read_to_string(game_root().join("content/maps/test_field.json5"))
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(doc.text(), json);
        let hgt = std::fs::read(game_root().join("assets/maps/test_field.hgt")).unwrap();
        let bytes: Vec<u8> = doc
            .quantised()
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert_eq!(bytes, hgt);
        assert!(!doc.dirty);
        let loaded = doc.to_loaded(&regs).unwrap();
        assert_eq!((loaded.height_cols, loaded.height_rows), (201, 151));
        assert_eq!(loaded.zone_handles.len(), 7);
    }

    #[test]
    fn a_blank_map_has_two_bands_and_flat_ground() {
        let regs = regs();
        let blank = BlankMap {
            id: ContentId::new("mymod:flat").unwrap(),
            size: [400.0, 300.0],
            height_cell: 4.0,
            base_zone: ContentId::new("rome:open").unwrap(),
        };
        let doc = MapDocument::blank(&blank, &regs).unwrap();
        assert_eq!(doc.height_dims(), (101, 76));
        assert_eq!(doc.heights.len(), 101 * 76);
        assert!(doc.dirty);
        assert_eq!(doc.item(), "flat");
        assert_eq!(doc.def.heightmap.path, "maps/flat.hgt");
        assert_eq!(doc.def.name_key, "mymod.maps.flat.name");
        assert_eq!(doc.def.deployment.len(), 2);
        assert_eq!(doc.def.reinforcement_edges.len(), 2);
        // The north band ends 40 m short of the top edge and the bands are
        // a quarter of the map deep on a short map.
        let north = &doc.def.deployment[1].polygon;
        assert_eq!(north[2].y, S::from_f32_data(260.0));
        assert_eq!(north[0].y, S::from_f32_data(185.0));
        assert!(doc.def.base_zone_handle.is_some());
        let loaded = doc.to_loaded(&regs).unwrap();
        assert_eq!(loaded.height_at(V2::from_f32_data(100.0, 100.0)), S::ZERO);
        let text = doc.text();
        assert!(text.starts_with(EDITOR_HEADER));
        assert!(text.contains("  zones: [],\n  rivers: [],\n  deployment: [\n"));

        let bad = BlankMap {
            base_zone: ContentId::new("mymod:lava").unwrap(),
            ..blank
        };
        assert!(matches!(
            MapDocument::blank(&bad, &regs),
            Err(EditorError::UnknownZone(_))
        ));
    }

    #[test]
    fn history_caps_at_64_and_swaps_documents_back_and_forth() {
        let regs = regs();
        let mut doc = test_field(&regs);
        let mut history = History::new();
        assert!(!history.can_undo());
        for i in 0..70u32 {
            history.push(doc.clone());
            doc.def.campaign_terrain_tags = vec![format!("t{i}")];
        }
        assert_eq!(history.undo_len(), History::CAP);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.def.campaign_terrain_tags, ["t68"]);
        assert!(doc.dirty);
        assert!(history.can_redo());
        assert!(history.redo(&mut doc));
        assert_eq!(doc.def.campaign_terrain_tags, ["t69"]);
        assert!(!history.can_redo());
        // A new edit clears the redo side.
        history.undo(&mut doc);
        history.push(doc.clone());
        doc.def.campaign_terrain_tags.clear();
        assert!(!history.can_redo());
        // The oldest steps fell off the front: pushes 6 to 69 kept t5..t68,
        // then the undo and the new push left t5..t67 plus t68.
        let mut n = 0;
        while history.undo(&mut doc) {
            n += 1;
        }
        assert_eq!(n, History::CAP);
        assert_eq!(doc.def.campaign_terrain_tags, ["t5"]);
    }

    #[test]
    fn save_refuses_a_folder_without_a_manifest() {
        let regs = regs();
        let mut doc = test_field(&regs);
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/il_editor_test/not_a_mod");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(doc.save(&dir), Err(EditorError::NotAMod { .. })));
        assert!(!doc.dirty);
    }
}
