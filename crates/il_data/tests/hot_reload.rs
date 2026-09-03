//! Hot reload (T1-025): index-stable rebuilds, structural events, failures
//! keeping the old registries, and the file watcher end to end.
#![cfg(feature = "hot-reload")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use il_core::{S, Scalar};
use il_data::hot_reload::{HotReload, ReloadEvent};
use il_data::{ContentId, KindTag, ModSet, Registries, discover};

fn scratch(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/il_data_test/hot_reload")
        .join(name);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A private copy of game/ (content and locale) to edit.
fn game_copy(name: &str) -> PathBuf {
    let game = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game");
    let dst = scratch(name).join("game");
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::copy(game.join("mod.json5"), dst.join("mod.json5")).unwrap();
    copy_dir(&game.join("content"), &dst.join("content"));
    copy_dir(&game.join("locale"), &dst.join("locale"));
    copy_dir(&game.join("assets/maps"), &dst.join("assets/maps"));
    dst
}

fn start(root: &Path) -> HotReload {
    let found = discover(std::slice::from_ref(&root.to_path_buf())).unwrap();
    let set = ModSet::all(&found).unwrap();
    let regs = Arc::new(il_data::load(&set).unwrap_or_else(|e| panic!("{e}")));
    HotReload::new(set, regs).expect("watcher starts")
}

fn hastati(regs: &Registries) -> (u32, S) {
    let h = regs
        .units
        .lookup(&ContentId::new("rome:hastati").unwrap())
        .unwrap();
    (h.index(), regs.units.get(h).speed_walk)
}

fn set_speed(root: &Path, speed: &str) {
    let path = root.join("content/units/hastati.json5");
    let text = std::fs::read_to_string(&path).unwrap();
    let start = text.find("speed_walk:").expect("hastati has speed_walk");
    let end = start + text[start..].find(',').expect("terminated by a comma");
    let text = format!("{}speed_walk: {speed}{}", &text[..start], &text[end..]);
    std::fs::write(path, text).unwrap();
}

#[test]
fn value_change_keeps_every_index() {
    let root = game_copy("value");
    let mut hr = start(&root);
    let before = Arc::clone(hr.current());
    let (idx, speed) = hastati(&before);
    assert_eq!(speed, S::from_f32_data(1.6));

    set_speed(&root, "2.5");
    let next = hr.rebuild_now().expect("rebuild succeeds");
    let (idx2, speed2) = hastati(&next);
    assert_eq!(idx2, idx);
    assert_eq!(speed2, S::from_f32_data(2.5));
    let old_ids: Vec<_> = before.units.ids().cloned().collect();
    let new_ids: Vec<_> = next.units.ids().cloned().collect();
    assert_eq!(old_ids, new_ids, "layout is unchanged");
    assert_eq!(
        before.formations.ids().collect::<Vec<_>>(),
        next.formations.ids().collect::<Vec<_>>()
    );
    let events = hr.take_events();
    assert!(
        matches!(events.last(), Some(ReloadEvent::Swapped { .. })),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ReloadEvent::Structural { .. }))
    );
}

#[test]
fn new_ids_append_and_deleted_ids_keep_their_slot() {
    let root = game_copy("structural");
    let mut hr = start(&root);
    let before = Arc::clone(hr.current());
    let (hastati_idx, _) = hastati(&before);
    let velites_id = ContentId::new("rome:velites").unwrap();
    let velites_h = before.units.lookup(&velites_id).unwrap();

    // Add a unit sorted before every existing id, delete velites.
    std::fs::write(
        root.join("content/units/aaa_new.json5"),
        r#"{ id: "rome:aaa_new", name_key: "rome.units.aaa_new.name", category: "infantry",
            hp: 1, speed_walk: 1.6, speed_run: 4.0, attack: 1, defence: 1, damage: 1,
            formations: ["rome:line"], sprite_set: "rome:sprites_infantry", cost: 1, upkeep: 1 }"#,
    )
    .unwrap();
    std::fs::remove_file(root.join("content/units/velites.json5")).unwrap();

    let next = hr.rebuild_now().expect("rebuild succeeds");
    assert_eq!(
        hastati(&next).0,
        hastati_idx,
        "existing handles keep their index"
    );
    let new_h = next
        .units
        .lookup(&ContentId::new("rome:aaa_new").unwrap())
        .unwrap();
    assert_eq!(
        new_h.index() as usize,
        before.units.slots(),
        "new ids are appended after every old slot"
    );
    assert!(
        next.units.lookup(&velites_id).is_none(),
        "removed ids no longer resolve"
    );
    assert_eq!(
        next.units.get(velites_h).id,
        velites_id,
        "but old handles still read the old item"
    );
    assert!(next.units.is_removed(velites_h));
    assert!(
        next.units.iter().all(|(h, _)| h != velites_h),
        "iteration skips removed slots"
    );

    let events = hr.take_events();
    let structural = events.iter().find_map(|e| match e {
        ReloadEvent::Structural { added, removed } => Some((added.clone(), removed.clone())),
        _ => None,
    });
    let (added, removed) = structural.expect("structural event");
    assert_eq!(
        added,
        vec![(KindTag::Unit, ContentId::new("rome:aaa_new").unwrap())]
    );
    assert_eq!(removed, vec![(KindTag::Unit, velites_id)]);
}

#[test]
fn a_failing_edit_keeps_the_old_registries() {
    let root = game_copy("failed");
    let mut hr = start(&root);
    let before = Arc::clone(hr.current());
    set_speed(&root, "-1");
    assert!(hr.rebuild_now().is_none());
    assert!(Arc::ptr_eq(hr.current(), &before));
    let events = hr.take_events();
    match events.last() {
        Some(ReloadEvent::Failed(diags)) => {
            assert!(diags.to_string().contains("speed_walk"), "{diags}");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    // Fixing the file recovers.
    set_speed(&root, "1.7");
    let next = hr.rebuild_now().expect("recovers");
    assert_eq!(hastati(&next).1, S::from_f32_data(1.7));
}

#[test]
fn the_watcher_picks_up_an_edit() {
    let root = game_copy("watch");
    let mut hr = start(&root);
    std::thread::sleep(std::time::Duration::from_millis(300));
    set_speed(&root, "3.1");
    // Poll like a frame loop would; the debounce needs a few quiet polls.
    let mut swapped = None;
    for _ in 0..200 {
        if let Some(regs) = hr.poll() {
            swapped = Some(regs);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let regs = swapped.expect("watcher reported the edit within 5 s");
    assert_eq!(hastati(&regs).1, S::from_f32_data(3.1));
    assert!(hr.take_events().iter().any(|e| matches!(e, ReloadEvent::Swapped { files } if files.iter().any(|f| f.ends_with("hastati.json5")))));
}
