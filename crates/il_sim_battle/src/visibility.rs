//! Line of sight and fog of war (SIM-VIS-001..006, TDD §8.4; T2-060). The
//! state is declared here in T2-050 together with the milestone's hash and
//! snapshot layout; the systems and queries arrive with T2-060, until which
//! every mask is empty and every regiment counts as visible.

use bevy_ecs::prelude::*;
use il_core::{Angle, S, Tick, V2};
use serde::{Deserialize, Serialize};

/// SIM-VIS-005: what an observer side last saw of an enemy regiment (UI
/// ghosts only; stored in snapshots, never hashed, never read by the sim).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub pos: V2,
    pub facing: Angle<S>,
    pub count: u16,
    /// Tick of the last sighting; the memory expires `memory_ticks` later.
    pub tick: Tick,
}

/// Per side (outer index) and regiment (inner index into
/// `Ids.regiment_entities`): whether the side currently sees the regiment
/// (`masks`, hashed and snapshotted: it is refreshed every
/// `visibility.period_ticks`, so a snapshot between refreshes cannot
/// rederive it) and what it remembers (`memory`, snapshotted only).
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct Visibility {
    pub masks: Vec<Vec<bool>>,
    pub memory: Vec<Vec<Option<Seen>>>,
}

impl Visibility {
    /// Whether `side` sees the regiment at index `regiment`; `true` while no
    /// mask exists (before T2-060 and for indices past the mask).
    pub fn sees(&self, side: u8, regiment: usize) -> bool {
        self.masks
            .get(usize::from(side))
            .and_then(|m| m.get(regiment))
            .copied()
            .unwrap_or(true)
    }
}
