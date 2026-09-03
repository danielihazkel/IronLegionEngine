//! `il_cli bench` (T1-080, REQ-TOOL-002, REQ-PERF-005, TDD §17).
//!
//! Steps a generated move-and-reform battle on the Phase 1 test map and
//! reports the wall time of every schedule stage (mean, p95, max over the
//! run), then compares the means against a checked-in baseline
//! (`benches/baseline.json`) and flags anything more than
//! [`REGRESSION_PCT`] slower.
//!
//! The setup is built in code, not read from a file, so the same command
//! covers 2k, 10k and 20k soldiers: regiments of [`REGIMENT_SIZE`] on a
//! grid north of the river, a scripted advance, a column change for half
//! of them, a wheel for the other half, a return march, a reform to line
//! and a halt, all inside [`SCRIPT_TICKS`] ticks.
//!
//! Timing goes through a [`StageObserver`], which sees the stage
//! boundaries and nothing else: the clock never reaches the sim.
// Wall-clock durations are reported in milliseconds as f64; nothing here
// feeds the sim (SIM-DET-006 is about sim inputs).
#![allow(clippy::float_arithmetic)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, anyhow, bail};
use il_core::{Angle, PlayerId, RegimentId, Tick, V2};
use il_data::ContentId;
use il_sim_battle::{
    BattleSetup, BattleWorld, Command, CommandKind, GeneralSetup, RegimentSetup, SOLDIER_CAP,
    Scenario, SideSetup, SpeedMode, Stage, StageObserver, VictoryRules,
};
use serde::{Deserialize, Serialize};

use crate::load_registries;

/// Soldiers per generated regiment; `--soldiers` must be a multiple.
pub const REGIMENT_SIZE: u16 = 200;
/// Ticks the scripted command stream spans; runs shorter than this stop
/// before the last commands fire.
pub const SCRIPT_TICKS: u32 = 600;
/// A stage (or the tick) is a regression when its mean is this much slower
/// than the baseline.
pub const REGRESSION_PCT: f64 = 20.0;
/// Stages whose baseline mean is under this are noise and never compared.
pub const NOISE_FLOOR_MS: f64 = 0.05;
/// The Phase 1 stages (Formation through Collision, SAD §6.2), whose sum at
/// 2k soldiers is the T1-080 done-when.
pub const PHASE1_STAGES: std::ops::RangeInclusive<usize> = 2..=7;

const MAP_ID: &str = "rome:test_field";
const UNIT_TYPES: [&str; 3] = ["rome:hastati", "rome:velites", "greece:hoplite"];
const GENERAL_UNIT: &str = "rome:hastati";
/// Regiments per row; 12 × 60 m fits the 800 m map with a margin.
const COLUMNS: u32 = 12;
const X_ORIGIN: f32 = 70.0;
const X_PITCH: f32 = 60.0;
const Y_ORIGIN: f32 = 30.0;
const Y_PITCH: f32 = 24.0;
/// Rows that stay north of the river (y ≈ 284 at its highest bank) after
/// the [`ADVANCE`]: 30 + 8 × 24 + 50 = 272.
const MAX_ROWS: u32 = 9;
/// Metres the whole army advances (+y) and then marches back.
const ADVANCE: f32 = 50.0;

/// Options of `il_cli bench`.
#[derive(Clone, Debug)]
pub struct BenchOptions {
    /// Soldier count; a multiple of [`REGIMENT_SIZE`].
    pub soldiers: u32,
    /// Ticks to step.
    pub ticks: u32,
    /// Worker threads (`1` = single-threaded executor).
    pub threads: usize,
    /// Mod root holding `mod.json5` and `content/`.
    pub content_root: PathBuf,
    /// Write the report here as JSON.
    pub json: Option<PathBuf>,
    /// Compare against this baseline file.
    pub baseline: Option<PathBuf>,
    /// With `baseline`: fail on any regression instead of warning.
    pub strict: bool,
    /// Insert this run into the baseline file (created if missing).
    pub record_baseline: Option<PathBuf>,
    /// Machine description written when recording.
    pub machine: Option<String>,
    /// Date written when recording.
    pub recorded: Option<String>,
}

impl BenchOptions {
    pub fn new(soldiers: u32, ticks: u32) -> Self {
        Self {
            soldiers,
            ticks,
            threads: 8,
            content_root: PathBuf::from("game"),
            json: None,
            baseline: None,
            strict: false,
            record_baseline: None,
            machine: None,
            recorded: None,
        }
    }
}

/// Mean, 95th percentile and maximum of a set of millisecond samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub mean_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

impl Summary {
    /// Sorts `samples` in place; empty input gives zeros.
    pub fn of(samples: &mut [f64]) -> Self {
        if samples.is_empty() {
            return Self::default();
        }
        samples.sort_by(f64::total_cmp);
        let n = samples.len();
        let mean_ms = samples.iter().sum::<f64>() / n as f64;
        // Nearest-rank p95: the smallest sample at or above 95 % of the set.
        let rank = ((n as f64 * 0.95).ceil() as usize).clamp(1, n);
        Self {
            mean_ms,
            p95_ms: samples[rank - 1],
            max_ms: samples[n - 1],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageReport {
    pub index: usize,
    pub name: String,
    #[serde(flatten)]
    pub summary: Summary,
}

/// One bench run; the unit stored per soldier count in a [`Baseline`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    pub soldiers: u32,
    pub regiments: u32,
    pub ticks: u32,
    pub threads: usize,
    /// `"release"` or `"debug"`; comparisons across profiles are warned about.
    pub profile: String,
    pub stages: Vec<StageReport>,
    pub tick: Summary,
    /// Sum of the stage means over [`PHASE1_STAGES`].
    pub phase1_stages_mean_ms: f64,
}

/// `benches/baseline.json`: one report per soldier count, keyed by the
/// count as a decimal string.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub machine: String,
    pub recorded: String,
    pub runs: BTreeMap<String, BenchReport>,
}

impl Baseline {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading baseline {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing baseline {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text + "\n")
            .with_context(|| format!("writing baseline {}", path.display()))
    }

    pub fn run_for(&self, soldiers: u32) -> Option<&BenchReport> {
        self.runs.get(&soldiers.to_string())
    }
}

/// A stage (or `"tick"`) whose mean grew by more than [`REGRESSION_PCT`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Regression {
    pub name: String,
    pub baseline_ms: f64,
    pub now_ms: f64,
    pub change_pct: f64,
}

/// Percentage change of `now` over `base`.
fn change_pct(base: f64, now: f64) -> f64 {
    (now - base) / base * 100.0
}

/// Compares stage and tick means; stages under [`NOISE_FLOOR_MS`] in the
/// baseline are skipped.
pub fn compare(now: &BenchReport, base: &BenchReport) -> Vec<Regression> {
    let mut out = Vec::new();
    let pairs = now
        .stages
        .iter()
        .filter_map(|s| {
            base.stages
                .iter()
                .find(|b| b.name == s.name)
                .map(|b| (s.name.as_str(), b.summary.mean_ms, s.summary.mean_ms))
        })
        .chain(std::iter::once((
            "tick",
            base.tick.mean_ms,
            now.tick.mean_ms,
        )));
    for (name, baseline_ms, now_ms) in pairs {
        if baseline_ms < NOISE_FLOOR_MS {
            continue;
        }
        let pct = change_pct(baseline_ms, now_ms);
        if pct > REGRESSION_PCT {
            out.push(Regression {
                name: name.to_owned(),
                baseline_ms,
                now_ms,
                change_pct: pct,
            });
        }
    }
    out
}

fn cid(s: &str) -> ContentId {
    ContentId::new(s).unwrap_or_else(|e| panic!("literal content id {s}: {e}"))
}

/// Anchor of the `i`-th generated regiment.
fn anchor(i: u32) -> [f32; 2] {
    let (row, col) = (i / COLUMNS, i % COLUMNS);
    [
        X_ORIGIN + col as f32 * X_PITCH,
        Y_ORIGIN + row as f32 * Y_PITCH,
    ]
}

/// Builds the bench scenario for `soldiers` soldiers: `soldiers / 200`
/// regiments of alternating infantry types on the test map, plus the
/// scripted move/reform stream. Errors if the count is not a positive
/// multiple of [`REGIMENT_SIZE`] or more regiments than the grid holds.
pub fn generate_scenario(soldiers: u32) -> anyhow::Result<Scenario> {
    let size = u32::from(REGIMENT_SIZE);
    if soldiers == 0 || !soldiers.is_multiple_of(size) {
        bail!("--soldiers must be a positive multiple of {size}, got {soldiers}");
    }
    if soldiers > SOLDIER_CAP {
        bail!("--soldiers {soldiers} exceeds the cap of {SOLDIER_CAP} (SIM-CORE-006)");
    }
    let regiments = soldiers / size;
    let capacity = COLUMNS * MAX_ROWS;
    if regiments > capacity {
        bail!(
            "--soldiers {soldiers} needs {regiments} regiments; the generated grid holds {capacity} ({} soldiers)",
            capacity * size
        );
    }
    let setup = BattleSetup {
        map_id: cid(MAP_ID),
        seed: 7,
        weather: Default::default(),
        time_of_day: 12,
        time_limit_ticks: 48_000,
        reveal_deployment: false,
        sides: vec![SideSetup {
            faction: cid("rome:rome"),
            player: PlayerId(0),
            deployment_zone: 0,
            general: GeneralSetup {
                unit_type: cid(GENERAL_UNIT),
                rank: 1,
                name_key: "rome.generals.placeholder".to_owned(),
            },
            regiments: (0..regiments)
                .map(|i| RegimentSetup {
                    id: i + 1,
                    unit_type: cid(UNIT_TYPES[(i % UNIT_TYPES.len() as u32) as usize]),
                    count: REGIMENT_SIZE,
                    experience: 0,
                    fatigue: 0.0,
                    formation: None,
                    position: Some(anchor(i)),
                    facing_deg: Some(90.0),
                })
                .collect(),
            reinforcements: Vec::new(),
        }],
        victory: VictoryRules::default(),
    };

    // Regiment ids are the spawn order (0..regiments).
    let all: Vec<RegimentId> = (0..regiments).map(RegimentId).collect();
    let evens: Vec<RegimentId> = all.iter().copied().step_by(2).collect();
    let odds: Vec<RegimentId> = all.iter().copied().skip(1).step_by(2).collect();
    let target = |i: u32, dy: f32| {
        let [x, y] = anchor(i);
        V2::from_f32_data(x, y + dy)
    };
    let mut commands = Vec::new();
    let mut push = |tick: u32, seq: u16, kind: CommandKind| {
        commands.push(Command {
            tick: Tick(tick),
            player: PlayerId(0),
            seq,
            kind,
        });
    };
    // 1 s: every regiment advances 50 m at a run (one command per regiment,
    // each to its own target, so the grid keeps its shape).
    for (seq, id) in all.iter().enumerate() {
        push(
            20,
            seq as u16,
            CommandKind::Move {
                regiments: vec![*id],
                target: target(id.0, ADVANCE),
                facing: Some(Angle::from_degrees_data(270.0)),
                speed: SpeedMode::Run,
            },
        );
    }
    // 10 s: half morph into column while moving; 15 s: the other half wheels.
    push(
        200,
        0,
        CommandKind::SetFormation {
            regiments: evens,
            template: cid("rome:column"),
            ranks: None,
        },
    );
    push(
        300,
        0,
        CommandKind::SetFacing {
            regiments: odds,
            facing: Angle::from_degrees_data(0.0),
        },
    );
    // 21 s: everyone marches back to the start line.
    for (seq, id) in all.iter().enumerate() {
        push(
            420,
            seq as u16,
            CommandKind::Move {
                regiments: vec![*id],
                target: target(id.0, 0.0),
                facing: Some(Angle::from_degrees_data(90.0)),
                speed: SpeedMode::Run,
            },
        );
    }
    // 26 s: back into line; 28 s: halt.
    push(
        520,
        0,
        CommandKind::SetFormation {
            regiments: all.clone(),
            template: cid("rome:line"),
            ranks: None,
        },
    );
    push(560, 0, CommandKind::Halt { regiments: all });
    debug_assert!(commands.iter().all(|c| c.tick.0 <= SCRIPT_TICKS));
    Ok(Scenario { setup, commands })
}

/// Collects per-stage and per-tick wall times (milliseconds).
#[derive(Default)]
pub struct StageTimer {
    stage_start: Option<Instant>,
    tick_start: Option<Instant>,
    /// One sample list per stage.
    pub stages: Vec<Vec<f64>>,
    pub ticks: Vec<f64>,
}

impl StageTimer {
    pub fn new() -> Self {
        Self {
            stages: vec![Vec::new(); Stage::COUNT],
            ..Default::default()
        }
    }
}

// The timer is the one place in il_cli that reads the clock: it observes
// stage boundaries from outside the sim and only ever writes reports.
#[allow(clippy::disallowed_methods)]
impl StageObserver for StageTimer {
    fn begin(&mut self, stage: Stage) {
        let now = Instant::now();
        if stage == Stage::ALL[0] {
            self.tick_start = Some(now);
        }
        self.stage_start = Some(now);
    }

    fn end(&mut self, stage: Stage) {
        let now = Instant::now();
        if let Some(start) = self.stage_start.take() {
            self.stages[stage.index()].push(now.duration_since(start).as_secs_f64() * 1000.0);
        }
        if stage == Stage::ALL[Stage::COUNT - 1]
            && let Some(start) = self.tick_start.take()
        {
            self.ticks
                .push(now.duration_since(start).as_secs_f64() * 1000.0);
        }
    }
}

impl StageTimer {
    pub fn report(mut self, soldiers: u32, regiments: u32, threads: usize) -> BenchReport {
        let stages: Vec<StageReport> = Stage::ALL
            .iter()
            .map(|stage| StageReport {
                index: stage.index(),
                name: stage.name().to_owned(),
                summary: Summary::of(&mut self.stages[stage.index()]),
            })
            .collect();
        let phase1_stages_mean_ms = stages[PHASE1_STAGES]
            .iter()
            .map(|s| s.summary.mean_ms)
            .sum();
        BenchReport {
            soldiers,
            regiments,
            ticks: self.ticks.len() as u32,
            threads,
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
            .to_owned(),
            stages,
            tick: Summary::of(&mut self.ticks),
            phase1_stages_mean_ms,
        }
    }
}

/// Steps the generated scenario and returns the timings.
pub fn measure(opts: &BenchOptions) -> anyhow::Result<BenchReport> {
    let regs = load_registries(&opts.content_root)?;
    let scenario = generate_scenario(opts.soldiers)?;
    let regiments = scenario.setup.sides[0].regiments.len() as u32;
    let mut script = scenario.script();
    let mut world = BattleWorld::new(&scenario.setup, regs)?;
    world.set_threads(opts.threads);
    let mut timer = StageTimer::new();
    while world.tick().0 < opts.ticks {
        let commands = script.take_for(world.tick().next());
        world.step_observed(&commands, &mut timer);
    }
    Ok(timer.report(opts.soldiers, regiments, opts.threads))
}

fn write_table(
    out: &mut dyn Write,
    report: &BenchReport,
    base: Option<&BenchReport>,
) -> anyhow::Result<()> {
    writeln!(
        out,
        "bench: {} soldiers in {} regiments, {} ticks, {} thread(s), {} build",
        report.soldiers, report.regiments, report.ticks, report.threads, report.profile
    )?;
    match base {
        Some(_) => writeln!(
            out,
            "{:<20} {:>9} {:>9} {:>9} {:>10} {:>8}",
            "stage", "mean ms", "p95 ms", "max ms", "base ms", "change"
        )?,
        None => writeln!(
            out,
            "{:<20} {:>9} {:>9} {:>9}",
            "stage", "mean ms", "p95 ms", "max ms"
        )?,
    }
    let rows = report
        .stages
        .iter()
        .map(|s| (format!("{:>2} {}", s.index, s.name), s.summary))
        .chain(std::iter::once(("tick".to_owned(), report.tick)));
    for (name, s) in rows {
        write!(
            out,
            "{name:<20} {:>9.3} {:>9.3} {:>9.3}",
            s.mean_ms, s.p95_ms, s.max_ms
        )?;
        if let Some(b) = base {
            let base_mean = if name == "tick" {
                Some(b.tick.mean_ms)
            } else {
                b.stages
                    .iter()
                    .find(|x| name.ends_with(x.name.as_str()))
                    .map(|x| x.summary.mean_ms)
            };
            match base_mean {
                Some(m) if m >= NOISE_FLOOR_MS => {
                    write!(out, " {m:>10.3} {:>+7.1}%", change_pct(m, s.mean_ms))?;
                }
                Some(m) => write!(out, " {m:>10.3} {:>8}", "-")?,
                None => write!(out, " {:>10} {:>8}", "-", "-")?,
            }
        }
        writeln!(out)?;
    }
    writeln!(
        out,
        "stages {}-{} (Phase 1) mean sum: {:.3} ms",
        PHASE1_STAGES.start(),
        PHASE1_STAGES.end(),
        report.phase1_stages_mean_ms
    )?;
    Ok(())
}

/// Runs the bench, prints the table, writes `--json`, records the baseline
/// and returns the report with any regressions against `--baseline`.
pub fn bench(
    opts: &BenchOptions,
    out: &mut dyn Write,
) -> anyhow::Result<(BenchReport, Vec<Regression>)> {
    let report = measure(opts)?;
    let baseline = match &opts.baseline {
        Some(path) => Some(Baseline::load(path)?),
        None => None,
    };
    let base_run = baseline.as_ref().and_then(|b| b.run_for(opts.soldiers));
    write_table(out, &report, base_run)?;

    let mut regressions = Vec::new();
    if let Some(b) = &baseline {
        match base_run {
            None => writeln!(
                out,
                "baseline: no run for {} soldiers in {}",
                opts.soldiers,
                opts.baseline
                    .as_ref()
                    .map_or_else(String::new, |p| p.display().to_string())
            )?,
            Some(base) => {
                if base.profile != report.profile || base.threads != report.threads {
                    writeln!(
                        out,
                        "baseline: recorded as a {} build on {} thread(s) ({}); this run is a {} build on {} thread(s)",
                        base.profile, base.threads, b.machine, report.profile, report.threads
                    )?;
                }
                regressions = compare(&report, base);
                for r in &regressions {
                    writeln!(
                        out,
                        "{}: {} is {:+.1}% slower than the baseline ({:.3} ms vs {:.3} ms)",
                        if opts.strict { "regression" } else { "warning" },
                        r.name,
                        r.change_pct,
                        r.now_ms,
                        r.baseline_ms
                    )?;
                }
                if regressions.is_empty() {
                    writeln!(out, "baseline: no stage more than {REGRESSION_PCT}% slower")?;
                }
            }
        }
    }

    if let Some(path) = &opts.json {
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")
            .with_context(|| format!("writing {}", path.display()))?;
    }
    if let Some(path) = &opts.record_baseline {
        let mut b = if path.exists() {
            Baseline::load(path)?
        } else {
            Baseline::default()
        };
        if let Some(m) = &opts.machine {
            b.machine.clone_from(m);
        }
        if let Some(d) = &opts.recorded {
            b.recorded.clone_from(d);
        }
        b.runs.insert(opts.soldiers.to_string(), report.clone());
        b.save(path)?;
        writeln!(
            out,
            "baseline: recorded {} soldiers in {}",
            opts.soldiers,
            path.display()
        )?;
    }
    Ok((report, regressions))
}

/// Error text for `--strict` failures.
pub fn strict_error(regressions: &[Regression]) -> anyhow::Error {
    let names: Vec<&str> = regressions.iter().map(|r| r.name.as_str()).collect();
    anyhow!(
        "{} stage(s) more than {REGRESSION_PCT}% slower than the baseline: {}",
        regressions.len(),
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(name: &str, index: usize, mean: f64) -> StageReport {
        StageReport {
            index,
            name: name.to_owned(),
            summary: Summary {
                mean_ms: mean,
                p95_ms: mean,
                max_ms: mean,
            },
        }
    }

    fn report(stages: Vec<StageReport>, tick: f64) -> BenchReport {
        BenchReport {
            soldiers: 2000,
            regiments: 10,
            ticks: 10,
            threads: 1,
            profile: "test".to_owned(),
            stages,
            tick: Summary {
                mean_ms: tick,
                p95_ms: tick,
                max_ms: tick,
            },
            phase1_stages_mean_ms: 0.0,
        }
    }

    #[test]
    fn summary_uses_nearest_rank_p95() {
        let mut samples: Vec<f64> = (1..=100).map(f64::from).collect();
        let s = Summary::of(&mut samples);
        assert_eq!(s.mean_ms, 50.5);
        assert_eq!(s.p95_ms, 95.0);
        assert_eq!(s.max_ms, 100.0);
        assert_eq!(Summary::of(&mut []), Summary::default());
        assert_eq!(Summary::of(&mut [3.0]).p95_ms, 3.0);
    }

    #[test]
    fn compare_flags_over_twenty_percent_and_skips_noise() {
        let base = report(
            vec![
                stage("Collision", 7, 1.0),
                stage("Ai", 1, 0.001),
                stage("Integrate", 5, 0.5),
            ],
            2.0,
        );
        let now = report(
            vec![
                stage("Collision", 7, 1.25),
                stage("Ai", 1, 0.1),
                stage("Integrate", 5, 0.6),
            ],
            2.5,
        );
        let regs = compare(&now, &base);
        let names: Vec<&str> = regs.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["Collision", "tick"]);
        assert!((regs[0].change_pct - 25.0).abs() < 1e-9);
        assert!(compare(&base, &base).is_empty());
    }

    #[test]
    fn generated_scenario_has_the_expected_shape() {
        let s = generate_scenario(2_000).unwrap();
        assert_eq!(s.setup.sides[0].regiments.len(), 10);
        assert_eq!(s.setup.soldier_total(), 2_000);
        let s = generate_scenario(20_000).unwrap();
        let regs = &s.setup.sides[0].regiments;
        assert_eq!(regs.len(), 100);
        for r in regs {
            let [x, y] = r.position.unwrap();
            assert!((40.0..=760.0).contains(&x), "x = {x}");
            assert!(y + ADVANCE < 284.0, "y = {y} would reach the river");
        }
        assert_eq!(s.commands.len(), 2 * 100 + 4);
        assert!(s.commands.iter().all(|c| c.tick.0 <= SCRIPT_TICKS));
        // Commands are stamped in ascending tick order with unique seqs per tick.
        let mut seen = std::collections::BTreeSet::new();
        for c in &s.commands {
            assert!(seen.insert((c.tick, c.seq)));
        }
    }

    #[test]
    fn generated_scenario_rejects_bad_counts() {
        assert!(generate_scenario(0).is_err());
        assert!(generate_scenario(2_001).is_err());
        assert!(generate_scenario(40_000).is_err());
        assert!(generate_scenario(21_600).is_ok());
        assert!(generate_scenario(21_800).is_err());
    }

    #[test]
    fn timer_report_sums_the_phase_one_stages() {
        let mut t = StageTimer::new();
        for _ in 0..3 {
            for stage in Stage::ALL {
                t.begin(stage);
                t.end(stage);
            }
        }
        let r = t.report(2_000, 10, 1);
        assert_eq!(r.ticks, 3);
        assert_eq!(r.stages.len(), Stage::COUNT);
        assert_eq!(r.stages[2].name, "Formation");
        assert_eq!(r.stages[7].name, "Collision");
        let sum: f64 = r.stages[2..=7].iter().map(|s| s.summary.mean_ms).sum();
        assert_eq!(r.phase1_stages_mean_ms, sum);
    }

    #[test]
    fn baseline_round_trips_through_json() {
        let mut b = Baseline {
            machine: "test".to_owned(),
            recorded: "2026-09-03".to_owned(),
            runs: BTreeMap::new(),
        };
        b.runs.insert(
            "2000".to_owned(),
            report(vec![stage("Collision", 7, 1.0)], 2.0),
        );
        let text = serde_json::to_string(&b).unwrap();
        let back: Baseline = serde_json::from_str(&text).unwrap();
        assert_eq!(back, b);
        assert!(back.run_for(2000).is_some());
        assert!(back.run_for(10_000).is_none());
    }
}
