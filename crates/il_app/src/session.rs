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
    /// The result carried by `Ended` (T2-070); the sim stops stepping then.
    result: Option<il_sim_battle::BattleResult>,
    /// Players whose sides go to the engine AI at tick 1 (`--ai`, T2-081).
    ai_players: Vec<PlayerId>,
    /// Per side, the soldiers lost so far, tallied from the events
    /// (T2-090, plan decision 11): the casualties line reads these.
    casualties: Vec<SideCasualties>,
}

/// One side's running losses (T2-090); `alive` comes from the view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SideCasualties {
    pub killed: u32,
    pub fled: u32,
    pub withdrawn: u32,
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
            events: VecDeque::with_capacity(EVENT_RING),
            corpses: Vec::new(),
            result: None,
            ai_players,
            casualties: Vec::new(),
        }
    }

    /// The battle's result once the phase is Ended (T2-070).
    pub fn result(&self) -> Option<&il_sim_battle::BattleResult> {
        self.result.as_ref()
    }

    /// Per side, the soldiers killed, fled and withdrawn so far (T2-090).
    pub fn casualties(&self) -> &[SideCasualties] {
        &self.casualties
    }

    /// `Surrender` for every side the local player owns (T2-090, pause
    /// menu; SIM-FLOW-017).
    pub fn surrender(&mut self) {
        self.queue(CommandKind::Surrender);
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
        // SIM-FLOW-010 (T2-070): nothing moves after the end.
        while self.accumulator >= TICK && self.world.phase() != il_sim_battle::BattlePhase::Ended {
            outputs.push(self.step_once(observer));
            self.accumulator -= TICK;
        }
        if self.world.phase() == il_sim_battle::BattlePhase::Ended {
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
        // Networking Spec §2.7: the AI's commands for the next tick join the
        // log (a replay may feed them with the AI off, T2-101).
        self.command_log.extend(out.ai_commands.iter().cloned());
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
        let sides = self.world.view().sides().len();
        if self.casualties.len() < sides {
            self.casualties.resize(sides, SideCasualties::default());
        }
        for e in &out.events {
            if let BattleEvent::Ended { result } = e {
                self.result = Some((**result).clone());
            }
            // T2-090: the casualties line's tallies (the regiment row
            // outlives its last soldier, so the side is always known).
            let lost = match e {
                BattleEvent::SoldierDied { regiment, .. } => Some((*regiment, 0)),
                BattleEvent::SoldierFled { regiment, .. } => Some((*regiment, 1)),
                BattleEvent::SoldierWithdrew { regiment, .. } => Some((*regiment, 2)),
                _ => None,
            };
            if let Some((regiment, kind)) = lost
                && let Some(side) = self.world.view().regiment(regiment).map(|r| r.side)
                && let Some(c) = self.casualties.get_mut(usize::from(side))
            {
                match kind {
                    0 => c.killed += 1,
                    1 => c.fled += 1,
                    _ => c.withdrawn += 1,
                }
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

    /// T2-081: `--ai` hands the listed players' sides to the engine at
    /// tick 1 and the AI's commands join the log.
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
        // Whatever tick 1's Stage 1 decided for the new side is logged for
        // tick 2 (a lone regiment with nobody in sight decides nothing).
        let logged_ai = s
            .command_log()
            .iter()
            .filter(|c| c.player == PlayerId::ENGINE_AI && c.tick == Tick(2))
            .count();
        assert_eq!(logged_ai, outs[0].ai_commands.len());
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
}

#[cfg(test)]
mod casualty_tests {
    use super::*;
    use il_core::{RegimentId, SoldierId, V2};

    /// T2-090 (decision 11): deaths, flights and withdrawals are tallied per
    /// side from the events.
    #[test]
    fn casualties_are_tallied_per_side_from_events() {
        use il_data::ContentId;
        use il_sim_battle::{BattleSetup, GeneralSetup, RegimentSetup, SideSetup};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        let regs = il_cli::load_registries(&root).unwrap_or_else(|e| panic!("{e:#}"));
        let cid = |s: &str| ContentId::new(s).unwrap();
        let side = |player: u8, zone: u8, id: u32, x: f32| SideSetup {
            faction: cid("rome:rome"),
            player: PlayerId(player),
            deployment_zone: zone,
            general: GeneralSetup {
                unit_type: cid("rome:general"),
                rank: 1,
                name_key: String::new(),
                bodyguard: None,
            },
            regiments: vec![RegimentSetup {
                id,
                unit_type: cid("rome:hastati"),
                count: 5,
                experience: 0,
                fatigue: 0.0,
                formation: None,
                position: Some([x, 150.0]),
                facing_deg: Some(0.0),
            }],
            reinforcements: vec![],
            ai_profile: None,
        };
        let setup = BattleSetup {
            map_id: cid("rome:test_field"),
            seed: 1,
            weather: Default::default(),
            time_of_day: 12,
            time_limit_ticks: 48_000,
            reveal_deployment: false,
            sides: vec![side(0, 0, 1, 300.0), side(1, 1, 2, 500.0)],
            victory: Default::default(),
        };
        let world = BattleWorld::new(&setup, regs).unwrap();
        let mut s = BattleSession::new(world, PlayerId(0), ScriptedCommands::default(), Vec::new());
        let pos = V2::from_f32_data(300.0, 150.0);
        let out = StepOutput {
            hash: s.world.hash(),
            events: vec![
                BattleEvent::SoldierDied {
                    id: SoldierId(0),
                    regiment: RegimentId(0),
                    killer: None,
                    pos,
                },
                BattleEvent::SoldierDied {
                    id: SoldierId(1),
                    regiment: RegimentId(0),
                    killer: None,
                    pos,
                },
                BattleEvent::SoldierFled {
                    id: SoldierId(2),
                    regiment: RegimentId(0),
                    pos,
                },
                BattleEvent::SoldierWithdrew {
                    id: SoldierId(7),
                    regiment: RegimentId(1),
                    pos,
                },
            ],
            rejected: Vec::new(),
            ai_commands: Vec::new(),
        };
        s.route_events(Tick(1), &out);
        let c = s.casualties();
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].killed, c[0].fled, c[0].withdrawn), (2, 1, 0));
        assert_eq!((c[1].killed, c[1].fled, c[1].withdrawn), (0, 0, 1));
    }
}
