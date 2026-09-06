//! `BattleSession`: the fixed-step accumulator around one `BattleWorld`
//! (SAD §6.1, TDD §15, REQ-SIM-031).
//!
//! Speed multipliers scale the accumulator, never the tick length. Pause sets
//! the multiplier to zero and is also recorded as a `Pause` command so replays
//! and peers see it (SIM-DET-008). Events and rejected commands are routed
//! to a ring the developer panel shows (T1-070 routing stub; audio subscribes
//! in its phase). The session is also the replay recorder (T2-101, plan I1):
//! it keeps what it fed the sim, what the engine AI produced and every tick's
//! hash, can snapshot itself into a battle save and continue from one, and
//! can play a recording back with the AI on, checking every hash.

use std::collections::VecDeque;
use std::sync::Arc;

use il_core::{PlayerId, Scalar, StateHash, TICK_SECONDS, Tick};
use il_data::Registries;
use il_render::Corpse;
use il_save::{BattleSave, Replay, SaveError};
use il_sim_battle::{
    BattleEvent, BattlePhase, BattleSetup, BattleWorld, Command, CommandKind, NoopObserver,
    ScriptedCommands, Snapshot, StageObserver, StepOutput,
};
use il_ui::EventLine;

/// Wall seconds per simulation tick, as the accumulator's type.
pub const TICK: f64 = TICK_SECONDS as f64;

/// The sim never runs more than this many ticks in one frame; beyond it the
/// sim visibly slows instead of spiralling (`app.max_catchup_ticks`).
pub const MAX_CATCHUP_TICKS: u32 = 4;

/// Events kept for the developer panel.
pub const EVENT_RING: usize = 256;

/// Live play, or the watch-only playback of a recording (plan decision 18).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionMode {
    Live,
    Replay {
        /// The recorded hash per tick, tick 1 first.
        expected: Vec<StateHash>,
        /// The first tick whose hash differed from the recording.
        mismatch: Option<Tick>,
    },
}

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
    /// Every command handed to the sim, in order: the replay (T2-101).
    command_log: Vec<Command>,
    /// Every command the engine AI produced (`StepOutput.ai_commands`),
    /// kept apart from the fed ones (plan I1).
    ai_log: Vec<Command>,
    /// One state hash per completed tick.
    hashes: Vec<StateHash>,
    /// The last `EVENT_RING` events and rejections, oldest first.
    events: VecDeque<EventLine>,
    /// Fallen soldiers kept for `combat.corpse_ticks` (T2-022, SIM-CORE-008).
    corpses: Vec<Corpse>,
    /// The result carried by `Ended` (T2-070); the sim stops stepping then.
    result: Option<il_sim_battle::BattleResult>,
    /// Players whose sides go to the engine AI at tick 1 (`--ai`, T2-081).
    ai_players: Vec<PlayerId>,
    mode: SessionMode,
    /// The scenario file's stem, for the replay's file name.
    scenario_stem: String,
}

impl BattleSession {
    pub fn new(
        world: BattleWorld,
        local_player: PlayerId,
        script: ScriptedCommands,
        ai_players: Vec<PlayerId>,
    ) -> Self {
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
            ai_log: Vec::new(),
            hashes: Vec::new(),
            events: VecDeque::with_capacity(EVENT_RING),
            corpses: Vec::new(),
            result: None,
            ai_players,
            mode: SessionMode::Live,
            scenario_stem: String::from("battle"),
        }
    }

    /// Names the replay file after the scenario.
    pub fn with_stem(mut self, stem: impl Into<String>) -> Self {
        self.scenario_stem = stem.into();
        self
    }

    pub fn scenario_stem(&self) -> &str {
        &self.scenario_stem
    }

    /// Playback of a recording (plan decision 18): the setup's world, the
    /// fed commands as the script, the AI on (plan I1), no local commands.
    pub fn from_replay(
        replay: Replay,
        regs: Arc<Registries>,
        threads: usize,
    ) -> Result<Self, SaveError> {
        let mut world =
            BattleWorld::new(&replay.setup, regs).map_err(|e| SaveError::Setup(e.to_string()))?;
        world.set_threads(threads.max(1));
        let mut s = Self::new(
            world,
            PlayerId(0),
            ScriptedCommands::new(replay.commands),
            Vec::new(),
        );
        s.mode = SessionMode::Replay {
            expected: replay.hashes,
            mismatch: None,
        };
        Ok(s)
    }

    /// Continues a battle save (plan I12, REQ-SAVE-006): the snapshot's
    /// world, the logs and hashes so far, the script's remainder.
    pub fn from_save(
        save: BattleSave,
        regs: Arc<Registries>,
        threads: usize,
    ) -> Result<Self, SaveError> {
        let snapshot =
            Snapshot::from_bytes(&save.snapshot).map_err(|e| SaveError::Decode(e.to_string()))?;
        let mut world =
            BattleWorld::restore(&snapshot, regs).map_err(|e| SaveError::Decode(e.to_string()))?;
        world.set_threads(threads.max(1));
        let mut s = Self::new(
            world,
            save.local_player,
            ScriptedCommands::new(save.script),
            Vec::new(),
        );
        s.next_seq = save
            .replay
            .commands
            .iter()
            .filter(|c| c.player == save.local_player)
            .map(|c| c.seq.wrapping_add(1))
            .max()
            .unwrap_or(0);
        s.command_log = save.replay.commands;
        s.ai_log = save.replay.ai_commands;
        s.hashes = save.replay.hashes;
        s.scenario_stem = save.scenario_stem;
        Ok(s)
    }

    pub fn mode(&self) -> &SessionMode {
        &self.mode
    }

    pub fn is_replay(&self) -> bool {
        matches!(self.mode, SessionMode::Replay { .. })
    }

    /// Playback: every recorded tick has been stepped.
    pub fn replay_finished(&self) -> bool {
        match &self.mode {
            SessionMode::Replay { expected, .. } => self.hashes.len() >= expected.len(),
            SessionMode::Live => false,
        }
    }

    /// Playback: the first tick that disagreed with the recording.
    pub fn replay_mismatch(&self) -> Option<Tick> {
        match &self.mode {
            SessionMode::Replay { mismatch, .. } => *mismatch,
            SessionMode::Live => None,
        }
    }

    /// The recording so far (T2-101); `None` for a world without a setup.
    pub fn replay(&self) -> Option<Replay> {
        Some(Replay {
            setup: self.world.setup()?.clone(),
            commands: self.command_log.clone(),
            ai_commands: self.ai_log.clone(),
            hashes: self.hashes.clone(),
            checkpoints: Vec::new(),
            ended_tick: (self.world.phase() == BattlePhase::Ended).then(|| self.world.tick().0),
        })
    }

    /// A battle save of this moment (plan I12); `None` without a setup.
    pub fn save(&self) -> Option<BattleSave> {
        Some(BattleSave {
            snapshot: self.world.snapshot().to_bytes(),
            replay: self.replay()?,
            script: self.script.remaining_commands().to_vec(),
            local_player: self.local_player,
            scenario_stem: self.scenario_stem.clone(),
        })
    }

    /// The setup the world was built from (Rematch, T2-091).
    #[allow(dead_code, reason = "the result screen's Rematch arrives with T2-091")]
    pub fn setup(&self) -> Option<&BattleSetup> {
        self.world.setup()
    }

    /// The battle's result once the phase is Ended (T2-070).
    pub fn result(&self) -> Option<&il_sim_battle::BattleResult> {
        self.result.as_ref()
    }

    /// `Surrender` for every side the local player owns (T2-090, pause
    /// menu; SIM-FLOW-017).
    pub fn surrender(&mut self) {
        self.queue(CommandKind::Surrender);
    }

    /// A line for the developer panel (quick save and load notes, T2-101).
    pub fn note(&mut self, text: String) {
        let tick = self.world.tick();
        self.push_event(tick, text);
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

    /// The first side the local player owns: whose fog of war the window
    /// shows (T2-060); `None` for a spectator.
    pub fn observer_side(&self) -> Option<u8> {
        self.world
            .view()
            .sides()
            .iter()
            .position(|s| s.player == self.local_player)
            .map(|i| i as u8)
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

    /// Queues a command from the local player for the next tick. A
    /// playback accepts none (the recording is the only input).
    pub fn queue(&mut self, kind: CommandKind) {
        if self.is_replay() {
            return;
        }
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
        // SIM-FLOW-010 (T2-070): nothing moves after the end; a playback
        // stops at the recording's end (plan I14).
        while self.accumulator >= TICK
            && self.world.phase() != BattlePhase::Ended
            && !self.replay_finished()
        {
            outputs.push(self.step_once(observer));
            self.accumulator -= TICK;
        }
        if self.world.phase() == BattlePhase::Ended || self.replay_finished() {
            self.accumulator = 0.0;
        }
        outputs
    }

    fn step_once(&mut self, observer: &mut dyn StageObserver) -> StepOutput {
        let next = self.world.tick().next();
        let (mut now, later): (Vec<Command>, Vec<Command>) =
            self.pending.drain(..).partition(|c| c.tick <= next);
        self.pending = later;
        now.extend(self.script.take_for(next));
        // `--ai` (T2-081, SIM-CMD-002): the engine takes the listed players'
        // sides at tick 1, issued as the engine so ownership passes.
        if next == Tick(1) {
            for (seq, from) in self.ai_players.iter().enumerate() {
                now.push(Command {
                    tick: next,
                    player: PlayerId::ENGINE_AI,
                    seq: seq as u16,
                    kind: CommandKind::TransferControl {
                        from: *from,
                        to: PlayerId::ENGINE_AI,
                    },
                });
            }
        }
        self.command_log.extend(now.iter().cloned());
        let out = self.world.step_observed(&now, observer);
        // Networking Spec §2.7: the AI's commands for the next tick are
        // kept apart from the fed ones (T2-101, plan I1).
        self.ai_log.extend(out.ai_commands.iter().cloned());
        let index = self.hashes.len();
        self.hashes.push(out.hash);
        if let SessionMode::Replay { expected, mismatch } = &mut self.mode
            && mismatch.is_none()
            && expected.get(index).is_some_and(|h| *h != out.hash)
        {
            *mismatch = Some(next);
        }
        self.route_events(next, &out);
        out
    }

    /// Event routing (SAD §6.1): every event goes to the developer ring;
    /// `SoldierDied` also leaves a corpse (audio subscribes in its phase).
    fn route_events(&mut self, tick: Tick, out: &StepOutput) {
        let corpse_ticks = u32::from(self.world.registries().rules.combat.corpse_ticks);
        self.corpses
            .retain(|c| tick.0.saturating_sub(c.died.0) < corpse_ticks);
        for e in &out.events {
            if let BattleEvent::Ended { result } = e {
                self.result = Some((**result).clone());
            }
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

    /// The commands fed to the sim so far.
    pub fn command_log(&self) -> &[Command] {
        &self.command_log
    }

    /// The commands the engine AI produced so far.
    pub fn ai_log(&self) -> &[Command] {
        &self.ai_log
    }

    /// One hash per completed tick.
    pub fn hashes(&self) -> &[StateHash] {
        &self.hashes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_data::Registries;
    use il_sim_battle::BattlePhase;
    use std::path::Path;
    use std::sync::Arc;

    fn session() -> BattleSession {
        let world = BattleWorld::empty(42, Arc::new(Registries::default()), BattlePhase::Battle);
        BattleSession::new(world, PlayerId(0), ScriptedCommands::default(), Vec::new())
    }

    #[test]
    fn steps_once_per_tick_of_wall_time() {
        let mut s = session();
        assert!(s.advance(TICK * 0.5).is_empty());
        assert_eq!(s.advance(TICK * 0.5).len(), 1);
        assert_eq!(s.world.tick(), Tick(1));
        assert_eq!(s.advance(TICK * 2.0).len(), 2);
        assert_eq!(s.world.tick(), Tick(3));
        assert_eq!(s.hashes().len(), 3);
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

    fn game_regs() -> Arc<Registries> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        il_cli::load_registries(&root).unwrap_or_else(|e| panic!("{e:#}"))
    }

    /// A session over the flagship content with one five-man regiment.
    fn game_session() -> BattleSession {
        use il_data::ContentId;
        use il_sim_battle::{BattleSetup, GeneralSetup, RegimentSetup, SideSetup};
        let regs = game_regs();
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
                    unit_type: cid("rome:general"),
                    rank: 1,
                    name_key: String::new(),
                    bodyguard: None,
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
                ai_profile: None,
            }],
            victory: Default::default(),
        };
        let world = BattleWorld::new(&setup, regs).unwrap();
        BattleSession::new(world, PlayerId(0), ScriptedCommands::default(), Vec::new())
    }

    /// The AI-versus-AI skirmish of the determinism corpus, as a session.
    fn skirmish_session(regs: Arc<Registries>) -> BattleSession {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/scenarios/ai_skirmish_300.json5");
        let scenario = il_cli::load_scenario(&path).unwrap();
        let world = BattleWorld::new(&scenario.setup, regs).unwrap();
        BattleSession::new(world, PlayerId(0), scenario.script(), Vec::new())
            .with_stem("ai_skirmish_300")
    }

    fn run_to(s: &mut BattleSession, tick: u32) {
        while s.world.tick().0 < tick {
            s.advance(TICK);
        }
    }

    /// T2-081: `--ai` hands the listed players' sides to the engine at
    /// tick 1 and the AI's commands join the AI log (T2-101).
    #[test]
    fn ai_players_are_transferred_at_tick_one_and_logged() {
        let mut s = game_session();
        s.ai_players = vec![PlayerId(0)];
        let outs = s.advance(TICK * 1.5);
        assert_eq!(outs.len(), 1);
        assert!(outs[0].rejected.is_empty(), "{:?}", outs[0].rejected);
        assert!(s.command_log().iter().any(|c| matches!(
            c.kind,
            CommandKind::TransferControl {
                from: PlayerId(0),
                to: PlayerId::ENGINE_AI
            }
        )));
        assert!(
            s.world.view().sides()[0].player == PlayerId::ENGINE_AI,
            "side 0 belongs to the engine"
        );
        // Whatever tick 1's Stage 1 decided for the new side is in the AI
        // log for tick 2 (a lone regiment with nobody in sight decides nothing).
        assert_eq!(s.ai_log().len(), outs[0].ai_commands.len());
        assert!(
            s.command_log()
                .iter()
                .all(|c| c.player != PlayerId::ENGINE_AI
                    || matches!(c.kind, CommandKind::TransferControl { .. }))
        );
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
            ai_commands: Vec::new(),
        };
        s.route_events(Tick(1), &out);
        assert_eq!(s.corpses().len(), 1);
        assert_eq!(s.corpses()[0].died, Tick(1));
        let empty = StepOutput {
            hash: s.world.hash(),
            events: Vec::new(),
            rejected: Vec::new(),
            ai_commands: Vec::new(),
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

    /// T2-101 (plan I1): a recording of an AI-driven battle holds the fed
    /// commands, the AI's commands apart, one hash per tick, and verifies
    /// by re-simulation with the AI on.
    #[test]
    fn a_recording_verifies_and_a_corrupted_hash_names_its_tick() {
        let regs = game_regs();
        let mut s = skirmish_session(regs.clone());
        s.set_paused(true);
        s.set_paused(false);
        run_to(&mut s, 200);
        let replay = s.replay().expect("the world has a setup");
        assert_eq!(replay.ticks(), 200);
        assert!(replay.ended_tick.is_none());
        assert!(
            !replay.ai_commands.is_empty(),
            "two engine sides decide something in 200 ticks"
        );
        assert!(
            replay
                .commands
                .iter()
                .all(|c| c.player != PlayerId::ENGINE_AI),
            "the AI's commands are not in the fed log"
        );
        assert_eq!(
            replay
                .commands
                .iter()
                .filter(|c| matches!(c.kind, CommandKind::Pause))
                .count(),
            2
        );
        let report = il_save::verify(&replay, regs.clone(), 1).unwrap();
        assert!(report.ok(), "{report:?}");
        assert_eq!(report.ticks, 200);
        // The same recording verifies on eight threads too.
        assert!(il_save::verify(&replay, regs.clone(), 8).unwrap().ok());
        let mut broken = replay.clone();
        broken.hashes[149] = StateHash(broken.hashes[149].0 ^ 1);
        let report = il_save::verify(&broken, regs, 1).unwrap();
        let d = report.divergence.expect("divergence found");
        assert_eq!(d.tick, Tick(150));
        assert_eq!(report.ticks, 150);
    }

    /// T2-101 (plan decision 14, REQ-SAVE-006): saving at tick 400 and
    /// loading in a fresh session gives the same hashes to tick 800 as the
    /// uninterrupted run, and the loaded session's replay covers the whole
    /// battle from tick 0.
    #[test]
    fn save_and_load_keep_the_hash_sequence() {
        let regs = game_regs();
        let mut s = skirmish_session(regs.clone());
        run_to(&mut s, 400);
        let save = s.save().expect("the world has a setup");
        assert_eq!(save.replay.ticks(), 400);
        assert_eq!(save.scenario_stem, "ai_skirmish_300");
        let bytes = save.to_bytes();
        run_to(&mut s, 800);
        let uninterrupted: Vec<StateHash> = s.hashes()[400..].to_vec();

        let loaded = BattleSave::from_bytes(&bytes).unwrap();
        let mut l = BattleSession::from_save(loaded, regs.clone(), 1).unwrap();
        assert_eq!(l.world.tick(), Tick(400));
        assert_eq!(l.hashes().len(), 400);
        assert_eq!(l.scenario_stem(), "ai_skirmish_300");
        run_to(&mut l, 800);
        assert_eq!(
            l.hashes()[400..],
            uninterrupted[..],
            "the hash sequence changed across save and load"
        );
        let replay = l.replay().unwrap();
        assert_eq!(replay.ticks(), 800);
        assert!(il_save::verify(&replay, regs, 1).unwrap().ok());
    }

    /// T2-101 (plan decision 18): a playback feeds the recording, accepts
    /// no local commands, stops at the recording's end and notices a hash
    /// that differs.
    #[test]
    fn playback_replays_the_recording_and_checks_every_hash() {
        let regs = game_regs();
        let mut s = skirmish_session(regs.clone());
        run_to(&mut s, 120);
        let replay = s.replay().unwrap();
        let mut p = BattleSession::from_replay(replay.clone(), regs.clone(), 1).unwrap();
        assert!(p.is_replay());
        p.queue(CommandKind::Surrender);
        run_to(&mut p, 120);
        assert!(p.replay_finished());
        assert_eq!(p.replay_mismatch(), None);
        assert_eq!(p.hashes(), replay.hashes.as_slice());
        assert!(
            p.advance(TICK * 3.0).is_empty(),
            "stops at the recording's end"
        );
        assert!(
            p.command_log()
                .iter()
                .all(|c| !matches!(c.kind, CommandKind::Surrender)),
            "no local command in a playback"
        );
        let mut broken = replay;
        broken.hashes[59] = StateHash(broken.hashes[59].0 ^ 1);
        let mut p = BattleSession::from_replay(broken, regs, 1).unwrap();
        run_to(&mut p, 120);
        assert_eq!(p.replay_mismatch(), Some(Tick(60)));
    }
}
