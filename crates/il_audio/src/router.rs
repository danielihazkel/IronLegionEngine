//! `EventRouter`: battle events in, play requests and the roar gain out
//! (TDD §12, plan I2..I5).
//!
//! Pure: no clock, no device. The app passes the frame's wall time, the
//! camera, the engaged count and the regiment positions; the router keeps
//! only its rate-limit bookkeeping between frames.

use std::collections::BTreeMap;

use il_core::{RegimentId, Scalar, Tick};
use il_data::{Handle, Registry, SoundEvent, SoundSet, UnitCategory, UnitType};
use il_sim_battle::BattleEvent;
use il_sim_battle::components::MoraleState;

use crate::sink::{AudioSink, PlayRequest, SampleId};

/// Plays a frame may start at most (plan I5).
pub const MAX_PLAYS_PER_FRAME: usize = 8;
/// A sample whose length the sink never reported is assumed this long.
pub const DEFAULT_DURATION_MS: u32 = 300;

/// What the app knows about the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameInput {
    /// World metres the camera looks at.
    pub camera_center: [f32; 2],
    /// Camera pixels per metre.
    pub zoom: f32,
    /// World bounds of the view (min, max).
    pub bounds: ([f32; 2], [f32; 2]),
    /// Soldiers of regiments whose `engaged` flag is set.
    pub engaged: u32,
    /// Wall milliseconds since the app started.
    pub now_ms: u64,
    /// The side whose victory is `victory` (a spectator hears `phase`).
    pub observer_side: Option<u8>,
    /// The sim tick the events came from (variant choice).
    pub tick: Tick,
}

/// A regiment's anchor this frame, for regiment-only events (plan I4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegimentPos {
    pub id: RegimentId,
    pub side: u8,
    pub pos: [f32; 2],
    pub unit: Handle<UnitType>,
}

/// `sat(ln(zoom / far) / ln(near / far))`: 0 at or below `far` pixels per
/// metre, 1 at or above `near` (plan I3).
pub fn near_weight(zoom: f32, far: f32, near: f32) -> f32 {
    if !(far > 0.0 && near > far) || zoom <= 0.0 {
        return 0.0;
    }
    ((zoom / far).ln() / (near / far).ln()).clamp(0.0, 1.0)
}

/// `sat(engaged / ref) × (1 − 0.5 × near)` (plan I3).
pub fn roar_gain(engaged: u32, ref_engaged: u32, near: f32) -> f32 {
    if ref_engaged == 0 {
        return 0.0;
    }
    (engaged as f32 / ref_engaged as f32).clamp(0.0, 1.0) * (1.0 - 0.5 * near.clamp(0.0, 1.0))
}

#[derive(Clone, Copy, Debug)]
struct EventTable {
    first: u16,
    count: u16,
    min_interval_ms: u32,
    max_voices: u8,
    gain: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct UnitOverride {
    charge: Option<SampleId>,
    die: Option<SampleId>,
    cavalry: bool,
}

/// One live voice: which event started it and when it ends.
#[derive(Clone, Copy, Debug)]
struct Voice {
    event: SoundEvent,
    ends_ms: u64,
}

/// Sample paths deduplicated into [`SampleId`]s.
#[derive(Default)]
struct Interner {
    samples: Vec<String>,
    by_path: BTreeMap<String, SampleId>,
}

impl Interner {
    fn intern(&mut self, path: &str) -> SampleId {
        if let Some(id) = self.by_path.get(path) {
            return *id;
        }
        let id = SampleId(self.samples.len() as u16);
        self.samples.push(path.to_owned());
        self.by_path.insert(path.to_owned(), id);
        id
    }

    fn len(&self) -> u16 {
        self.samples.len() as u16
    }
}

/// A play the frame decided on, before the rate limits.
struct Candidate {
    event: SoundEvent,
    sample: SampleId,
    gain: f32,
    pan: f32,
}

pub struct EventRouter {
    /// Every sample path in [`SampleId`] order (the sink loads them in
    /// this order).
    samples: Vec<String>,
    durations: Vec<u32>,
    events: [Option<EventTable>; SoundEvent::ALL.len()],
    units: BTreeMap<Handle<UnitType>, UnitOverride>,
    roar_ref: u32,
    zoom_far: f32,
    zoom_near: f32,
    max_voices: u16,
    cull_pad: f32,
    last_play: BTreeMap<SoundEvent, u64>,
    voices: Vec<Voice>,
}

impl EventRouter {
    /// Builds the tables from a sound set and the units' overrides.
    pub fn new(set: &SoundSet, units: &Registry<UnitType>) -> Self {
        let mut interner = Interner::default();
        let mut events = [None; SoundEvent::ALL.len()];
        for (event, sounds) in &set.events {
            let first = interner.len();
            for p in &sounds.samples {
                interner.intern(p);
            }
            events[*event as usize] = Some(EventTable {
                first,
                count: interner.len().saturating_sub(first),
                min_interval_ms: sounds.min_interval_ms,
                max_voices: sounds.max_voices,
                gain: sounds.gain,
            });
        }
        let mut unit_overrides = BTreeMap::new();
        for (h, unit) in units.iter() {
            let cavalry = unit.category == UnitCategory::Cavalry;
            let charge = unit.sounds.charge.as_deref().map(|p| interner.intern(p));
            let die = unit.sounds.die.as_deref().map(|p| interner.intern(p));
            if cavalry || charge.is_some() || die.is_some() {
                unit_overrides.insert(
                    h,
                    UnitOverride {
                        charge,
                        die,
                        cavalry,
                    },
                );
            }
        }
        let samples = interner.samples;
        let durations = vec![DEFAULT_DURATION_MS; samples.len()];
        Self {
            samples,
            durations,
            events,
            units: unit_overrides,
            roar_ref: set.roar.ref_engaged,
            zoom_far: set.zoom.far,
            zoom_near: set.zoom.near,
            max_voices: set.max_voices,
            cull_pad: set.cull_pad_m,
            last_play: BTreeMap::new(),
            voices: Vec::new(),
        }
    }

    /// The sample paths in [`SampleId`] order, relative to the assets root.
    pub fn sample_paths(&self) -> &[String] {
        &self.samples
    }

    /// Sample lengths in milliseconds, in [`SampleId`] order (from the
    /// engine after loading); missing entries keep the default.
    pub fn set_durations(&mut self, ms: &[u32]) {
        for (slot, d) in self.durations.iter_mut().zip(ms) {
            *slot = *d;
        }
    }

    /// Voices that have not ended by `now_ms`.
    pub fn live_voices(&self, now_ms: u64) -> usize {
        self.voices.iter().filter(|v| v.ends_ms > now_ms).count()
    }

    /// Forgets the rate-limit state and silences the sink (a battle ended
    /// or was dropped).
    pub fn reset(&mut self, sink: &mut dyn AudioSink) {
        self.last_play.clear();
        self.voices.clear();
        sink.stop_all();
    }

    /// Routes one frame's events.
    pub fn route(
        &mut self,
        f: &FrameInput,
        events: &[BattleEvent],
        regiments: &[RegimentPos],
        sink: &mut dyn AudioSink,
    ) {
        let near = near_weight(f.zoom, self.zoom_far, self.zoom_near);
        sink.set_mix(near, roar_gain(f.engaged, self.roar_ref, near));
        self.voices.retain(|v| v.ends_ms > f.now_ms);

        let by_id: BTreeMap<RegimentId, &RegimentPos> =
            regiments.iter().map(|r| (r.id, r)).collect();
        let mut started = 0usize;
        for e in events {
            if started >= MAX_PLAYS_PER_FRAME {
                break;
            }
            let Some(c) = self.candidate(f, e, &by_id) else {
                continue;
            };
            if !self.admit(c.event, f.now_ms) {
                continue;
            }
            let duration = self
                .durations
                .get(usize::from(c.sample.0))
                .copied()
                .unwrap_or(DEFAULT_DURATION_MS);
            self.voices.push(Voice {
                event: c.event,
                ends_ms: f.now_ms + u64::from(duration),
            });
            self.last_play.insert(c.event, f.now_ms);
            sink.play(&PlayRequest {
                sample: c.sample,
                gain: c.gain,
                pan: c.pan,
            });
            started += 1;
        }
    }

    /// The rate limits (plan I5): the event's interval and voice cap, the
    /// global voice cap.
    fn admit(&self, event: SoundEvent, now_ms: u64) -> bool {
        let Some(table) = self.events[event as usize] else {
            return false;
        };
        if let Some(last) = self.last_play.get(&event)
            && now_ms.saturating_sub(*last) < u64::from(table.min_interval_ms)
            && now_ms != *last
        {
            return false;
        }
        if let Some(last) = self.last_play.get(&event)
            && *last == now_ms
            && table.min_interval_ms > 0
        {
            // Two plays in the same frame count as closer than any interval.
            return false;
        }
        let same = self.voices.iter().filter(|v| v.event == event).count();
        if same >= usize::from(table.max_voices) {
            return false;
        }
        self.voices.len() < usize::from(self.max_voices)
    }

    /// Maps an event to a sound, picks the variant and weighs it by
    /// position; `None` for silent events, missing keys and culled
    /// positions (plan I2, I3).
    fn candidate(
        &self,
        f: &FrameInput,
        e: &BattleEvent,
        by_id: &BTreeMap<RegimentId, &RegimentPos>,
    ) -> Option<Candidate> {
        let anchor_of = |id: RegimentId| by_id.get(&id).copied();
        // (event key, world position, variant salt, unit override sample)
        let (event, pos, salt, override_sample): (
            SoundEvent,
            Option<[f32; 2]>,
            u64,
            Option<SampleId>,
        ) = match e {
            BattleEvent::Charge { regiment: id, .. } => {
                let r = anchor_of(*id)?;
                let o = self.units.get(&r.unit).copied().unwrap_or_default();
                let event = if o.charge.is_none() && o.cavalry {
                    SoundEvent::CavalryCharge
                } else {
                    SoundEvent::Charge
                };
                (event, Some(r.pos), u64::from(id.0), o.charge)
            }
            BattleEvent::Engaged { regiment: id } => {
                let r = anchor_of(*id)?;
                (SoundEvent::Clash, Some(r.pos), u64::from(id.0), None)
            }
            BattleEvent::VolleyFired { regiment: id, .. } => {
                let r = anchor_of(*id)?;
                (SoundEvent::Volley, Some(r.pos), u64::from(id.0), None)
            }
            BattleEvent::ProjectileLanded { pos, hit: true, .. } => (
                SoundEvent::ArrowHit,
                Some(world(pos)),
                (pos.x.to_f32_render() * 7.0) as u64,
                None,
            ),
            BattleEvent::SoldierDied {
                id, regiment, pos, ..
            } => {
                let o = anchor_of(*regiment)
                    .and_then(|r| self.units.get(&r.unit).copied())
                    .unwrap_or_default();
                let event = if o.die.is_none() && o.cavalry {
                    SoundEvent::CavalryDeath
                } else {
                    SoundEvent::Death
                };
                (event, Some(world(pos)), u64::from(id.0), o.die)
            }
            BattleEvent::MoraleChanged {
                regiment: id,
                to: MoraleState::Routing,
                ..
            } => {
                let r = anchor_of(*id)?;
                (SoundEvent::Rout, Some(r.pos), u64::from(id.0), None)
            }
            BattleEvent::Rallied { regiment: id } => {
                let r = anchor_of(*id)?;
                (SoundEvent::Rally, Some(r.pos), u64::from(id.0), None)
            }
            BattleEvent::Shattered { regiment: id } => {
                let r = anchor_of(*id)?;
                (SoundEvent::Shatter, Some(r.pos), u64::from(id.0), None)
            }
            BattleEvent::GeneralDied { side, .. } => {
                (SoundEvent::GeneralDied, None, u64::from(*side), None)
            }
            BattleEvent::AbilityUsed { regiment: id, .. } => {
                let r = anchor_of(*id)?;
                (SoundEvent::Ability, Some(r.pos), u64::from(id.0), None)
            }
            BattleEvent::PhaseChanged { to, .. } => (SoundEvent::Phase, None, *to as u64 % 4, None),
            BattleEvent::Ended { result } => {
                let event = match (result.winner, f.observer_side) {
                    (Some(w), Some(o)) if w == o => SoundEvent::Victory,
                    (Some(_), Some(_)) => SoundEvent::Defeat,
                    _ => SoundEvent::Phase,
                };
                (event, None, 0, None)
            }
            _ => return None,
        };
        let table = self.events[event as usize];
        let (sample, gain) = match (override_sample, table) {
            (Some(s), t) => (s, t.map_or(1.0, |t| t.gain)),
            (None, Some(t)) if t.count > 0 => {
                let variant = (u64::from(f.tick.0) + salt) % u64::from(t.count);
                (SampleId(t.first + variant as u16), t.gain)
            }
            _ => return None,
        };
        let (gain, pan) = match pos {
            None => (gain, 0.0),
            Some(p) => {
                let (min, max) = f.bounds;
                if p[0] < min[0] - self.cull_pad
                    || p[0] > max[0] + self.cull_pad
                    || p[1] < min[1] - self.cull_pad
                    || p[1] > max[1] + self.cull_pad
                {
                    return None;
                }
                let half = [(max[0] - min[0]) * 0.5, (max[1] - min[1]) * 0.5];
                let half_diag = (half[0] * half[0] + half[1] * half[1]).sqrt().max(1.0);
                let dx = p[0] - f.camera_center[0];
                let dy = p[1] - f.camera_center[1];
                let d = (dx * dx + dy * dy).sqrt();
                let falloff = (1.0 - 0.5 * d / half_diag).clamp(0.0, 1.0);
                let pan = (dx / half[0].max(1.0)).clamp(-1.0, 1.0);
                (gain * falloff, pan)
            }
        };
        Some(Candidate {
            event,
            sample,
            gain,
            pan,
        })
    }
}

fn world(p: &il_core::V2) -> [f32; 2] {
    [p.x.to_f32_render(), p.y.to_f32_render()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_core::{SoldierId, V2};
    use il_data::{ContentId, EventSounds, Roar, Zoom};
    use il_sim_battle::BattlePhase;
    use il_sim_battle::BattleResult;

    use crate::sink::NullSink;

    fn set() -> SoundSet {
        let mut events = BTreeMap::new();
        let ev = |samples: &[&str], min_interval_ms: u32, max_voices: u8| EventSounds {
            samples: samples.iter().map(|s| (*s).to_owned()).collect(),
            min_interval_ms,
            max_voices,
            gain: 1.0,
        };
        events.insert(
            SoundEvent::Clash,
            ev(&["clash_1.wav", "clash_2.wav"], 200, 4),
        );
        events.insert(SoundEvent::Death, ev(&["death.wav"], 0, 2));
        events.insert(SoundEvent::CavalryDeath, ev(&["horse.wav"], 0, 2));
        events.insert(SoundEvent::Charge, ev(&["charge.wav"], 0, 4));
        events.insert(SoundEvent::Victory, ev(&["victory.wav"], 0, 1));
        events.insert(SoundEvent::Defeat, ev(&["defeat.wav"], 0, 1));
        events.insert(SoundEvent::Phase, ev(&["phase.wav"], 0, 1));
        SoundSet {
            id: ContentId::new("test:set").unwrap(),
            events,
            roar: Roar {
                sample: "roar.wav".to_owned(),
                ref_engaged: 100,
            },
            zoom: Zoom {
                far: 3.0,
                near: 24.0,
            },
            max_voices: 5,
            cull_pad_m: 10.0,
            deprecated: None,
        }
    }

    fn frame(zoom: f32, now_ms: u64) -> FrameInput {
        FrameInput {
            camera_center: [100.0, 100.0],
            zoom,
            bounds: ([0.0, 0.0], [200.0, 200.0]),
            engaged: 0,
            now_ms,
            observer_side: Some(0),
            tick: Tick(10),
        }
    }

    fn router() -> EventRouter {
        EventRouter::new(&set(), &Registry::new())
    }

    fn test_units() -> Registry<UnitType> {
        il_data::Registries::load_root(&test_game_root())
            .expect("the flagship content loads")
            .units
    }

    fn test_game_root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game")
    }

    /// One regiment at the camera centre; any handle does for a router
    /// built without unit overrides.
    fn regs() -> Vec<RegimentPos> {
        let unit = test_units().iter().next().expect("one unit").0;
        vec![RegimentPos {
            id: RegimentId(0),
            side: 0,
            pos: [100.0, 100.0],
            unit,
        }]
    }

    fn engaged(id: u32) -> BattleEvent {
        BattleEvent::Engaged {
            regiment: RegimentId(id),
        }
    }

    fn died(id: u32, regiment: u32, pos: [f32; 2]) -> BattleEvent {
        BattleEvent::SoldierDied {
            id: SoldierId(id),
            regiment: RegimentId(regiment),
            killer: None,
            pos: V2::from_f32_data(pos[0], pos[1]),
        }
    }

    #[test]
    fn zoom_curve_endpoints_and_midpoint() {
        assert_eq!(near_weight(3.0, 3.0, 24.0), 0.0);
        assert_eq!(near_weight(2.0, 3.0, 24.0), 0.0);
        assert_eq!(near_weight(24.0, 3.0, 24.0), 1.0);
        assert_eq!(near_weight(96.0, 3.0, 24.0), 1.0);
        // Geometric midpoint of 3..24 is sqrt(72) ≈ 8.49.
        assert!((near_weight(72f32.sqrt(), 3.0, 24.0) - 0.5).abs() < 1e-4);
        assert_eq!(near_weight(12.0, 24.0, 3.0), 0.0, "far >= near is silent");
    }

    #[test]
    fn roar_follows_the_engaged_count_and_halves_near() {
        assert_eq!(roar_gain(0, 100, 0.0), 0.0);
        assert_eq!(roar_gain(100, 100, 0.0), 1.0);
        assert_eq!(roar_gain(500, 100, 0.0), 1.0);
        assert_eq!(roar_gain(100, 100, 1.0), 0.5);
        assert_eq!(roar_gain(100, 0, 0.0), 0.0);
    }

    #[test]
    fn far_zoom_plays_only_the_roar() {
        let mut r = router();
        let mut sink = NullSink::default();
        let mut f = frame(2.0, 1000);
        f.engaged = 50;
        r.route(&f, &[engaged(0)], &regs(), &mut sink);
        assert_eq!(sink.mix, (0.0, 0.5));
        // The clash still went out as a play; the effects track is at 0.
        assert_eq!(sink.plays.len(), 1);
        let f = frame(48.0, 2000);
        r.route(&f, &[], &regs(), &mut sink);
        assert_eq!(sink.mix.0, 1.0);
    }

    #[test]
    fn min_interval_drops_the_second_clash() {
        let mut r = router();
        let mut sink = NullSink::default();
        r.route(&frame(12.0, 1000), &[engaged(0)], &regs(), &mut sink);
        r.route(&frame(12.0, 1100), &[engaged(0)], &regs(), &mut sink);
        assert_eq!(sink.plays.len(), 1, "100 ms < 200 ms interval");
        r.route(&frame(12.0, 1250), &[engaged(0)], &regs(), &mut sink);
        assert_eq!(sink.plays.len(), 2);
        // Two in one frame: only the first.
        r.route(
            &frame(12.0, 5000),
            &[engaged(0), engaged(0)],
            &regs(),
            &mut sink,
        );
        assert_eq!(sink.plays.len(), 3);
    }

    #[test]
    fn voice_caps_per_event_and_global() {
        let mut r = router();
        r.set_durations(&[1000; 8]);
        let mut sink = NullSink::default();
        let deaths: Vec<BattleEvent> = (0..6).map(|i| died(i, 0, [100.0, 100.0])).collect();
        r.route(&frame(12.0, 1000), &deaths, &regs(), &mut sink);
        assert_eq!(sink.plays.len(), 2, "death max_voices 2");
        // Voices end after their duration.
        r.route(&frame(12.0, 2100), &deaths, &regs(), &mut sink);
        assert_eq!(sink.plays.len(), 4);
        // The global cap of 5 over every event.
        let mut r = router();
        r.set_durations(&[10_000; 8]);
        let mut sink = NullSink::default();
        let mut events = vec![engaged(0)];
        events.extend((0..6).map(|i| died(i, 0, [100.0, 100.0])));
        events.push(BattleEvent::Charge {
            regiment: RegimentId(0),
            target: RegimentId(1),
        });
        events.push(BattleEvent::PhaseChanged {
            from: BattlePhase::Deployment,
            to: BattlePhase::Battle,
        });
        // A second charge would be the sixth voice; general_died has no
        // samples in this set and is silent.
        events.push(BattleEvent::Charge {
            regiment: RegimentId(0),
            target: RegimentId(1),
        });
        events.push(BattleEvent::GeneralDied {
            side: 0,
            soldier: SoldierId(1),
        });
        r.route(&frame(12.0, 1000), &events, &regs(), &mut sink);
        assert_eq!(
            sink.plays.len(),
            5,
            "clash, 2 deaths, charge, phase; the global cap"
        );
        assert_eq!(r.live_voices(1000), 5);
    }

    #[test]
    fn events_outside_the_view_are_culled_and_inside_are_panned() {
        let mut r = router();
        let mut sink = NullSink::default();
        let events = [
            died(1, 0, [400.0, 100.0]),
            died(2, 0, [190.0, 100.0]),
            died(3, 0, [100.0, 100.0]),
        ];
        r.route(&frame(12.0, 1000), &events, &regs(), &mut sink);
        assert_eq!(sink.plays.len(), 2);
        assert!(
            sink.plays[0].pan > 0.8,
            "right of centre: {:?}",
            sink.plays[0]
        );
        assert!(
            sink.plays[0].gain < sink.plays[1].gain,
            "farther is quieter"
        );
        assert_eq!(sink.plays[1].pan, 0.0);
        assert_eq!(sink.plays[1].gain, 1.0);
    }

    #[test]
    fn cavalry_fall_back_to_their_own_samples_and_overrides_win() {
        let units = test_units();
        let cavalry = units
            .lookup(&ContentId::new("persia:cavalry").unwrap())
            .expect("cavalry");
        let hastati = units
            .lookup(&ContentId::new("rome:hastati").unwrap())
            .expect("hastati");
        let mut r = EventRouter::new(&set(), &units);
        let paths = r.sample_paths().to_vec();
        let horse = paths.iter().position(|p| p == "horse.wav").unwrap();
        let override_die = paths
            .iter()
            .position(|p| p == "sounds/cavalry_death.wav")
            .expect("the unit's own die sample is interned");
        let regiments = [
            RegimentPos {
                id: RegimentId(0),
                side: 0,
                pos: [100.0, 100.0],
                unit: cavalry,
            },
            RegimentPos {
                id: RegimentId(1),
                side: 0,
                pos: [100.0, 100.0],
                unit: hastati,
            },
        ];
        let mut sink = NullSink::default();
        r.route(
            &frame(12.0, 1000),
            &[died(1, 0, [100.0, 100.0]), died(2, 1, [100.0, 100.0])],
            &regiments,
            &mut sink,
        );
        assert_eq!(sink.plays.len(), 2);
        // The flagship cavalry carry `sounds.die`, so the override wins
        // over the set's cavalry_death.
        assert_eq!(usize::from(sink.plays[0].sample.0), override_die);
        assert_ne!(usize::from(sink.plays[0].sample.0), horse);
        let death = paths.iter().position(|p| p == "death.wav").unwrap();
        assert_eq!(usize::from(sink.plays[1].sample.0), death);
    }

    #[test]
    fn variant_choice_is_stable_for_the_same_tick_and_id() {
        let mut r = router();
        let mut sink = NullSink::default();
        r.route(&frame(12.0, 1000), &[engaged(0)], &regs(), &mut sink);
        let mut r2 = router();
        let mut sink2 = NullSink::default();
        r2.route(&frame(12.0, 1000), &[engaged(0)], &regs(), &mut sink2);
        assert_eq!(sink.plays, sink2.plays);
        let mut f = frame(12.0, 5000);
        f.tick = Tick(11);
        r.route(&f, &[engaged(0)], &regs(), &mut sink);
        assert_ne!(
            sink.plays[0].sample, sink.plays[1].sample,
            "the other clash variant"
        );
    }

    #[test]
    fn ended_maps_to_victory_defeat_or_phase() {
        let ended = |winner: Option<u8>| BattleEvent::Ended {
            result: Box::new(BattleResult {
                winner,
                duration_ticks: 1,
                sides: Vec::new(),
                summary: Default::default(),
            }),
        };
        let paths: Vec<String> = router().sample_paths().to_vec();
        let name = |s: &PlayRequest| paths[usize::from(s.sample.0)].clone();
        let mut r = router();
        let mut sink = NullSink::default();
        r.route(&frame(12.0, 1000), &[ended(Some(0))], &regs(), &mut sink);
        assert_eq!(name(&sink.plays[0]), "victory.wav");
        r.route(&frame(12.0, 2000), &[ended(Some(1))], &regs(), &mut sink);
        assert_eq!(name(&sink.plays[1]), "defeat.wav");
        r.route(&frame(12.0, 3000), &[ended(None)], &regs(), &mut sink);
        assert_eq!(name(&sink.plays[2]), "phase.wav");
        let mut f = frame(12.0, 4000);
        f.observer_side = None;
        r.route(&f, &[ended(Some(0))], &regs(), &mut sink);
        assert_eq!(
            name(&sink.plays[3]),
            "phase.wav",
            "a spectator hears the drum"
        );
    }

    #[test]
    fn reset_clears_the_limits_and_stops_the_sink() {
        let mut r = router();
        let mut sink = NullSink::default();
        r.route(&frame(12.0, 1000), &[engaged(0)], &regs(), &mut sink);
        r.reset(&mut sink);
        assert_eq!(sink.stops, 1);
        assert_eq!(r.live_voices(1000), 0);
        r.route(&frame(12.0, 1010), &[engaged(0)], &regs(), &mut sink);
        assert_eq!(sink.plays.len(), 2, "the interval was forgotten");
    }
}
