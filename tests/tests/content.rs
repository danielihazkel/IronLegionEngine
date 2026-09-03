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
