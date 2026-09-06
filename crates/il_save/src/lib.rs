//! Iron Legion save container, replay and battle-save bodies (T2-101; TDD
//! §14, SAD §7, REQ-SAVE-002, REQ-SAVE-005, REQ-SAVE-006, REQ-TECH-006).
//!
//! A file is `ILSV`, a little-endian `u32` header length, the JSON
//! [`SaveHeader`] and the postcard body. The header names the engine and
//! schema versions, the mods and their versions, the content and mod-list
//! hashes, the creation time and the body's compression (`none` in Phase 2;
//! zstd may join later without a format break, plan decision 16).
//!
//! A [`Replay`] is the `BattleSetup`, the commands the session fed the sim
//! (local, scripted and the `--ai` transfers), the commands the engine AI
//! produced (kept apart, plan I1: playback and verification run with the AI
//! on and feed only the fed commands, so the recorded hashes match exactly)
//! and one state hash per tick. A [`BattleSave`] is a snapshot plus the
//! replay so far and the scripted commands still to come, so a loaded battle
//! continues with the same hash sequence and its replay covers the whole
//! battle (REQ-SAVE-006).

use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use il_core::{PlayerId, StateHash, Tick};
use il_data::Registries;
use il_sim_battle::{BattleSetup, BattleWorld, Command, ScriptedCommands};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The container's magic bytes.
pub const MAGIC: &[u8; 4] = b"ILSV";
/// Schema version of the [`Replay`] body.
pub const REPLAY_VERSION: u32 = 1;
/// The engine version written into headers.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// File extensions.
pub const REPLAY_EXTENSION: &str = "ilrp";
pub const SAVE_EXTENSION: &str = "ilsv";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SaveKind {
    Battle,
    Replay,
    /// Phase 4.
    Campaign,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    #[default]
    None,
}

/// The JSON header (REQ-SAVE-002).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaveHeader {
    pub engine_version: String,
    /// `SNAPSHOT_VERSION` for battle bodies, [`REPLAY_VERSION`] for replays.
    pub schema_version: u32,
    pub kind: SaveKind,
    /// `(id, version)` in load order.
    pub mods: Vec<(String, String)>,
    pub content_registry_hash: u64,
    pub mod_list_hash: u64,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created: String,
    pub turn: Option<u32>,
    pub tick: Option<u32>,
    pub summary: String,
    #[serde(default)]
    pub compression: Compression,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SaveFile {
    pub header: SaveHeader,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("not an Iron Legion save (bad magic)")]
    BadMagic,
    #[error("truncated save file")]
    Truncated,
    #[error("bad header: {0}")]
    BadHeader(String),
    #[error("unsupported compression {0:?}")]
    UnsupportedCompression(String),
    #[error("body does not decode: {0}")]
    Decode(String),
    #[error("the replay's setup is not valid for this content: {0}")]
    Setup(String),
    #[error("schema version {found} is not {expected}")]
    SchemaVersion { found: u32, expected: u32 },
    #[error("this save has kind {found:?}, not {expected:?}")]
    Kind { found: SaveKind, expected: SaveKind },
}

/// One version step of a body (REQ-SAVE-003); no step exists yet.
#[derive(Debug, Error)]
#[error("no migration from schema version {from}")]
pub struct MigrateError {
    pub from: u32,
}

pub trait Migrate {
    fn migrate(from: u32, body: Vec<u8>) -> Result<Vec<u8>, MigrateError>;
}

/// The container bytes for a header and body.
pub fn encode(header: &SaveHeader, body: &[u8]) -> Vec<u8> {
    let json = serde_json::to_vec(header).expect("the header is plain data");
    let mut out = Vec::with_capacity(8 + json.len() + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(body);
    out
}

/// The inverse of [`encode`].
pub fn decode(bytes: &[u8]) -> Result<SaveFile, SaveError> {
    let (header, rest) = decode_header(bytes)?;
    Ok(SaveFile {
        header,
        body: rest.to_vec(),
    })
}

fn decode_header(bytes: &[u8]) -> Result<(SaveHeader, &[u8]), SaveError> {
    if bytes.len() < 8 {
        return Err(if bytes.starts_with(&MAGIC[..bytes.len().min(4)]) {
            SaveError::Truncated
        } else {
            SaveError::BadMagic
        });
    }
    if &bytes[..4] != MAGIC {
        return Err(SaveError::BadMagic);
    }
    let len = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let rest = &bytes[8..];
    if rest.len() < len {
        return Err(SaveError::Truncated);
    }
    let header: SaveHeader =
        serde_json::from_slice(&rest[..len]).map_err(|e| SaveError::BadHeader(e.to_string()))?;
    Ok((header, &rest[len..]))
}

/// Writes `header` and `body` to `path`, creating the parent directories.
pub fn write(path: &Path, header: &SaveHeader, body: &[u8]) -> Result<(), SaveError> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::File::create(path)?;
    f.write_all(&encode(header, body))?;
    f.flush()?;
    Ok(())
}

/// Reads only the header (the load screen lists files with it).
pub fn read_header(path: &Path) -> Result<SaveHeader, SaveError> {
    let mut f = std::fs::File::open(path)?;
    let mut prefix = [0u8; 8];
    f.read_exact(&mut prefix)
        .map_err(|_| SaveError::Truncated)?;
    if &prefix[..4] != MAGIC {
        return Err(SaveError::BadMagic);
    }
    let len = u32::from_le_bytes([prefix[4], prefix[5], prefix[6], prefix[7]]) as usize;
    let mut json = vec![0u8; len];
    f.read_exact(&mut json).map_err(|_| SaveError::Truncated)?;
    serde_json::from_slice(&json).map_err(|e| SaveError::BadHeader(e.to_string()))
}

pub fn read(path: &Path) -> Result<SaveFile, SaveError> {
    decode(&std::fs::read(path)?)
}

/// A header for the loaded content: mods, hashes and the time of writing.
pub fn header_for(
    regs: &Registries,
    kind: SaveKind,
    schema_version: u32,
    tick: Option<u32>,
    summary: String,
) -> SaveHeader {
    SaveHeader {
        engine_version: ENGINE_VERSION.to_string(),
        schema_version,
        kind,
        mods: regs
            .mods
            .iter()
            .map(|m| (m.id.clone(), m.version.clone()))
            .collect(),
        content_registry_hash: regs.content_registry_hash,
        mod_list_hash: regs.mod_list_hash,
        created: utc_timestamp(SystemTime::now()),
        turn: None,
        tick,
        summary,
        compression: Compression::None,
    }
}

/// Whether the loaded content is the content that wrote `header` (the
/// verify policy, plan decision 19).
pub fn content_matches(header: &SaveHeader, regs: &Registries) -> bool {
    header.content_registry_hash == regs.content_registry_hash
}

/// `(year, month, day)` of a day count since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`), so timestamps need no calendar crate.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

fn civil(now: SystemTime) -> (i64, u32, u32, u32, u32, u32) {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// `YYYY-MM-DDTHH:MM:SSZ` for the header.
pub fn utc_timestamp(now: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = civil(now);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// `YYYYMMDD-HHMMSS` for file names.
pub fn file_stamp(now: SystemTime) -> String {
    let (y, mo, d, h, mi, s) = civil(now);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}")
}

/// A recorded battle (REQ-SAVE-005).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Replay {
    pub setup: BattleSetup,
    /// What the session fed the sim, in tick order.
    pub commands: Vec<Command>,
    /// What the engine AI produced (`StepOutput.ai_commands`), for viewers
    /// and peers; never fed back while the AI runs.
    pub ai_commands: Vec<Command>,
    /// One per completed tick, tick 1 first.
    pub hashes: Vec<StateHash>,
    /// Seek points (Phase 3); empty until then.
    pub checkpoints: Vec<(Tick, Vec<u8>)>,
    /// The tick the battle ended at, if it did.
    pub ended_tick: Option<u32>,
}

impl Replay {
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("replay types are postcard-serialisable")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SaveError> {
        postcard::from_bytes(bytes).map_err(|e| SaveError::Decode(e.to_string()))
    }

    /// Completed ticks recorded.
    pub fn ticks(&self) -> u32 {
        self.hashes.len() as u32
    }
}

/// A battle in progress (REQ-SAVE-001 quick save, REQ-SAVE-006).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BattleSave {
    /// The postcard `Snapshot`.
    pub snapshot: Vec<u8>,
    /// The replay up to the snapshot's tick.
    pub replay: Replay,
    /// Scripted commands not yet fed.
    pub script: Vec<Command>,
    pub local_player: PlayerId,
    /// The scenario file's stem, for the replay's file name.
    pub scenario_stem: String,
}

impl BattleSave {
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).expect("save types are postcard-serialisable")
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SaveError> {
        postcard::from_bytes(bytes).map_err(|e| SaveError::Decode(e.to_string()))
    }
}

/// Where a re-simulation first disagreed with the recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divergence {
    pub tick: Tick,
    pub expected: StateHash,
    pub got: StateHash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// Ticks re-simulated.
    pub ticks: u32,
    pub divergence: Option<Divergence>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.divergence.is_none()
    }
}

/// Re-simulates `replay` with the AI on, feeding its fed commands, and
/// compares every hash (`il_cli replay --verify`, plan decision 19).
pub fn verify(
    replay: &Replay,
    regs: Arc<Registries>,
    threads: usize,
) -> Result<VerifyReport, SaveError> {
    let mut world =
        BattleWorld::new(&replay.setup, regs).map_err(|e| SaveError::Setup(e.to_string()))?;
    world.set_threads(threads.max(1));
    let mut script = ScriptedCommands::new(replay.commands.clone());
    let mut ticks = 0;
    for expected in &replay.hashes {
        let next = world.tick().next();
        let out = world.step(&script.take_for(next));
        ticks += 1;
        if out.hash != *expected {
            return Ok(VerifyReport {
                ticks,
                divergence: Some(Divergence {
                    tick: world.tick(),
                    expected: *expected,
                    got: out.hash,
                }),
            });
        }
    }
    Ok(VerifyReport {
        ticks,
        divergence: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn header() -> SaveHeader {
        SaveHeader {
            engine_version: "0.1.0".into(),
            schema_version: 9,
            kind: SaveKind::Replay,
            mods: vec![("rome".into(), "0.1.0".into())],
            content_registry_hash: 0xdead_beef,
            mod_list_hash: 7,
            created: "2026-09-06T12:00:00Z".into(),
            turn: None,
            tick: Some(100),
            summary: "a test".into(),
            compression: Compression::None,
        }
    }

    #[test]
    fn container_round_trips_and_rejects_bad_files() {
        let body = b"hello body".to_vec();
        let bytes = encode(&header(), &body);
        assert_eq!(&bytes[..4], MAGIC);
        let file = decode(&bytes).unwrap();
        assert_eq!(file.header, header());
        assert_eq!(file.body, body);
        assert!(matches!(
            decode(b"XXXX....").unwrap_err(),
            SaveError::BadMagic
        ));
        assert!(matches!(
            decode(&bytes[..3]).unwrap_err(),
            SaveError::Truncated
        ));
        assert!(matches!(
            decode(&bytes[..20]).unwrap_err(),
            SaveError::Truncated
        ));
        let mut bad = bytes.clone();
        bad[8] = b'!';
        assert!(matches!(decode(&bad).unwrap_err(), SaveError::BadHeader(_)));
        // The header's JSON survives an unknown compression name as an error.
        let json = serde_json::to_string(&header())
            .unwrap()
            .replace("\"none\"", "\"zstd\"");
        assert!(serde_json::from_str::<SaveHeader>(&json).is_err());
    }

    #[test]
    fn files_write_and_read_back_with_header_only_reads() {
        let dir = std::env::temp_dir().join(format!("il_save_{}", std::process::id()));
        let path = dir.join("nested").join("a.ilrp");
        write(&path, &header(), b"body").unwrap();
        assert_eq!(read_header(&path).unwrap(), header());
        let file = read(&path).unwrap();
        assert_eq!(file.body, b"body");
        std::fs::write(&path, b"IL").unwrap();
        assert!(matches!(
            read_header(&path).unwrap_err(),
            SaveError::Truncated
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timestamps_are_utc_civil_dates() {
        let at = |secs: u64| UNIX_EPOCH + Duration::from_secs(secs);
        assert_eq!(utc_timestamp(at(0)), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(at(951_782_400)), "2000-02-29T00:00:00Z");
        assert_eq!(utc_timestamp(at(1_700_000_000)), "2023-11-14T22:13:20Z");
        assert_eq!(utc_timestamp(at(1_788_912_000)), "2026-09-09T00:00:00Z");
        assert_eq!(file_stamp(at(1_700_000_000)), "20231114-221320");
    }

    #[test]
    fn replay_and_save_bodies_round_trip() {
        let setup: BattleSetup =
            serde_json::from_str(r#"{ "map_id": "rome:test_field", "seed": 1, "sides": [] }"#)
                .unwrap();
        let replay = Replay {
            setup: setup.clone(),
            commands: vec![Command {
                tick: Tick(1),
                player: PlayerId(0),
                seq: 0,
                kind: il_sim_battle::CommandKind::Pause,
            }],
            ai_commands: vec![],
            hashes: vec![StateHash(1), StateHash(2)],
            checkpoints: vec![],
            ended_tick: Some(2),
        };
        assert_eq!(Replay::from_bytes(&replay.to_bytes()).unwrap(), replay);
        assert_eq!(replay.ticks(), 2);
        let save = BattleSave {
            snapshot: vec![1, 2, 3],
            replay,
            script: vec![],
            local_player: PlayerId(0),
            scenario_stem: "x".into(),
        };
        assert_eq!(BattleSave::from_bytes(&save.to_bytes()).unwrap(), save);
        assert!(matches!(
            Replay::from_bytes(&[0xff, 0xff, 0xff]).unwrap_err(),
            SaveError::Decode(_)
        ));
    }
}
