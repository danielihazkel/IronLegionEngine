//! Iron Legion data: JSON5 loading, registries, handles, diagnostics (TDD §3).
//!
//! Phase 0 scope: a single mod root, the `units` kind with the fields the
//! sim reads, and diagnostics with file, line and column. Manifests, load
//! order, schema validation, overrides, locale and hot reload are Phase 1.

pub mod content_id;
pub mod diagnostic;
pub mod handle;
pub mod loader;
pub mod registry;
pub mod unit_type;

pub use content_id::{ContentId, InvalidContentId};
pub use diagnostic::{Diagnostic, Diagnostics};
pub use handle::Handle;
pub use loader::Registries;
pub use registry::{ContentKind, DuplicateId, Registry};
pub use unit_type::{UnitCategory, UnitType};
