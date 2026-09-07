//! Iron Legion audio (TDD §12, REQ-AUD-001, REQ-AUD-002; T2-100).
//!
//! The sim never calls audio: the app hands each frame's `BattleEvent`s to
//! the [`EventRouter`], which turns them into rate-limited, distance- and
//! zoom-weighted [`PlayRequest`]s for an [`AudioSink`]. The one real sink is
//! the kira-backed [`AudioEngine`]; the [`NullSink`] records what it was
//! asked and serves the tests and machines without a device.

pub mod engine;
pub mod router;
pub mod sink;

pub use engine::AudioEngine;
pub use router::{EventRouter, FrameInput, RegimentPos, near_weight, roar_gain};
pub use sink::{AudioSink, NullSink, PlayRequest, SampleId, Volumes};
