//! Pure morale rules (T2-041; SIM-MOR-001..027, TDD §8.3): the fourteen
//! factors, the per-tick delta, the one-time shock amounts and the
//! hysteresis state machine. Every tunable comes from `Rules`; the systems
//! in `tick.rs` only gather inputs and apply results.

use il_core::{S, Scalar};
use il_data::{CombatRules, FormationRules, MoraleRules, MoraleWeights};

use crate::components::MoraleState;
use crate::resources::ShockKind;

/// Number of weighted factors (SIM-MOR-010..024 without the shocks), in
/// `MoraleWeights` field order.
pub const FACTORS: usize = 14;

/// What one regiment's factors are computed from, gathered at the start
/// of Stage 14 (plan G25: every state below is as it stood before the pass).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoraleInputs {
    /// Living soldiers and the spawn strength (SIM-MOR-010/011).
    pub count: u16,
    pub initial: u16,
    /// Own deaths in the last five seconds (the casualty ring sum).
    pub own_deaths_5s: u32,
    /// Deaths in the ring of enemy regiments whose anchor lies within
    /// `safe_radius` (SIM-MOR-023, plan G23).
    pub enemy_deaths_5s: u32,
    /// SIM-FAT-005 regiment mean.
    pub fatigue_mean: S,
    /// SIM-MOR-013: inside a living general's aura (false until T2-043).
    pub in_aura: bool,
    /// Same-side regiments with soldiers within `ally_radius`, by state
    /// (SIM-MOR-015/016; Shattered counts as routing).
    pub allies_steady: u32,
    pub allies_routing: u32,
    /// Anchor height minus the nearest enemy anchor's height; `None`
    /// without an enemy (SIM-MOR-017).
    pub height_delta: Option<S>,
    /// SIM-MOR-018: a `fear` status is active (false until T2-050).
    pub fear: bool,
    /// SIM-MOR-019: attacked through that arc within the last second;
    /// `surrounded` when all three arcs were.
    pub hit_flank: bool,
    pub hit_rear: bool,
    pub surrounded: bool,
    /// Soldiers within `outnumber_radius` of the anchor by side
    /// (SIM-MOR-020; `own` includes the regiment itself).
    pub enemies_near: u32,
    pub own_near: u32,
    /// SIM-FORM-030 integrity.
    pub integrity: S,
    /// Ticks since `Combat.engaged` rose; `None` while not engaged.
    pub engaged_ticks: Option<u32>,
    /// An enemy anchor lies within `safe_radius` (SIM-MOR-024).
    pub enemy_within_safe: bool,
    pub state: MoraleState,
}

/// `sat(x)`: clamp to `[0, 1]`.
pub fn sat(x: S) -> S {
    x.clamp(S::ZERO, S::ONE)
}

/// Safe ratio: `0` when the denominator is not positive (zeroed rules).
fn ratio(num: S, den: S) -> S {
    if den > S::ZERO { num / den } else { S::ZERO }
}

/// SIM-MOR-010..024: `x_f` per factor, `MoraleWeights` order. Each value
/// is the factor's activation in `[0, 1]`; the sign lives in the weight
/// (`w.casualty_rate` is −6, `w.recovery` +3). Exceptions: `high_ground`
/// is bipolar in `[−1, 1]` and `flanked` reaches 2 when surrounded (plan
/// decision 7).
pub fn morale_factors(
    i: &MoraleInputs,
    m: &MoraleRules,
    c: &CombatRules,
    f: &FormationRules,
) -> [S; FACTORS] {
    let count = S::from_i32(i32::from(i.count));
    let initial = S::from_i32(i32::from(i.initial));
    let own_deaths = S::from_i32(i.own_deaths_5s as i32);
    let enemy_deaths = S::from_i32(i.enemy_deaths_5s as i32);
    let rate_ref = count * m.casualty_rate_ref;
    // SIM-MOR-010.
    let casualty_rate = sat(ratio(own_deaths, rate_ref));
    // SIM-MOR-011 (a level, applied per tick like every rate: plan G16).
    let casualty_total = sat(ratio(ratio(initial - count, initial), m.casualty_total_ref));
    // SIM-MOR-012.
    let fatigue = sat(ratio(
        i.fatigue_mean - m.fatigue_start,
        S::ONE - m.fatigue_start,
    ));
    // SIM-MOR-013.
    let general_aura = if i.in_aura { S::ONE } else { S::ZERO };
    // SIM-MOR-015 / 016.
    let allies_near = sat(ratio(S::from_i32(i.allies_steady as i32), m.allies_ref));
    let allies_routing = sat(ratio(S::from_i32(i.allies_routing as i32), m.routing_ref));
    // SIM-MOR-017.
    let high_ground = i
        .height_delta
        .map_or(S::ZERO, |d| ratio(d, c.height_ref).clamp(-S::ONE, S::ONE));
    // SIM-MOR-018.
    let fear = if i.fear { S::ONE } else { S::ZERO };
    // SIM-MOR-019: the rear supersedes the flank; surrounded adds a full point.
    let arc = if i.hit_rear {
        S::ONE
    } else if i.hit_flank {
        S::HALF
    } else {
        S::ZERO
    };
    let flanked = arc + if i.surrounded { S::ONE } else { S::ZERO };
    // SIM-MOR-020 (plan G17).
    let outnumbered = if i.enemies_near == 0 {
        S::ZERO
    } else if i.own_near == 0 {
        S::ONE
    } else {
        let excess = ratio(
            S::from_i32(i.enemies_near as i32),
            S::from_i32(i.own_near as i32),
        ) - S::ONE;
        sat(ratio(excess, m.outnumber_ref))
    };
    // SIM-MOR-021.
    let thr = f.integrity_morale_threshold;
    let integrity = sat(ratio(thr - i.integrity, thr));
    // SIM-MOR-022.
    let engaged_duration = i.engaged_ticks.map_or(S::ZERO, |t| {
        sat(ratio(
            S::from_i32(t.min(i32::MAX as u32) as i32),
            S::from_i32(m.engage_fatigue_ticks.min(i32::MAX as u32) as i32),
        ))
    });
    // SIM-MOR-023: `sat` clamps the losing side at 0.
    let winning = sat(ratio(enemy_deaths - own_deaths, rate_ref));
    // SIM-MOR-024.
    let recovery = if i.engaged_ticks.is_none()
        && !i.enemy_within_safe
        && !matches!(i.state, MoraleState::Routing | MoraleState::Shattered)
    {
        S::ONE
    } else {
        S::ZERO
    };
    [
        casualty_rate,
        casualty_total,
        fatigue,
        general_aura,
        allies_near,
        allies_routing,
        high_ground,
        fear,
        flanked,
        outnumbered,
        integrity,
        engaged_duration,
        winning,
        recovery,
    ]
}

/// SIM-MOR-002: `Σ w_f × x_f × dt`, summed in factor order.
pub fn morale_delta(x: &[S; FACTORS], w: &MoraleWeights, dt: S) -> S {
    let weights = [
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
    ];
    let mut sum = S::ZERO;
    for (xf, wf) in x.iter().zip(weights) {
        sum = sum + wf * *xf;
    }
    sum * dt
}

/// SIM-MOR-014, 025, 026, 033: the morale lost to a shock, resolved from
/// the rules when applied; a general's death costs half to a regiment
/// already Shaken or worse, a frontal charge half of `charged_penalty`.
pub fn shock_amount(kind: ShockKind, state: MoraleState, r: &MoraleRules) -> S {
    match kind {
        ShockKind::Disengage => r.disengage_penalty,
        ShockKind::ChargedFront => r.charged_penalty * S::HALF,
        ShockKind::ChargedFlank => r.charged_penalty,
        ShockKind::GeneralDeath => {
            if state as u8 >= MoraleState::Shaken as u8 {
                r.general_death_shock * S::HALF
            } else {
                r.general_death_shock
            }
        }
        ShockKind::Rout => r.rout_shock,
    }
}

/// The band `m` falls into by the thresholds alone (SIM-MOR-003).
fn band(m: S, r: &MoraleRules) -> MoraleState {
    if m <= r.t_routing {
        MoraleState::Routing
    } else if m <= r.t_broken {
        MoraleState::Broken
    } else if m <= r.t_shaken {
        MoraleState::Shaken
    } else if m <= r.t_unsettled {
        MoraleState::Unsettled
    } else {
        MoraleState::Steady
    }
}

/// SIM-MOR-003 with hysteresis: a regiment drops straight to the band its
/// morale falls into, and climbs one state per tick only once `m` exceeds
/// the state's upper threshold plus `hysteresis`. `Routing` and `Shattered`
/// are never entered or left here by threshold alone: entering Routing is
/// the caller's transition (SIM-MOR-030/032) and leaving it is the rally
/// rule (SIM-MOR-031, plan decision 2), so both return `current`.
pub fn morale_state(m: S, current: MoraleState, r: &MoraleRules) -> MoraleState {
    if matches!(current, MoraleState::Routing | MoraleState::Shattered) {
        return current;
    }
    let target = band(m, r);
    if target as u8 > current as u8 {
        return target;
    }
    let (upper, better) = match current {
        MoraleState::Unsettled => (r.t_unsettled, MoraleState::Steady),
        MoraleState::Shaken => (r.t_shaken, MoraleState::Unsettled),
        MoraleState::Broken => (r.t_broken, MoraleState::Shaken),
        _ => return current,
    };
    if m > upper + r.hysteresis {
        better
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> il_data::Rules {
        let mut r = il_data::Rules::zeroed();
        let sf = S::from_f32_data;
        let m = &mut r.morale;
        m.t_unsettled = sf(70.0);
        m.t_shaken = sf(50.0);
        m.t_broken = sf(30.0);
        m.t_routing = sf(15.0);
        m.hysteresis = sf(5.0);
        m.general_death_shock = sf(20.0);
        m.rout_shock = sf(5.0);
        m.disengage_penalty = sf(5.0);
        m.charged_penalty = sf(8.0);
        m.casualty_rate_ref = sf(0.05);
        m.casualty_total_ref = sf(0.5);
        m.fatigue_start = sf(0.5);
        m.allies_ref = sf(3.0);
        m.routing_ref = sf(2.0);
        m.outnumber_ref = sf(2.0);
        m.engage_fatigue_ticks = 2400;
        r.combat.height_ref = sf(5.0);
        r.formation.integrity_morale_threshold = sf(0.5);
        r
    }

    fn quiet() -> MoraleInputs {
        MoraleInputs {
            count: 120,
            initial: 120,
            own_deaths_5s: 0,
            enemy_deaths_5s: 0,
            fatigue_mean: S::ZERO,
            in_aura: false,
            allies_steady: 0,
            allies_routing: 0,
            height_delta: None,
            fear: false,
            hit_flank: false,
            hit_rear: false,
            surrounded: false,
            enemies_near: 0,
            own_near: 120,
            integrity: S::ONE,
            engaged_ticks: None,
            enemy_within_safe: false,
            state: MoraleState::Steady,
        }
    }

    fn factors(i: &MoraleInputs) -> [S; FACTORS] {
        let r = rules();
        morale_factors(i, &r.morale, &r.combat, &r.formation)
    }

    #[test]
    fn a_quiet_regiment_only_recovers() {
        let x = factors(&quiet());
        assert_eq!(x[13], S::ONE, "recovery");
        assert!(x[..13].iter().all(|v| *v == S::ZERO), "{x:?}");
    }

    #[test]
    fn casualty_factors_saturate_at_their_references() {
        // 6 of 120 in five seconds is the full drain; 3 is half.
        let mut i = quiet();
        i.own_deaths_5s = 3;
        assert_eq!(factors(&i)[0], S::HALF);
        i.own_deaths_5s = 12;
        assert_eq!(factors(&i)[0], S::ONE);
        // 30 of 120 lost is a quarter, half the reference.
        let mut i = quiet();
        i.count = 90;
        assert_eq!(factors(&i)[1], S::HALF);
        i.count = 0;
        assert_eq!(factors(&i)[1], S::ONE);
        // Winning: enemy losses beyond ours, clamped at zero below.
        let mut i = quiet();
        i.enemy_deaths_5s = 3;
        assert_eq!(factors(&i)[12], S::HALF);
        i.own_deaths_5s = 9;
        assert_eq!(factors(&i)[12], S::ZERO);
    }

    #[test]
    fn fatigue_allies_ground_and_integrity() {
        let sf = S::from_f32_data;
        let mut i = quiet();
        i.fatigue_mean = sf(0.75);
        assert_eq!(factors(&i)[2], S::HALF);
        i.fatigue_mean = sf(0.25);
        assert_eq!(factors(&i)[2], S::ZERO);
        let mut i = quiet();
        i.in_aura = true;
        i.allies_steady = 2;
        i.allies_routing = 1;
        let x = factors(&i);
        assert_eq!(x[3], S::ONE);
        assert_eq!(x[4], sf(2.0) / sf(3.0));
        assert_eq!(x[5], S::HALF);
        let mut i = quiet();
        i.height_delta = Some(sf(2.5));
        assert_eq!(factors(&i)[6], S::HALF);
        i.height_delta = Some(sf(-50.0));
        assert_eq!(factors(&i)[6], -S::ONE);
        let mut i = quiet();
        i.integrity = sf(0.25);
        assert_eq!(factors(&i)[10], S::HALF);
        i.integrity = sf(0.9);
        assert_eq!(factors(&i)[10], S::ZERO);
    }

    #[test]
    fn flanked_reaches_minus_two_and_outnumbered_handles_zero() {
        let mut i = quiet();
        i.hit_flank = true;
        assert_eq!(factors(&i)[8], S::HALF);
        i.hit_rear = true;
        assert_eq!(factors(&i)[8], S::ONE);
        i.surrounded = true;
        assert_eq!(factors(&i)[8], S::from_i32(2));
        let mut i = quiet();
        i.enemies_near = 120;
        i.own_near = 120;
        assert_eq!(factors(&i)[9], S::ZERO, "even");
        i.enemies_near = 240;
        assert_eq!(factors(&i)[9], S::HALF, "twice: excess 1 of ref 2");
        i.own_near = 0;
        assert_eq!(factors(&i)[9], S::ONE, "nobody of ours nearby");
        i.enemies_near = 0;
        assert_eq!(factors(&i)[9], S::ZERO, "no enemy");
    }

    #[test]
    fn engagement_fear_and_recovery() {
        let mut i = quiet();
        i.engaged_ticks = Some(1200);
        i.enemy_within_safe = true;
        let x = factors(&i);
        assert_eq!(x[11], S::HALF);
        assert_eq!(x[13], S::ZERO, "no recovery while engaged");
        i.engaged_ticks = None;
        assert_eq!(factors(&i)[13], S::ZERO, "no recovery with an enemy near");
        i.enemy_within_safe = false;
        i.state = MoraleState::Routing;
        assert_eq!(factors(&i)[13], S::ZERO, "no recovery while routing");
        i.state = MoraleState::Broken;
        i.fear = true;
        let x = factors(&i);
        assert_eq!(x[7], S::ONE);
        assert_eq!(x[13], S::ONE);
    }

    #[test]
    fn delta_sums_weights_in_order() {
        let r = rules();
        let mut w = r.morale.w.clone();
        w.casualty_rate = S::from_i32(-6);
        w.recovery = S::from_i32(3);
        let mut x = [S::ZERO; FACTORS];
        x[0] = S::HALF;
        x[13] = S::ONE;
        let dt = S::from_f32_data(0.05);
        // −6 × 0.5 + 3 × 1 = 0 per second.
        assert_eq!(morale_delta(&x, &w, dt), S::ZERO);
    }

    #[test]
    fn shock_amounts_follow_the_rules() {
        use MoraleState::*;
        use ShockKind::*;
        let r = rules().morale;
        let sf = S::from_f32_data;
        assert_eq!(shock_amount(Disengage, Steady, &r), sf(5.0));
        assert_eq!(shock_amount(ChargedFront, Steady, &r), sf(4.0));
        assert_eq!(shock_amount(ChargedFlank, Steady, &r), sf(8.0));
        assert_eq!(shock_amount(GeneralDeath, Unsettled, &r), sf(20.0));
        assert_eq!(shock_amount(GeneralDeath, Shaken, &r), sf(10.0));
        assert_eq!(shock_amount(GeneralDeath, Routing, &r), sf(10.0));
        assert_eq!(shock_amount(Rout, Broken, &r), sf(5.0));
    }

    #[test]
    fn hysteresis_table() {
        use MoraleState::*;
        let r = rules().morale;
        let sf = S::from_f32_data;
        // (m, current) -> next, across every threshold and its hysteresis.
        let table = [
            (71.0, Steady, Steady),
            (70.0, Steady, Unsettled),
            (50.0, Steady, Shaken),
            (30.0, Steady, Broken),
            (15.0, Steady, Routing),
            (10.0, Steady, Routing),
            (75.0, Unsettled, Unsettled),
            (75.1, Unsettled, Steady),
            (49.0, Unsettled, Shaken),
            (14.0, Unsettled, Routing),
            (55.0, Shaken, Shaken),
            (55.1, Shaken, Unsettled),
            (90.0, Shaken, Unsettled),
            (29.0, Shaken, Broken),
            (35.0, Broken, Broken),
            (35.1, Broken, Shaken),
            (99.0, Broken, Shaken),
            (14.0, Broken, Routing),
            (99.0, Routing, Routing),
            (0.0, Routing, Routing),
            (99.0, Shattered, Shattered),
        ];
        for (m, current, next) in table {
            assert_eq!(
                morale_state(sf(m), current, &r),
                next,
                "m {m} from {current:?}"
            );
        }
    }
}
