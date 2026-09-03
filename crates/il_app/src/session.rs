//! `BattleSession`: the fixed-step accumulator around one `BattleWorld`
//! (SAD §6.1, TDD §15, REQ-SIM-031).
//!
//! Speed multipliers scale the accumulator, never the tick length. Pause sets
//! the multiplier to zero and is also recorded as a `Pause` command so replays
//! and peers see it (SIM-DET-008).

use il_core::{PlayerId, TICK_SECONDS, Tick};
use il_sim_battle::{
    BattleWorld, Command, CommandKind, NoopObserver, ScriptedCommands, StageObserver, StepOutput,
};

/// Wall seconds per simulation tick, as the accumulator's type.
pub const TICK: f64 = TICK_SECONDS as f64;

/// The sim never runs more than this many ticks in one frame; beyond it the
/// sim visibly slows instead of spiralling (`app.max_catchup_ticks`).
pub const MAX_CATCHUP_TICKS: u32 = 4;

pub struct BattleSession {
    pub world: BattleWorld,
    accumulator: f64,
    speed: f32,
    paused: bool,
    local_player: PlayerId,
    input_delay: u32,
    next_seq: u16,
    /// Commands queued this frame, stamped for the tick they will run in.
    pending: Vec<Command>,
    /// The scenario's scripted stream, fed tick by tick (T1-081).
    script: ScriptedCommands,
    /// Every command handed to the sim, in order: the replay-to-be (T2-101).
    command_log: Vec<Command>,
}

impl BattleSession {
    pub fn new(world: BattleWorld, local_player: PlayerId, script: ScriptedCommands) -> Self {
        Self {
            world,
            accumulator: 0.0,
            speed: 1.0,
            paused: false,
            local_player,
            input_delay: 0,
            next_seq: 0,
            pending: Vec::new(),
            script,
            command_log: Vec::new(),
        }
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    /// Sets the speed multiplier and records it as a command (`mult_x100`).
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.0);
        let mult_x100 = (self.speed * 100.0).round().clamp(0.0, f32::from(u16::MAX)) as u16;
        self.queue(CommandKind::SetSpeed { mult_x100 });
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// Pauses or resumes; the `Pause` command is recorded either way.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.queue(CommandKind::Pause);
    }

    /// The tick the next `step` will simulate, plus the input delay.
    pub fn target_tick(&self) -> Tick {
        Tick(self.world.tick().0 + 1 + self.input_delay)
    }

    /// Queues a command from the local player for the next tick.
    pub fn queue(&mut self, kind: CommandKind) {
        let command = Command {
            tick: self.target_tick(),
            player: self.local_player,
            seq: self.next_seq,
            kind,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        self.pending.push(command);
    }

    /// Advances wall time by `dt` seconds and steps the sim zero or more
    /// times. Returns one `StepOutput` per tick stepped.
    #[allow(dead_code, reason = "headless convenience; the app always profiles")]
    pub fn advance(&mut self, dt: f64) -> Vec<StepOutput> {
        self.advance_with(dt, &mut NoopObserver)
    }

    /// [`advance`](Self::advance) with a stage observer (the profiler).
    pub fn advance_with(&mut self, dt: f64, observer: &mut dyn StageObserver) -> Vec<StepOutput> {
        let mult = if self.paused {
            0.0
        } else {
            f64::from(self.speed)
        };
        self.accumulator += dt.max(0.0) * mult;
        let cap = TICK * f64::from(MAX_CATCHUP_TICKS);
        if self.accumulator > cap {
            self.accumulator = cap;
        }
        let mut outputs = Vec::new();
        while self.accumulator >= TICK {
            outputs.push(self.step_once(observer));
            self.accumulator -= TICK;
        }
        outputs
    }

    fn step_once(&mut self, observer: &mut dyn StageObserver) -> StepOutput {
        let next = self.world.tick().next();
        let (mut now, later): (Vec<Command>, Vec<Command>) =
            self.pending.drain(..).partition(|c| c.tick <= next);
        self.pending = later;
        now.extend(self.script.take_for(next));
        self.command_log.extend(now.iter().cloned());
        self.world.step_observed(&now, observer)
    }

    /// Interpolation factor for rendering: how far into the next tick wall
    /// time has advanced, in `[0, 1)`.
    #[allow(dead_code, reason = "consumed by build_snapshot from T1-052")]
    pub fn alpha(&self) -> f32 {
        (self.accumulator / TICK).clamp(0.0, 0.999_999) as f32
    }

    pub fn command_log(&self) -> &[Command] {
        &self.command_log
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_data::Registries;
    use il_sim_battle::BattlePhase;
    use std::sync::Arc;

    fn session() -> BattleSession {
        let world = BattleWorld::empty(42, Arc::new(Registries::default()), BattlePhase::Battle);
        BattleSession::new(world, PlayerId(0), ScriptedCommands::default())
    }

    #[test]
    fn steps_once_per_tick_of_wall_time() {
        let mut s = session();
        assert!(s.advance(TICK * 0.5).is_empty());
        assert_eq!(s.advance(TICK * 0.5).len(), 1);
        assert_eq!(s.world.tick(), Tick(1));
        assert_eq!(s.advance(TICK * 2.0).len(), 2);
        assert_eq!(s.world.tick(), Tick(3));
    }

    #[test]
    fn never_more_than_max_catchup_ticks_per_frame() {
        let mut s = session();
        assert_eq!(s.advance(10.0).len() as u32, MAX_CATCHUP_TICKS);
        // The excess is dropped, not carried: the next small frame steps at most once more.
        assert!(s.advance(TICK * 0.25).is_empty());
    }

    #[test]
    fn alpha_stays_in_unit_interval() {
        let mut s = session();
        s.advance(TICK * 0.75);
        let a = s.alpha();
        assert!((0.74..0.76).contains(&a));
        s.advance(TICK * 0.25);
        assert!(s.alpha() < 0.01);
    }

    #[test]
    fn pause_stops_time_and_records_a_command() {
        let mut s = session();
        s.set_paused(true);
        assert!(s.advance(1.0).is_empty());
        s.set_paused(false);
        let out = s.advance(TICK);
        assert_eq!(out.len(), 1);
        assert!(
            out[0].rejected.is_empty(),
            "Pause is a no-op, never rejected"
        );
        let kinds: Vec<_> = s
            .command_log()
            .iter()
            .map(|c| matches!(c.kind, CommandKind::Pause))
            .collect();
        assert_eq!(kinds, vec![true, true]);
        assert!(s.command_log().windows(2).all(|w| w[0].seq < w[1].seq));
    }

    #[test]
    fn speed_scales_the_accumulator_not_the_tick() {
        let mut s = session();
        s.set_speed(2.0);
        assert_eq!(s.advance(TICK).len(), 2);
        s.set_speed(0.5);
        assert!(s.advance(TICK).is_empty());
        assert_eq!(s.advance(TICK).len(), 1);
    }
}
