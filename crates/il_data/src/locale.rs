//! `Locale`: user-visible strings by key (T1-024, REQ-LOC-001, Modding SDK §7).
//!
//! Every mod's `locale/<lang>.json5` is flattened (`a: { b: "x" }` → `a.b`)
//! and merged in load order, later mods overriding earlier keys. Lookup falls
//! back from the current language to `en` and finally to the key itself,
//! which is logged once as a warning. `show_keys` (a debug toggle) returns
//! keys instead of strings so untranslated UI is easy to spot.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Display;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::json5::{FileId, SpannedValue, ValueKind, parse_json5};
use crate::load_order::ModSet;
use crate::source::Sources;

pub const FALLBACK_LANGUAGE: &str = "en";

#[derive(Debug)]
pub struct Locale {
    /// Language → flattened key → string.
    tables: BTreeMap<String, BTreeMap<String, String>>,
    current: String,
    show_keys: AtomicBool,
    /// Keys already reported missing (warned once each).
    missing: Mutex<BTreeSet<String>>,
}

impl Default for Locale {
    fn default() -> Self {
        Self {
            tables: BTreeMap::new(),
            current: FALLBACK_LANGUAGE.to_string(),
            show_keys: AtomicBool::new(false),
            missing: Mutex::new(BTreeSet::new()),
        }
    }
}

impl Clone for Locale {
    fn clone(&self) -> Self {
        Self {
            tables: self.tables.clone(),
            current: self.current.clone(),
            show_keys: AtomicBool::new(self.show_keys.load(Ordering::Relaxed)),
            missing: Mutex::new(self.missing.lock().map(|m| m.clone()).unwrap_or_default()),
        }
    }
}

impl Locale {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or overrides) one key in one language.
    pub fn insert(&mut self, lang: &str, key: &str, text: &str) {
        self.tables
            .entry(lang.to_string())
            .or_default()
            .insert(key.to_string(), text.to_string());
    }

    pub fn language(&self) -> &str {
        &self.current
    }

    /// Switches the current language. Returns `false` (and keeps the old one)
    /// when no enabled mod provides `lang`.
    pub fn set_language(&mut self, lang: &str) -> bool {
        if self.tables.contains_key(lang) || lang == FALLBACK_LANGUAGE {
            self.current = lang.to_string();
            true
        } else {
            false
        }
    }

    /// Languages at least one enabled mod provides.
    pub fn languages(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(String::as_str)
    }

    /// Debug toggle: `get` returns the key itself while on.
    pub fn set_show_keys(&self, on: bool) {
        self.show_keys.store(on, Ordering::Relaxed);
    }

    pub fn show_keys(&self) -> bool {
        self.show_keys.load(Ordering::Relaxed)
    }

    pub fn has(&self, key: &str) -> bool {
        self.lookup(key).is_some()
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        self.tables
            .get(&self.current)
            .and_then(|t| t.get(key))
            .or_else(|| self.tables.get(FALLBACK_LANGUAGE).and_then(|t| t.get(key)))
            .map(String::as_str)
    }

    /// The string for `key` in the current language, else in `en`, else the
    /// key itself (warned once).
    pub fn get<'a>(&'a self, key: &'a str) -> &'a str {
        if self.show_keys() {
            return key;
        }
        match self.lookup(key) {
            Some(s) => s,
            None => {
                if let Ok(mut missing) = self.missing.lock()
                    && missing.insert(key.to_string())
                {
                    tracing::warn!(key, "missing localisation key");
                }
                key
            }
        }
    }

    /// `get` with `{name}` placeholders substituted from `args`; unknown
    /// placeholders stay literal.
    pub fn fmt(&self, key: &str, args: &[(&str, &dyn Display)]) -> String {
        let template = self.get(key);
        let mut out = String::with_capacity(template.len());
        let mut rest = template;
        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('}') {
                Some(end) => {
                    let name = &after[..end];
                    match args.iter().find(|(n, _)| *n == name) {
                        Some((_, v)) => out.push_str(&v.to_string()),
                        None => {
                            out.push('{');
                            out.push_str(name);
                            out.push('}');
                        }
                    }
                    rest = &after[end + 1..];
                }
                None => {
                    out.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Keys that were requested but missing, sorted.
    pub fn missing_keys(&self) -> Vec<String> {
        self.missing
            .lock()
            .map(|m| m.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Flattens a locale object into `(key, text)` pairs; non-string leaves are
/// diagnostics.
fn flatten(
    value: &SpannedValue,
    prefix: &str,
    out: &mut Vec<(String, String)>,
    file: &Path,
    diags: &mut Diagnostics,
) {
    match &value.kind {
        ValueKind::Object(entries) => {
            for (k, v) in entries {
                let key = if prefix.is_empty() {
                    k.name.clone()
                } else {
                    format!("{prefix}.{}", k.name)
                };
                flatten(v, &key, out, file, diags);
            }
        }
        ValueKind::String(s) => out.push((prefix.to_string(), s.clone())),
        _ => diags.push(
            Diagnostic::file_level(
                file,
                format!(
                    "locale values must be strings or nested objects, found {}",
                    value.type_name()
                ),
            )
            .at(value.span.line, value.span.col)
            .field(prefix),
        ),
    }
}

/// Loads every `locale/<lang>.json5` of every mod in load order.
pub fn load_locales(set: &ModSet, sources: &mut Sources, diags: &mut Diagnostics) -> Locale {
    let mut locale = Locale::new();
    for (mod_index, m) in set.mods.iter().enumerate() {
        let dir = m.root.join("locale");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<_> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "json5"))
            .collect();
        files.sort();
        for abs in files {
            let Some(lang) = abs.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            let rel = abs.strip_prefix(&m.root).unwrap_or(&abs);
            let file_id: FileId = sources.add(mod_index, &m.manifest.id, rel, &abs);
            let display = sources.display(file_id);
            let text = match std::fs::read_to_string(&abs) {
                Ok(t) => t,
                Err(e) => {
                    diags.push(Diagnostic::file_level(
                        &display,
                        format!("cannot read: {e}"),
                    ));
                    continue;
                }
            };
            let value = match parse_json5(&text, file_id) {
                Ok(v) => v,
                Err(e) => {
                    diags.push(
                        Diagnostic::file_level(&display, e.message).at(e.span.line, e.span.col),
                    );
                    continue;
                }
            };
            if value.as_object().is_none() {
                diags.push(
                    Diagnostic::file_level(
                        &display,
                        format!("expected an object, found {}", value.type_name()),
                    )
                    .at(value.span.line, value.span.col),
                );
                continue;
            }
            let mut pairs = Vec::new();
            flatten(&value, "", &mut pairs, &display, diags);
            for (key, text) in pairs {
                locale.insert(&lang, &key, &text);
            }
        }
    }
    locale
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locale() -> Locale {
        let mut l = Locale::new();
        l.insert("en", "rome.units.hastati.name", "Hastati");
        l.insert(
            "en",
            "il.event.gold",
            "Coastal trade brings {amount} gold to {city}.",
        );
        l.insert("de", "rome.units.hastati.name", "Hastaten");
        l
    }

    #[test]
    fn missing_key_returns_the_key_and_is_recorded_once() {
        let l = locale();
        for _ in 0..3 {
            assert_eq!(l.get("rome.units.zzz.name"), "rome.units.zzz.name");
        }
        assert_eq!(l.missing_keys(), vec!["rome.units.zzz.name".to_string()]);
        assert!(!l.has("rome.units.zzz.name"));
        assert!(l.has("rome.units.hastati.name"));
    }

    #[test]
    fn fallback_chain_current_then_en_then_key() {
        let mut l = locale();
        assert!(l.set_language("de"));
        assert_eq!(l.get("rome.units.hastati.name"), "Hastaten");
        assert_eq!(
            l.get("il.event.gold"),
            "Coastal trade brings {amount} gold to {city}.",
            "falls back to en"
        );
        assert!(!l.set_language("fr"));
        assert_eq!(l.language(), "de");
        assert_eq!(l.languages().collect::<Vec<_>>(), vec!["de", "en"]);
    }

    #[test]
    fn fmt_substitutes_and_keeps_unknown_placeholders() {
        let l = locale();
        assert_eq!(
            l.fmt("il.event.gold", &[("amount", &120), ("city", &"Tarentum")]),
            "Coastal trade brings 120 gold to Tarentum."
        );
        assert_eq!(
            l.fmt("il.event.gold", &[("amount", &1)]),
            "Coastal trade brings 1 gold to {city}."
        );
        assert_eq!(l.fmt("nope.key", &[]), "nope.key");
    }

    #[test]
    fn show_keys_returns_keys() {
        let l = locale();
        l.set_show_keys(true);
        assert_eq!(l.get("rome.units.hastati.name"), "rome.units.hastati.name");
        l.set_show_keys(false);
        assert_eq!(l.get("rome.units.hastati.name"), "Hastati");
    }

    #[test]
    fn flattening_and_non_string_leaves() {
        let src = r#"{ rome: { mod: { name: "Rome" }, units: { hastati: { name: "Hastati" } } }, bad: { n: 5 } }"#;
        let v = parse_json5(src, FileId(0)).unwrap();
        let mut out = Vec::new();
        let mut diags = Diagnostics::new();
        flatten(
            &v,
            "",
            &mut out,
            Path::new("rome/locale/en.json5"),
            &mut diags,
        );
        assert_eq!(
            out,
            vec![
                ("rome.mod.name".to_string(), "Rome".to_string()),
                ("rome.units.hastati.name".to_string(), "Hastati".to_string())
            ]
        );
        assert_eq!(diags.len(), 1);
        assert_eq!(diags.0[0].field, "bad.n");
    }
}
