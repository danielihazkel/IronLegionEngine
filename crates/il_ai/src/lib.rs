//! Iron Legion utility-AI framework (SAD §5.3, TDD §8.5, SIM-AI-001; T2-080).
//!
//! The data lives in `il_data::ai` (validated at load); this crate scores
//! and selects. Everything here is a pure function of its arguments: no
//! world access, no clock, no allocation beyond the caller's, so a choice
//! is the same on every thread and every peer (REQ-AI-005).
//!
//! `score = base × Π_c curve_c(sat(raw_c / scale_c))`, then an optional
//! multiplicative jitter from the AI stream (plan decision 7); inside one
//! decision channel the highest score of at least the action's `threshold`
//! wins, ties by list order (plan decision 5).

use il_core::{RngStream, S, Scalar, Tick};
pub use il_data::ai::{
    ActionDef, ActionKind, AiActionSet, AiProfile, Channel, Consideration, Curve, InputId,
    InputScope,
};

/// Clamps to `[0, 1]`.
#[inline]
pub fn sat(x: S) -> S {
    x.clamp(S::ZERO, S::ONE)
}

/// `sat(raw / scale)`; a non-positive `scale` counts as 1 (the schema
/// forbids it, the arithmetic must not divide by zero).
#[inline]
pub fn normalise(raw: S, scale: S) -> S {
    if scale > S::ZERO {
        sat(raw / scale)
    } else {
        sat(raw)
    }
}

/// Evaluates a response curve at `x ∈ [0, 1]`; the result is clamped to
/// `[0, 1]` (plan I2). `logistic` is the algebraic sigmoid of plan
/// decision 6: `0.5 + 0.5 · t / (1 + |t|)`, `t = k · (x − mid)`.
pub fn evaluate(curve: &Curve, x: S) -> S {
    let y = match *curve {
        Curve::Linear { m, b } => m * x + b,
        Curve::Quadratic { k } => k * x * x,
        Curve::Logistic { k, mid } => {
            let t = k * (x - mid);
            S::HALF + S::HALF * t / (S::ONE + t.abs())
        }
        Curve::Step { threshold } => {
            if x >= threshold {
                S::ONE
            } else {
                S::ZERO
            }
        }
    };
    sat(y)
}

/// Supplies the raw input values of one decision context.
pub trait InputProvider {
    fn input(&self, id: InputId) -> S;
}

/// One consideration's factor.
#[inline]
pub fn consider(c: &Consideration, inputs: &dyn InputProvider) -> S {
    evaluate(&c.curve, normalise(inputs.input(c.input), c.scale))
}

/// `base × Π curve_c(x_c)`, then `× (1 + noise · (2u − 1))` with `u` drawn
/// from `rng` only when `noise > 0` (so zero-noise content never touches
/// the stream). `rng: None` disables the jitter altogether.
pub fn score(action: &ActionDef, inputs: &dyn InputProvider, rng: Option<&mut RngStream>) -> S {
    let mut s = action.base;
    for c in &action.considerations {
        s = s * consider(c, inputs);
    }
    if action.noise > S::ZERO
        && let Some(rng) = rng
    {
        let u: S = rng.unit();
        let two = S::from_i32(2);
        s = s * (S::ONE + action.noise * (two * u - S::ONE));
    }
    s
}

/// The winner of one channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Choice<'a> {
    /// Index into `AiActionSet::actions`.
    pub index: usize,
    pub action: &'a ActionDef,
    pub score: S,
}

/// SIM-AI-001 per channel (plan decision 5): scores every action of
/// `channel` that `eligible` admits, in list order, and returns the first
/// with the highest score provided it reaches the action's `threshold`.
/// Noise draws happen in list order, so the stream state after a call is
/// reproducible.
pub fn select<'a>(
    set: &'a AiActionSet,
    channel: Channel,
    eligible: &mut dyn FnMut(usize, &ActionDef) -> bool,
    inputs: &dyn InputProvider,
    mut rng: Option<&mut RngStream>,
) -> Option<Choice<'a>> {
    let mut best: Option<Choice<'a>> = None;
    for (index, action) in set.channel_actions(channel) {
        if !eligible(index, action) {
            continue;
        }
        let s = score(action, inputs, rng.as_deref_mut());
        if s < action.threshold {
            continue;
        }
        // Strict `>`: the first of equal scores keeps the win (list order).
        if best.is_none_or(|b| s > b.score) {
            best = Some(Choice {
                index,
                action,
                score: s,
            });
        }
    }
    best
}

/// SIM-AI-002 stagger (plan I7): `tick % period == key % period`, with a
/// period of 0 read as 1.
#[inline]
pub fn due(tick: Tick, period: u16, key: u32) -> bool {
    let p = u32::from(period.max(1));
    tick.0 % p == key % p
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_core::StreamId;
    use il_data::ContentId;

    fn sf(v: f32) -> S {
        S::from_f32_data(v)
    }

    struct Fixed(Vec<(InputId, S)>);
    impl InputProvider for Fixed {
        fn input(&self, id: InputId) -> S {
            self.0
                .iter()
                .find(|(i, _)| *i == id)
                .map_or(S::ZERO, |(_, v)| *v)
        }
    }

    fn action(name: &str, kind: ActionKind, base: f32, threshold: f32, noise: f32) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            kind,
            base: sf(base),
            threshold: sf(threshold),
            noise: sf(noise),
            considerations: Vec::new(),
        }
    }

    fn set(actions: Vec<ActionDef>) -> AiActionSet {
        AiActionSet {
            id: ContentId::new("t:s").unwrap(),
            scope: InputScope::Regiment,
            actions,
            deprecated: None,
        }
    }

    #[test]
    fn curves_clamp_and_step_is_inclusive() {
        let lin = Curve::Linear {
            m: sf(-1.0),
            b: S::ONE,
        };
        assert_eq!(evaluate(&lin, S::ZERO), S::ONE);
        assert_eq!(evaluate(&lin, S::ONE), S::ZERO);
        assert_eq!(
            evaluate(
                &Curve::Linear {
                    m: sf(3.0),
                    b: S::ZERO
                },
                S::ONE
            ),
            S::ONE
        );
        assert_eq!(evaluate(&Curve::Quadratic { k: sf(2.0) }, S::HALF), S::HALF);
        let step = Curve::Step { threshold: sf(0.5) };
        assert_eq!(evaluate(&step, sf(0.5)), S::ONE);
        assert_eq!(evaluate(&step, sf(0.49)), S::ZERO);
        let up = Curve::Logistic {
            k: sf(8.0),
            mid: sf(0.5),
        };
        assert_eq!(evaluate(&up, sf(0.5)), S::HALF);
        assert!(evaluate(&up, sf(0.9)) > sf(0.8));
        assert!(evaluate(&up, sf(0.1)) < sf(0.2));
        let down = Curve::Logistic {
            k: sf(-8.0),
            mid: sf(0.5),
        };
        assert!(evaluate(&down, sf(0.9)) < sf(0.2));
        assert_eq!(normalise(sf(40.0), sf(80.0)), S::HALF);
        assert_eq!(normalise(sf(400.0), sf(80.0)), S::ONE);
        assert_eq!(normalise(sf(0.5), S::ZERO), S::HALF);
    }

    #[test]
    fn select_takes_the_highest_over_threshold_and_ties_by_order() {
        let s = set(vec![
            action("a", ActionKind::HoldPosition, 0.7, 0.0, 0.0),
            action("b", ActionKind::EngageNearest, 0.7, 0.0, 0.0),
            action("c", ActionKind::FallBack, 0.9, 0.95, 0.0),
            action("d", ActionKind::FireAtWill, 5.0, 0.0, 0.0),
        ]);
        let inputs = Fixed(Vec::new());
        let choice = select(&s, Channel::Movement, &mut |_, _| true, &inputs, None).unwrap();
        assert_eq!(choice.action.name, "a", "first of the equal scores wins");
        let choice = select(&s, Channel::Movement, &mut |i, _| i != 0, &inputs, None).unwrap();
        assert_eq!(choice.action.name, "b", "ineligible actions are skipped");
        let s2 = set(vec![action("c", ActionKind::FallBack, 0.9, 0.95, 0.0)]);
        assert!(select(&s2, Channel::Movement, &mut |_, _| true, &inputs, None).is_none());
        assert_eq!(
            select(&s, Channel::Fire, &mut |_, _| true, &inputs, None)
                .unwrap()
                .action
                .name,
            "d"
        );
        assert!(select(&s, Channel::Stance, &mut |_, _| true, &inputs, None).is_none());
    }

    #[test]
    fn considerations_multiply_and_noise_draws_only_when_set() {
        let mut a = action("a", ActionKind::HoldPosition, 2.0, 0.0, 0.0);
        a.considerations = vec![
            Consideration {
                input: InputId::OwnMorale,
                scale: S::ONE,
                curve: Curve::Linear {
                    m: S::ONE,
                    b: S::ZERO,
                },
            },
            Consideration {
                input: InputId::DistanceToNearestEnemy,
                scale: sf(100.0),
                curve: Curve::Linear {
                    m: sf(-1.0),
                    b: S::ONE,
                },
            },
        ];
        let inputs = Fixed(vec![
            (InputId::OwnMorale, sf(0.5)),
            (InputId::DistanceToNearestEnemy, sf(25.0)),
        ]);
        assert_eq!(score(&a, &inputs, None), sf(0.75));
        let mut rng = RngStream::from_seed(3, StreamId::AiRegiment);
        let before = rng.clone();
        assert_eq!(score(&a, &inputs, Some(&mut rng)), sf(0.75));
        assert_eq!(rng, before, "noise 0 draws nothing");
        a.noise = sf(0.5);
        let noisy = score(&a, &inputs, Some(&mut rng));
        assert_ne!(rng, before, "noise draws once");
        assert!(noisy >= sf(0.375) && noisy <= sf(1.125), "{noisy:?}");
        let mut again = before.clone();
        assert_eq!(score(&a, &inputs, Some(&mut again)), noisy, "reproducible");
    }

    #[test]
    fn due_staggers_by_key() {
        assert!(due(Tick(20), 20, 0));
        assert!(!due(Tick(20), 20, 1));
        assert!(due(Tick(21), 20, 1));
        assert!(due(Tick(41), 20, 41));
        assert!(due(Tick(7), 0, 7), "period 0 counts as 1");
    }
}
