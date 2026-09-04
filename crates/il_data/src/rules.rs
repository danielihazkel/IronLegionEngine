//! Engine tunables (`content/rules/*.json5`, Simulation Spec §15.1). One
//! merged object per file across the mod set; every field is required, the
//! engine carries no numeric defaults (Phase 1 decision). `Rules::zeroed` is
//! for tests that never run a system that reads them.
//!
//! Phase 2 (T2-010) adds `combat`, `morale`, `fatigue`, `general`,
//! `visibility` and `battle_flow`; the `ai.*` tunables of §15.1 belong to
//! the `AiProfile` content kind (T2-080), not to a rules file.

use il_core::{S, Scalar, StateHasher};
use serde::{Deserialize, Deserializer};

use crate::de::de_s;

/// `[S; 3]` from three JSON numbers.
fn de_s3<'de, D: Deserializer<'de>>(d: D) -> Result<[S; 3], D::Error> {
    let v = <[f32; 3]>::deserialize(d)?;
    Ok(v.map(S::from_f32_data))
}

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

/// Melee, charges, terrain defence, experience, pursuit and projectiles
/// (Simulation Spec §6, TDD §8.1).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CombatRules {
    // SIM-CMBT-011 hit roll.
    #[serde(deserialize_with = "de_s")]
    pub base_hit: S,
    #[serde(deserialize_with = "de_s")]
    pub hit_scale: S,
    #[serde(deserialize_with = "de_s")]
    pub min_hit: S,
    #[serde(deserialize_with = "de_s")]
    pub max_hit: S,
    /// SIM-CMBT-013 damage floor.
    #[serde(deserialize_with = "de_s")]
    pub min_damage: S,
    // SIM-CMBT-002 targeting.
    #[serde(deserialize_with = "de_s")]
    pub engage_radius: S,
    pub retarget_period_ticks: u16,
    #[serde(deserialize_with = "de_s")]
    pub reach_slack: S,
    // SIM-CMBT-004, SIM-CMBT-015 charges.
    pub charge_window_ticks: u16,
    #[serde(deserialize_with = "de_s")]
    pub charge_dmg_share: S,
    #[serde(deserialize_with = "de_s")]
    pub charge_distance: S,
    /// Mass multiplier of charging soldiers in collision push resolution.
    #[serde(deserialize_with = "de_s")]
    pub charge_mass_mult: S,
    #[serde(deserialize_with = "de_s")]
    pub brace_integrity: S,
    // SIM-CMBT-014 arcs.
    #[serde(deserialize_with = "de_s")]
    pub flank_dmg_mult: S,
    #[serde(deserialize_with = "de_s")]
    pub rear_dmg_mult: S,
    #[serde(deserialize_with = "de_s")]
    pub flank_def_mult: S,
    #[serde(deserialize_with = "de_s")]
    pub rear_def_mult: S,
    // SIM-CMBT-016, SIM-PROJ-002 height.
    #[serde(deserialize_with = "de_s")]
    pub height_defence: S,
    #[serde(deserialize_with = "de_s")]
    pub height_range: S,
    #[serde(deserialize_with = "de_s")]
    pub height_ref: S,
    /// SIM-CMBT-012.
    #[serde(deserialize_with = "de_s")]
    pub second_rank_reach_bonus: S,
    /// SIM-CMBT-017.
    #[serde(deserialize_with = "de_s")]
    pub exp_step: S,
    /// SIM-MOR-034.
    #[serde(deserialize_with = "de_s")]
    pub pursuit_hit_mult: S,
    /// SIM-CMBT-004.
    pub pursue_repath_ticks: u16,
    /// SIM-CORE-008 (render-only corpse lifetime).
    pub corpse_ticks: u16,
    /// SIM-CMBT-005: an `AttackMove` regiment acquires the nearest enemy
    /// regiment whose anchor lies within this radius of its own.
    #[serde(deserialize_with = "de_s")]
    pub attack_move_radius: S,
    // SIM-PROJ-001..009 (read from T2-030).
    pub projectile_cap: u32,
    #[serde(deserialize_with = "de_s")]
    pub projectile_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub scatter_scale: S,
    #[serde(deserialize_with = "de_s")]
    pub direct_apex: S,
    #[serde(deserialize_with = "de_s")]
    pub gravity: S,
    #[serde(deserialize_with = "de_s")]
    pub shield_mult: S,
    #[serde(deserialize_with = "de_s")]
    pub stat_hit_base: S,
    #[serde(deserialize_with = "de_s")]
    pub friendly_block_dist: S,
    pub volley: bool,
    pub ranged_retarget_ticks: u16,
}

/// SIM-MOR-004: multipliers of one morale state.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateMults {
    #[serde(deserialize_with = "de_s")]
    pub attack: S,
    #[serde(deserialize_with = "de_s")]
    pub defence: S,
    #[serde(deserialize_with = "de_s")]
    pub interval: S,
    #[serde(deserialize_with = "de_s")]
    pub speed: S,
}

/// SIM-MOR-004: one `StateMults` per morale state.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateMultsTable {
    pub steady: StateMults,
    pub unsettled: StateMults,
    pub shaken: StateMults,
    pub broken: StateMults,
    pub routing: StateMults,
}

impl StateMultsTable {
    /// The row for a `MoraleState` discriminant (`Steady = 0` ..
    /// `Routing = 4`); `Shattered` (5) and anything larger use the routing
    /// row. The sim crate owns the enum, so the discriminant crosses here.
    pub fn for_state(&self, discriminant: u8) -> &StateMults {
        match discriminant {
            0 => &self.steady,
            1 => &self.unsettled,
            2 => &self.shaken,
            3 => &self.broken,
            _ => &self.routing,
        }
    }
}

/// SIM-MOR-002 factor weights, points per second at full effect, in the
/// order of `morale_factors` (SIM-MOR-010..024 without the one-time shocks).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoraleWeights {
    #[serde(deserialize_with = "de_s")]
    pub casualty_rate: S,
    #[serde(deserialize_with = "de_s")]
    pub casualty_total: S,
    #[serde(deserialize_with = "de_s")]
    pub fatigue: S,
    #[serde(deserialize_with = "de_s")]
    pub general_aura: S,
    #[serde(deserialize_with = "de_s")]
    pub allies_near: S,
    #[serde(deserialize_with = "de_s")]
    pub allies_routing: S,
    #[serde(deserialize_with = "de_s")]
    pub high_ground: S,
    #[serde(deserialize_with = "de_s")]
    pub fear: S,
    #[serde(deserialize_with = "de_s")]
    pub flanked: S,
    #[serde(deserialize_with = "de_s")]
    pub outnumbered: S,
    #[serde(deserialize_with = "de_s")]
    pub integrity: S,
    #[serde(deserialize_with = "de_s")]
    pub engaged_duration: S,
    #[serde(deserialize_with = "de_s")]
    pub winning: S,
    #[serde(deserialize_with = "de_s")]
    pub recovery: S,
}

/// Morale value, states, factors, routing (Simulation Spec §7, TDD §8.3).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoraleRules {
    // SIM-MOR-003 thresholds.
    #[serde(deserialize_with = "de_s")]
    pub t_unsettled: S,
    #[serde(deserialize_with = "de_s")]
    pub t_shaken: S,
    #[serde(deserialize_with = "de_s")]
    pub t_broken: S,
    #[serde(deserialize_with = "de_s")]
    pub t_routing: S,
    #[serde(deserialize_with = "de_s")]
    pub hysteresis: S,
    // SIM-MOR-031..033 rally, shatter, contagion.
    #[serde(deserialize_with = "de_s")]
    pub rally_margin: S,
    #[serde(deserialize_with = "de_s")]
    pub rally_safe_radius: S,
    pub max_routs: u8,
    #[serde(deserialize_with = "de_s")]
    pub shatter_strength: S,
    #[serde(deserialize_with = "de_s")]
    pub general_death_shock: S,
    #[serde(deserialize_with = "de_s")]
    pub rout_shock: S,
    #[serde(deserialize_with = "de_s")]
    pub rout_shock_radius: S,
    // SIM-MOR-025..026 one-time penalties.
    #[serde(deserialize_with = "de_s")]
    pub disengage_penalty: S,
    #[serde(deserialize_with = "de_s")]
    pub charged_penalty: S,
    // SIM-MOR-010..024 factor references.
    #[serde(deserialize_with = "de_s")]
    pub casualty_rate_ref: S,
    #[serde(deserialize_with = "de_s")]
    pub casualty_total_ref: S,
    #[serde(deserialize_with = "de_s")]
    pub fatigue_start: S,
    #[serde(deserialize_with = "de_s")]
    pub ally_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub allies_ref: S,
    #[serde(deserialize_with = "de_s")]
    pub routing_ref: S,
    #[serde(deserialize_with = "de_s")]
    pub outnumber_ref: S,
    pub engage_fatigue_ticks: u32,
    #[serde(deserialize_with = "de_s")]
    pub safe_radius: S,
    /// SIM-MOR-001.
    #[serde(deserialize_with = "de_s")]
    pub exp_bonus: S,
    pub w: MoraleWeights,
    pub state_mults: StateMultsTable,
}

/// Fatigue (Simulation Spec §8, TDD §8.3).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FatigueRules {
    // SIM-FAT-002 rates per second by activity.
    #[serde(deserialize_with = "de_s")]
    pub rate_idle: S,
    #[serde(deserialize_with = "de_s")]
    pub rate_walk: S,
    #[serde(deserialize_with = "de_s")]
    pub rate_march: S,
    #[serde(deserialize_with = "de_s")]
    pub rate_run: S,
    #[serde(deserialize_with = "de_s")]
    pub rate_fighting: S,
    #[serde(deserialize_with = "de_s")]
    pub rate_routing: S,
    /// Added to every positive rate per point of `unit.armour`.
    #[serde(deserialize_with = "de_s")]
    pub armour_rate: S,
    /// SIM-FAT-003: Fresh / Active / Tired upper bounds.
    #[serde(deserialize_with = "de_s3")]
    pub thresholds: [S; 3],
    // SIM-FAT-004 multipliers.
    #[serde(deserialize_with = "de_s")]
    pub speed_loss: S,
    #[serde(deserialize_with = "de_s")]
    pub attack_loss: S,
    #[serde(deserialize_with = "de_s")]
    pub defence_loss: S,
    #[serde(deserialize_with = "de_s")]
    pub interval_gain: S,
}

/// Generals and auras (Simulation Spec §9).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralRules {
    #[serde(deserialize_with = "de_s")]
    pub aura_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub aura_attack: S,
    /// Metres of aura radius per general rank (SIM-GEN-002).
    #[serde(deserialize_with = "de_s")]
    pub aura_per_rank: S,
    #[serde(deserialize_with = "de_s")]
    pub hp_mult: S,
    /// Fraction of hp below which the fate is `Wounded` (SIM-GEN-004).
    #[serde(deserialize_with = "de_s")]
    pub wounded_hp: S,
}

/// Line of sight and fog of war (Simulation Spec §11).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisibilityRules {
    pub period_ticks: u16,
    #[serde(deserialize_with = "de_s")]
    pub conceal_radius: S,
    #[serde(deserialize_with = "de_s")]
    pub height_bonus: S,
    #[serde(deserialize_with = "de_s")]
    pub eye_height: S,
    #[serde(deserialize_with = "de_s")]
    pub los_sample: S,
    pub memory_ticks: u32,
}

/// SIM-FLOW-013: who wins when the timer expires.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum TimeoutWinner {
    Defender = 0,
    MostSoldiers = 1,
}

/// Battle phases, timers and results (Simulation Spec §12).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BattleFlowRules {
    pub time_limit_ticks: u32,
    /// 0 = no deployment timeout (SIM-FLOW-011).
    pub deploy_timeout_ticks: u32,
    pub pursuit_ticks: u32,
    #[serde(deserialize_with = "de_s")]
    pub fled_return_fraction: S,
    pub timeout_winner: TimeoutWinner,
    #[serde(deserialize_with = "de_s")]
    pub exp_per_kill: S,
    #[serde(deserialize_with = "de_s")]
    pub exp_survive: S,
    #[serde(deserialize_with = "de_s")]
    pub loot_per_enemy_killed: S,
}

/// Every rules file (`content/rules/<name>.json5`).
#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub movement: MovementRules,
    pub formation: FormationRules,
    pub combat: CombatRules,
    pub morale: MoraleRules,
    pub fatigue: FatigueRules,
    pub general: GeneralRules,
    pub visibility: VisibilityRules,
    pub battle_flow: BattleFlowRules,
}

impl Rules {
    /// All-zero rules for tests that build a world without content. Never a
    /// substitute for the flagship files: every real load requires them.
    pub fn zeroed() -> Self {
        let z = S::ZERO;
        let mults = || StateMults {
            attack: z,
            defence: z,
            interval: z,
            speed: z,
        };
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
            combat: CombatRules {
                base_hit: z,
                hit_scale: z,
                min_hit: z,
                max_hit: z,
                min_damage: z,
                engage_radius: z,
                retarget_period_ticks: 0,
                reach_slack: z,
                charge_window_ticks: 0,
                charge_dmg_share: z,
                charge_distance: z,
                charge_mass_mult: z,
                brace_integrity: z,
                flank_dmg_mult: z,
                rear_dmg_mult: z,
                flank_def_mult: z,
                rear_def_mult: z,
                height_defence: z,
                height_range: z,
                height_ref: z,
                second_rank_reach_bonus: z,
                exp_step: z,
                pursuit_hit_mult: z,
                pursue_repath_ticks: 0,
                corpse_ticks: 0,
                attack_move_radius: z,
                projectile_cap: 0,
                projectile_radius: z,
                scatter_scale: z,
                direct_apex: z,
                gravity: z,
                shield_mult: z,
                stat_hit_base: z,
                friendly_block_dist: z,
                volley: false,
                ranged_retarget_ticks: 0,
            },
            morale: MoraleRules {
                t_unsettled: z,
                t_shaken: z,
                t_broken: z,
                t_routing: z,
                hysteresis: z,
                rally_margin: z,
                rally_safe_radius: z,
                max_routs: 0,
                shatter_strength: z,
                general_death_shock: z,
                rout_shock: z,
                rout_shock_radius: z,
                disengage_penalty: z,
                charged_penalty: z,
                casualty_rate_ref: z,
                casualty_total_ref: z,
                fatigue_start: z,
                ally_radius: z,
                allies_ref: z,
                routing_ref: z,
                outnumber_ref: z,
                engage_fatigue_ticks: 0,
                safe_radius: z,
                exp_bonus: z,
                w: MoraleWeights {
                    casualty_rate: z,
                    casualty_total: z,
                    fatigue: z,
                    general_aura: z,
                    allies_near: z,
                    allies_routing: z,
                    high_ground: z,
                    fear: z,
                    flanked: z,
                    outnumbered: z,
                    integrity: z,
                    engaged_duration: z,
                    winning: z,
                    recovery: z,
                },
                state_mults: StateMultsTable {
                    steady: mults(),
                    unsettled: mults(),
                    shaken: mults(),
                    broken: mults(),
                    routing: mults(),
                },
            },
            fatigue: FatigueRules {
                rate_idle: z,
                rate_walk: z,
                rate_march: z,
                rate_run: z,
                rate_fighting: z,
                rate_routing: z,
                armour_rate: z,
                thresholds: [z; 3],
                speed_loss: z,
                attack_loss: z,
                defence_loss: z,
                interval_gain: z,
            },
            general: GeneralRules {
                aura_radius: z,
                aura_attack: z,
                aura_per_rank: z,
                hp_mult: z,
                wounded_hp: z,
            },
            visibility: VisibilityRules {
                period_ticks: 0,
                conceal_radius: z,
                height_bonus: z,
                eye_height: z,
                los_sample: z,
                memory_ticks: 0,
            },
            battle_flow: BattleFlowRules {
                time_limit_ticks: 0,
                deploy_timeout_ticks: 0,
                pursuit_ticks: 0,
                fled_return_fraction: z,
                timeout_winner: TimeoutWinner::Defender,
                exp_per_kill: z,
                exp_survive: z,
                loot_per_enemy_killed: z,
            },
        }
    }

    /// Every tunable, in a fixed order (content registry hash): the files
    /// in `Rules` field order, each in struct field order.
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
        let c = &self.combat;
        h.write(&c.base_hit);
        h.write(&c.hit_scale);
        h.write(&c.min_hit);
        h.write(&c.max_hit);
        h.write(&c.min_damage);
        h.write(&c.engage_radius);
        h.write_u16(c.retarget_period_ticks);
        h.write(&c.reach_slack);
        h.write_u16(c.charge_window_ticks);
        h.write(&c.charge_dmg_share);
        h.write(&c.charge_distance);
        h.write(&c.charge_mass_mult);
        h.write(&c.brace_integrity);
        h.write(&c.flank_dmg_mult);
        h.write(&c.rear_dmg_mult);
        h.write(&c.flank_def_mult);
        h.write(&c.rear_def_mult);
        h.write(&c.height_defence);
        h.write(&c.height_range);
        h.write(&c.height_ref);
        h.write(&c.second_rank_reach_bonus);
        h.write(&c.exp_step);
        h.write(&c.pursuit_hit_mult);
        h.write_u16(c.pursue_repath_ticks);
        h.write_u16(c.corpse_ticks);
        h.write(&c.attack_move_radius);
        h.write_u32(c.projectile_cap);
        h.write(&c.projectile_radius);
        h.write(&c.scatter_scale);
        h.write(&c.direct_apex);
        h.write(&c.gravity);
        h.write(&c.shield_mult);
        h.write(&c.stat_hit_base);
        h.write(&c.friendly_block_dist);
        h.write(&c.volley);
        h.write_u16(c.ranged_retarget_ticks);
        let mo = &self.morale;
        h.write(&mo.t_unsettled);
        h.write(&mo.t_shaken);
        h.write(&mo.t_broken);
        h.write(&mo.t_routing);
        h.write(&mo.hysteresis);
        h.write(&mo.rally_margin);
        h.write(&mo.rally_safe_radius);
        h.write_u8(mo.max_routs);
        h.write(&mo.shatter_strength);
        h.write(&mo.general_death_shock);
        h.write(&mo.rout_shock);
        h.write(&mo.rout_shock_radius);
        h.write(&mo.disengage_penalty);
        h.write(&mo.charged_penalty);
        h.write(&mo.casualty_rate_ref);
        h.write(&mo.casualty_total_ref);
        h.write(&mo.fatigue_start);
        h.write(&mo.ally_radius);
        h.write(&mo.allies_ref);
        h.write(&mo.routing_ref);
        h.write(&mo.outnumber_ref);
        h.write_u32(mo.engage_fatigue_ticks);
        h.write(&mo.safe_radius);
        h.write(&mo.exp_bonus);
        let w = &mo.w;
        for v in [
            w.casualty_rate,
            w.casualty_total,
            w.fatigue,
            w.general_aura,
            w.allies_near,
            w.allies_routing,
            w.high_ground,
            w.fear,
            w.flanked,
            w.outnumbered,
            w.integrity,
            w.engaged_duration,
            w.winning,
            w.recovery,
        ] {
            h.write(&v);
        }
        let t = &mo.state_mults;
        for s in [&t.steady, &t.unsettled, &t.shaken, &t.broken, &t.routing] {
            h.write(&s.attack);
            h.write(&s.defence);
            h.write(&s.interval);
            h.write(&s.speed);
        }
        let fa = &self.fatigue;
        h.write(&fa.rate_idle);
        h.write(&fa.rate_walk);
        h.write(&fa.rate_march);
        h.write(&fa.rate_run);
        h.write(&fa.rate_fighting);
        h.write(&fa.rate_routing);
        h.write(&fa.armour_rate);
        for v in fa.thresholds {
            h.write(&v);
        }
        h.write(&fa.speed_loss);
        h.write(&fa.attack_loss);
        h.write(&fa.defence_loss);
        h.write(&fa.interval_gain);
        let g = &self.general;
        h.write(&g.aura_radius);
        h.write(&g.aura_attack);
        h.write(&g.aura_per_rank);
        h.write(&g.hp_mult);
        h.write(&g.wounded_hp);
        let v = &self.visibility;
        h.write_u16(v.period_ticks);
        h.write(&v.conceal_radius);
        h.write(&v.height_bonus);
        h.write(&v.eye_height);
        h.write(&v.los_sample);
        h.write_u32(v.memory_ticks);
        let b = &self.battle_flow;
        h.write_u32(b.time_limit_ticks);
        h.write_u32(b.deploy_timeout_ticks);
        h.write_u32(b.pursuit_ticks);
        h.write(&b.fled_return_fraction);
        h.write_u8(b.timeout_winner as u8);
        h.write(&b.exp_per_kill);
        h.write(&b.exp_survive);
        h.write(&b.loot_per_enemy_killed);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sf(v: f32) -> S {
        S::from_f32_data(v)
    }

    #[test]
    fn state_mults_table_maps_every_discriminant() {
        let mut r = Rules::zeroed();
        r.morale.state_mults.routing.speed = sf(1.1);
        r.morale.state_mults.steady.attack = S::ONE;
        assert_eq!(r.morale.state_mults.for_state(0).attack, S::ONE);
        assert_eq!(r.morale.state_mults.for_state(4).speed, sf(1.1));
        // Shattered and out-of-range discriminants fall back to routing.
        assert_eq!(r.morale.state_mults.for_state(5).speed, sf(1.1));
        assert_eq!(r.morale.state_mults.for_state(200).speed, sf(1.1));
    }

    #[test]
    fn thresholds_deserialise_as_three_scalars() {
        let json = r#"{"rate_idle":-0.01,"rate_walk":0.004,"rate_march":0.002,"rate_run":0.02,
            "rate_fighting":0.015,"rate_routing":0.02,"armour_rate":0.0002,
            "thresholds":[0.25,0.5,0.75],"speed_loss":0.3,"attack_loss":0.3,
            "defence_loss":0.2,"interval_gain":0.4}"#;
        let f: FatigueRules = serde_json::from_str(json).unwrap();
        assert_eq!(f.thresholds, [sf(0.25), sf(0.5), sf(0.75)]);
        assert_eq!(f.rate_idle, sf(-0.01));
    }

    #[test]
    fn every_file_rejects_unknown_fields() {
        let json = r#"{"aura_radius":60,"aura_attack":0.05,"aura_per_rank":5,"hp_mult":3,
            "wounded_hp":0.3,"extra":1}"#;
        let err = serde_json::from_str::<GeneralRules>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field `extra`"), "{err}");
        let json = r#"{"time_limit_ticks":48000,"deploy_timeout_ticks":0,"pursuit_ticks":2400,
            "fled_return_fraction":0.5,"timeout_winner":"most_soldiers","exp_per_kill":0.01,
            "exp_survive":1,"loot_per_enemy_killed":10}"#;
        let b: BattleFlowRules = serde_json::from_str(json).unwrap();
        assert_eq!(b.timeout_winner, TimeoutWinner::MostSoldiers);
    }

    #[test]
    fn hash_covers_every_phase_2_field() {
        let mut h = il_core::StateHasher::new();
        Rules::zeroed().hash_content(&mut h);
        let base = h.finish();
        for mutate in [
            (|r: &mut Rules| r.combat.attack_move_radius = S::ONE) as fn(&mut Rules),
            |r| r.combat.volley = true,
            |r| r.morale.w.recovery = S::ONE,
            |r| r.morale.state_mults.broken.interval = S::ONE,
            |r| r.fatigue.thresholds[2] = S::ONE,
            |r| r.general.wounded_hp = S::ONE,
            |r| r.visibility.memory_ticks = 1,
            |r| r.battle_flow.timeout_winner = TimeoutWinner::MostSoldiers,
        ] {
            let mut r = Rules::zeroed();
            mutate(&mut r);
            let mut h = il_core::StateHasher::new();
            r.hash_content(&mut h);
            assert_ne!(h.finish(), base);
        }
    }
}
