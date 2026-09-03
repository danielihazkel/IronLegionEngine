//! `--bench-sprites`: the T1-051 synthetic test. Draws 32,768 instances with
//! vsync off for a fixed number of frames and prints the frame time.

use il_render::{AtlasId, SpriteInstance, SpriteScene};

pub const INSTANCES: u32 = 32_768;
pub const FRAMES: u32 = 300;

pub struct SpriteBench {
    pub scene: SpriteScene,
    pub frames_done: u32,
    pub seconds: f64,
    pub worst: f64,
}

impl SpriteBench {
    /// A deterministic scattering over the screen, cycling through atlases.
    pub fn new(atlases: &[AtlasId], width: f32, height: f32) -> Self {
        let mut scene = SpriteScene::default();
        let mut state: u32 = 0x9E37_79B9;
        let mut next = move || {
            // LCG; quality is irrelevant, determinism is not.
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / (1u32 << 24) as f32
        };
        let per_atlas = INSTANCES / atlases.len().max(1) as u32;
        for (i, atlas) in atlases.iter().enumerate() {
            let tint = [
                [220, 60, 60, 255],
                [60, 90, 220, 255],
                [230, 200, 80, 255],
                [80, 200, 120, 255],
                [200, 120, 220, 255],
                [220, 220, 220, 255],
            ][i % 6];
            let instances = (0..per_atlas).map(|_| {
                let y = next() * height;
                SpriteInstance {
                    pos: [next() * width, y],
                    depth: 1.0 - y / height,
                    frame_facing: SpriteInstance::pack_frame_facing(
                        (next() * 5.0) as u32,
                        (next() * 8.0) as u8,
                    ),
                    tint,
                    scale: 0.5,
                    flags: 0,
                    _reserved: 0,
                }
            });
            scene.push_batch(*atlas, instances);
        }
        Self {
            scene,
            frames_done: 0,
            seconds: 0.0,
            worst: 0.0,
        }
    }

    pub fn record(&mut self, frame_seconds: f64) {
        self.frames_done += 1;
        self.seconds += frame_seconds;
        self.worst = self.worst.max(frame_seconds);
    }

    pub fn done(&self) -> bool {
        self.frames_done >= FRAMES
    }

    /// Prints the result; returns whether the average beat 60 FPS.
    pub fn report(&self) -> bool {
        let avg_ms = self.seconds * 1000.0 / f64::from(self.frames_done.max(1));
        let fps = if avg_ms > 0.0 { 1000.0 / avg_ms } else { 0.0 };
        let pass = fps > 60.0;
        println!(
            "sprite bench: {} instances, {} frames, avg {avg_ms:.3} ms ({fps:.0} FPS), worst {:.3} ms -> {}",
            self.scene.instances.len(),
            self.frames_done,
            self.worst * 1000.0,
            if pass {
                "PASS (> 60 FPS)"
            } else {
                "FAIL (<= 60 FPS)"
            }
        );
        pass
    }
}
