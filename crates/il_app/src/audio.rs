//! The app's audio wiring (T2-100, TDD §12, §15): the kira engine when a
//! device exists, the event router, and the frame's inputs from the camera
//! and the view. Without a device (or with `--mute`'s master at 0) the
//! router still runs, into a null sink, so the code path is the same.

use std::path::Path;

use glam::Vec2;
use il_audio::{AudioEngine, AudioSink, EventRouter, FrameInput, NullSink, RegimentPos, Volumes};
use il_core::Scalar;
use il_data::{ContentId, Registries};
use il_render::Camera;
use il_sim_battle::{BattleEvent, BattleView, StepOutput};

use crate::settings::Volume;

pub struct AppAudio {
    engine: Option<AudioEngine>,
    null: NullSink,
    router: Option<EventRouter>,
    muted: bool,
    /// The events of every tick stepped this frame.
    events: Vec<BattleEvent>,
}

impl AppAudio {
    /// Opens the device; a failure is one line on stderr and silence
    /// (plan decision 8).
    pub fn new(mute: bool, volumes: &Volume) -> Self {
        let engine = match AudioEngine::try_new() {
            Ok(e) => Some(e),
            Err(reason) => {
                eprintln!("audio disabled: {reason}");
                None
            }
        };
        let mut a = Self {
            engine,
            null: NullSink::default(),
            router: None,
            muted: mute,
            events: Vec::new(),
        };
        a.set_volumes(volumes);
        a
    }

    fn sink(&mut self) -> &mut dyn AudioSink {
        match self.engine.as_mut() {
            Some(e) => e,
            None => &mut self.null,
        }
    }

    /// The settings' sliders; `--mute` keeps the master at 0.
    pub fn set_volumes(&mut self, v: &Volume) {
        let volumes = Volumes {
            master: if self.muted { 0.0 } else { v.master },
            effects: v.effects,
            music: v.music,
        };
        self.sink().set_volumes(volumes);
    }

    pub fn has_battle(&self) -> bool {
        self.router.is_some()
    }

    /// Keeps this frame's events for [`Self::frame`].
    pub fn collect(&mut self, outputs: &[StepOutput]) {
        for o in outputs {
            self.events.extend(o.events.iter().cloned());
        }
    }

    /// Picks the observer's faction's sound set (plan I1), loads its
    /// samples and starts the roar loop silent.
    pub fn start_battle(
        &mut self,
        regs: &Registries,
        faction: Option<&ContentId>,
        assets_root: &Path,
    ) {
        self.stop_battle();
        let from_faction = faction
            .and_then(|id| regs.factions.lookup(id))
            .and_then(|h| regs.factions.get(h).sound_set_handle);
        let set = match from_faction {
            Some(h) => regs.sound_sets.get(h),
            None => match regs.sound_sets.iter().next() {
                Some((_, set)) => set,
                None => return,
            },
        };
        let mut router = EventRouter::new(set, &regs.units);
        if let Some(engine) = self.engine.as_mut() {
            let (durations, warnings) =
                engine.load(router.sample_paths(), &set.roar.sample, assets_root);
            for w in warnings {
                eprintln!("audio: {w}");
            }
            router.set_durations(&durations);
        }
        self.router = Some(router);
    }

    /// Stops the roar and forgets the rate limits.
    pub fn stop_battle(&mut self) {
        if let Some(mut router) = self.router.take() {
            let sink: &mut dyn AudioSink = match self.engine.as_mut() {
                Some(e) => e,
                None => &mut self.null,
            };
            router.reset(sink);
        }
        self.events.clear();
    }

    /// Routes the frame's events with the camera as the listener.
    pub fn frame(
        &mut self,
        view: &BattleView,
        camera: &Camera,
        screen: Vec2,
        now_ms: u64,
        observer_side: Option<u8>,
    ) {
        let Some(router) = self.router.as_mut() else {
            self.events.clear();
            return;
        };
        let (min, max) = camera.visible_bounds(screen, 0.0);
        let mut engaged = 0u32;
        let regiments: Vec<RegimentPos> = view
            .regiments()
            .map(|r| {
                if r.engaged {
                    engaged += r.soldier_count;
                }
                RegimentPos {
                    id: r.id,
                    side: r.side,
                    pos: [
                        r.anchor_pos.x.to_f32_render(),
                        r.anchor_pos.y.to_f32_render(),
                    ],
                    unit: r.unit,
                }
            })
            .collect();
        let input = FrameInput {
            camera_center: [camera.center.x, camera.center.y],
            zoom: camera.zoom,
            bounds: ([min.x, min.y], [max.x, max.y]),
            engaged,
            now_ms,
            observer_side,
            tick: view.tick(),
        };
        let sink: &mut dyn AudioSink = match self.engine.as_mut() {
            Some(e) => e,
            None => &mut self.null,
        };
        router.route(&input, &self.events, &regiments, sink);
        self.events.clear();
        // The null sink only records; keep it from growing forever.
        self.null.plays.clear();
    }
}
