//! `il_cli autoresolve --record-replay` and `il_cli replay --verify` end to
//! end (T2-101, REQ-SAVE-005): a capped AI-versus-AI skirmish records a
//! replay that verifies on one and eight threads; a corrupted hash names its
//! tick and exits 1; a different content (an extra mod) is refused unless
//! forced; the header prints without `--verify`.

use std::path::{Path, PathBuf};

use il_cli::autoresolve::{AiPlayers, AutoresolveOptions, autoresolve};
use il_cli::replay::{ReplayOptions, replay};
use il_core::Tick;
use il_save::SaveKind;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = root().join("target/il_cli_replay_test");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}_{}.ilrp", std::process::id()))
}

fn record(path: &Path, ticks: u32) {
    let mut out = Vec::new();
    let opts = AutoresolveOptions {
        scenario: root().join("tests/scenarios/ai_skirmish_300.json5"),
        max_ticks: Some(ticks),
        threads: 1,
        json: None,
        content_root: root().join("game"),
        mods: Vec::new(),
        ai: AiPlayers::All,
        record_replay: Some(path.to_path_buf()),
    };
    let (_, ended) = autoresolve(&opts, &mut out).unwrap();
    assert!(!ended, "the cap cuts the skirmish short");
}

fn options(file: &Path, verify: bool) -> ReplayOptions {
    ReplayOptions {
        file: file.to_path_buf(),
        verify,
        threads: 1,
        content_root: root().join("game"),
        mods: Vec::new(),
        force: false,
    }
}

#[test]
fn a_recorded_autoresolve_verifies_and_prints_its_header() {
    let file = scratch("ok");
    record(&file, 300);
    let mut out = Vec::new();
    let outcome = replay(&options(&file, false), &mut out).unwrap();
    assert_eq!(outcome.header.kind, SaveKind::Replay);
    assert_eq!(outcome.header.tick, Some(300));
    assert!(outcome.report.is_none());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\"kind\": \"replay\""), "{text}");
    assert!(text.contains("ai_skirmish_300"), "{text}");

    let mut out = Vec::new();
    let outcome = replay(&options(&file, true), &mut out).unwrap();
    let report = outcome.report.unwrap();
    assert!(report.ok(), "{report:?}");
    assert_eq!(report.ticks, 300);
    assert_eq!(outcome.exit_code(), 0);
    let text = String::from_utf8(out).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("verified 300 ticks"));
    // T3-070: the recording's last hash follows, sixteen hex digits.
    let last = lines.next().expect("a final hash line");
    assert!(last.starts_with("final hash "), "{last}");
    assert_eq!(last.len(), "final hash ".len() + 16, "{last}");

    let eight = ReplayOptions {
        threads: 8,
        ..options(&file, true)
    };
    assert!(
        replay(&eight, &mut Vec::new())
            .unwrap()
            .report
            .unwrap()
            .ok()
    );
    let _ = std::fs::remove_file(file);
}

#[test]
fn a_corrupted_hash_names_its_tick_and_exits_one() {
    let file = scratch("bad");
    record(&file, 200);
    let (header, mut rec) = il_cli::replay::read_replay(&file).unwrap();
    rec.hashes[99] = il_core::StateHash(rec.hashes[99].0 ^ 0x55);
    il_save::write(&file, &header, &rec.to_bytes()).unwrap();
    let mut out = Vec::new();
    let outcome = replay(&options(&file, true), &mut out).unwrap();
    let d = outcome.report.unwrap().divergence.expect("divergence");
    assert_eq!(d.tick, Tick(100));
    assert_eq!(outcome.exit_code(), 1);
    assert!(
        String::from_utf8(out)
            .unwrap()
            .starts_with("divergence at tick 100:")
    );
    let _ = std::fs::remove_file(file);
}

#[test]
fn a_different_content_is_refused_unless_forced() {
    let file = scratch("mods");
    record(&file, 100);
    let with_mod = ReplayOptions {
        mods: vec![root().join("tests/mods/speed_override")],
        ..options(&file, true)
    };
    let err = replay(&with_mod, &mut Vec::new()).unwrap_err();
    assert!(err.to_string().contains("--force"), "{err:#}");
    let forced = ReplayOptions {
        force: true,
        ..with_mod
    };
    // The override changes hastati speed, so the re-simulation diverges
    // somewhere once they move; either way it runs.
    let outcome = replay(&forced, &mut Vec::new()).unwrap();
    assert!(outcome.report.is_some());
    let _ = std::fs::remove_file(file);
}

#[test]
fn a_file_that_is_not_a_replay_is_refused() {
    let file = scratch("notreplay");
    std::fs::write(&file, b"not a save").unwrap();
    assert!(replay(&options(&file, false), &mut Vec::new()).is_err());
    let _ = std::fs::remove_file(file);
}
