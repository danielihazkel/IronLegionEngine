//! Workspace benchmarks (`cargo bench -p il_benches`): criterion
//! micro-benches under `benches/benches/`, one per hot sim path. Shared
//! generators live here.
#![allow(clippy::float_arithmetic)]

use bevy_ecs::entity::Entity;
use il_core::{S, Scalar, SoldierId, V2};
use il_sim_battle::GridEntry;

/// Deterministic pseudo-random positions (no `rand`): `n` soldiers spread
/// over a `width × height` metre field.
pub fn scattered_soldiers(n: u32, width: i32, height: i32, seed: u64) -> Vec<GridEntry<SoldierId>> {
    let mut state = seed;
    let mut unit = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        S::from_i32(((state >> 40) & 0xffff) as i32) / S::from_i32(0x1_0000)
    };
    (0..n)
        .map(|i| GridEntry {
            id: SoldierId(i),
            entity: Entity::PLACEHOLDER,
            pos: V2::new(unit() * S::from_i32(width), unit() * S::from_i32(height)),
        })
        .collect()
}
