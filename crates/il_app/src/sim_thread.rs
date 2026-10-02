//! The sim thread (T3-032, SAD §8, §12 T-16, TDD §15): the battle steps on
//! its own thread against its own clock and hands the main thread an owned
//! frame per published tick, so a 41 ms tick at 20,000 soldiers no longer
//! takes the frame with it.
//!
//! The thread owns the `BattleSession` unchanged: the accumulator, the
//! pending queue (commands are stamped at the sim's own tick, as the sim
//! requires, SIM-CMD-001), the logs, the hashes, the replay check, the
//! event ring and the corpses. The main thread sends `SimRequest`s and
//! reads the newest `SimFrame` from a one-slot mailbox; a frame it never
//! read hands its events and lines on to the next, so audio and the event
//! panel miss nothing. `--single-thread-sim` runs the same driver inline on
//! the main thread, once per frame (`SimHost::Inline`), so both modes read
//! the same frames.
//!
//! Interpolation: a frame carries the accumulator left after its tick and
//! the instant it was published; the main thread's `alpha` grows with wall
//! time from there, so the soldiers keep moving between published ticks
//! and each tick's motion is shown over one tick period.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use il_core::{PlayerId, Tick};
use il_data::Registries;
use il_render::Corpse;
use il_sim_battle::{
    BattleEvent, BattleFrame, BattleResult, BattleSetup, CommandKind, FrameDetail, FrameStatics,
};
use il_ui::{EventLine, ProfilerStats};

use crate::profiler::Profiler;
use crate::session::{BattleSession, EVENT_RING, SessionMode, TICK};

/// How long before a tick is due the thread stops sleeping and yields
/// instead (the OS timer can overshoot a sleep by a whole timer period).
const SPIN: Duration = Duration::from_millis(2);

/// What the main thread asks of the sim.
pub enum SimRequest {
    /// A local player's command, stamped for the sim's next tick.
    Queue(CommandKind),
    SetSpeed(f32),
    SetPaused(bool),
    /// A line for the event panel.
    Note(String),
    /// Hot-reloaded content (between ticks).
    ReplaceRegistries(Arc<Registries>),
    /// What the debug overlays need captured.
    SetDetail(FrameDetail),
    /// Runs on the session (quick save, the replay); the closure replies
    /// through its own channel.
    Call(Box<dyn FnOnce(&mut BattleSession) + Send>),
}

/// When a frame was published and how far the accumulator was then.
#[derive(Clone, Copy, Debug)]
pub struct FrameClock {
    pub at: Instant,
    /// Wall seconds banked toward the next tick at `at`.
    pub acc: f64,
    /// The accumulator's rate: the speed, 0 while paused or ended.
    pub mult: f64,
}

impl FrameClock {
    /// How far into the next tick wall time has advanced, in `[0, 1)`.
    pub fn alpha(&self, now: Instant) -> f32 {
        let since = now.saturating_duration_since(self.at).as_secs_f64();
        ((self.acc + since * self.mult) / TICK).clamp(0.0, 0.999_999) as f32
    }
}

/// One published tick and what happened since the previous one.
pub struct SimFrame {
    pub battle: BattleFrame,
    pub clock: FrameClock,
    /// Ticks stepped since the previous frame.
    pub ticks: u32,
    /// The events of those ticks, in order.
    pub events: Vec<BattleEvent>,
    /// Event panel lines routed since the previous frame.
    pub lines: Vec<EventLine>,
    pub corpses: Vec<Corpse>,
    pub result: Option<Arc<BattleResult>>,
    pub replay_mismatch: Option<Tick>,
    pub replay_finished: bool,
    /// Commands fed to the sim and produced by the AI so far.
    pub fed_commands: usize,
    pub ai_commands: usize,
    pub observer_side: Option<u8>,
    /// The stage rows and tick totals (`Profiler::stats`).
    pub stats: ProfilerStats,
    /// Wall seconds spent stepping since the previous frame.
    pub step_seconds: f64,
}

impl SimFrame {
    fn new(battle: BattleFrame, now: Instant) -> Self {
        Self {
            battle,
            clock: FrameClock {
                at: now,
                acc: 0.0,
                mult: 0.0,
            },
            ticks: 0,
            events: Vec::new(),
            lines: Vec::new(),
            corpses: Vec::new(),
            result: None,
            replay_mismatch: None,
            replay_finished: false,
            fed_commands: 0,
            ai_commands: 0,
            observer_side: None,
            stats: ProfilerStats::default(),
            step_seconds: 0.0,
        }
    }

    /// Puts an unread older frame's history in front of this one's.
    fn absorb(&mut self, older: &mut SimFrame) {
        self.ticks += older.ticks;
        self.step_seconds += older.step_seconds;
        older.events.append(&mut self.events);
        std::mem::swap(&mut self.events, &mut older.events);
        older.lines.append(&mut self.lines);
        std::mem::swap(&mut self.lines, &mut older.lines);
    }
}

/// The session with what the frames need: runs inline or on the thread.
pub struct SimDriver {
    session: BattleSession,
    profiler: Profiler,
    statics: Arc<FrameStatics>,
    detail: FrameDetail,
    last: Option<Instant>,
    ticks: u32,
    events: Vec<BattleEvent>,
    step_seconds: f64,
    lines_sent: u64,
    result: Option<Arc<BattleResult>>,
}

impl SimDriver {
    pub fn new(session: BattleSession) -> Self {
        let statics = Arc::new(FrameStatics::of(&session.world));
        Self {
            session,
            profiler: Profiler::default(),
            statics,
            detail: FrameDetail::default(),
            last: None,
            ticks: 0,
            events: Vec::new(),
            step_seconds: 0.0,
            lines_sent: 0,
            result: None,
        }
    }

    pub fn session(&self) -> &BattleSession {
        &self.session
    }

    fn blank(&self, now: Instant) -> Box<SimFrame> {
        Box::new(SimFrame::new(
            BattleFrame::new(
                self.session.world.registries().clone(),
                self.statics.clone(),
            ),
            now,
        ))
    }

    /// Applies one request. Speed and pause first bring the clock up to
    /// `now` at the old rate.
    pub fn handle(&mut self, request: SimRequest, now: Instant) {
        match request {
            SimRequest::Queue(kind) => self.session.queue(kind),
            SimRequest::SetSpeed(speed) => {
                self.pump(now);
                self.session.set_speed(speed);
            }
            SimRequest::SetPaused(paused) => {
                self.pump(now);
                self.session.set_paused(paused);
            }
            SimRequest::Note(text) => self.session.note(text),
            SimRequest::ReplaceRegistries(regs) => self.session.world.replace_registries(regs),
            SimRequest::SetDetail(detail) => self.detail = detail,
            SimRequest::Call(f) => f(&mut self.session),
        }
    }

    /// Feeds the wall time since the last pump to the accumulator and steps
    /// what is due; returns the ticks stepped.
    pub fn pump(&mut self, now: Instant) -> u32 {
        let dt = self
            .last
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f64());
        self.last = Some(now);
        let before = Instant::now();
        let outputs = self.session.advance_with(dt, &mut self.profiler);
        if outputs.is_empty() {
            return 0;
        }
        self.step_seconds += before.elapsed().as_secs_f64();
        let n = outputs.len() as u32;
        self.ticks += n;
        for o in outputs {
            self.events.extend(o.events);
        }
        n
    }

    /// The wait until the next tick is due; `None` while nothing advances.
    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        let mult = self.session.multiplier();
        if mult <= 0.0 {
            return None;
        }
        let since = self
            .last
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f64());
        let left = (TICK - self.session.accumulator()) / mult - since;
        Some(Duration::from_secs_f64(left.max(0.0)))
    }

    /// Writes the battle as it stands and the history since the last fill.
    pub fn fill(&mut self, f: &mut SimFrame, now: Instant) {
        let s = &self.session;
        f.battle
            .capture_into(&s.world.view(), &self.statics, self.detail);
        f.clock = FrameClock {
            at: now,
            acc: s.accumulator(),
            mult: s.multiplier(),
        };
        f.ticks = std::mem::take(&mut self.ticks);
        f.events.clear();
        f.events.append(&mut self.events);
        f.lines.clear();
        let new = s.lines_pushed() - self.lines_sent;
        let ring = s.events();
        let n = (new as usize).min(ring.len());
        f.lines.extend(ring.iter().skip(ring.len() - n).cloned());
        self.lines_sent = s.lines_pushed();
        f.corpses.clear();
        f.corpses.extend_from_slice(s.corpses());
        if self.result.is_none() {
            self.result = s.result().cloned().map(Arc::new);
        }
        f.result.clone_from(&self.result);
        f.replay_mismatch = s.replay_mismatch();
        f.replay_finished = s.replay_finished();
        f.fed_commands = s.command_log().len();
        f.ai_commands = s.ai_log().len();
        f.observer_side = s.observer_side();
        f.stats = self.profiler.stats();
        f.step_seconds = std::mem::take(&mut self.step_seconds);
    }
}

/// The newest unread frame and the spent ones for reuse.
#[derive(Default)]
struct FrameSlot {
    latest: Mutex<Option<Box<SimFrame>>>,
    #[allow(
        clippy::vec_box,
        reason = "frames move between the slot and the handle by pointer"
    )]
    spare: Mutex<Vec<Box<SimFrame>>>,
}

impl FrameSlot {
    fn put(&self, mut frame: Box<SimFrame>) {
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mut older) = latest.take() {
            frame.absorb(&mut older);
            self.recycle(older);
        }
        *latest = Some(frame);
    }

    fn take(&self) -> Option<Box<SimFrame>> {
        self.latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn recycle(&self, frame: Box<SimFrame>) {
        let mut spare = self.spare.lock().unwrap_or_else(PoisonError::into_inner);
        if spare.len() < 2 {
            spare.push(frame);
        }
    }

    fn spare(&self) -> Option<Box<SimFrame>> {
        self.spare
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
    }
}

/// The thread's loop: sleep until a request or the next tick, apply the
/// requests, step what is due, publish.
fn run(mut driver: SimDriver, rx: Receiver<SimRequest>, slot: Arc<FrameSlot>) {
    loop {
        let wait = driver.due_in(Instant::now());
        let first = match wait {
            None => match rx.recv() {
                Ok(r) => Some(r),
                Err(_) => return,
            },
            Some(d) if d > SPIN => match rx.recv_timeout(d - SPIN) {
                Ok(r) => Some(r),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            Some(_) => None,
        };
        let mut changed = false;
        if let Some(r) = first {
            driver.handle(r, Instant::now());
            changed = true;
        }
        loop {
            match rx.try_recv() {
                Ok(r) => {
                    driver.handle(r, Instant::now());
                    changed = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if !changed {
            // The last stretch before the tick: yield, not sleep.
            while driver
                .due_in(Instant::now())
                .is_some_and(|d| d > Duration::ZERO)
            {
                std::thread::yield_now();
            }
        }
        let stepped = driver.pump(Instant::now());
        if stepped > 0 || changed {
            let now = Instant::now();
            let mut frame = slot.spare().unwrap_or_else(|| driver.blank(now));
            driver.fill(&mut frame, now);
            slot.put(frame);
        }
    }
}

/// The sim on its own thread; dropping it stops and joins the thread.
pub struct SimThread {
    tx: Option<Sender<SimRequest>>,
    slot: Arc<FrameSlot>,
    join: Option<JoinHandle<()>>,
}

impl SimThread {
    fn spawn(driver: SimDriver) -> Self {
        let (tx, rx) = channel();
        let slot = Arc::new(FrameSlot::default());
        let thread_slot = slot.clone();
        let join = std::thread::Builder::new()
            .name("il_sim".to_string())
            .spawn(move || run(driver, rx, thread_slot))
            .expect("spawning the sim thread");
        Self {
            tx: Some(tx),
            slot,
            join: Some(join),
        }
    }

    fn send(&self, request: SimRequest) {
        if let Some(tx) = &self.tx {
            // A thread that panicked has gone; the next frame never comes.
            let _ = tx.send(request);
        }
    }
}

impl Drop for SimThread {
    fn drop(&mut self) {
        // Closing the channel ends the loop.
        self.tx = None;
        if let Some(join) = self.join.take()
            && join.join().is_err()
        {
            eprintln!("the sim thread panicked");
        }
    }
}

/// Where the session runs.
pub enum SimHost {
    /// `--single-thread-sim`: stepped on the main thread once per frame.
    Inline(Box<SimDriver>),
    Thread(SimThread),
}

/// The main thread's handle on a battle: the newest frame, the event
/// panel's ring and the local mirrors of what only the main thread sets.
pub struct BattleHandle {
    host: SimHost,
    frame: Box<SimFrame>,
    /// Inline: the frame being refilled (swapped with `frame`).
    spare: Option<Box<SimFrame>>,
    events: VecDeque<EventLine>,
    paused: bool,
    speed: f32,
    detail: FrameDetail,
    is_replay: bool,
    replay_ticks: usize,
    local_player: PlayerId,
    stem: String,
    /// Inline: something changed that the next update must capture.
    dirty: bool,
}

impl BattleHandle {
    /// Captures the first frame, then starts the thread (or keeps the
    /// driver inline).
    pub fn new(session: BattleSession, threaded: bool) -> Self {
        let now = Instant::now();
        let mut driver = SimDriver::new(session);
        let mut frame = driver.blank(now);
        driver.fill(&mut frame, now);
        let s = driver.session();
        let paused = s.paused();
        let speed = s.speed();
        let is_replay = s.is_replay();
        let replay_ticks = match s.mode() {
            SessionMode::Replay { expected, .. } => expected.len(),
            SessionMode::Live => 0,
        };
        let local_player = s.local_player();
        let stem = s.scenario_stem().to_string();
        let mut events = VecDeque::with_capacity(EVENT_RING);
        push_lines(&mut events, &frame.lines);
        let host = if threaded {
            SimHost::Thread(SimThread::spawn(driver))
        } else {
            SimHost::Inline(Box::new(driver))
        };
        Self {
            host,
            frame,
            spare: None,
            events,
            paused,
            speed,
            detail: FrameDetail::default(),
            is_replay,
            replay_ticks,
            local_player,
            stem,
            dirty: false,
        }
    }

    pub fn is_threaded(&self) -> bool {
        matches!(self.host, SimHost::Thread(_))
    }

    /// Takes the newest frame (inline: steps and captures first). Returns
    /// whether a new frame arrived; its `events` and `lines` are then this
    /// update's.
    pub fn update(&mut self, now: Instant) -> bool {
        let fresh = match &mut self.host {
            SimHost::Inline(driver) => {
                let stepped = driver.pump(now);
                if stepped > 0 || self.dirty {
                    let mut f = self.spare.take().unwrap_or_else(|| driver.blank(now));
                    driver.fill(&mut f, now);
                    let old = std::mem::replace(&mut self.frame, f);
                    self.spare = Some(old);
                    self.dirty = false;
                    true
                } else {
                    false
                }
            }
            SimHost::Thread(t) => match t.slot.take() {
                Some(f) => {
                    let old = std::mem::replace(&mut self.frame, f);
                    t.slot.recycle(old);
                    true
                }
                None => false,
            },
        };
        if fresh {
            let lines = std::mem::take(&mut self.frame.lines);
            push_lines(&mut self.events, &lines);
            self.frame.lines = lines;
        }
        fresh
    }

    fn send(&mut self, request: SimRequest) {
        match &mut self.host {
            SimHost::Inline(driver) => {
                driver.handle(request, Instant::now());
                self.dirty = true;
            }
            SimHost::Thread(t) => t.send(request),
        }
    }

    /// Runs `f` on the session and returns its answer: inline at once, on
    /// the thread between two ticks (at most one tick's wait).
    pub fn call<R: Send + 'static>(
        &mut self,
        f: impl FnOnce(&mut BattleSession) -> R + Send + 'static,
    ) -> Option<R> {
        let (tx, rx) = channel();
        self.send(SimRequest::Call(Box::new(move |s| {
            let _ = tx.send(f(s));
        })));
        rx.recv().ok()
    }

    /// The newest frame.
    pub fn frame(&self) -> &SimFrame {
        &self.frame
    }

    /// The battle as of the newest frame.
    pub fn view(&self) -> &BattleFrame {
        &self.frame.battle
    }

    pub fn alpha(&self, now: Instant) -> f32 {
        self.frame.clock.alpha(now)
    }

    /// Queues a command from the local player; a playback takes none.
    pub fn queue(&mut self, kind: CommandKind) {
        if !self.is_replay {
            self.send(SimRequest::Queue(kind));
        }
    }

    /// `Surrender` for every side the local player owns (T2-090, pause
    /// menu; SIM-FLOW-017).
    pub fn surrender(&mut self) {
        self.queue(CommandKind::Surrender);
    }

    pub fn note(&mut self, text: String) {
        self.send(SimRequest::Note(text));
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.send(SimRequest::SetPaused(paused));
    }

    pub fn speed(&self) -> f32 {
        self.speed
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.max(0.0);
        self.send(SimRequest::SetSpeed(self.speed));
    }

    pub fn replace_registries(&mut self, regs: Arc<Registries>) {
        self.send(SimRequest::ReplaceRegistries(regs));
    }

    /// What the debug overlays need; sent only when it changes.
    pub fn set_detail(&mut self, detail: FrameDetail) {
        if detail != self.detail {
            self.detail = detail;
            self.send(SimRequest::SetDetail(detail));
        }
    }

    pub fn is_replay(&self) -> bool {
        self.is_replay
    }

    /// Ticks in the recording being played back (0 when live).
    pub fn replay_ticks(&self) -> usize {
        self.replay_ticks
    }

    pub fn local_player(&self) -> PlayerId {
        self.local_player
    }

    pub fn scenario_stem(&self) -> &str {
        &self.stem
    }

    pub fn observer_side(&self) -> Option<u8> {
        self.frame.observer_side
    }

    pub fn result(&self) -> Option<&BattleResult> {
        self.frame.result.as_deref()
    }

    pub fn setup(&self) -> Option<&BattleSetup> {
        self.frame.battle.setup()
    }

    pub fn corpses(&self) -> &[Corpse] {
        &self.frame.corpses
    }

    /// The event panel's lines, oldest first.
    pub fn events(&self) -> &VecDeque<EventLine> {
        &self.events
    }
}

fn push_lines(ring: &mut VecDeque<EventLine>, lines: &[EventLine]) {
    for line in lines {
        if ring.len() == EVENT_RING {
            ring.pop_front();
        }
        ring.push_back(line.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_sim_battle::{BattlePhase, BattleWorld, ScriptedCommands};
    use std::path::Path;

    fn game_regs() -> Arc<Registries> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
        il_cli::load_registries(&root).unwrap_or_else(|e| panic!("{e:#}"))
    }

    fn skirmish(regs: Arc<Registries>) -> BattleSession {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/scenarios/ai_skirmish_300.json5");
        let scenario = il_cli::load_scenario(&path).unwrap();
        let world = BattleWorld::new(&scenario.setup, regs).unwrap();
        BattleSession::new(world, PlayerId(0), scenario.script(), Vec::new())
            .with_stem("ai_skirmish_300")
    }

    fn empty() -> BattleSession {
        let world = BattleWorld::empty(7, Arc::new(Registries::default()), BattlePhase::Battle);
        BattleSession::new(world, PlayerId(0), ScriptedCommands::default(), Vec::new())
    }

    fn line(tick: u32, text: &str) -> EventLine {
        EventLine {
            tick: Tick(tick),
            text: text.to_string(),
        }
    }

    #[test]
    fn alpha_follows_the_frame_clock() {
        let at = Instant::now();
        let clock = FrameClock {
            at,
            acc: TICK * 0.5,
            mult: 1.0,
        };
        assert!((clock.alpha(at) - 0.5).abs() < 1e-6);
        let quarter = Duration::from_secs_f64(TICK * 0.25);
        assert!((clock.alpha(at + quarter) - 0.75).abs() < 1e-4);
        assert!(
            clock.alpha(at + quarter * 8) < 1.0,
            "held below the next tick"
        );
        let fast = FrameClock { mult: 2.0, ..clock };
        assert!((fast.alpha(at + quarter) - 1.0).abs() < 1e-4);
        let paused = FrameClock { mult: 0.0, ..clock };
        assert!((paused.alpha(at + quarter * 3) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn an_unread_frame_hands_its_history_to_the_next() {
        let driver = SimDriver::new(empty());
        let now = Instant::now();
        let slot = FrameSlot::default();
        let mut a = driver.blank(now);
        a.ticks = 2;
        a.lines = vec![line(1, "a1"), line(2, "a2")];
        let mut b = driver.blank(now);
        b.ticks = 1;
        b.lines = vec![line(3, "b")];
        slot.put(a);
        slot.put(b);
        let got = slot.take().expect("the newest frame");
        assert_eq!(got.ticks, 3);
        let texts: Vec<&str> = got.lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["a1", "a2", "b"]);
        assert!(slot.take().is_none());
        assert!(slot.spare().is_some(), "the older frame was kept for reuse");
    }

    /// The inline host steps exactly as the bare session does for the same
    /// wall times, and the frames carry every event line once.
    #[test]
    fn inline_frames_follow_the_session() {
        let regs = game_regs();
        let mut reference = skirmish(regs.clone());
        let mut h = BattleHandle::new(skirmish(regs), false);
        assert!(!h.is_threaded());
        let t0 = Instant::now();
        h.update(t0);
        for k in 1..=120u32 {
            reference.advance(TICK);
            // A nanosecond over the tick (`TICK` is the f32 0.05 widened,
            // a hair above 50 ms), so every update steps exactly one.
            let fresh = h.update(t0 + Duration::from_nanos(50_000_001 * u64::from(k)));
            assert!(fresh, "tick {k}");
            assert_eq!(h.view().tick(), reference.world.tick());
            assert_eq!(h.frame().ticks, 1);
        }
        let hashes = h.call(|s| s.hashes().to_vec()).unwrap();
        assert_eq!(hashes, reference.hashes());
        let mine: Vec<&EventLine> = h.events().iter().collect();
        let theirs: Vec<&EventLine> = reference.events().iter().collect();
        assert_eq!(mine, theirs);
    }

    /// T3-032: the battle stepped on the thread, re-simulated inline from
    /// its own recording, gives the same hash every tick; the speed and
    /// pause commands were stamped at the sim's own tick (no StaleTick).
    #[test]
    fn the_thread_steps_the_same_battle_and_its_recording_verifies() {
        let regs = game_regs();
        let mut h = BattleHandle::new(skirmish(regs.clone()), true);
        assert!(h.is_threaded());
        h.set_speed(8.0);
        let start = Instant::now();
        while h.view().tick().0 < 200 {
            assert!(
                start.elapsed() < Duration::from_secs(120),
                "the thread stalled"
            );
            h.update(Instant::now());
            std::thread::sleep(Duration::from_millis(5));
        }
        h.set_paused(true);
        let (hashes, replay) = h
            .call(|s| (s.hashes().to_vec(), s.replay().unwrap()))
            .unwrap();
        h.update(Instant::now());
        assert!(hashes.len() >= 200);
        assert!(
            h.events().iter().all(|l| !l.text.contains("StaleTick")),
            "every command was stamped for the sim's next tick"
        );

        // Played back inline from what the thread fed it, every tick's
        // hash agrees with the thread's.
        assert_eq!(replay.hashes, hashes);
        let mut playback = BattleSession::from_replay(replay, regs, 1).unwrap();
        while !playback.replay_finished() {
            playback.advance(TICK * 4.0);
        }
        assert_eq!(playback.replay_mismatch(), None);
        drop(h);
    }
}
