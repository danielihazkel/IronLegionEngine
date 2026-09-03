//! Iron Legion renderer (`il_render`, TDD §10, REQ-TECH-003).
//!
//! Phase 1 scope: window surface and device (T1-050), instanced sprites
//! (T1-051), isometric camera and interpolation (T1-052), terrain (T1-053),
//! debug overlays (T1-054), and the egui paint pass (T1-060). The renderer
//! only ever reads simulation state through `BattleView` (SAD §5.2).

pub mod atlas;
pub mod camera;
mod egui_pass;
mod renderer;
pub mod scene;
pub mod snapshot;
pub mod sprite;

pub use atlas::{Atlas, AtlasError, AtlasId, anim_column, atlas_path};
pub use camera::Camera;
pub use egui_pass::EguiPaint;
pub use renderer::{ClearColour, RenderError, Renderer};
pub use scene::{SetAtlas, scene_from_snapshot, side_tint};
pub use snapshot::{
    EntityCounts, RegimentBlock, RenderSnapshot, SnapshotInput, SoldierInst, build_snapshot,
};
pub use sprite::{SpriteBatch, SpriteInstance, SpriteScene};
