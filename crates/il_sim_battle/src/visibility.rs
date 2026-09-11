//! Line of sight and fog of war (SIM-VIS-001..006, TDD §8.4; T2-060).
//!
//! `Visibility` holds one mask per side over the regiments (`Ids` order):
//! whether the side currently sees each regiment. Stage 8
//! `visibility_update` refreshes one side's mask on its stagger tick
//! (`tick % period_ticks == side % period_ticks`, SIM-VIS-004), so the mask
//! is state: hashed and snapshotted (a snapshot between refreshes cannot
//! rederive it). The last sightings (`memory`, SIM-VIS-005) feed the UI's
//! ghosts only: snapshotted, never hashed, never read by the sim.
//!
//! Targeting obeys the mask where SIM-VIS-004 says (`AttackRegiment`,
//! `FireMode::Target`, `UseAbility` enemy targets, fire-at-will and
//! `AttackMove` acquisition); melee targeting at contact range and the
//! morale factors do not (plan decision 14).

use bevy_ecs::prelude::*;
use il_core::{Angle, RegimentId, S, Scalar, Tick, V2};
use il_data::{CombatRules, VisibilityRules};
use serde::{Deserialize, Serialize};

use crate::components::{Anchor, Pos, Regiment, Statuses};
use crate::map::LoadedMap;
use crate::resources::{BattlePhase, Clock, Ids, MapRes, Phase, Regs, SetupRes, Sides};

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
/// (`masks`, hashed and snapshotted) and what it remembers (`memory`,
/// snapshotted only). A side always sees its own regiments.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct Visibility {
    pub masks: Vec<Vec<bool>>,
    pub memory: Vec<Vec<Option<Seen>>>,
}

impl Visibility {
    /// Whether `side` sees the regiment at index `regiment`; `true` while no
    /// mask exists (worlds built by `BattleWorld::empty`) and for indices
    /// past the mask (a regiment appended since the last refresh).
    pub fn sees(&self, side: u8, regiment: usize) -> bool {
        self.masks
            .get(usize::from(side))
            .and_then(|m| m.get(regiment))
            .copied()
            .unwrap_or(true)
    }

    /// Grows the tables to `sides × regiments` (new entries unseen and
    /// unremembered); never shrinks.
    pub fn resize(&mut self, sides: usize, regiments: usize) {
        self.masks.resize(sides, Vec::new());
        self.memory.resize(sides, Vec::new());
        for m in &mut self.masks {
            if m.len() < regiments {
                m.resize(regiments, false);
            }
        }
        for m in &mut self.memory {
            if m.len() < regiments {
                m.resize(regiments, None);
            }
        }
    }
}

/// SIM-VIS-001: `weather.los_mult` is `1` until the weather rules of
/// Phase 4 (like SIM-FAT-002's `weather.fatigue_mult`).
pub fn weather_los_mult() -> S {
    S::ONE
}

/// SIM-VIS-001: a regiment's line-of-sight radius: `unit.los_radius ×
/// zone.los_mult × weather.los_mult × (1 + height_bonus × sat((h_anchor −
/// h_mean_map) / combat.height_ref)) × status_los_mult` (SIM-ABIL-005), the
/// saturation clamped to `[−1, 1]`.
pub fn los_radius(
    unit_los: S,
    zone_los_mult: S,
    h_anchor: S,
    h_mean: S,
    status_los: S,
    v: &VisibilityRules,
    c: &CombatRules,
) -> S {
    let sat = if c.height_ref > S::ZERO {
        ((h_anchor - h_mean) / c.height_ref).clamp(-S::ONE, S::ONE)
    } else {
        S::ZERO
    };
    unit_los * zone_los_mult * weather_los_mult() * (S::ONE + v.height_bonus * sat) * status_los
}

/// SIM-VIS-002: whether the sight line from `from` to `to`, both at
/// `eye_height` above the ground, clears the heightmap. The segment is
/// split into `floor(d / los_sample) + 1` equal intervals and the ground is
/// read at every interior point (the endpoints are the eyes); it is blocked
/// where the ground rises strictly above the line. Walls arrive in Phase 5.
pub fn segment_clear(map: &LoadedMap, from: V2, to: V2, v: &VisibilityRules) -> bool {
    let d = from.distance(to);
    if v.los_sample <= S::ZERO || d <= v.los_sample {
        return true;
    }
    let n = (d / v.los_sample).floor_i32() + 1;
    let ha = map.height_at(from) + v.eye_height;
    let hb = map.height_at(to) + v.eye_height;
    let delta = to - from;
    for k in 1..n {
        let t = S::from_i32(k) / S::from_i32(n);
        let p = from + delta * t;
        let line = ha + (hb - ha) * t;
        if map.height_at(p) > line {
            return false;
        }
    }
    true
}

/// SIM-VIS-003 (plan I11): the indices of the up-to-four sampled soldiers
/// of a regiment of `len`: `k × len / 4` for `k ∈ 0..4`, deduplicated.
pub fn sample_indices(len: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(4);
    for k in 0..4usize {
        let i = k * len / 4;
        if i < len && !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

/// One observing regiment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observer {
    pub anchor: V2,
    pub los_radius: S,
}

/// One regiment as seen from outside.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub anchor: V2,
    /// The anchor stands in a `conceal` zone (SIM-VIS-003).
    pub conceal: bool,
    /// Positions of the sampled soldiers.
    pub samples: Vec<V2>,
}

/// SIM-VIS-003: `observer` sees `target` when the target's anchor or any
/// sampled soldier lies within the observer's radius with a clear line
/// (SIM-VIS-002); a concealed target is seen exactly when the observer's
/// anchor is within `conceal_radius` of its anchor, whatever the terrain.
pub fn regiment_visible(
    map: &LoadedMap,
    v: &VisibilityRules,
    observer: &Observer,
    target: &Target,
) -> bool {
    if target.conceal {
        return observer.anchor.distance(target.anchor) <= v.conceal_radius;
    }
    core::iter::once(target.anchor)
        .chain(target.samples.iter().copied())
        .any(|p| {
            observer.anchor.distance(p) <= observer.los_radius
                && segment_clear(map, observer.anchor, p, v)
        })
}

/// Whether `side` sees regiment `target` (SIM-VIS-004); a regiment not in
/// `Ids` counts as visible so the caller's own checks report it.
pub fn sees_regiment(world: &World, side: u8, target: RegimentId) -> bool {
    world
        .resource::<Ids>()
        .regiment_index(target)
        .is_none_or(|i| world.resource::<Visibility>().sees(side, i))
}

/// One regiment as the update reads it (`Ids` order).
struct Row {
    side: u8,
    count: u16,
    anchor: V2,
    facing: Angle<S>,
    conceal: bool,
    los_radius: S,
    samples: Vec<V2>,
}

fn rows(world: &World) -> Vec<Row> {
    let regs = &world.resource::<Regs>().0;
    let map = &world.resource::<MapRes>().0;
    let ids = world.resource::<Ids>();
    let v = &regs.rules.visibility;
    let c = &regs.rules.combat;
    ids.regiment_entities
        .iter()
        .map(|(_, entity)| {
            let (Some(r), Some(a)) = (world.get::<Regiment>(*entity), world.get::<Anchor>(*entity))
            else {
                return Row {
                    side: u8::MAX,
                    count: 0,
                    anchor: V2::ZERO,
                    facing: Angle::default(),
                    conceal: false,
                    los_radius: S::ZERO,
                    samples: Vec::new(),
                };
            };
            // The largest group's sight (T3-040).
            let unit_los = crate::composition::los_radius(regs, &r.units);
            let (zone_los, conceal) = map.zone_at(a.pos).map_or((S::ONE, false), |h| {
                let z = regs.zones.get(h);
                (z.los_mult, z.conceal)
            });
            let status_los = world
                .get::<Statuses>(*entity)
                .map_or(S::ONE, |s| s.mults.los_radius);
            let samples = sample_indices(r.soldiers.len())
                .into_iter()
                .filter_map(|k| ids.soldier_entity(r.soldiers[k]))
                .filter_map(|e| world.get::<Pos>(e).map(|p| p.p))
                .collect();
            Row {
                side: r.side,
                count: r.soldiers.len() as u16,
                anchor: a.pos,
                facing: a.facing,
                conceal,
                los_radius: los_radius(
                    unit_los,
                    zone_los,
                    map.height_at(a.pos),
                    map.mean_height,
                    status_los,
                    v,
                    c,
                ),
                samples,
            }
        })
        .collect()
}

/// Recomputes the masks of `sides` from the current positions (and the
/// memory of what they see), or fills them when `reveal` (SIM-VIS-006).
fn refresh(world: &mut World, sides: &[u8], reveal: bool) {
    let tick = world.resource::<Clock>().tick;
    let memory_ticks = world.resource::<Regs>().0.rules.visibility.memory_ticks;
    let n_sides = world.resource::<Sides>().0.len();
    let rows = rows(world);
    let n = rows.len();
    let map = world.resource::<MapRes>().0.clone();
    let regs = world.resource::<Regs>().0.clone();
    let v = &regs.rules.visibility;
    let mut vis = world.resource_mut::<Visibility>();
    vis.resize(n_sides, n);
    for &side in sides {
        let s = usize::from(side);
        let mut mask = vec![false; n];
        for (j, target) in rows.iter().enumerate() {
            if target.side == side {
                mask[j] = true;
                continue;
            }
            if reveal {
                mask[j] = target.count > 0;
                continue;
            }
            if target.count == 0 {
                continue;
            }
            let t = Target {
                anchor: target.anchor,
                conceal: target.conceal,
                samples: target.samples.clone(),
            };
            // Observers ascend by id; the first sighting settles it.
            mask[j] = rows.iter().any(|o| {
                o.side == side
                    && o.count > 0
                    && regiment_visible(
                        &map,
                        v,
                        &Observer {
                            anchor: o.anchor,
                            los_radius: o.los_radius,
                        },
                        &t,
                    )
            });
        }
        for (j, target) in rows.iter().enumerate() {
            let slot = &mut vis.memory[s][j];
            if target.side == side {
                *slot = None;
            } else if mask[j] {
                *slot = Some(Seen {
                    pos: target.anchor,
                    facing: target.facing,
                    count: target.count,
                    tick,
                });
            } else if slot.is_some_and(|seen| tick.0.saturating_sub(seen.tick.0) > memory_ticks) {
                *slot = None;
            }
        }
        vis.masks[s] = mask;
    }
}

/// Whether the deployment reveal is on (SIM-VIS-006).
fn revealing(world: &World) -> bool {
    world.resource::<Phase>().0 == BattlePhase::Deployment
        && world
            .resource::<SetupRes>()
            .0
            .as_ref()
            .is_some_and(|s| s.reveal_deployment)
}

/// Stage 8 `visibility_update` (SIM-VIS-004): side `s` refreshes when
/// `tick % period_ticks == s % period_ticks`; during a revealed deployment
/// every side refreshes to full masks.
pub fn visibility_update(world: &mut World) {
    let tick = world.resource::<Clock>().tick;
    let period = u32::from(
        world
            .resource::<Regs>()
            .0
            .rules
            .visibility
            .period_ticks
            .max(1),
    );
    let reveal = revealing(world);
    let n_sides = world.resource::<Sides>().0.len();
    let due: Vec<u8> = (0..n_sides as u8)
        .filter(|s| reveal || tick.0 % period == u32::from(*s) % period)
        .collect();
    if !due.is_empty() {
        refresh(world, &due, reveal);
    }
}

/// Every side's mask from scratch: `BattleWorld::new` (so tick 1 is not
/// blind); `restore` reads the stored masks instead.
pub fn recompute_all(world: &mut World) {
    let n_sides = world.resource::<Sides>().0.len();
    let sides: Vec<u8> = (0..n_sides as u8).collect();
    let reveal = revealing(world);
    refresh(world, &sides, reveal);
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_data::Rules;

    fn rules() -> (VisibilityRules, CombatRules) {
        let mut r = Rules::zeroed();
        r.visibility.period_ticks = 10;
        r.visibility.conceal_radius = S::from_i32(25);
        r.visibility.height_bonus = S::HALF;
        r.visibility.eye_height = S::from_f32_data(1.7);
        r.visibility.los_sample = S::from_i32(4);
        r.visibility.memory_ticks = 400;
        r.combat.height_ref = S::from_i32(5);
        (r.visibility, r.combat)
    }

    /// A 40 × 8 m strip with a ridge of `ridge` metres at x = 20.
    fn ridge_map(ridge: S) -> LoadedMap {
        let mut map = LoadedMap::flat(S::from_i32(40), S::from_i32(8));
        map.height_cell = S::from_i32(4);
        map.height_cols = 11;
        map.height_rows = 3;
        map.heights = vec![S::ZERO; 33];
        for row in 0..3 {
            map.heights[row * 11 + 5] = ridge;
        }
        map
    }

    #[test]
    fn los_radius_follows_height_zone_and_status() {
        let (v, c) = rules();
        let r = |h: i32, zone: S, st: S| {
            los_radius(S::from_i32(200), zone, S::from_i32(h), S::ZERO, st, &v, &c)
        };
        assert_eq!(r(0, S::ONE, S::ONE), S::from_i32(200));
        assert_eq!(r(5, S::ONE, S::ONE), S::from_i32(300), "+5 m: × 1.5");
        assert_eq!(r(50, S::ONE, S::ONE), S::from_i32(300), "saturates");
        assert_eq!(r(-5, S::ONE, S::ONE), S::from_i32(100), "−5 m: × 0.5");
        assert_eq!(r(0, S::HALF, S::ONE), S::from_i32(100), "forest");
        assert_eq!(r(0, S::ONE, S::from_i32(2)), S::from_i32(400), "status");
    }

    #[test]
    fn a_ridge_blocks_the_line_and_a_bump_does_not() {
        let (v, _) = rules();
        let a = V2::new(S::ZERO, S::from_i32(4));
        let b = V2::new(S::from_i32(40), S::from_i32(4));
        assert!(!segment_clear(&ridge_map(S::from_i32(6)), a, b, &v));
        assert!(
            segment_clear(&ridge_map(S::ONE), a, b, &v),
            "1 m under 1.7 m eyes"
        );
        assert!(segment_clear(&ridge_map(S::from_i32(6)), a, a, &v));
        // Both eyes on the ridge see each other; one below sees the ridge
        // top but not past it.
        let top = V2::new(S::from_i32(20), S::from_i32(4));
        assert!(segment_clear(&ridge_map(S::from_i32(6)), a, top, &v));
    }

    #[test]
    fn sample_indices_spread_over_the_list() {
        assert_eq!(sample_indices(0), Vec::<usize>::new());
        assert_eq!(sample_indices(1), vec![0]);
        assert_eq!(sample_indices(3), vec![0, 1, 2]);
        assert_eq!(sample_indices(4), vec![0, 1, 2, 3]);
        assert_eq!(sample_indices(9), vec![0, 2, 4, 6]);
        assert_eq!(sample_indices(120), vec![0, 30, 60, 90]);
    }

    #[test]
    fn concealment_ignores_terrain_and_samples_extend_the_anchor() {
        let (v, _) = rules();
        let map = ridge_map(S::from_i32(6));
        let observer = Observer {
            anchor: V2::new(S::ZERO, S::from_i32(4)),
            los_radius: S::from_i32(100),
        };
        let hidden = Target {
            anchor: V2::new(S::from_i32(24), S::from_i32(4)),
            conceal: true,
            samples: Vec::new(),
        };
        assert!(regiment_visible(&map, &v, &observer, &hidden), "24 m < 25");
        let far = Target {
            anchor: V2::new(S::from_i32(26), S::from_i32(4)),
            ..hidden.clone()
        };
        assert!(!regiment_visible(&map, &v, &observer, &far), "26 m > 25");
        // Behind the ridge: the anchor is hidden, a soldier on the near
        // slope is not.
        let behind = Target {
            anchor: V2::new(S::from_i32(30), S::from_i32(4)),
            conceal: false,
            samples: vec![V2::new(S::from_i32(10), S::from_i32(4))],
        };
        assert!(regiment_visible(&map, &v, &observer, &behind));
        let behind_only = Target {
            samples: Vec::new(),
            ..behind
        };
        assert!(!regiment_visible(&map, &v, &observer, &behind_only));
        // Out of radius.
        let near = Observer {
            los_radius: S::from_i32(5),
            ..observer
        };
        let open = Target {
            anchor: V2::new(S::from_i32(8), S::from_i32(4)),
            conceal: false,
            samples: Vec::new(),
        };
        assert!(!regiment_visible(&map, &v, &near, &open));
    }
}
