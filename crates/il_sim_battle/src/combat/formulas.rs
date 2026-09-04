//! Pure melee formulas (T2-021; SIM-CMBT-010..017, SIM-FAT-004,
//! SIM-MOR-004, TDD §8.1). Every tunable comes from `Rules`; the fatigue,
//! morale, status and aura multipliers are wired here so the systems that
//! produce them (T2-040, T2-041, T2-050, T2-043) only have to feed values.

use il_core::{Angle, S, Scalar, V2};
use il_data::{CombatRules, FatigueRules, MoraleRules, MovementRules, StateMults};

use crate::components::{MoraleState, OrderKind};
use crate::movement::regiment::deg_to_rad;

/// SIM-CMBT-014: where an attack comes from, seen by the defender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Arc {
    Front = 0,
    Flank = 1,
    Rear = 2,
}

/// SIM-CMBT-014: attacks from more than this many degrees off the
/// defender's facing are rear attacks; the frontal arc is per unit.
pub const FLANK_HALF_ARC_DEG: i32 = 150;

/// SIM-CMBT-011: `clamp(base_hit + hit_scale × (A − D) / (A + D), min_hit, max_hit)`.
pub fn hit_probability(a: S, d: S, r: &CombatRules) -> S {
    let sum = a + d;
    let p = if sum > S::ZERO {
        r.base_hit + r.hit_scale * (a - d) / sum
    } else {
        r.base_hit
    };
    p.clamp(r.min_hit, r.max_hit)
}

/// SIM-CMBT-013: `max(dmg × mults − armour × (1 − pen), min_damage)`.
pub fn melee_damage(dmg: S, armour: S, pen: S, mults: S, r: &CombatRules) -> S {
    (dmg * mults - armour * (S::ONE - pen)).max(r.min_damage)
}

/// SIM-CMBT-014: the arc of an attacker at `to_attacker` (defender to
/// attacker) against a defender facing `defender_facing`; a coincident
/// attacker counts as frontal.
pub fn attack_arc(defender_facing: Angle<S>, to_attacker: V2, frontal_arc_deg: S) -> Arc {
    if to_attacker == V2::ZERO {
        return Arc::Front;
    }
    let off = defender_facing
        .delta(Angle::from_direction(to_attacker))
        .abs();
    if off <= deg_to_rad(frontal_arc_deg * S::HALF) {
        Arc::Front
    } else if off <= deg_to_rad(S::from_i32(FLANK_HALF_ARC_DEG)) {
        Arc::Flank
    } else {
        Arc::Rear
    }
}

/// SIM-CMBT-014: `(damage multiplier, defence multiplier)` of an arc.
pub fn arc_mults(arc: Arc, r: &CombatRules) -> (S, S) {
    match arc {
        Arc::Front => (S::ONE, S::ONE),
        Arc::Flank => (r.flank_dmg_mult, r.flank_def_mult),
        Arc::Rear => (r.rear_dmg_mult, r.rear_def_mult),
    }
}

/// SIM-FAT-004 multipliers at fatigue `f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FatigueMults {
    pub speed: S,
    pub attack: S,
    pub defence: S,
    pub interval: S,
}

/// SIM-FAT-004: continuous in `f` (`1` everywhere at `f = 0`).
pub fn fatigue_mults(f: S, r: &FatigueRules) -> FatigueMults {
    FatigueMults {
        speed: S::ONE - r.speed_loss * f,
        attack: S::ONE - r.attack_loss * f,
        defence: S::ONE - r.defence_loss * f,
        interval: S::ONE + r.interval_gain * f,
    }
}

/// SIM-MOR-004: the multipliers of a morale state (Shattered reads the
/// routing row).
pub fn morale_mults(state: MoraleState, r: &MoraleRules) -> &StateMults {
    r.state_mults.for_state(state as u8)
}

/// SIM-ABIL-005 placeholder until T2-050: no status effects exist.
pub fn status_mult() -> S {
    S::ONE
}

/// SIM-GEN-002 placeholder until T2-043: no auras exist.
pub fn aura_attack_mult() -> S {
    S::ONE
}

/// SIM-CMBT-017: `1 + exp_step × experience`.
pub fn experience_mult(experience: u8, r: &CombatRules) -> S {
    S::ONE + r.exp_step * S::from_i32(i32::from(experience))
}

/// SIM-CMBT-015: `(charge_mult, charge_dmg_mult)`; both `1` outside the
/// window or when a braced anti-cavalry defender negates the charge.
pub fn charge_mults(charge_bonus: S, charging: bool, negated: bool, r: &CombatRules) -> (S, S) {
    if charging && !negated {
        (
            S::ONE + charge_bonus,
            S::ONE + charge_bonus * r.charge_dmg_share,
        )
    } else {
        (S::ONE, S::ONE)
    }
}

/// SIM-CMBT-015: a unit with an anti-cavalry bonus is braced when it is
/// not moving (an Idle order, or engaged), its integrity is at least
/// `brace_integrity`, and the attack comes through its frontal arc.
pub fn braced(
    anti_cavalry_bonus: S,
    integrity: S,
    order: OrderKind,
    engaged: bool,
    arc: Arc,
    r: &CombatRules,
) -> bool {
    anti_cavalry_bonus > S::ZERO
        && integrity >= r.brace_integrity
        && (order == OrderKind::Idle || engaged)
        && arc == Arc::Front
}

/// SIM-CMBT-016: `zone.defence_mult × ford_mult × (1 + height_defence ×
/// sat((h_j − h_i) / height_ref))` with `sat` clamping to `[−1, 1]`;
/// `h_j` is the defender's height.
pub fn terrain_defence_mult(
    zone_defence_mult: S,
    ford: bool,
    h_j: S,
    h_i: S,
    movement: &MovementRules,
    r: &CombatRules,
) -> S {
    let ford_mult = if ford {
        movement.ford_defence_mult
    } else {
        S::ONE
    };
    let sat = if r.height_ref > S::ZERO {
        ((h_j - h_i) / r.height_ref).clamp(-S::ONE, S::ONE)
    } else {
        S::ZERO
    };
    zone_defence_mult * ford_mult * (S::ONE + r.height_defence * sat)
}

/// SIM-CMBT-010: the attack interval scaled by the fatigue, morale and
/// status multipliers, rounded to the nearest tick, at least 2.
pub fn cooldown_ticks(base: u16, fatigue_interval: S, morale_interval: S, status: S) -> u16 {
    let x = S::from_i32(i32::from(base)) * fatigue_interval * morale_interval * status;
    let ticks = (x + S::HALF).floor_i32().max(2);
    u16::try_from(ticks).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sf(v: f32) -> S {
        S::from_f32_data(v)
    }

    fn rules() -> il_data::Rules {
        let mut r = il_data::Rules::zeroed();
        let c = &mut r.combat;
        c.base_hit = S::HALF;
        c.hit_scale = S::HALF;
        c.min_hit = sf(0.05);
        c.max_hit = sf(0.95);
        c.min_damage = S::ONE;
        c.flank_dmg_mult = sf(1.25);
        c.rear_dmg_mult = sf(1.5);
        c.flank_def_mult = sf(0.8);
        c.rear_def_mult = sf(0.6);
        c.height_defence = sf(0.15);
        c.height_ref = S::from_i32(5);
        c.exp_step = sf(0.03);
        c.charge_dmg_share = S::HALF;
        c.brace_integrity = sf(0.7);
        r.movement.ford_defence_mult = sf(0.7);
        let f = &mut r.fatigue;
        f.speed_loss = sf(0.3);
        f.attack_loss = sf(0.3);
        f.defence_loss = sf(0.2);
        f.interval_gain = sf(0.4);
        r.morale.state_mults.routing.attack = S::ZERO;
        r.morale.state_mults.routing.speed = sf(1.1);
        r
    }

    #[test]
    fn hit_probability_is_monotonic_and_clamped() {
        let r = rules();
        let c = &r.combat;
        assert_eq!(hit_probability(sf(30.0), sf(30.0), c), S::HALF);
        let mut last = S::ZERO;
        for a in 1..200 {
            let p = hit_probability(S::from_i32(a), sf(30.0), c);
            assert!(p >= last, "not monotonic in attack at {a}");
            last = p;
        }
        let mut last = S::ONE;
        for d in 1..200 {
            let p = hit_probability(sf(30.0), S::from_i32(d), c);
            assert!(p <= last, "not anti-monotonic in defence at {d}");
            last = p;
        }
        assert_eq!(hit_probability(sf(1000.0), S::ZERO, c), sf(0.95));
        assert_eq!(hit_probability(S::ZERO, sf(1000.0), c), sf(0.05));
        assert_eq!(hit_probability(S::ZERO, S::ZERO, c), S::HALF);
    }

    #[test]
    fn damage_has_a_floor_and_honours_penetration() {
        let r = rules();
        let c = &r.combat;
        assert_eq!(
            melee_damage(sf(30.0), sf(8.0), S::ZERO, S::ONE, c),
            sf(22.0)
        );
        assert_eq!(
            melee_damage(sf(30.0), sf(8.0), S::HALF, S::ONE, c),
            sf(26.0)
        );
        assert_eq!(melee_damage(sf(5.0), sf(80.0), S::ZERO, S::ONE, c), S::ONE);
        assert_eq!(
            melee_damage(sf(30.0), S::ZERO, S::ZERO, sf(1.5), c),
            sf(45.0)
        );
    }

    #[test]
    fn arcs_follow_the_frontal_arc_and_the_150_degree_flank_limit() {
        let facing = Angle::new(S::ZERO); // +x
        let from = |deg: f32| {
            let a: S = Angle::<S>::from_degrees_data(deg).radians();
            V2::new(a.cos(), a.sin())
        };
        let arc = |deg: f32| attack_arc(facing, from(deg), sf(120.0));
        assert_eq!(arc(0.0), Arc::Front);
        assert_eq!(arc(59.0), Arc::Front);
        assert_eq!(arc(-59.0), Arc::Front);
        assert_eq!(arc(61.0), Arc::Flank);
        assert_eq!(arc(149.0), Arc::Flank);
        assert_eq!(arc(-149.0), Arc::Flank);
        assert_eq!(arc(151.0), Arc::Rear);
        assert_eq!(arc(180.0), Arc::Rear);
        assert_eq!(attack_arc(facing, V2::ZERO, sf(120.0)), Arc::Front);
        let r = rules();
        assert_eq!(arc_mults(Arc::Rear, &r.combat), (sf(1.5), sf(0.6)));
    }

    #[test]
    fn charge_is_negated_only_by_a_braced_defender() {
        let r = rules();
        let c = &r.combat;
        assert_eq!(charge_mults(sf(0.8), true, false, c), (sf(1.8), sf(1.4)));
        assert_eq!(charge_mults(sf(0.8), true, true, c), (S::ONE, S::ONE));
        assert_eq!(charge_mults(sf(0.8), false, false, c), (S::ONE, S::ONE));
        // Braced: bonus, integrity, standing (Idle or engaged), frontal.
        assert!(braced(
            S::HALF,
            sf(0.9),
            OrderKind::Idle,
            false,
            Arc::Front,
            c
        ));
        assert!(braced(
            S::HALF,
            sf(0.9),
            OrderKind::Move,
            true,
            Arc::Front,
            c
        ));
        assert!(!braced(
            S::ZERO,
            sf(0.9),
            OrderKind::Idle,
            false,
            Arc::Front,
            c
        ));
        assert!(!braced(
            S::HALF,
            sf(0.5),
            OrderKind::Idle,
            false,
            Arc::Front,
            c
        ));
        assert!(!braced(
            S::HALF,
            sf(0.9),
            OrderKind::Move,
            false,
            Arc::Front,
            c
        ));
        assert!(!braced(
            S::HALF,
            sf(0.9),
            OrderKind::Idle,
            false,
            Arc::Flank,
            c
        ));
    }

    #[test]
    fn fatigue_and_morale_multipliers_match_the_rules() {
        let r = rules();
        let fresh = fatigue_mults(S::ZERO, &r.fatigue);
        assert_eq!(
            fresh,
            FatigueMults {
                speed: S::ONE,
                attack: S::ONE,
                defence: S::ONE,
                interval: S::ONE
            }
        );
        let spent = fatigue_mults(S::ONE, &r.fatigue);
        assert_eq!(
            spent,
            FatigueMults {
                speed: sf(0.7),
                attack: sf(0.7),
                defence: sf(0.8),
                interval: sf(1.4)
            }
        );
        assert_eq!(morale_mults(MoraleState::Routing, &r.morale).speed, sf(1.1));
        assert_eq!(
            morale_mults(MoraleState::Shattered, &r.morale).speed,
            sf(1.1)
        );
        assert_eq!(experience_mult(0, &r.combat), S::ONE);
        assert_eq!(experience_mult(9, &r.combat), sf(1.27));
    }

    #[test]
    fn terrain_defence_clamps_the_height_term_symmetrically() {
        let r = rules();
        let m = |zone: f32, ford: bool, hj: f32, hi: f32| {
            terrain_defence_mult(sf(zone), ford, sf(hj), sf(hi), &r.movement, &r.combat)
        };
        assert_eq!(m(1.0, false, 0.0, 0.0), S::ONE);
        assert_eq!(m(1.0, false, 5.0, 0.0), sf(1.15));
        assert_eq!(m(1.0, false, 50.0, 0.0), sf(1.15));
        assert_eq!(m(1.0, false, 0.0, 50.0), sf(0.85));
        assert_eq!(m(1.0, true, 0.0, 0.0), sf(0.7));
        assert!((m(0.8, false, 2.5, 0.0) - sf(0.86)).abs() < sf(1e-5));
    }

    #[test]
    fn cooldown_rounds_to_the_nearest_tick_with_a_floor_of_two() {
        assert_eq!(cooldown_ticks(30, S::ONE, S::ONE, S::ONE), 30);
        assert_eq!(cooldown_ticks(30, sf(1.4), sf(1.05), S::ONE), 44); // 44.1
        assert_eq!(cooldown_ticks(30, sf(1.4), sf(1.15), S::ONE), 48); // 48.3
        assert_eq!(cooldown_ticks(1, S::ONE, S::ONE, S::ONE), 2);
        assert_eq!(cooldown_ticks(0, S::ONE, S::ONE, S::ONE), 2);
    }
}
