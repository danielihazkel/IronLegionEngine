//! User settings (T2-091, REQ-UI-007, REQ-INP-005; plan decisions 3, 7, 13,
//! I9): `settings.json5` under the user's config directory, found through
//! the environment (no platform crate): `%APPDATA%\IronLegion` on Windows,
//! `$XDG_CONFIG_HOME/IronLegion` or `~/.config/IronLegion` elsewhere, the
//! working directory when none exists; `--settings` overrides. The file
//! holds the UI scale, vsync, fullscreen, the sim thread count, the audio
//! volumes (stored now, read by T2-100), the replay and save folders and a
//! list of key-binding overrides applied on top of the mods' bindings by
//! action. Every field has a default so an older file still loads. The sim
//! never reads it.

use std::path::{Path, PathBuf};

use il_data::json5::{FileId, parse_json5};
use il_data::{Binding, InputBindings, Registries};
use il_ui::{BindingError, Bindings};
use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "settings.json5";
pub const APP_DIR: &str = "IronLegion";
/// Bounds of `ui_scale` (decision 7).
pub const UI_SCALE_RANGE: (f32, f32) = (0.5, 2.0);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Volume {
    pub master: f32,
    pub effects: f32,
    pub music: f32,
}

impl Default for Volume {
    fn default() -> Self {
        Self {
            master: 1.0,
            effects: 1.0,
            music: 1.0,
        }
    }
}

/// One action's chords, replacing the mods' list for that action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingOverride {
    pub action: String,
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ui_scale: f32,
    pub vsync: bool,
    pub fullscreen: bool,
    /// Simulation worker threads (`--threads` on the command line wins).
    pub threads: usize,
    pub volume: Volume,
    pub bindings: Vec<BindingOverride>,
    pub replays_dir: String,
    pub saves_dir: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui_scale: 1.0,
            vsync: true,
            fullscreen: false,
            threads: 1,
            volume: Volume::default(),
            bindings: Vec::new(),
            replays_dir: "replays".to_string(),
            saves_dir: "saves".to_string(),
        }
    }
}

/// The user's config directory for the engine (decision 3).
pub fn config_dir() -> PathBuf {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(app) = env("APPDATA") {
        return app.join(APP_DIR);
    }
    if let Some(xdg) = env("XDG_CONFIG_HOME") {
        return xdg.join(APP_DIR);
    }
    if let Some(home) = env("HOME") {
        return home.join(".config").join(APP_DIR);
    }
    PathBuf::from(".")
}

pub fn default_path() -> PathBuf {
    config_dir().join(FILE_NAME)
}

/// Reads the file; a missing file is the defaults, a bad one is the
/// defaults plus a warning (the app prints it and carries on).
pub fn load(path: &Path) -> (Settings, Option<String>) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Settings::default(), None),
        Err(e) => {
            return (
                Settings::default(),
                Some(format!("{}: {e}", path.display())),
            );
        }
    };
    match parse(&text) {
        Ok(s) => (s, None),
        Err(e) => (
            Settings::default(),
            Some(format!("{}: {e}", path.display())),
        ),
    }
}

/// Parses the JSON5 text (the engine's own parser, as scenarios use).
pub fn parse(text: &str) -> anyhow::Result<Settings> {
    let value = parse_json5(text, FileId(0)).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut s: Settings = serde_json::from_value(value.to_json())?;
    s.ui_scale = s.ui_scale.clamp(UI_SCALE_RANGE.0, UI_SCALE_RANGE.1);
    s.threads = s.threads.max(1);
    Ok(s)
}

/// Pretty JSON, which is valid JSON5.
pub fn to_text(settings: &Settings) -> String {
    serde_json::to_string_pretty(settings).expect("settings are plain data")
}

pub fn save(path: &Path, settings: &Settings) -> anyhow::Result<()> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, to_text(settings))?;
    Ok(())
}

/// The mods' bindings with the overrides applied by action (replace).
pub fn effective_input(base: &InputBindings, settings: &Settings) -> InputBindings {
    let mut bindings = base.bindings.clone();
    for o in &settings.bindings {
        match bindings.iter_mut().find(|b| b.action == o.action) {
            Some(b) => b.keys = o.keys.clone(),
            None => bindings.push(Binding {
                action: o.action.clone(),
                keys: o.keys.clone(),
            }),
        }
    }
    InputBindings { bindings }
}

/// The parsed bindings the app runs with.
pub fn effective_bindings(regs: &Registries, settings: &Settings) -> (Bindings, Vec<BindingError>) {
    Bindings::from_content(&effective_input(&regs.input, settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_and_an_older_file_still_loads() {
        let s = Settings::default();
        assert_eq!(parse(&to_text(&s)).unwrap(), s);
        // A file from before a field existed: every field has a default.
        let old =
            parse("{ ui_scale: 1.5, bindings: [ { action: \"order_halt\", keys: [\"J\"] } ] }")
                .unwrap();
        assert!((old.ui_scale - 1.5).abs() < 1e-6);
        assert!(old.vsync);
        assert_eq!(old.bindings[0].action, "order_halt");
        // Out-of-range values are clamped, not refused.
        let clamped = parse("{ ui_scale: 9, threads: 0 }").unwrap();
        assert!((clamped.ui_scale - UI_SCALE_RANGE.1).abs() < 1e-6);
        assert_eq!(clamped.threads, 1);
        assert!(parse("{ ui_scale: \"big\" }").is_err());
    }

    #[test]
    fn a_missing_file_is_the_defaults_and_a_bad_one_warns() {
        let dir = std::env::temp_dir().join(format!("il_settings_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("deeper").join(FILE_NAME);
        let (s, warning) = load(&path);
        assert_eq!(s, Settings::default());
        assert!(warning.is_none());
        let custom = Settings {
            fullscreen: true,
            bindings: vec![BindingOverride {
                action: "order_halt".into(),
                keys: vec!["J".into()],
            }],
            ..Settings::default()
        };
        save(&path, &custom).unwrap();
        let (back, warning) = load(&path);
        assert_eq!(back, custom);
        assert!(warning.is_none());
        std::fs::write(&path, "{ not json5").unwrap();
        let (s, warning) = load(&path);
        assert_eq!(s, Settings::default());
        assert!(warning.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overrides_replace_one_action_and_leave_the_rest() {
        let base = InputBindings {
            bindings: vec![
                Binding {
                    action: "order_halt".into(),
                    keys: vec!["H".into()],
                },
                Binding {
                    action: "pause".into(),
                    keys: vec!["Space".into()],
                },
            ],
        };
        let settings = Settings {
            bindings: vec![
                BindingOverride {
                    action: "order_halt".into(),
                    keys: vec!["J".into(), "K".into()],
                },
                BindingOverride {
                    action: "no_such_action".into(),
                    keys: vec!["L".into()],
                },
            ],
            ..Settings::default()
        };
        let merged = effective_input(&base, &settings);
        assert_eq!(merged.keys_for("order_halt"), ["J", "K"]);
        assert_eq!(merged.keys_for("pause"), ["Space"]);
        let (bindings, errors) = Bindings::from_content(&merged);
        assert_eq!(bindings.chords(il_ui::Action::OrderHalt).len(), 2);
        // The unknown action is reported, not fatal.
        assert!(matches!(errors[..], [BindingError::UnknownAction(_)]));
    }

    #[test]
    fn the_config_dir_follows_the_environment() {
        let dir = config_dir();
        assert!(dir.ends_with(APP_DIR) || dir == Path::new("."));
    }
}
