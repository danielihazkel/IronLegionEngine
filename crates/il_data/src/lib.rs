//! Iron Legion data: JSON5 loading, registries, handles, diagnostics (TDD §3).
//!
//! Phase 1 builds the full content framework here: a span-carrying JSON5
//! parser, manifests, discovery and load order (T1-020); schema validation
//! (T1-021); override and merge semantics (T1-022); every registry, the
//! rules and the content hash (T1-023); localisation (T1-024); hot reload
//! (T1-025). The only crate that touches the filesystem at load.

pub mod content_id;
pub mod de;
pub mod diagnostic;
pub mod discover;
pub mod faction;
pub mod formation;
pub mod handle;
#[cfg(feature = "hot-reload")]
pub mod hot_reload;
pub mod json5;
pub mod load_order;
pub mod loader;
pub mod locale;
pub mod manifest;
pub mod map_def;
pub mod merge;
pub mod pipeline;
pub mod registries;
pub mod registry;
pub mod rules;
pub mod schema;
pub mod source;
pub mod sprite_set;
pub mod text;
pub mod unit_type;
pub mod validate;
pub mod zone;

pub use content_id::{ContentId, InvalidContentId};
pub use de::Rgb;
pub use diagnostic::{Diagnostic, Diagnostics, Severity};
pub use discover::discover;
pub use faction::{DiplomacyPersonality, Faction};
pub use formation::{FormationTemplate, GroupFormationTemplate, GroupKind, Layout, RoleZone};
pub use handle::Handle;
pub use load_order::{Edge, EdgeKind, LoadOrderError, LoadedMod, ModSet, resolve_load_order};
pub use locale::{FALLBACK_LANGUAGE, Locale};
pub use manifest::{Dependency, Manifest, ManifestWithPath, read_manifest};
pub use map_def::{
    DeploymentZone, HeightmapRef, MapDef, MapEdge, MapSize, ReinforcementEdge, River, ZonePolygon,
};
pub use merge::{KindAccumulator, MergedItem, Tombstone};
pub use pipeline::{discover_set, load, load_report, load_roots};
pub use registries::{ModInfo, Registries};
pub use registry::{ContentKind, DuplicateId, Lookup, Registry, ResolveError};
pub use rules::{Binding, FormationRules, InputBindings, MovementRules, Rules};
pub use schema::KindTag;
pub use source::{SourceFile, Sources};
pub use sprite_set::{Anim, SpriteSet};
pub use unit_type::{ExperienceTier, ProjectileArc, Ranged, UnitCategory, UnitSounds, UnitType};
pub use validate::{validate_merged, validate_value};
pub use zone::ZoneType;
