//! Per-stage tick timings (T1-060, REQ-TOOL-003). Implements the sim's
//! `StageObserver` with the wall clock, which only the app may read.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use il_sim_battle::{Stage, StageObserver};
use il_ui::{ProfilerStats, StageStat};

/// Ticks kept for the means and maxima.
const WINDOW_TICKS: usize = 60;

pub struct Profiler {
    stage_start: Option<Instant>,
    tick_start: Option<Instant>,
    /// Stage durations of the tick in progress.
    current: [Duration; Stage::COUNT],
    /// Per-tick stage durations, newest last.
    ring: VecDeque<[f32; Stage::COUNT]>,
    tick_totals: VecDeque<f32>,
    frame_ms: f32,
    ticks_last_frame: u32,
}

impl Default for Profiler {
    fn default() -> Self {
        Self {
            stage_start: None,
            tick_start: None,
            current: [Duration::ZERO; Stage::COUNT],
            ring: VecDeque::with_capacity(WINDOW_TICKS),
            tick_totals: VecDeque::with_capacity(WINDOW_TICKS),
            frame_ms: 0.0,
            ticks_last_frame: 0,
        }
    }
}

impl Profiler {
    /// Call once per frame with the frame's wall time.
    pub fn frame(&mut self, frame_seconds: f64, ticks_stepped: u32) {
        let ms = frame_seconds as f32 * 1000.0;
        self.frame_ms = if self.frame_ms == 0.0 {
            ms
        } else {
            self.frame_ms * 0.9 + ms * 0.1
        };
        self.ticks_last_frame = ticks_stepped;
    }

    pub fn stats(&self) -> ProfilerStats {
        let n = self.ring.len().max(1) as f32;
        let stages = Stage::ALL
            .iter()
            .map(|stage| {
                let i = stage.index();
                let last = self.ring.back().map_or(0.0, |t| t[i]);
                let mean = self.ring.iter().map(|t| t[i]).sum::<f32>() / n;
                let max = self.ring.iter().map(|t| t[i]).fold(0.0, f32::max);
                StageStat {
                    name: stage.name(),
                    last_ms: last,
                    mean_ms: mean,
                    max_ms: max,
                }
            })
            .collect();
        ProfilerStats {
            stages,
            tick_last_ms: self.tick_totals.back().copied().unwrap_or(0.0),
            tick_mean_ms: self.tick_totals.iter().sum::<f32>() / n,
            tick_max_ms: self.tick_totals.iter().copied().fold(0.0, f32::max),
            ticks_sampled: self.ring.len() as u32,
            frame_ms: self.frame_ms,
            fps: if self.frame_ms > 0.0 {
                1000.0 / self.frame_ms
            } else {
                0.0
            },
            ticks_last_frame: self.ticks_last_frame,
            ..ProfilerStats::default()
        }
    }
}

impl StageObserver for Profiler {
    fn begin(&mut self, stage: Stage) {
        let now = Instant::now();
        if stage == Stage::ALL[0] {
            self.tick_start = Some(now);
            self.current = [Duration::ZERO; Stage::COUNT];
        }
        self.stage_start = Some(now);
    }

    fn end(&mut self, stage: Stage) {
        let now = Instant::now();
        if let Some(start) = self.stage_start.take() {
            self.current[stage.index()] = now.duration_since(start);
        }
        if stage == Stage::ALL[Stage::COUNT - 1] {
            let ms = self.current.map(|d| d.as_secs_f32() * 1000.0);
            if self.ring.len() == WINDOW_TICKS {
                self.ring.pop_front();
                self.tick_totals.pop_front();
            }
            self.ring.push_back(ms);
            let total = self
                .tick_start
                .take()
                .map_or(0.0, |s| now.duration_since(s).as_secs_f32() * 1000.0);
            self.tick_totals.push_back(total);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_accumulates_a_tick_and_keeps_a_window() {
        let mut p = Profiler::default();
        for _ in 0..(WINDOW_TICKS + 5) {
            for stage in Stage::ALL {
                p.begin(stage);
                p.end(stage);
            }
        }
        let stats = p.stats();
        assert_eq!(stats.ticks_sampled, WINDOW_TICKS as u32);
        assert_eq!(stats.stages.len(), Stage::COUNT);
        assert_eq!(stats.stages[0].name, "ApplyCommands");
        assert_eq!(stats.stages[17].name, "EventsAndHash");
        assert!(stats.tick_mean_ms >= 0.0);
        p.frame(0.016, 1);
        assert!((p.stats().frame_ms - 16.0).abs() < 0.01);
        assert_eq!(p.stats().ticks_last_frame, 1);
    }
}
