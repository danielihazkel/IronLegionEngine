//! Engine tunables (`content/rules/*.json5`, Simulation Spec §15.1). One
//! merged object per file across the mod set; every field is required, the
//! engine carries no numeric defaults (Phase 1 decision). `Rules::zeroed` is
//! for tests that never run a system that reads them.

use il_core::{S, Scalar, StateHasher};
use serde::Deserialize;

use crate::de::de_s;

/// Movement, pathfinding, steering, collision (Simulation Spec §5).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementRules {
    #[serde(deserialize_with = "de_s")]
    pub nav_cell: S,
    pub hpa_cluster: u16,
    pub hpa_gate_split: u16,
    /// Degrees per second.
    #[serde(deserialize_with = "de_s")]
    pub wheel_rate: S,
    #[serde(deserialize_with = "de_s")]
    pub waypoint_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub slot_arrive_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub slot_leave_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub sep_weight: S,
    #[serde(deserialize_with = "de_s")]
    pub sep_margin: S,
    pub sep_max_neighbours: u16,
    #[serde(deserialize_with = "de_s")]
    pub arrive_damping: S,
    pub lookahead_ticks: u16,
    /// Degrees per second.
    #[serde(deserialize_with = "de_s")]
    pub soldier_turn_rate: S,
    /// In file spacings.
    #[serde(deserialize_with = "de_s")]
    pub straggler_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub straggler_fraction: S,
    #[serde(deserialize_with = "de_s")]
    pub straggler_slowdown: S,
    #[serde(deserialize_with = "de_s")]
    pub slope_penalty: S,
    #[serde(deserialize_with = "de_s")]
    pub slope_bonus: S,
    #[serde(deserialize_with = "de_s")]
    pub slope_min_mult: S,
    #[serde(deserialize_with = "de_s")]
    pub slope_max_mult: S,
    pub collision_iterations: u16,
    pub paths_per_tick: u16,
    #[serde(deserialize_with = "de_s")]
    pub ford_defence_mult: S,
    #[serde(deserialize_with = "de_s")]
    pub spatial_cell: S,
    #[serde(deserialize_with = "de_s")]
    pub anchor_cell: S,
    #[serde(deserialize_with = "de_s")]
    pub zone_cell: S,
}

/// Formations (Simulation Spec §4).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormationRules {
    #[serde(deserialize_with = "de_s")]
    pub keep_slot_radius: S,
    /// In file spacings.
    #[serde(deserialize_with = "de_s")]
    pub integrity_radius: S,
    pub integrity_period_ticks: u16,
    #[serde(deserialize_with = "de_s")]
    pub integrity_morale_threshold: S,
    #[serde(deserialize_with = "de_s")]
    pub morph_speed_mult: S,
    #[serde(deserialize_with = "de_s")]
    pub group_gap: S,
    #[serde(deserialize_with = "de_s")]
    pub skirmish_offset: S,
    #[serde(deserialize_with = "de_s")]
    pub width_tolerance: S,
    #[serde(deserialize_with = "de_s")]
    pub assign_search_radius: S,
    pub swap_passes: u8,
    /// Degrees.
    #[serde(deserialize_with = "de_s")]
    pub reform_angle: S,
    /// Degrees.
    #[serde(deserialize_with = "de_s")]
    pub turn_in_place_angle: S,
}

/// Every rules file. Phase 2 adds combat, morale, fatigue, general,
/// visibility and battle_flow.
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub movement: MovementRules,
    pub formation: FormationRules,
}

impl Rules {
    /// All-zero rules for tests that build a world without content. Never a
    /// substitute for the flagship files: every real load requires them.
    pub fn zeroed() -> Self {
        let z = S::ZERO;
        Self {
            movement: MovementRules {
                nav_cell: z,
                hpa_cluster: 0,
                hpa_gate_split: 0,
                wheel_rate: z,
                waypoint_radius: z,
                slot_arrive_radius: z,
                slot_leave_radius: z,
                sep_weight: z,
                sep_margin: z,
                sep_max_neighbours: 0,
                arrive_damping: z,
                lookahead_ticks: 0,
                soldier_turn_rate: z,
                straggler_radius: z,
                straggler_fraction: z,
                straggler_slowdown: z,
                slope_penalty: z,
                slope_bonus: z,
                slope_min_mult: z,
                slope_max_mult: z,
                collision_iterations: 0,
                paths_per_tick: 0,
                ford_defence_mult: z,
                spatial_cell: z,
                anchor_cell: z,
                zone_cell: z,
            },
            formation: FormationRules {
                keep_slot_radius: z,
                integrity_radius: z,
                integrity_period_ticks: 0,
                integrity_morale_threshold: z,
                morph_speed_mult: z,
                group_gap: z,
                skirmish_offset: z,
                width_tolerance: z,
                assign_search_radius: z,
                swap_passes: 0,
                reform_angle: z,
                turn_in_place_angle: z,
            },
        }
    }

    /// Every tunable, in a fixed order (content registry hash).
    pub fn hash_content(&self, h: &mut StateHasher) {
        let m = &self.movement;
        h.write(&m.nav_cell);
        h.write_u16(m.hpa_cluster);
        h.write_u16(m.hpa_gate_split);
        h.write(&m.wheel_rate);
        h.write(&m.waypoint_radius);
        h.write(&m.slot_arrive_radius);
        h.write(&m.slot_leave_radius);
        h.write(&m.sep_weight);
        h.write(&m.sep_margin);
        h.write_u16(m.sep_max_neighbours);
        h.write(&m.arrive_damping);
        h.write_u16(m.lookahead_ticks);
        h.write(&m.soldier_turn_rate);
        h.write(&m.straggler_radius);
        h.write(&m.straggler_fraction);
        h.write(&m.straggler_slowdown);
        h.write(&m.slope_penalty);
        h.write(&m.slope_bonus);
        h.write(&m.slope_min_mult);
        h.write(&m.slope_max_mult);
        h.write_u16(m.collision_iterations);
        h.write_u16(m.paths_per_tick);
        h.write(&m.ford_defence_mult);
        h.write(&m.spatial_cell);
        h.write(&m.anchor_cell);
        h.write(&m.zone_cell);
        let f = &self.formation;
        h.write(&f.keep_slot_radius);
        h.write(&f.integrity_radius);
        h.write_u16(f.integrity_period_ticks);
        h.write(&f.integrity_morale_threshold);
        h.write(&f.morph_speed_mult);
        h.write(&f.group_gap);
        h.write(&f.skirmish_offset);
        h.write(&f.width_tolerance);
        h.write(&f.assign_search_radius);
        h.write_u8(f.swap_passes);
        h.write(&f.reform_angle);
        h.write(&f.turn_in_place_angle);
    }
}

/// Key bindings (`content/input/bindings.json5`, TDD §11). Plain data here;
/// `il_ui` maps chord strings to key codes.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputBindings {
    pub bindings: Vec<Binding>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub action: String,
    pub keys: Vec<String>,
}

impl InputBindings {
    /// The chords bound to `action`, in file order.
    pub fn keys_for(&self, action: &str) -> &[String] {
        self.bindings
            .iter()
            .find(|b| b.action == action)
            .map_or(&[], |b| b.keys.as_slice())
    }
}
