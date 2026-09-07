//! What the router talks to: a sink that plays samples and holds the mix.

/// Index into the router's sample list (`EventRouter::sample_paths`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SampleId(pub u16);

/// One play: which sample, at what linear gain, panned where (−1 left,
/// +1 right).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayRequest {
    pub sample: SampleId,
    pub gain: f32,
    pub pan: f32,
}

/// The settings' sliders, linear 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volumes {
    pub master: f32,
    pub effects: f32,
    pub music: f32,
}

impl Default for Volumes {
    fn default() -> Self {
        Self {
            master: 1.0,
            effects: 1.0,
            music: 1.0,
        }
    }
}

pub trait AudioSink {
    /// Plays a sample on the effects track.
    fn play(&mut self, req: &PlayRequest);
    /// The zoom mix (plan I3): the effects track's gain and the roar
    /// loop's gain, both linear 0..1, before the settings' volumes.
    fn set_mix(&mut self, effects: f32, roar: f32);
    /// The settings' volumes.
    fn set_volumes(&mut self, v: Volumes);
    /// The battle is over or gone: stop the roar, silence the mix.
    fn stop_all(&mut self);
}

/// Records every call; the tests' sink and the no-device fallback.
#[derive(Debug, Default)]
pub struct NullSink {
    pub plays: Vec<PlayRequest>,
    pub mix: (f32, f32),
    pub volumes: Option<Volumes>,
    pub stops: u32,
}

impl AudioSink for NullSink {
    fn play(&mut self, req: &PlayRequest) {
        self.plays.push(*req);
    }

    fn set_mix(&mut self, effects: f32, roar: f32) {
        self.mix = (effects, roar);
    }

    fn set_volumes(&mut self, v: Volumes) {
        self.volumes = Some(v);
    }

    fn stop_all(&mut self) {
        self.stops += 1;
        self.mix = (0.0, 0.0);
    }
}
