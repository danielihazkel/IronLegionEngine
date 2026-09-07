//! `AudioEngine`: the kira-backed [`AudioSink`] (TDD §12, plan I6).
//!
//! Tracks: the main track carries the master volume; `effects` (the
//! settings' effects slider × the zoom mix), `roar` under it (the roar
//! loop, its gain from the engaged count) and `music` (silent until
//! Phase 4). Gains are linear on the way in and decibels inside kira.

use std::path::Path;
use std::time::Duration;

use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Panning, Tween};

use crate::sink::{AudioSink, PlayRequest, Volumes};

/// How long a mix change (zoom, engaged count) takes to settle.
const MIX_TWEEN: Duration = Duration::from_millis(250);

/// Linear gain to kira decibels; 0 is silence.
pub fn decibels(gain: f32) -> Decibels {
    if gain <= 1e-4 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * gain.log10())
    }
}

pub struct AudioEngine {
    manager: AudioManager<DefaultBackend>,
    effects: TrackHandle,
    roar_track: TrackHandle,
    music: TrackHandle,
    samples: Vec<Option<StaticSoundData>>,
    roar: Option<StaticSoundData>,
    roar_handle: Option<StaticSoundHandle>,
    volumes: Volumes,
    mix_effects: f32,
    mix_roar: f32,
}

impl AudioEngine {
    /// Opens the default device; the error text is for one log line
    /// (plan decision 8: the app then runs without audio).
    pub fn try_new() -> Result<Self, String> {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| e.to_string())?;
        let mut effects = manager
            .add_sub_track(TrackBuilder::new())
            .map_err(|e| e.to_string())?;
        let roar_track = effects
            .add_sub_track(TrackBuilder::new().volume(Decibels::SILENCE))
            .map_err(|e| e.to_string())?;
        let music = manager
            .add_sub_track(TrackBuilder::new().volume(Decibels::SILENCE))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            manager,
            effects,
            roar_track,
            music,
            samples: Vec::new(),
            roar: None,
            roar_handle: None,
            volumes: Volumes::default(),
            mix_effects: 0.0,
            mix_roar: 0.0,
        })
    }

    /// Loads the router's samples (in `SampleId` order) and the roar loop
    /// from `assets_dir`, starts the loop silent, and returns the sample
    /// lengths in milliseconds plus one warning per file that failed.
    pub fn load(
        &mut self,
        sample_paths: &[String],
        roar_path: &str,
        assets_dir: &Path,
    ) -> (Vec<u32>, Vec<String>) {
        let mut warnings = Vec::new();
        self.stop_roar();
        self.samples = sample_paths
            .iter()
            .map(
                |rel| match StaticSoundData::from_file(assets_dir.join(rel)) {
                    Ok(data) => Some(data),
                    Err(e) => {
                        warnings.push(format!("{rel}: {e}"));
                        None
                    }
                },
            )
            .collect();
        let durations = self
            .samples
            .iter()
            .map(|s| s.as_ref().map_or(0, |d| d.duration().as_millis() as u32))
            .collect();
        self.roar = match StaticSoundData::from_file(assets_dir.join(roar_path)) {
            Ok(data) => Some(data.loop_region(..)),
            Err(e) => {
                warnings.push(format!("{roar_path}: {e}"));
                None
            }
        };
        if let Some(roar) = self.roar.clone() {
            self.roar_handle = self.roar_track.play(roar).ok();
        }
        (durations, warnings)
    }

    fn stop_roar(&mut self) {
        if let Some(mut h) = self.roar_handle.take() {
            h.stop(Tween::default());
        }
    }

    fn apply_volumes(&mut self, tween: Tween) {
        self.manager
            .main_track()
            .set_volume(decibels(self.volumes.master), tween);
        self.effects
            .set_volume(decibels(self.volumes.effects * self.mix_effects), tween);
        self.roar_track.set_volume(decibels(self.mix_roar), tween);
        self.music
            .set_volume(decibels(self.volumes.music * 0.0), tween);
    }
}

impl AudioSink for AudioEngine {
    fn play(&mut self, req: &PlayRequest) {
        let Some(data) = self
            .samples
            .get(usize::from(req.sample.0))
            .and_then(|s| s.clone())
        else {
            return;
        };
        let sound = data
            .volume(decibels(req.gain))
            .panning(Panning(req.pan.clamp(-1.0, 1.0)));
        // A full mixer (the sound limit) drops the play; nothing to report.
        let _ = self.effects.play(sound);
    }

    fn set_mix(&mut self, effects: f32, roar: f32) {
        let changed =
            (effects - self.mix_effects).abs() > 0.01 || (roar - self.mix_roar).abs() > 0.01;
        if !changed {
            return;
        }
        self.mix_effects = effects.clamp(0.0, 1.0);
        self.mix_roar = roar.clamp(0.0, 1.0);
        self.apply_volumes(Tween {
            duration: MIX_TWEEN,
            ..Tween::default()
        });
    }

    fn set_volumes(&mut self, v: Volumes) {
        self.volumes = Volumes {
            master: v.master.clamp(0.0, 1.0),
            effects: v.effects.clamp(0.0, 1.0),
            music: v.music.clamp(0.0, 1.0),
        };
        self.apply_volumes(Tween::default());
    }

    fn stop_all(&mut self) {
        self.stop_roar();
        self.mix_effects = 0.0;
        self.mix_roar = 0.0;
        self.apply_volumes(Tween::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_to_decibels() {
        assert_eq!(decibels(1.0), Decibels(0.0));
        assert_eq!(decibels(0.0), Decibels::SILENCE);
        assert!((decibels(0.5).0 + 6.02).abs() < 0.01);
    }

    /// With or without a device the constructor returns, never panics
    /// (decision 8).
    #[test]
    fn try_new_returns_without_panicking() {
        match AudioEngine::try_new() {
            Ok(mut engine) => {
                let (durations, warnings) = engine.load(
                    &["missing.wav".to_owned()],
                    "missing_roar.wav",
                    Path::new("nowhere"),
                );
                assert_eq!(durations, [0]);
                assert_eq!(warnings.len(), 2);
                engine.set_mix(0.5, 0.5);
                engine.play(&PlayRequest {
                    sample: crate::SampleId(0),
                    gain: 1.0,
                    pan: 0.0,
                });
                engine.stop_all();
            }
            Err(reason) => assert!(!reason.is_empty()),
        }
    }
}
