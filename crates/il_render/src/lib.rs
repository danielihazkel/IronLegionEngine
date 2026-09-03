//! Iron Legion renderer (`il_render`, TDD §10, REQ-TECH-003).
//!
//! Phase 1 scope: window surface and device (T1-050), instanced sprites
//! (T1-051), isometric camera and interpolation (T1-052), terrain (T1-053),
//! debug overlays (T1-054), and the egui paint pass (T1-060). The renderer
//! only ever reads simulation state through `BattleView` (SAD §5.2).

pub mod atlas;
mod renderer;
pub mod sprite;

pub use atlas::{Anim, Atlas, AtlasError, AtlasId, SpriteSheet};
pub use renderer::{ClearColour, RenderError, Renderer};
pub use sprite::{SpriteBatch, SpriteInstance, SpriteScene};
