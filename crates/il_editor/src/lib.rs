//! Iron Legion editors (`il_editor`, TDD §16, SAD §5.2, REQ-TOOL-004,
//! REQ-MOD-009).
//!
//! The map editor (Phase 3): a [`MapDocument`] over an `il_data::MapDef`
//! and its heightmap, an undo [`History`], the [`EditorSession`] the app
//! shows behind `AppState::Editor`, the picker screen that opens a registry
//! map or a blank one, and the save that writes `content/maps/<id>.json5`
//! and `assets/maps/<id>.hgt` into a mod folder exactly as `il_cli genmap`
//! does (`il_data::write_map`). A presentation crate: it reads the sim only
//! for `LoadedMap` and `NavGrid`, draws through `il_render` and `il_ui`, and
//! is the one presentation crate that writes files.

pub mod document;
pub mod panels;
pub mod picker;
pub mod session;

pub use document::{BlankMap, EditorError, History, MapDocument, Saved};
pub use picker::{PickerAction, PickerState, picker_screen};
pub use session::{EditorEffect, EditorInput, EditorSession, Tool};
