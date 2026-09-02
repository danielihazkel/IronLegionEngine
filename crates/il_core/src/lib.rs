//! Iron Legion core: ids, `Scalar`, `Vec2`/`Angle`, deterministic hash and RNG,
//! tick and turn, event base (TDD §2).

pub mod angle;
pub mod hash;
pub mod ids;
pub mod scalar;
pub mod time;
pub mod vec;

pub use angle::Angle;
pub use hash::{Hashable, StateHash, StateHasher, hash_of};
pub use ids::*;
pub use scalar::{S, Scalar};
pub use time::*;
pub use vec::{V2, Vec2};
