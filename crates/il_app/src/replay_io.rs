//! Replay and battle-save files for the app (T2-101, REQ-SAVE-005,
//! REQ-SAVE-006): where they go, what their headers say, and how a session
//! is rebuilt from one. The container itself is `il_save`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Context, anyhow};
use il_data::Registries;
use il_save::{BattleSave, Replay, SaveKind};
use il_sim_battle::{BattleSetup, SNAPSHOT_VERSION};

use crate::session::BattleSession;

/// The quick save's file name under the saves directory.
pub const QUICK_SAVE: &str = "quick.ilsv";

/// `<dir>/<stem>-<YYYYMMDD-HHMMSS>.ilrp`.
pub fn replay_path(dir: &Path, stem: &str, now: SystemTime) -> PathBuf {
    let stem = stem
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    dir.join(format!(
        "{stem}-{}.{}",
        il_save::file_stamp(now),
        il_save::REPLAY_EXTENSION
    ))
}

/// `<stem>: <sides> sides, <soldiers> soldiers` for the header.
pub fn summary(stem: &str, setup: &BattleSetup) -> String {
    let soldiers: u32 = setup
        .sides
        .iter()
        .flat_map(|s| s.regiments.iter().map(|r| u32::from(r.count)))
        .sum();
    format!("{stem}: {} sides, {soldiers} soldiers", setup.sides.len())
}

/// Writes a replay under `dir`; returns the file written.
pub fn write_replay(
    dir: &Path,
    stem: &str,
    regs: &Registries,
    replay: &Replay,
) -> anyhow::Result<PathBuf> {
    let path = replay_path(dir, stem, SystemTime::now());
    let header = il_save::header_for(
        regs,
        SaveKind::Replay,
        il_save::REPLAY_VERSION,
        Some(replay.ticks()),
        summary(stem, &replay.setup),
    );
    il_save::write(&path, &header, &replay.to_bytes())
        .with_context(|| format!("writing the replay {}", path.display()))?;
    Ok(path)
}

/// Writes a battle save to `path`.
pub fn write_save(path: &Path, regs: &Registries, save: &BattleSave) -> anyhow::Result<()> {
    let header = il_save::header_for(
        regs,
        SaveKind::Battle,
        SNAPSHOT_VERSION,
        Some(save.replay.ticks()),
        summary(&save.scenario_stem, &save.replay.setup),
    );
    il_save::write(path, &header, &save.to_bytes())
        .with_context(|| format!("writing the save {}", path.display()))
}

/// Checks a header against the loaded content and the expected kind.
fn check(
    header: &il_save::SaveHeader,
    regs: &Registries,
    kind: SaveKind,
    schema: u32,
) -> anyhow::Result<()> {
    if header.kind != kind {
        return Err(anyhow!("this file is a {:?}, not a {kind:?}", header.kind));
    }
    if header.schema_version != schema {
        return Err(anyhow!(
            "schema version {} (this build reads {schema})",
            header.schema_version
        ));
    }
    if !il_save::content_matches(header, regs) {
        return Err(anyhow!(
            "the loaded content differs from the content that wrote it (mods {:?})",
            header.mods
        ));
    }
    Ok(())
}

/// Rebuilds a session from a battle save.
pub fn load_save(
    path: &Path,
    regs: Arc<Registries>,
    threads: usize,
) -> anyhow::Result<BattleSession> {
    let file = il_save::read(path).with_context(|| format!("reading {}", path.display()))?;
    check(&file.header, &regs, SaveKind::Battle, SNAPSHOT_VERSION)?;
    let save = BattleSave::from_bytes(&file.body)?;
    Ok(BattleSession::from_save(save, regs, threads)?)
}

/// Starts a watch-only playback of a replay file.
pub fn load_replay(
    path: &Path,
    regs: Arc<Registries>,
    threads: usize,
) -> anyhow::Result<BattleSession> {
    let file = il_save::read(path).with_context(|| format!("reading {}", path.display()))?;
    check(
        &file.header,
        &regs,
        SaveKind::Replay,
        il_save::REPLAY_VERSION,
    )?;
    let replay = Replay::from_bytes(&file.body)?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(BattleSession::from_replay(replay, regs, threads)?.with_stem(stem))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn replay_paths_carry_the_stem_and_the_stamp() {
        let at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let p = replay_path(Path::new("replays"), "my scenario/x", at);
        assert_eq!(
            p,
            PathBuf::from("replays").join("my_scenario_x-20231114-221320.ilrp")
        );
    }
}
