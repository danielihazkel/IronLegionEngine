//! The map editor's end-to-end checks (T3-060, TDD §16, §17): the
//! registry's test field saved into `tests/mods/editor_out/` is byte for
//! byte the committed map and sidecar, and the two mods validate clean
//! together (the override resolves to the same map, its heightmap read
//! from the editor's output).

use std::path::PathBuf;

use il_cli::validate::{ValidateOptions, validate};
use il_data::ContentId;
use il_editor::MapDocument;

fn editor_out() -> PathBuf {
    il_tests::workspace_root().join("tests/mods/editor_out")
}

/// T3-060 done-when: open `rome:test_field`, save it into the editor_out
/// mod, compare the bytes, validate the pair with warnings denied.
#[test]
fn test_field_saves_byte_identically_and_validates() {
    let game = il_tests::game_root();
    let regs = il_data::load_roots(std::slice::from_ref(&game)).unwrap_or_else(|d| panic!("{d}"));
    let id = ContentId::new("rome:test_field").unwrap();
    let mut doc = MapDocument::from_registry(&regs, &id, std::slice::from_ref(&game)).unwrap();
    let out = editor_out();
    let saved = doc.save(&out).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(saved.json5, out.join("content/maps/test_field.json5"));
    assert_eq!(saved.hgt, out.join("assets/maps/test_field.hgt"));
    assert!(!doc.dirty);

    // A checkout may carry CRLF (CI's does); the editor writes LF.
    let original = std::fs::read_to_string(game.join("content/maps/test_field.json5"))
        .unwrap()
        .replace("\r\n", "\n");
    let written = std::fs::read_to_string(&saved.json5).unwrap();
    assert_eq!(written, original, "the JSON5 must be byte-identical");
    assert_eq!(
        std::fs::read(&saved.hgt).unwrap(),
        std::fs::read(game.join("assets/maps/test_field.hgt")).unwrap(),
        "the heightmap sidecar must be byte-identical"
    );

    let mut text = Vec::new();
    let report = validate(
        &ValidateOptions {
            roots: vec![game, out],
            deny_warnings: true,
            verbose: true,
        },
        &mut text,
    )
    .expect("validate runs");
    let text = String::from_utf8(text).unwrap();
    assert_eq!(report.errors, 0, "{text}");
    assert_eq!(report.warnings, 0, "{text}");
    assert_eq!(
        report.mods,
        vec!["rome".to_string(), "editor_out".to_string()]
    );
    assert!(
        text.contains("0 errors, 0 warnings in 2 mods (order: rome, editor_out)"),
        "{text}"
    );
}
