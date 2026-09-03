//! SAD §5.2 hard dependency rules, enforced by parsing every crate manifest.
//!
//! - Simulation crates (`il_core`, `il_data`, `il_ai`, `il_sim_battle`,
//!   `il_sim_campaign`) must not depend on rendering, windowing, UI, audio,
//!   OS-seeded randomness, or on any non-sim engine crate.
//! - No `il_*` crate may depend on `game_rules` (REQ-VIS-020).
//! - Presentation crates (`il_render`, `il_ui`) read the sim through
//!   `il_core`, `il_data`, `il_sim_battle` only; the renderer never sees the
//!   window library and the UI never sees the GPU library.

use std::path::{Path, PathBuf};

const SIM_CRATES: &[&str] = &[
    "il_core",
    "il_data",
    "il_ai",
    "il_sim_battle",
    "il_sim_campaign",
];

/// Crates a simulation crate may never pull in, directly or as a dev/build dependency.
const FORBIDDEN_IN_SIM: &[&str] = &[
    "wgpu",
    "winit",
    "egui",
    "egui-wgpu",
    "egui-winit",
    "kira",
    "rodio",
    "cpal",
    "rand",
    "glam",
    "game_rules",
    "il_render",
    "il_ui",
    "il_audio",
    "il_app",
    "il_cli",
    "il_save",
    "il_net",
    "il_editor",
    "il_script",
];

/// Presentation crates and the workspace crates each may depend on (SAD §5.2).
const PRESENTATION_ALLOWED: &[(&str, &[&str])] = &[
    ("il_render", &["il_core", "il_data", "il_sim_battle"]),
    ("il_ui", &["il_core", "il_data", "il_sim_battle"]),
];

/// External crates a presentation crate must not pull in.
const PRESENTATION_FORBIDDEN: &[(&str, &[&str])] =
    &[("il_render", &["winit"]), ("il_ui", &["wgpu"])];

const DEP_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ lives directly under the workspace root")
        .to_path_buf()
}

fn crate_manifests() -> Vec<(String, toml::Table)> {
    let crates_dir = workspace_root().join("crates");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&crates_dir).expect("crates/ exists") {
        let dir = entry.expect("readable entry").path();
        let manifest = dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).expect("manifest readable");
        let table: toml::Table = text.parse().expect("manifest parses as TOML");
        let name = table["package"]["name"]
            .as_str()
            .expect("package.name is a string")
            .to_string();
        out.push((name, table));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(
        !out.is_empty(),
        "no crate manifests found under {}",
        crates_dir.display()
    );
    out
}

fn dependency_names(table: &toml::Table) -> Vec<String> {
    let mut names = Vec::new();
    for key in DEP_TABLES {
        if let Some(deps) = table.get(*key).and_then(|v| v.as_table()) {
            for (name, value) in deps {
                // `foo = { package = "bar" }` renames; the real crate is `bar`.
                let real = value
                    .as_table()
                    .and_then(|t| t.get("package"))
                    .and_then(|p| p.as_str())
                    .unwrap_or(name);
                names.push(real.to_string());
            }
        }
    }
    names
}

#[test]
fn sim_crates_do_not_depend_on_forbidden_crates() {
    let mut violations = Vec::new();
    for (name, table) in crate_manifests() {
        if !SIM_CRATES.contains(&name.as_str()) {
            continue;
        }
        for dep in dependency_names(&table) {
            if FORBIDDEN_IN_SIM.contains(&dep.as_str()) {
                violations.push(format!("{name} depends on {dep}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "SAD §5.2 dependency rule violations:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn no_engine_crate_depends_on_game_rules() {
    let mut violations = Vec::new();
    for (name, table) in crate_manifests() {
        if name.starts_with("il_") && dependency_names(&table).iter().any(|d| d == "game_rules") {
            violations.push(name);
        }
    }
    assert!(
        violations.is_empty(),
        "REQ-VIS-020: engine crates depending on game_rules: {violations:?}"
    );
}

#[test]
fn presentation_crates_only_read_the_sim() {
    let manifests = crate_manifests();
    let mut violations = Vec::new();
    for (crate_name, allowed) in PRESENTATION_ALLOWED {
        let Some((_, table)) = manifests.iter().find(|(n, _)| n == crate_name) else {
            panic!("missing crate {crate_name}");
        };
        let deps = dependency_names(table);
        for dep in &deps {
            let workspace_crate = dep.starts_with("il_") || dep == "game_rules";
            if workspace_crate && !allowed.contains(&dep.as_str()) {
                violations.push(format!("{crate_name} depends on {dep}"));
            }
        }
        if let Some((_, forbidden)) = PRESENTATION_FORBIDDEN.iter().find(|(n, _)| n == crate_name) {
            for dep in &deps {
                if forbidden.contains(&dep.as_str()) {
                    violations.push(format!("{crate_name} depends on {dep}"));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "SAD §5.2 presentation rule violations:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn every_sim_crate_manifest_is_present() {
    let names: Vec<String> = crate_manifests().into_iter().map(|(n, _)| n).collect();
    for sim in SIM_CRATES {
        assert!(names.iter().any(|n| n == sim), "missing crate {sim}");
    }
}
