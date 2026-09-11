//! `il_cli replay <file> [--verify]` (T2-101, REQ-SAVE-005, TDD §14, §17):
//! prints a replay's header, or re-simulates it with the loaded content and
//! reports the first tick whose hash differs from the recording. The
//! content must be the content that wrote the file (plan decision 19):
//! a different content hash refuses unless `--force`.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use il_save::{Replay, SaveHeader, SaveKind, VerifyReport};

pub struct ReplayOptions {
    pub file: PathBuf,
    /// Re-simulate and compare hashes; without it only the header prints.
    pub verify: bool,
    pub threads: usize,
    pub content_root: PathBuf,
    pub mods: Vec<PathBuf>,
    /// Verify even when the content hash differs.
    pub force: bool,
}

/// What `replay` did: the header, and the verification when asked.
#[derive(Debug)]
pub struct ReplayOutcome {
    pub header: SaveHeader,
    pub report: Option<VerifyReport>,
}

impl ReplayOutcome {
    /// The process exit code: 1 on a divergence.
    pub fn exit_code(&self) -> i32 {
        match &self.report {
            Some(r) if !r.ok() => 1,
            _ => 0,
        }
    }
}

/// Reads a replay file's header and body.
pub fn read_replay(file: &std::path::Path) -> anyhow::Result<(SaveHeader, Replay)> {
    let save = il_save::read(file).with_context(|| format!("reading {}", file.display()))?;
    if save.header.kind != SaveKind::Replay {
        return Err(anyhow!(
            "{} is a {:?}, not a replay",
            file.display(),
            save.header.kind
        ));
    }
    if save.header.schema_version != il_save::REPLAY_VERSION {
        return Err(anyhow!(
            "replay schema version {} (this build reads {})",
            save.header.schema_version,
            il_save::REPLAY_VERSION
        ));
    }
    let replay = Replay::from_bytes(&save.body)?;
    Ok((save.header, replay))
}

pub fn replay(opts: &ReplayOptions, out: &mut dyn Write) -> anyhow::Result<ReplayOutcome> {
    let (header, replay) = read_replay(&opts.file)?;
    if !opts.verify {
        writeln!(out, "{}", serde_json::to_string_pretty(&header)?)?;
        return Ok(ReplayOutcome {
            header,
            report: None,
        });
    }
    let regs = crate::load_registries_with_mods(&opts.content_root, &opts.mods)?;
    if !il_save::content_matches(&header, &regs) {
        let mods: Vec<String> = header
            .mods
            .iter()
            .map(|(id, v)| format!("{id} {v}"))
            .collect();
        if opts.force {
            eprintln!(
                "replay: the loaded content differs from the content that wrote the file ({}); verifying anyway (--force)",
                mods.join(", ")
            );
        } else {
            return Err(anyhow!(
                "the loaded content differs from the content that wrote the replay ({}); pass --force to verify anyway",
                mods.join(", ")
            ));
        }
    }
    let report = il_save::verify(&replay, regs, opts.threads)?;
    match report.divergence {
        None => {
            writeln!(out, "verified {} ticks", report.ticks)?;
            // T3-070: the recording's last hash, for a comparison with a
            // fresh `il_cli run` to the same tick (the nightly's 20k line).
            if let Some(last) = replay.hashes.last() {
                writeln!(out, "final hash {last}")?;
            }
        }
        Some(d) => writeln!(
            out,
            "divergence at tick {}: expected {} got {}",
            d.tick.0, d.expected, d.got
        )?,
    }
    Ok(ReplayOutcome {
        header,
        report: Some(report),
    })
}
