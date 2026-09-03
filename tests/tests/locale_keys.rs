//! REQ-LOC-001: every engine UI string comes from the locale. The check is
//! mechanical: each `"il.<key>"` literal in the UI and app sources must exist
//! in the flagship English locale, and no panel may call egui with a bare
//! English label.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ lives directly under the workspace root")
        .to_path_buf()
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Every `"il.<a>.<b>"` literal in the given source trees.
fn engine_keys_in(dirs: &[&str]) -> BTreeSet<String> {
    let root = workspace_root();
    let mut files = Vec::new();
    for d in dirs {
        rust_files(&root.join(d), &mut files);
    }
    let mut keys = BTreeSet::new();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (i, _) in text.match_indices("\"il.") {
            let rest = &text[i + 1..];
            let end = rest.find('"').unwrap();
            let key = &rest[..end];
            if key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_')
                && key.split('.').count() >= 2
            {
                keys.insert(key.to_owned());
            }
        }
    }
    keys
}

#[test]
fn every_engine_key_used_by_the_ui_exists_in_the_flagship_locale() {
    let regs =
        il_data::load_roots(&[workspace_root().join("game")]).unwrap_or_else(|d| panic!("{d}"));
    let keys = engine_keys_in(&["crates/il_ui/src", "crates/il_app/src"]);
    assert!(
        keys.len() >= 20,
        "expected the UI to reference engine keys, found {keys:?}"
    );
    let missing: Vec<&String> = keys.iter().filter(|k| !regs.locale.has(k)).collect();
    assert!(
        missing.is_empty(),
        "keys used by the UI but absent from game/locale/en.json5: {missing:?}"
    );
    // A lookup of every key returns text, not the key itself.
    for key in &keys {
        assert_ne!(
            regs.locale.get(key),
            key.as_str(),
            "{key} resolves to itself"
        );
    }
}

#[test]
fn panels_have_no_bare_english_labels() {
    // egui calls with a string literal are the only way a label could bypass
    // the locale; ids and format specs are allowed.
    let root = workspace_root();
    let src = std::fs::read_to_string(root.join("crates/il_ui/src/panels.rs")).unwrap();
    for call in [
        "ui.label(\"",
        "ui.heading(\"",
        "ui.button(\"",
        "ui.small_button(\"",
        "ui.strong(\"",
    ] {
        for (i, _) in src.match_indices(call) {
            let rest = &src[i + call.len()..];
            let literal = &rest[..rest.find('"').unwrap()];
            // Symbols such as "+" and "-" are not words.
            assert!(
                !literal.bytes().any(|b| b.is_ascii_alphabetic()),
                "bare label in panels.rs: {call}{literal}\""
            );
        }
    }
}
