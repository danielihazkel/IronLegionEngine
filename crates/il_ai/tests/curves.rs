//! T2-080 done-when: response-curve goldens (bit patterns checked in, so
//! any change to the arithmetic is caught) and selection that is identical
//! on every thread (REQ-AI-005).

use std::thread;

use il_ai::{
    ActionDef, ActionKind, AiActionSet, Channel, Consideration, Curve, InputId, InputProvider,
    InputScope, evaluate, select,
};
use il_core::{RngStream, S, Scalar, StreamId};
use il_data::ContentId;

fn sf(v: f32) -> S {
    S::from_f32_data(v)
}

const XS: [f32; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

fn curves() -> Vec<(&'static str, Curve)> {
    vec![
        (
            "linear(-1, 1)",
            Curve::Linear {
                m: sf(-1.0),
                b: sf(1.0),
            },
        ),
        (
            "linear(0.5, 0.5)",
            Curve::Linear {
                m: sf(0.5),
                b: sf(0.5),
            },
        ),
        ("quadratic(2)", Curve::Quadratic { k: sf(2.0) }),
        (
            "logistic(8, 0.4)",
            Curve::Logistic {
                k: sf(8.0),
                mid: sf(0.4),
            },
        ),
        (
            "logistic(-12, 0.35)",
            Curve::Logistic {
                k: sf(-12.0),
                mid: sf(0.35),
            },
        ),
        ("step(0.5)", Curve::Step { threshold: sf(0.5) }),
    ]
}

/// `evaluate(curve, x).to_bits()` for every curve above at every `x` of
/// `XS`, in that order.
const GOLDEN: [[u32; 5]; 6] = [
    [
        0x3f80_0000,
        0x3f40_0000,
        0x3f00_0000,
        0x3e80_0000,
        0x0000_0000,
    ],
    [
        0x3f00_0000,
        0x3f20_0000,
        0x3f40_0000,
        0x3f60_0000,
        0x3f80_0000,
    ],
    [
        0x0000_0000,
        0x3e00_0000,
        0x3f00_0000,
        0x3f80_0000,
        0x3f80_0000,
    ],
    [
        0x3df3_cf38,
        0x3e68_ba2e,
        0x3f38_e38e,
        0x3f5e_50d8,
        0x3f69_ee58,
    ],
    [
        0x3f67_6276,
        0x3f45_d174,
        0x3e36_db6e,
        0x3db0_8d3c,
        0x3d68_ba28,
    ],
    [
        0x0000_0000,
        0x0000_0000,
        0x3f80_0000,
        0x3f80_0000,
        0x3f80_0000,
    ],
];

#[test]
fn curve_goldens() {
    let mut report = String::new();
    let mut ok = true;
    for (i, (name, curve)) in curves().iter().enumerate() {
        let got: Vec<u32> = XS
            .iter()
            .map(|x| evaluate(curve, sf(*x)).to_f32_render().to_bits())
            .collect();
        if got.as_slice() != GOLDEN[i] {
            ok = false;
        }
        report.push_str(&format!(
            "    [0x{:04x}_{:04x}, 0x{:04x}_{:04x}, 0x{:04x}_{:04x}, 0x{:04x}_{:04x}, 0x{:04x}_{:04x}], // {name}\n",
            got[0] >> 16,
            got[0] & 0xffff,
            got[1] >> 16,
            got[1] & 0xffff,
            got[2] >> 16,
            got[2] & 0xffff,
            got[3] >> 16,
            got[3] & 0xffff,
            got[4] >> 16,
            got[4] & 0xffff,
        ));
    }
    assert!(ok, "curve golden mismatch; actual table:\n{report}");
}

struct Situation {
    morale: S,
    distance: S,
}

impl InputProvider for Situation {
    fn input(&self, id: InputId) -> S {
        match id {
            InputId::OwnMorale => self.morale,
            InputId::DistanceToNearestEnemy => self.distance,
            InputId::Constant => S::ONE,
            _ => S::ZERO,
        }
    }
}

fn movement_set() -> AiActionSet {
    let c = |input, scale: f32, curve| Consideration {
        input,
        scale: sf(scale),
        curve,
    };
    let action = |name: &str, kind, base: f32, noise: f32, considerations| ActionDef {
        name: name.to_string(),
        kind,
        base: sf(base),
        threshold: S::ZERO,
        noise: sf(noise),
        considerations,
    };
    AiActionSet {
        id: ContentId::new("t:movement").unwrap(),
        scope: InputScope::Regiment,
        deprecated: None,
        actions: vec![
            action(
                "engage",
                ActionKind::EngageNearest,
                1.0,
                0.05,
                vec![c(
                    InputId::DistanceToNearestEnemy,
                    80.0,
                    Curve::Linear {
                        m: sf(-1.0),
                        b: sf(1.0),
                    },
                )],
            ),
            action(
                "hold",
                ActionKind::HoldPosition,
                0.5,
                0.05,
                vec![c(
                    InputId::Constant,
                    1.0,
                    Curve::Step { threshold: sf(0.5) },
                )],
            ),
            action(
                "back",
                ActionKind::FallBack,
                1.2,
                0.05,
                vec![c(
                    InputId::OwnMorale,
                    1.0,
                    Curve::Logistic {
                        k: sf(-12.0),
                        mid: sf(0.35),
                    },
                )],
            ),
        ],
    }
}

/// The same set, situations and stream give the same choices on eight
/// threads, in the same order, with the same stream state afterwards.
#[test]
fn selection_is_identical_across_threads() {
    let run = move || {
        let situations: Vec<Situation> = (0..64)
            .map(|i| Situation {
                morale: sf((i % 11) as f32 / 10.0),
                distance: sf((i * 7 % 120) as f32),
            })
            .collect();
        let set = movement_set();
        let mut rng = RngStream::from_seed(99, StreamId::AiRegiment);
        let picks: Vec<(usize, u32)> = situations
            .iter()
            .map(|s| {
                let c = select(&set, Channel::Movement, &mut |_, _| true, s, Some(&mut rng))
                    .expect("no thresholds");
                (c.index, c.score.to_f32_render().to_bits())
            })
            .collect();
        (picks, rng)
    };
    let reference = run();
    let handles: Vec<_> = (0..8).map(|_| thread::spawn(run)).collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), reference);
    }
    // The picks are not degenerate: every action wins somewhere.
    let winners: std::collections::BTreeSet<usize> = reference.0.iter().map(|(i, _)| *i).collect();
    assert_eq!(winners.len(), 3, "{winners:?}");
}
