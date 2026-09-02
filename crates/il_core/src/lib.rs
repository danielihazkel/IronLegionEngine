//! Iron Legion core: ids, `Scalar`, `Vec2`/`Angle`, deterministic hash and RNG,
//! tick and turn, event base (TDD §2).

pub mod ids;
pub mod time;

pub use ids::*;
pub use time::*;
