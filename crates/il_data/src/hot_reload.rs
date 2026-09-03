//! Hot reload (T1-025, REQ-MOD-008, SAD §9.4, Modding SDK §9), behind the
//! `hot-reload` feature.
//!
//! A file watcher over every enabled mod's content and locale folders marks
//! the set dirty; once the disk has been quiet for a few polls the whole
//! pipeline runs again against the previous registries, which keeps every
//! surviving ContentId at its old index (new ids are appended, deleted ids
//! keep their slot and are marked removed). The app swaps the new
//! `Arc<Registries>` into the sim between ticks, so handles held by entities
//! stay valid and numeric changes apply next tick. A load with errors keeps
//! the old registries and reports the diagnostics.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::content_id::ContentId;
use crate::diagnostic::Diagnostics;
use crate::load_order::ModSet;
use crate::pipeline::load_report_with_prev;
use crate::registries::Registries;
use crate::schema::KindTag;

/// Polls of no further file events before a rebuild (about 100 ms at 60 FPS).
pub const QUIET_POLLS: u32 = 6;

#[derive(Clone, Debug, PartialEq)]
pub enum ReloadEvent {
    /// Values changed; the new registries apply from the next tick.
    Swapped { files: Vec<PathBuf> },
    /// Ids appeared or disappeared; entities keep their handles, new ids are
    /// usable from the next battle load.
    Structural {
        added: Vec<(KindTag, ContentId)>,
        removed: Vec<(KindTag, ContentId)>,
    },
    /// The reload failed validation; the previous registries stay.
    Failed(Diagnostics),
    /// `mod.json5` changed: manifests are read only at startup.
    ManifestIgnored(PathBuf),
}

pub struct HotReload {
    _watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<notify::Event>>,
    set: ModSet,
    current: Arc<Registries>,
    dirty: Vec<PathBuf>,
    quiet_polls: u32,
    events: Vec<ReloadEvent>,
}

fn is_content_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "json5")
}

fn is_manifest(path: &Path) -> bool {
    path.file_name().is_some_and(|n| n == "mod.json5")
}

impl HotReload {
    /// Watches every mod of `set`; `current` is what the app loaded from it.
    pub fn new(set: ModSet, current: Arc<Registries>) -> notify::Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        for m in &set.mods {
            for dir in [m.content_dir(), m.root.join("locale")] {
                if dir.is_dir() {
                    watcher.watch(&dir, RecursiveMode::Recursive)?;
                }
            }
        }
        Ok(Self {
            _watcher: watcher,
            rx,
            set,
            current,
            dirty: Vec::new(),
            quiet_polls: 0,
            events: Vec::new(),
        })
    }

    pub fn current(&self) -> &Arc<Registries> {
        &self.current
    }

    pub fn mod_set(&self) -> &ModSet {
        &self.set
    }

    /// Events since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<ReloadEvent> {
        std::mem::take(&mut self.events)
    }

    /// Non-blocking; call once per frame. Returns the new registries when a
    /// rebuild succeeded during this call.
    pub fn poll(&mut self) -> Option<Arc<Registries>> {
        let mut saw_event = false;
        while let Ok(res) = self.rx.try_recv() {
            let Ok(event) = res else { continue };
            for path in event.paths {
                if is_manifest(&path) {
                    self.events.push(ReloadEvent::ManifestIgnored(path));
                } else if is_content_file(&path) {
                    saw_event = true;
                    if !self.dirty.contains(&path) {
                        self.dirty.push(path);
                    }
                }
            }
        }
        if self.dirty.is_empty() {
            return None;
        }
        if saw_event {
            self.quiet_polls = 0;
            return None;
        }
        self.quiet_polls += 1;
        if self.quiet_polls < QUIET_POLLS {
            return None;
        }
        self.rebuild_now()
    }

    /// Re-runs the pipeline immediately (tests, or a manual reload key).
    pub fn rebuild_now(&mut self) -> Option<Arc<Registries>> {
        let files = std::mem::take(&mut self.dirty);
        self.quiet_polls = 0;
        let report = load_report_with_prev(&self.set, Some(&self.current));
        let Some(next) = report.registries else {
            self.events.push(ReloadEvent::Failed(report.diagnostics));
            return None;
        };
        let (added, removed) = structural_diff(&self.current, &next);
        if !added.is_empty() || !removed.is_empty() {
            self.events.push(ReloadEvent::Structural { added, removed });
        }
        self.events.push(ReloadEvent::Swapped { files });
        let next = Arc::new(next);
        self.current = Arc::clone(&next);
        Some(next)
    }
}

type IdList = Vec<(KindTag, ContentId)>;

/// Ids that appeared or were marked removed between two layouts.
fn structural_diff(prev: &Registries, next: &Registries) -> (IdList, IdList) {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    macro_rules! diff {
        ($field:ident, $tag:expr) => {
            for id in next.$field.ids_added_after(prev.$field.slots()) {
                added.push(($tag, id.clone()));
            }
            for id in next.$field.removed_ids() {
                if prev.$field.lookup(id).is_some() {
                    removed.push(($tag, id.clone()));
                }
            }
        };
    }
    diff!(units, KindTag::Unit);
    diff!(formations, KindTag::Formation);
    diff!(group_formations, KindTag::GroupFormation);
    diff!(factions, KindTag::Faction);
    diff!(zones, KindTag::Zone);
    diff!(maps, KindTag::Map);
    diff!(sprite_sets, KindTag::SpriteSet);
    (added, removed)
}
