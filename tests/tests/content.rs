//! Content validation of the flagship game (REQ-TEST-005, T1-027) and the
//! exit-code semantics of `il_cli validate`.

use il_cli::validate::{ValidateOptions, validate};

#[test]
fn flagship_content_validates_clean() {
    let mut out = Vec::new();
    let report = validate(
        &ValidateOptions {
            roots: vec![il_tests::game_root()],
            deny_warnings: true,
            verbose: true,
        },
        &mut out,
    )
    .expect("validate runs");
    let text = String::from_utf8(out).unwrap();
    assert_eq!(report.errors, 0, "{text}");
    assert_eq!(report.warnings, 0, "{text}");
    assert_eq!(report.mods, vec!["rome".to_string()]);
    assert!(report.ok(false));
    assert!(text.contains("0 errors, 0 warnings in 1 mod"), "{text}");
    assert!(
        text.contains("content hash"),
        "verbose prints the hashes: {text}"
    );
}

#[test]
fn a_broken_mod_reports_every_error_with_its_line() {
    let mut out = Vec::new();
    let roots = vec![
        il_tests::game_root(),
        il_tests::workspace_root().join("tests/fixtures"),
    ];
    let report = validate(
        &ValidateOptions {
            roots,
            deny_warnings: false,
            verbose: false,
        },
        &mut out,
    )
    .expect("validate runs");
    let text = String::from_utf8(out).unwrap();
    assert_eq!(report.errors, 3, "{text}");
    assert!(!report.ok(false));
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("badmod/content/units/broken.json5:4:3 category:")),
        "{text}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("badmod/content/units/broken.json5:7:3 armour:")),
        "{text}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("badmod/content/units/broken.json5:10:3 amour:")),
        "{text}"
    );
    assert!(
        text.contains("3 errors, 0 warnings in 2 mods (order: rome, badmod)"),
        "{text}"
    );
}

/// Every `.json5` under `dir`, recursively, sorted.
fn json5_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("readable entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            json5_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "json5") {
            out.push(path);
        }
    }
}

const SCENARIO_SCHEMA: &str = include_str!("../../docs/schemas/scenario.schema.json");

/// T3-002: every scenario file under `tests/scenarios/` (the band files
/// included) validates against `docs/schemas/scenario.schema.json`.
#[test]
fn scenario_files_match_the_scenario_schema() {
    let mut files = Vec::new();
    json5_files(&il_tests::scenario_dir(), &mut files);
    assert!(files.len() >= 15, "found {} scenario files", files.len());
    let mut failures = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).unwrap();
        let value = il_data::json5::parse_json5(&text, il_data::json5::FileId(0))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .to_json();
        for line in il_data::schema::validate_free(SCENARIO_SCHEMA, &value) {
            failures.push(format!("{}: {line}", path.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// T3-002 (SIM-FORM-012): a regiment is the `unit_type` + `count` shorthand
/// or a `units` composition, never both; the composition may repeat a unit
/// type (every entry is its own group).
#[test]
fn scenario_schema_accepts_compositions_and_rejects_both_forms() {
    let scenario = |regiment: &str| -> serde_json::Value {
        serde_json::from_str(&format!(
            r#"{{ "map_id": "rome:test_field", "seed": 1, "sides": [ {{
                "faction": "rome:rome", "player": 0,
                "general": {{ "unit_type": "rome:general" }},
                "regiments": [ {regiment} ] }} ] }}"#
        ))
        .unwrap()
    };
    let ok = |regiment: &str| il_data::schema::validate_free(SCENARIO_SCHEMA, &scenario(regiment));
    assert!(ok(r#"{ "id": 1, "unit_type": "rome:hastati", "count": 120 }"#).is_empty());
    let mixed = r#"{ "id": 1, "units": [ { "unit_type": "rome:velites", "count": 40 },
        { "unit_type": "rome:hastati", "count": 100, "experience": 2 },
        { "unit_type": "rome:hastati", "count": 20 } ] }"#;
    assert!(ok(mixed).is_empty(), "{:?}", ok(mixed));
    let both = r#"{ "id": 1, "unit_type": "rome:hastati", "count": 120,
        "units": [ { "unit_type": "rome:velites", "count": 40 } ] }"#;
    assert!(!ok(both).is_empty(), "both forms must be rejected");
    assert!(
        !ok(r#"{ "id": 1, "units": [] }"#).is_empty(),
        "an empty composition"
    );
    assert!(!ok(r#"{ "id": 1 }"#).is_empty(), "neither form");
    assert!(
        !ok(r#"{ "id": 1, "units": [ { "unit_type": "rome:velites", "count": 0 } ] }"#).is_empty(),
        "a zero count"
    );
}
