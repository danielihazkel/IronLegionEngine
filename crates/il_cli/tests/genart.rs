//! The committed placeholder art must equal what `il_cli genart` produces, so
//! nobody edits generated files by hand and the generator stays deterministic.

use std::path::Path;

fn game_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../game")
        .canonicalize()
        .expect("game/ exists")
}

#[test]
fn committed_sheets_match_the_generator() {
    let artifacts = il_cli::genart::artifacts().expect("generation succeeds");
    assert_eq!(artifacts.len(), il_cli::genart::CATEGORIES.len() * 2);
    for (rel, bytes) in artifacts {
        let path = game_root().join(&rel);
        let on_disk = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("{} missing ({e}); run `il_cli genart`", path.display()));
        assert!(
            on_disk == bytes,
            "{} differs from the generator output; run `il_cli genart`",
            path.display()
        );
    }
}

#[test]
fn sheets_have_opaque_bodies_and_transparent_corners() {
    let rgba = il_cli::genart::render_sheet("infantry");
    let width = il_cli::genart::COLUMNS * il_cli::genart::FRAME_W;
    let at = |x: u32, y: u32| {
        let i = ((y * width + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };
    assert_eq!(at(0, 0)[3], 0, "frame corner is transparent");
    let (ox, oy) = il_cli::genart::ORIGIN;
    let body = at(ox, oy - 12);
    assert_eq!(body[3], 255, "body centre is opaque");
    assert!(body[0] < 120, "the category mark is dark on the body");
}
