//! Iron Legion core: ids, `Scalar`, `Vec2`/`Angle`, deterministic hash and RNG,
//! tick and turn, event base (TDD §2).

pub mod angle;
pub mod events;
pub mod hash;
pub mod ids;
pub mod rng;
pub mod scalar;
pub mod time;
pub mod vec;

pub use angle::Angle;
pub use events::{Event, EventQueue};
pub use hash::{Hashable, StateHash, StateHasher, hash_of};
pub use ids::*;
pub use rng::{RngStream, StreamId, hash_draw};
pub use scalar::{F32, S, Scalar};
pub use time::*;
pub use vec::{V2, Vec2};
