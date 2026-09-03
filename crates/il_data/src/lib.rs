//! Iron Legion data: JSON5 loading, registries, handles, diagnostics (TDD §3).
//!
//! Phase 1 builds the full content framework here: a span-carrying JSON5
//! parser, manifests, discovery and load order (T1-020); schema validation
//! (T1-021); override and merge semantics (T1-022); every registry, the
//! rules and the content hash (T1-023); localisation (T1-024); hot reload
//! (T1-025). The only crate that touches the filesystem at load.

pub mod content_id;
pub mod diagnostic;
pub mod discover;
pub mod handle;
pub mod json5;
pub mod load_order;
pub mod loader;
pub mod manifest;
pub mod merge;
pub mod pipeline;
pub mod registry;
pub mod schema;
pub mod source;
pub mod text;
pub mod unit_type;
pub mod validate;

pub use content_id::{ContentId, InvalidContentId};
pub use diagnostic::{Diagnostic, Diagnostics, Severity};
pub use discover::discover;
pub use handle::Handle;
pub use load_order::{Edge, EdgeKind, LoadOrderError, LoadedMod, ModSet, resolve_load_order};
pub use loader::Registries;
pub use manifest::{Dependency, Manifest, ManifestWithPath, read_manifest};
pub use merge::{KindAccumulator, MergedItem, Tombstone};
pub use pipeline::{load, load_roots};
pub use registry::{ContentKind, DuplicateId, Registry};
pub use schema::KindTag;
pub use source::{SourceFile, Sources};
pub use unit_type::{UnitCategory, UnitType};
pub use validate::{validate_merged, validate_value};
