//! `BattleSession`: the fixed-step accumulator around one `BattleWorld`
//! (SAD §6.1, TDD §15, REQ-SIM-031).
//!
//! Speed multipliers scale the accumulator, never the tick length. Pause sets
//! the multiplier to zero and is also recorded as a `Pause` command so replays
//! and peers see it (SIM-DET-008). Events and rejected commands are routed
//! to a ring the developer panel shows (T1-070 routing stub; audio and the
//! HUD subscribe in their phases).

use std::collections::VecDeque;

use il_core::{PlayerId, Scalar, TICK_SECONDS, Tick};
use il_render::Corpse;
use il_sim_battle::{
    BattleEvent, BattleWorld, Command, CommandKind, NoopObserver, ScriptedCommands, StageObserver,
    StepOutput,
};
use il_ui::EventLine;

/// Wall seconds per simulation tick, as the accumulator's type.
pub const TICK: f64 = TICK_SECONDS as f64;

/// The sim never runs more than this many ticks in one frame; beyond it the
/// sim visibly slows instead of spiralling (`app.max_catchup_ticks`).
pub const MAX_CATCHUP_TICKS: u32 = 4;

/// Events kept for the developer panel.
pub const EVENT_RING: usize = 256;

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
    /// The last `EVENT_RING` events and rejections, oldest first.
    events: VecDeque<EventLine>,
    /// Fallen soldiers kept for `combat.corpse_ticks` (T2-022, SIM-CORE-008).
    corpses: Vec<Corpse>,
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
            events: VecDeque::with_capacity(EVENT_RING),
            corpses: Vec::new(),
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

    pub fn local_player(&self) -> PlayerId {
        self.local_player
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
        let out = self.world.step_observed(&now, observer);
        self.route_events(next, &out);
        out
    }

    /// Event routing (SAD §6.1): every event goes to the developer ring;
    /// `SoldierDied` also leaves a corpse (audio and the HUD subscribe in
    /// their phases).
    fn route_events(&mut self, tick: Tick, out: &StepOutput) {
        let corpse_ticks = u32::from(self.world.registries().rules.combat.corpse_ticks);
        self.corpses
            .retain(|c| tick.0.saturating_sub(c.died.0) < corpse_ticks);
        for e in &out.events {
            if let BattleEvent::SoldierDied { regiment, pos, .. } = e
                && corpse_ticks > 0
                && let Some(row) = self.world.view().regiment(*regiment)
            {
                let regs = self.world.registries();
                self.corpses.push(Corpse {
                    pos: [pos.x.to_f32_render(), pos.y.to_f32_render()],
                    side: row.side,
                    sprite_set: regs.units.get(row.unit).sprite_set().index() as u16,
                    facing8: row.anchor_facing.to_facing8(),
                    died: tick,
                });
            }
            self.push_event(tick, format!("{e:?}"));
        }
        for (c, reason) in &out.rejected {
            self.push_event(
                tick,
                format!("rejected seq {} {:?}: {reason:?}", c.seq, c.kind),
            );
        }
    }

    fn push_event(&mut self, tick: Tick, text: String) {
        if self.events.len() == EVENT_RING {
            self.events.pop_front();
        }
        self.events.push_back(EventLine { tick, text });
    }

    /// Routed events, oldest first.
    pub fn events(&self) -> &VecDeque<EventLine> {
        &self.events
    }

    /// Corpses still on the ground (T2-022).
    pub fn corpses(&self) -> &[Corpse] {
        &self.corpses
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
    fn rejected_commands_and_events_land_in_the_ring() {
        let mut s = session();
        s.queue(CommandKind::Halt {
            regiments: vec![il_core::RegimentId(7)],
        });
        s.advance(TICK);
        let lines: Vec<_> = s.events().iter().map(|l| l.text.clone()).collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("CommandRejected"), "{}", lines[0]);
        assert!(lines[1].starts_with("rejected seq 0"), "{}", lines[1]);
        assert_eq!(s.events()[0].tick, Tick(1));
        for _ in 0..EVENT_RING {
            s.queue(CommandKind::Halt {
                regiments: vec![il_core::RegimentId(7)],
            });
            s.advance(TICK);
        }
        assert_eq!(s.events().len(), EVENT_RING);
    }

    /// A session over the flagship content with one five-man regiment.
    fn game_session() -> BattleSession {
        use il_data::ContentId;
        use il_sim_battle::{BattleSetup, GeneralSetup, RegimentSetup, SideSetup};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        let regs = il_cli::load_registries(&root).unwrap_or_else(|e| panic!("{e:#}"));
        let cid = |s: &str| ContentId::new(s).unwrap();
        let setup = BattleSetup {
            map_id: cid("rome:test_field"),
            seed: 1,
            weather: Default::default(),
            time_of_day: 12,
            time_limit_ticks: 48_000,
            reveal_deployment: false,
            sides: vec![SideSetup {
                faction: cid("rome:rome"),
                player: PlayerId(0),
                deployment_zone: 0,
                general: GeneralSetup {
                    unit_type: cid("rome:hastati"),
                    rank: 1,
                    name_key: String::new(),
                },
                regiments: vec![RegimentSetup {
                    id: 1,
                    unit_type: cid("rome:hastati"),
                    count: 5,
                    experience: 0,
                    fatigue: 0.0,
                    formation: None,
                    position: Some([300.0, 150.0]),
                    facing_deg: Some(0.0),
                }],
                reinforcements: vec![],
            }],
            victory: Default::default(),
        };
        let world = BattleWorld::new(&setup, regs).unwrap();
        BattleSession::new(world, PlayerId(0), ScriptedCommands::default())
    }

    #[test]
    fn a_death_leaves_a_corpse_that_expires() {
        let mut s = game_session();
        let regiment = s.world.regiment_ids().next().unwrap();
        let corpse_ticks = u32::from(s.world.registries().rules.combat.corpse_ticks);
        let out = StepOutput {
            hash: s.world.hash(),
            events: vec![BattleEvent::SoldierDied {
                id: il_core::SoldierId(0),
                regiment,
                killer: None,
                pos: il_core::V2::from_f32_data(300.0, 150.0),
            }],
            rejected: Vec::new(),
        };
        s.route_events(Tick(1), &out);
        assert_eq!(s.corpses().len(), 1);
        assert_eq!(s.corpses()[0].died, Tick(1));
        let empty = StepOutput {
            hash: s.world.hash(),
            events: Vec::new(),
            rejected: Vec::new(),
        };
        s.route_events(Tick(corpse_ticks), &empty);
        assert_eq!(s.corpses().len(), 1, "still within corpse_ticks");
        s.route_events(Tick(corpse_ticks + 1), &empty);
        assert!(s.corpses().is_empty(), "corpse outlived corpse_ticks");
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
