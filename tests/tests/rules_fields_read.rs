//! T2-010's deferred check, made permanent in T2-113 (plan I16): every
//! property of every `rules-*.schema.json` must be read by a simulation
//! system, or carry a recorded reason for not being read yet.
//!
//! "Read" is a whole-word match of the property's leaf name in the source
//! of `il_sim_battle` or `il_ai`. The allow-list names the fields read
//! elsewhere or by a later phase; a field that is neither read nor listed
//! fails the test and is either wired up or removed from the rules file,
//! its schema and Simulation Spec §15.1.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Fields not read by a sim system, with the reason (kept while the reason
/// holds; the Simulation Spec §15.1 table names the same phases).
const ALLOWED_UNREAD: &[(&str, &str)] = &[
    (
        "corpse_ticks",
        "render-only: il_app keeps corpses for this long (SIM-CORE-008)",
    ),
    ("hpa_cluster", "Phase 3: HPA* clusters (SIM-MOVE-003)"),
    (
        "hpa_gate_split",
        "Phase 3: HPA* gate splitting (SIM-MOVE-003)",
    ),
    (
        "fled_return_fraction",
        "Phase 4: the campaign returns this fraction of the fled (SIM-FLOW-018)",
    ),
    (
        "state_mults.steady",
        "read through StateMultsTable::for_state in il_data (SIM-MOR-004)",
    ),
    (
        "state_mults.unsettled",
        "read through StateMultsTable::for_state in il_data (SIM-MOR-004)",
    ),
    (
        "state_mults.shaken",
        "read through StateMultsTable::for_state in il_data (SIM-MOR-004)",
    ),
    (
        "state_mults.broken",
        "read through StateMultsTable::for_state in il_data (SIM-MOR-004)",
    ),
    (
        "state_mults.routing",
        "read through StateMultsTable::for_state in il_data (SIM-MOR-004)",
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ lives directly under the workspace root")
        .to_path_buf()
}

/// Every `.rs` file under `dir`, concatenated.
fn sources(dir: &Path, out: &mut String) {
    for entry in std::fs::read_dir(dir).expect("source dir exists") {
        let path = entry.expect("readable entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push_str(&std::fs::read_to_string(&path).expect("source readable"));
            out.push('\n');
        }
    }
}

/// `(schema file, dotted property path)` for every leaf property.
fn properties(schema: &Value, prefix: &str, file: &str, out: &mut Vec<(String, String)>) {
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (key, value) in props {
        if key.starts_with('$') {
            continue;
        }
        let path = format!("{prefix}{key}");
        if value.get("properties").is_some() {
            properties(value, &format!("{path}."), file, out);
        } else {
            out.push((file.to_owned(), path));
        }
    }
}

/// Whole-word occurrence of `word` in `text`.
fn mentions(text: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(word).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + word.len()..].chars().next();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

#[test]
fn every_rule_field_is_read_by_a_system_or_listed() {
    let root = workspace_root();
    let mut src = String::new();
    sources(&root.join("crates/il_sim_battle/src"), &mut src);
    sources(&root.join("crates/il_ai/src"), &mut src);

    let mut props = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("docs/schemas"))
        .expect("schemas dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("rules-"))
        })
        .collect();
    files.sort();
    assert_eq!(files.len(), 8, "eight rules schemas: {files:?}");
    for file in &files {
        let text = std::fs::read_to_string(file).expect("schema readable");
        let schema: Value = serde_json::from_str(&text).expect("schema is JSON");
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        properties(&schema, "", &name, &mut props);
    }
    assert!(props.len() > 100, "{} rule fields found", props.len());

    let mut unread = Vec::new();
    let mut listed_but_read = Vec::new();
    for (file, path) in &props {
        let leaf = path.rsplit('.').next().unwrap();
        let read = mentions(&src, leaf);
        let listed = ALLOWED_UNREAD.iter().any(|(p, _)| p == path);
        match (read, listed) {
            (false, false) => unread.push(format!("{file}: {path}")),
            (true, true) if !path.starts_with("state_mults.") => {
                listed_but_read.push(format!("{file}: {path}"));
            }
            _ => {}
        }
    }
    assert!(
        unread.is_empty(),
        "rule fields no simulation system reads (wire them up, or list them with a reason):\n  {}",
        unread.join("\n  ")
    );
    assert!(
        listed_but_read.is_empty(),
        "fields on the allow-list that a system now reads (drop them from the list):\n  {}",
        listed_but_read.join("\n  ")
    );
    for (path, _) in ALLOWED_UNREAD {
        assert!(
            props.iter().any(|(_, p)| p == path),
            "allow-listed field {path} no longer exists in any schema"
        );
    }
}
