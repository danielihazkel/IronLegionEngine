//! `il_cli bands`: the scenario outcome harness (T2-110; REQ-TEST-004,
//! Simulation Spec §15.3, TDD §17).
//!
//! A band file is an ordinary scenario (a `BattleSetup` plus `commands`)
//! with a top-level `bands` object: a seed count and base, a tick limit and
//! a list of assertions. Every seed runs headless on one thread; each
//! assertion is a per-seed boolean and holds when it is true in at least
//! `min_fraction` and at most `max_fraction` of the seeds. Assertions with
//! `active: false` are evaluated and printed as `skip` but never fail: the
//! rout clauses of §15.3 wait for morale (T2-041).
//!
//! The harness reads the sim only through `BattleView`, so it is a plain
//! statistics tool: float arithmetic here never touches the simulation.
#![allow(
    clippy::float_arithmetic,
    reason = "harness statistics over finished battles, never sim state"
)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, anyhow, bail};
use il_data::Registries;
use il_data::json5::{FileId, parse_json5};
use il_sim_battle::components::{MoraleState, SoldierState};
use il_sim_battle::{BattleWorld, Scenario};
use serde::{Deserialize, Serialize};

/// Options of `il_cli bands`.
#[derive(Clone, Debug)]
pub struct BandOptions {
    /// Folder of band files (`*.json5`), or one file.
    pub dir: PathBuf,
    /// Override every file's seed count.
    pub seeds: Option<u32>,
    /// Cap every file's tick limit (smoke runs).
    pub max_ticks: Option<u32>,
    /// Seeds run in parallel on this many OS threads (each world single-threaded).
    pub jobs: usize,
    /// Write the report as JSON.
    pub json: Option<PathBuf>,
    /// Mod root holding `mod.json5` and `content/`.
    pub content_root: PathBuf,
    /// Extra mod folders loaded after the game.
    pub mods: Vec<PathBuf>,
}

impl BandOptions {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            seeds: None,
            max_ticks: None,
            jobs: 1,
            json: None,
            content_root: PathBuf::from("game"),
            mods: Vec::new(),
        }
    }
}

/// The `bands` block of a band file.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bands {
    /// Seeds per run; the scenario's `seed` is replaced by `seed_base + i`.
    pub seeds: u32,
    pub seed_base: u64,
    /// Every seed stops here at the latest; it also stops as soon as any
    /// side has no living soldiers.
    pub tick_limit: u32,
    /// Extra mod folders (relative to the band file) loaded after the game
    /// and the `--mod` folders for this file only (T2-032: a rules
    /// override such as `projectile_cap: 0`).
    #[serde(default)]
    pub mods: Vec<PathBuf>,
    pub assertions: Vec<Assertion>,
}

fn d_one() -> f32 {
    1.0
}
fn d_true() -> bool {
    true
}

/// One band clause. `min_fraction`/`max_fraction` are the seed fractions
/// in which the per-seed boolean must hold.
#[derive(Clone, Debug, Deserialize)]
pub struct Assertion {
    pub name: String,
    #[serde(flatten)]
    pub kind: AssertionKind,
    #[serde(default = "d_one")]
    pub min_fraction: f32,
    #[serde(default = "d_one")]
    pub max_fraction: f32,
    /// `false` until the system the clause needs exists (printed as `skip`).
    #[serde(default = "d_true")]
    pub active: bool,
}

/// Per-seed booleans. Fractions are of the side's initial soldier count.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssertionKind {
    /// `side` wins: every other side is annihilated or ends with a strictly
    /// lower surviving fraction (Phase 2 plan decision 2; morale-free).
    Winner { side: u8 },
    /// `side` has lost between `min_lost` and `max_lost` of its soldiers at
    /// the end, or `within_ticks_of_contact` ticks after `contact_regiment`
    /// (default: the side's earliest contact) first fought.
    Casualties {
        side: u8,
        #[serde(default)]
        min_lost: f32,
        #[serde(default = "d_one")]
        max_lost: f32,
        #[serde(default)]
        within_ticks_of_contact: Option<u32>,
        #[serde(default)]
        contact_regiment: Option<u32>,
    },
    /// A regiment of `side` routs while the side has lost less than
    /// `loss_fraction` (T2-041).
    RoutedBeforeLoss { side: u8, loss_fraction: f32 },
    /// A regiment of `side` routs within `within_ticks` after `after`:
    /// `"contact"` (first fight of `contact_regiment`, default the side's
    /// earliest) or `"tick:N"` (T2-041).
    RoutWithin {
        side: u8,
        after: String,
        #[serde(default)]
        contact_regiment: Option<u32>,
        within_ticks: u32,
    },
    /// Cross-file (T2-032): the mean soldiers `side` lost over this file's
    /// seeds is within `tolerance` (a fraction) of the mean in the band
    /// file `reference` (its stem) of the same run. Evaluated once every
    /// file has run; `min_fraction`/`max_fraction` are ignored.
    MeanLossMatches {
        side: u8,
        reference: String,
        tolerance: f32,
    },
}

/// What one seed produced.
#[derive(Clone, Debug, Serialize)]
pub struct SeedOutcome {
    pub seed: u64,
    pub end_tick: u32,
    /// Commands the sim rejected (a band file must have none).
    pub rejected: u32,
    /// Soldiers per side at the start.
    pub initial: Vec<u32>,
    /// Soldiers per side at the end.
    pub survivors: Vec<u32>,
    /// First tick a soldier of each regiment was `Fighting`.
    pub first_contact: BTreeMap<u32, u32>,
    /// First tick each regiment was Routing.
    pub first_rout: BTreeMap<u32, u32>,
    pub hash: String,
    /// Soldiers per side after every completed tick (`counts[t - 1]`).
    #[serde(skip)]
    counts: Vec<Vec<u32>>,
    #[serde(skip)]
    regiment_side: BTreeMap<u32, u8>,
}

impl SeedOutcome {
    fn fraction(&self, side: u8, at_tick: Option<u32>) -> f32 {
        let s = usize::from(side);
        let initial = self.initial.get(s).copied().unwrap_or(0);
        if initial == 0 {
            return 0.0;
        }
        let count = match at_tick {
            Some(0) => initial,
            Some(t) => self
                .counts
                .get(t as usize - 1)
                .or_else(|| self.counts.last())
                .and_then(|c| c.get(s))
                .copied()
                .unwrap_or(0),
            None => self.survivors.get(s).copied().unwrap_or(0),
        };
        count as f32 / initial as f32
    }

    fn regiments_of(&self, side: u8) -> impl Iterator<Item = u32> + '_ {
        self.regiment_side
            .iter()
            .filter(move |(_, s)| **s == side)
            .map(|(r, _)| *r)
    }

    /// First contact of `regiment`, or the side's earliest contact.
    fn contact_tick(&self, side: u8, regiment: Option<u32>) -> Option<u32> {
        match regiment {
            Some(r) => self.first_contact.get(&r).copied(),
            None => self
                .regiments_of(side)
                .filter_map(|r| self.first_contact.get(&r))
                .min()
                .copied(),
        }
    }

    fn first_rout_of_side(&self, side: u8) -> Option<u32> {
        self.regiments_of(side)
            .filter_map(|r| self.first_rout.get(&r))
            .min()
            .copied()
    }

    /// The per-seed boolean of `kind`.
    pub fn holds(&self, kind: &AssertionKind) -> anyhow::Result<bool> {
        Ok(match kind {
            // Cross-file: settled by `run_bands`, never per seed.
            AssertionKind::MeanLossMatches { .. } => true,
            AssertionKind::Winner { side } => {
                let mine = self.fraction(*side, None);
                mine > 0.0
                    && (0..self.initial.len() as u8)
                        .filter(|s| s != side)
                        .all(|s| {
                            self.survivors[usize::from(s)] == 0 || self.fraction(s, None) < mine
                        })
            }
            AssertionKind::Casualties {
                side,
                min_lost,
                max_lost,
                within_ticks_of_contact,
                contact_regiment,
            } => {
                let at = match within_ticks_of_contact {
                    Some(within) => match self.contact_tick(*side, *contact_regiment) {
                        Some(c) => Some((c + within).min(self.end_tick)),
                        None => return Ok(false),
                    },
                    None => None,
                };
                let lost = 1.0 - self.fraction(*side, at);
                lost >= *min_lost && lost <= *max_lost
            }
            AssertionKind::RoutedBeforeLoss {
                side,
                loss_fraction,
            } => match self.first_rout_of_side(*side) {
                Some(t) => 1.0 - self.fraction(*side, Some(t)) < *loss_fraction,
                None => false,
            },
            AssertionKind::RoutWithin {
                side,
                after,
                contact_regiment,
                within_ticks,
            } => {
                let anchor = match parse_after(after)? {
                    After::Contact => match self.contact_tick(*side, *contact_regiment) {
                        Some(c) => c,
                        None => return Ok(false),
                    },
                    After::Tick(t) => t,
                };
                self.regiments_of(*side)
                    .filter_map(|r| self.first_rout.get(&r))
                    .any(|&t| t >= anchor && t <= anchor + within_ticks)
            }
        })
    }
}

enum After {
    Contact,
    Tick(u32),
}

fn parse_after(s: &str) -> anyhow::Result<After> {
    if s == "contact" {
        return Ok(After::Contact);
    }
    if let Some(n) = s.strip_prefix("tick:") {
        return n
            .parse()
            .map(After::Tick)
            .map_err(|_| anyhow!("bad `after` value {s:?}: expected \"contact\" or \"tick:N\""));
    }
    bail!("bad `after` value {s:?}: expected \"contact\" or \"tick:N\"")
}

/// How one assertion fared over the seeds.
#[derive(Clone, Debug, Serialize)]
pub struct AssertionResult {
    pub name: String,
    pub active: bool,
    pub held: u32,
    pub seeds: u32,
    pub min_fraction: f32,
    pub max_fraction: f32,
    /// `pass`, `FAIL`, `skip` or (cross-file, before settling) `pending`.
    pub status: String,
    /// Cross-file clauses print this instead of `held/seeds` (the measured
    /// difference) and `detail_need` instead of the seed fraction.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail_need: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct FileReport {
    pub file: String,
    pub seeds: u32,
    pub seed_base: u64,
    pub tick_limit: u32,
    pub outcomes: Vec<SeedOutcome>,
    pub assertions: Vec<AssertionResult>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BandReport {
    pub files: Vec<FileReport>,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    /// Rejected commands summed over every seed and file.
    pub rejected: u32,
}

/// Parses a band file into its scenario and `bands` block.
pub fn load_band_file(path: &Path) -> anyhow::Result<(Scenario, Bands)> {
    let (scenario, mut bands) = parse_band_file(path)?;
    // `mods` are written relative to the band file.
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    bands.mods = bands.mods.iter().map(|m| dir.join(m)).collect();
    Ok((scenario, bands))
}

fn parse_band_file(path: &Path) -> anyhow::Result<(Scenario, Bands)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading band file {}", path.display()))?;
    let value = parse_json5(&text, FileId(0))
        .map_err(|e| anyhow!("{e}"))
        .with_context(|| format!("parsing band file {}", path.display()))?;
    let json = value.to_json();
    let bands_value = json
        .get("bands")
        .cloned()
        .ok_or_else(|| anyhow!("{}: no `bands` block", path.display()))?;
    let scenario: Scenario =
        serde_json::from_value(json).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let bands: Bands = serde_json::from_value(bands_value)
        .map_err(|e| anyhow!("{}: bands: {e}", path.display()))?;
    if bands.seeds == 0 {
        bail!("{}: bands.seeds must be at least 1", path.display());
    }
    for a in &bands.assertions {
        if let AssertionKind::RoutWithin { after, .. } = &a.kind {
            parse_after(after).with_context(|| format!("{}: {}", path.display(), a.name))?;
        }
    }
    Ok((scenario, bands))
}

/// The band files of a folder (sorted), or the single file given.
pub fn band_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if dir.is_file() {
        return Ok(vec![dir.to_path_buf()]);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json5"))
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("no band files under {}", dir.display());
    }
    Ok(files)
}

fn side_counts(view: &il_sim_battle::BattleView<'_>, sides: usize) -> Vec<u32> {
    let mut counts = vec![0u32; sides];
    for r in view.regiments() {
        if let Some(c) = counts.get_mut(usize::from(r.side)) {
            *c += r.soldier_count;
        }
    }
    counts
}

/// Runs one seed of a band scenario to its tick limit or the first
/// annihilated side.
pub fn run_seed(
    scenario: &Scenario,
    seed: u64,
    tick_limit: u32,
    regs: Arc<Registries>,
) -> anyhow::Result<SeedOutcome> {
    let mut setup = scenario.setup.clone();
    setup.seed = seed;
    let mut world = BattleWorld::new(&setup, regs)?;
    world.set_threads(1);
    let mut script = scenario.script();
    let sides = world.view().sides().len();
    let regiment_side: BTreeMap<u32, u8> =
        world.view().regiments().map(|r| (r.id.0, r.side)).collect();
    let initial = side_counts(&world.view(), sides);
    let mut counts = Vec::with_capacity(tick_limit as usize);
    let mut first_contact = BTreeMap::new();
    let mut first_rout = BTreeMap::new();
    let mut rejected = 0u32;
    let mut hash = world.hash();
    while world.tick().0 < tick_limit {
        let commands = script.take_for(world.tick().next());
        let out = world.step(&commands);
        rejected += out.rejected.len() as u32;
        hash = out.hash;
        let view = world.view();
        let tick = view.tick().0;
        let now = side_counts(&view, sides);
        for r in view.regiments() {
            if r.morale_state == MoraleState::Routing {
                first_rout.entry(r.id.0).or_insert(tick);
            }
        }
        if first_contact.len() < regiment_side.len() {
            for s in view.soldiers_unordered() {
                if s.state == SoldierState::Fighting {
                    first_contact.entry(s.regiment.0).or_insert(tick);
                }
            }
        }
        let done = now.contains(&0);
        counts.push(now);
        if done {
            break;
        }
    }
    let survivors = counts.last().cloned().unwrap_or_else(|| initial.clone());
    Ok(SeedOutcome {
        seed,
        end_tick: world.tick().0,
        rejected,
        initial,
        survivors,
        first_contact,
        first_rout,
        hash: format!("{hash}"),
        counts,
        regiment_side,
    })
}

/// Runs every seed of one band file on `jobs` threads.
pub fn run_file(
    scenario: &Scenario,
    bands: &Bands,
    opts: &BandOptions,
    regs: &Arc<Registries>,
) -> anyhow::Result<Vec<SeedOutcome>> {
    let seeds = opts.seeds.unwrap_or(bands.seeds).max(1);
    let tick_limit = opts
        .max_ticks
        .map_or(bands.tick_limit, |m| m.min(bands.tick_limit));
    let jobs = opts.jobs.clamp(1, seeds as usize);
    let next = std::sync::atomic::AtomicU32::new(0);
    let results: Mutex<Vec<(u64, anyhow::Result<SeedOutcome>)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if i >= seeds {
                        break;
                    }
                    let seed = bands.seed_base + u64::from(i);
                    let r = run_seed(scenario, seed, tick_limit, regs.clone());
                    results.lock().expect("no poisoned seed").push((seed, r));
                }
            });
        }
    });
    let mut results = results.into_inner().expect("no poisoned seed");
    results.sort_by_key(|(seed, _)| *seed);
    results
        .into_iter()
        .map(|(seed, r)| r.with_context(|| format!("seed {seed}")))
        .collect()
}

/// Mean soldiers `side` lost per seed.
pub fn mean_lost(outcomes: &[SeedOutcome], side: u8) -> f64 {
    if outcomes.is_empty() {
        return 0.0;
    }
    let s = usize::from(side);
    outcomes
        .iter()
        .map(|o| {
            f64::from(o.initial.get(s).copied().unwrap_or(0))
                - f64::from(o.survivors.get(s).copied().unwrap_or(0))
        })
        .sum::<f64>()
        / outcomes.len() as f64
}

/// The `mean_loss_matches` verdict: `|a − b| ≤ tolerance × b` (a reference
/// mean of zero only matches a zero).
pub fn mean_loss_within(file_mean: f64, reference_mean: f64, tolerance: f32) -> bool {
    let tol = f64::from(tolerance);
    if reference_mean == 0.0 {
        file_mean == 0.0
    } else {
        (file_mean - reference_mean).abs() <= tol * reference_mean.abs()
    }
}

/// Evaluates the per-seed assertions of one file over its outcomes; the
/// cross-file `mean_loss_matches` clauses come back as `pending` and are
/// settled by [`run_bands`] once every file has run.
pub fn evaluate(bands: &Bands, outcomes: &[SeedOutcome]) -> anyhow::Result<Vec<AssertionResult>> {
    let seeds = outcomes.len() as u32;
    bands
        .assertions
        .iter()
        .map(|a| {
            if matches!(a.kind, AssertionKind::MeanLossMatches { .. }) {
                return Ok(AssertionResult {
                    name: a.name.clone(),
                    active: a.active,
                    held: 0,
                    seeds,
                    min_fraction: a.min_fraction,
                    max_fraction: a.max_fraction,
                    status: if a.active { "pending" } else { "skip" }.to_string(),
                    detail: String::new(),
                    detail_need: String::new(),
                });
            }
            let mut held = 0u32;
            for o in outcomes {
                if o.holds(&a.kind).with_context(|| a.name.clone())? {
                    held += 1;
                }
            }
            let fraction = if seeds == 0 {
                0.0
            } else {
                held as f32 / seeds as f32
            };
            const EPS: f32 = 1e-6;
            let ok = fraction + EPS >= a.min_fraction && fraction - EPS <= a.max_fraction;
            let status = if !a.active {
                "skip"
            } else if ok {
                "pass"
            } else {
                "FAIL"
            };
            Ok(AssertionResult {
                name: a.name.clone(),
                active: a.active,
                held,
                seeds,
                min_fraction: a.min_fraction,
                max_fraction: a.max_fraction,
                status: status.to_string(),
                detail: String::new(),
                detail_need: String::new(),
            })
        })
        .collect()
}

fn write_row(out: &mut dyn Write, file: &str, a: &AssertionResult) -> anyhow::Result<()> {
    let held = if a.detail.is_empty() {
        format!("{:>3}/{:<3}", a.held, a.seeds)
    } else {
        format!("{:>7}", a.detail)
    };
    writeln!(
        out,
        "{:<34} {:<40} {} {:<12} {}",
        file,
        a.name,
        held,
        need(a),
        a.status
    )?;
    Ok(())
}

fn need(a: &AssertionResult) -> String {
    if !a.detail.is_empty() {
        return a.detail_need.clone();
    }
    if a.max_fraction >= 1.0 {
        format!(">= {:.2}", a.min_fraction)
    } else {
        format!("{:.2}..{:.2}", a.min_fraction, a.max_fraction)
    }
}

/// Runs every band file under `opts.dir`, prints the table and returns the
/// report. The caller decides the exit code from `report.failed`.
pub fn run_bands(opts: &BandOptions, out: &mut dyn Write) -> anyhow::Result<BandReport> {
    let base_regs = crate::load_registries_with_mods(&opts.content_root, &opts.mods)?;
    let mut report = BandReport::default();
    let header = format!(
        "{:<34} {:<40} {:>7} {:<12} {}",
        "scenario", "assertion", "held", "need", "result"
    );
    writeln!(out, "{header}")?;
    // Cross-file clauses wait until every file has run: (file index,
    // assertion index, side, reference stem, tolerance).
    let mut deferred: Vec<(usize, usize, u8, String, f32)> = Vec::new();
    let mut deferred_bands: Vec<Bands> = Vec::new();
    for path in band_files(&opts.dir)? {
        let file = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (scenario, bands) = load_band_file(&path)?;
        let regs = if bands.mods.is_empty() {
            base_regs.clone()
        } else {
            for m in &bands.mods {
                if !m.join("mod.json5").is_file() {
                    bail!(
                        "{}: bands.mods entry {} has no mod.json5",
                        path.display(),
                        m.display()
                    );
                }
            }
            let mut mods = opts.mods.clone();
            mods.extend(bands.mods.iter().cloned());
            crate::load_registries_with_mods(&opts.content_root, &mods)
                .with_context(|| format!("{}: bands.mods", path.display()))?
        };
        let outcomes = run_file(&scenario, &bands, opts, &regs)?;
        let assertions = evaluate(&bands, &outcomes)?;
        let rejected: u32 = outcomes.iter().map(|o| o.rejected).sum();
        report.rejected += rejected;
        for (k, a) in assertions.iter().enumerate() {
            if let AssertionKind::MeanLossMatches {
                side,
                reference,
                tolerance,
            } = &bands.assertions[k].kind
                && a.status == "pending"
            {
                deferred.push((report.files.len(), k, *side, reference.clone(), *tolerance));
                continue;
            }
            match a.status.as_str() {
                "pass" => report.passed += 1,
                "FAIL" => report.failed += 1,
                _ => report.skipped += 1,
            }
            write_row(out, &file, a)?;
        }
        deferred_bands.push(bands.clone());
        let mean_end =
            outcomes.iter().map(|o| o.end_tick as f64).sum::<f64>() / outcomes.len().max(1) as f64;
        let survivors: Vec<String> = (0..outcomes.first().map_or(0, |o| o.initial.len()))
            .map(|s| {
                let mean = outcomes.iter().map(|o| o.survivors[s] as f64).sum::<f64>()
                    / outcomes.len().max(1) as f64;
                format!("side {s}: {mean:.1}/{}", outcomes[0].initial[s])
            })
            .collect();
        writeln!(
            out,
            "{:<34}   {} seeds, mean end tick {:.0}, mean survivors {}{}",
            "",
            outcomes.len(),
            mean_end,
            survivors.join(", "),
            if rejected > 0 {
                format!(", {rejected} rejected commands")
            } else {
                String::new()
            }
        )?;
        report.files.push(FileReport {
            file,
            seeds: outcomes.len() as u32,
            seed_base: bands.seed_base,
            tick_limit: opts
                .max_ticks
                .map_or(bands.tick_limit, |m| m.min(bands.tick_limit)),
            outcomes,
            assertions,
        });
    }
    let _ = deferred_bands;
    // Settle the cross-file clauses (T2-032).
    for (fi, k, side, reference, tolerance) in deferred {
        let stem = |name: &str| {
            Path::new(name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let reference_index = report
            .files
            .iter()
            .position(|f| stem(&f.file) == reference)
            .ok_or_else(|| {
                anyhow!(
                    "{}: mean_loss_matches names {reference}, which is not among the band files run",
                    report.files[fi].file
                )
            })?;
        let file_mean = mean_lost(&report.files[fi].outcomes, side);
        let reference_mean = mean_lost(&report.files[reference_index].outcomes, side);
        let ok = mean_loss_within(file_mean, reference_mean, tolerance);
        let diff = if reference_mean == 0.0 {
            0.0
        } else {
            (file_mean - reference_mean).abs() / reference_mean.abs() * 100.0
        };
        let file_name = report.files[fi].file.clone();
        let a = &mut report.files[fi].assertions[k];
        a.status = if ok { "pass" } else { "FAIL" }.to_string();
        a.detail = format!("{diff:.1}%");
        a.detail_need = format!(
            "<= {:.0}% of {:.1}",
            f64::from(tolerance) * 100.0,
            reference_mean
        );
        if ok {
            report.passed += 1;
        } else {
            report.failed += 1;
        }
        let a = a.clone();
        write_row(out, &file_name, &a)?;
        writeln!(
            out,
            "{:<34}   mean lost side {side}: {file_mean:.1} here, {reference_mean:.1} in {reference}",
            ""
        )?;
    }
    writeln!(
        out,
        "bands: {} pass, {} FAIL, {} skip{}",
        report.passed,
        report.failed,
        report.skipped,
        if report.rejected > 0 {
            format!(", {} rejected commands", report.rejected)
        } else {
            String::new()
        }
    )?;
    if let Some(path) = &opts.json {
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(initial: &[u32], survivors: &[u32]) -> SeedOutcome {
        SeedOutcome {
            seed: 1,
            end_tick: 100,
            rejected: 0,
            initial: initial.to_vec(),
            survivors: survivors.to_vec(),
            first_contact: BTreeMap::from([(0, 10), (1, 12)]),
            first_rout: BTreeMap::from([(1, 40)]),
            hash: String::new(),
            counts: (1..=100)
                .map(|t| {
                    initial
                        .iter()
                        .zip(survivors)
                        .map(|(&i, &s)| i - ((i - s) * t / 100))
                        .collect()
                })
                .collect(),
            regiment_side: BTreeMap::from([(0, 0), (1, 1)]),
        }
    }

    #[test]
    fn winner_needs_a_strictly_higher_surviving_fraction() {
        let o = outcome(&[100, 100], &[60, 40]);
        assert!(o.holds(&AssertionKind::Winner { side: 0 }).unwrap());
        assert!(!o.holds(&AssertionKind::Winner { side: 1 }).unwrap());
        let tie = outcome(&[100, 100], &[50, 50]);
        assert!(!tie.holds(&AssertionKind::Winner { side: 0 }).unwrap());
        let wiped = outcome(&[100, 100], &[1, 0]);
        assert!(wiped.holds(&AssertionKind::Winner { side: 0 }).unwrap());
    }

    #[test]
    fn casualties_can_be_measured_after_contact() {
        let o = outcome(&[100, 100], &[60, 40]);
        let at_end = AssertionKind::Casualties {
            side: 1,
            min_lost: 0.5,
            max_lost: 1.0,
            within_ticks_of_contact: None,
            contact_regiment: None,
        };
        assert!(o.holds(&at_end).unwrap());
        // Contact of regiment 1 at tick 12, plus 38 = tick 50: half the losses.
        let early = AssertionKind::Casualties {
            side: 1,
            min_lost: 0.5,
            max_lost: 1.0,
            within_ticks_of_contact: Some(38),
            contact_regiment: Some(1),
        };
        assert!(!o.holds(&early).unwrap());
    }

    #[test]
    fn rout_clauses_read_the_first_rout_tick() {
        let o = outcome(&[100, 100], &[60, 40]);
        // Side 1 routs at tick 40 having lost 24 %.
        assert!(
            o.holds(&AssertionKind::RoutedBeforeLoss {
                side: 1,
                loss_fraction: 0.5
            })
            .unwrap()
        );
        assert!(
            !o.holds(&AssertionKind::RoutedBeforeLoss {
                side: 1,
                loss_fraction: 0.1
            })
            .unwrap()
        );
        assert!(
            o.holds(&AssertionKind::RoutWithin {
                side: 1,
                after: "contact".into(),
                contact_regiment: Some(0),
                within_ticks: 30
            })
            .unwrap()
        );
        assert!(
            !o.holds(&AssertionKind::RoutWithin {
                side: 1,
                after: "tick:50".into(),
                contact_regiment: None,
                within_ticks: 30
            })
            .unwrap()
        );
        assert!(parse_after("tick:x").is_err());
    }

    #[test]
    fn evaluate_reports_fractions_and_skips_inactive_clauses() {
        let bands = Bands {
            seeds: 2,
            seed_base: 0,
            tick_limit: 100,
            mods: Vec::new(),
            assertions: vec![
                Assertion {
                    name: "win".into(),
                    kind: AssertionKind::Winner { side: 0 },
                    min_fraction: 0.5,
                    max_fraction: 1.0,
                    active: true,
                },
                Assertion {
                    name: "rout".into(),
                    kind: AssertionKind::RoutedBeforeLoss {
                        side: 0,
                        loss_fraction: 0.5,
                    },
                    min_fraction: 1.0,
                    max_fraction: 1.0,
                    active: false,
                },
            ],
        };
        let outcomes = [
            outcome(&[100, 100], &[60, 40]),
            outcome(&[100, 100], &[40, 60]),
        ];
        let r = evaluate(&bands, &outcomes).unwrap();
        assert_eq!((r[0].held, r[0].status.as_str()), (1, "pass"));
        assert_eq!((r[1].held, r[1].status.as_str()), (0, "skip"));
    }

    #[test]
    fn mean_loss_matches_compares_means_within_a_tolerance() {
        let a = [
            outcome(&[100, 100], &[60, 40]),
            outcome(&[100, 100], &[60, 50]),
        ];
        let b = [outcome(&[100, 100], &[60, 45])];
        assert_eq!(mean_lost(&a, 1), 55.0);
        assert_eq!(mean_lost(&b, 1), 55.0);
        assert!(mean_loss_within(55.0, 55.0, 0.0));
        assert!(mean_loss_within(60.0, 55.0, 0.10));
        assert!(!mean_loss_within(61.0, 55.0, 0.10));
        assert!(mean_loss_within(0.0, 0.0, 0.10));
        assert!(!mean_loss_within(1.0, 0.0, 0.10));
        let bands = Bands {
            seeds: 1,
            seed_base: 0,
            tick_limit: 100,
            mods: Vec::new(),
            assertions: vec![Assertion {
                name: "same".into(),
                kind: AssertionKind::MeanLossMatches {
                    side: 1,
                    reference: "other".into(),
                    tolerance: 0.1,
                },
                min_fraction: 1.0,
                max_fraction: 1.0,
                active: true,
            }],
        };
        let r = evaluate(&bands, &a).unwrap();
        assert_eq!(r[0].status, "pending");
    }

    #[test]
    fn band_block_parses_from_json5() {
        let text = r#"{ map_id: "rome:test_field", seed: 0, sides: [], commands: [],
          bands: { seeds: 3, seed_base: 10, tick_limit: 50, mods: ["../../mods/projectile_cap_zero"], assertions: [
            { name: "a", kind: "winner", side: 0, min_fraction: 0.9 },
            { name: "b", kind: "rout_within", side: 1, after: "contact", within_ticks: 600, active: false },
            { name: "c", kind: "mean_loss_matches", side: 1, reference: "volley_velites_vs_hastati", tolerance: 0.10 },
          ] } }"#;
        let json = parse_json5(text, FileId(0)).unwrap().to_json();
        let bands: Bands = serde_json::from_value(json["bands"].clone()).unwrap();
        assert_eq!(bands.assertions.len(), 3);
        assert!(!bands.assertions[1].active);
        assert_eq!(
            bands.mods,
            vec![PathBuf::from("../../mods/projectile_cap_zero")]
        );
        assert!(matches!(
            &bands.assertions[2].kind,
            AssertionKind::MeanLossMatches { side: 1, reference, .. } if reference == "volley_velites_vs_hastati"
        ));
        assert!(matches!(
            bands.assertions[0].kind,
            AssertionKind::Winner { side: 0 }
        ));
        // The scenario half ignores the `bands` key.
        let scenario: Scenario = serde_json::from_value(json).unwrap();
        assert_eq!(scenario.setup.seed, 0);
    }
}
