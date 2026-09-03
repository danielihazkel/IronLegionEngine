//! Schema validation of parsed objects with positioned diagnostics
//! (T1-021, REQ-MOD-007, Modding SDK §3.6).
//!
//! Every error found by the validator is mapped back to the span of the
//! offending key or value in the source file; a missing required field points
//! at the object that lacks it (`<root>` at the object's brace when it is the
//! document itself). Errors never stop at the first one. For merged objects
//! (T1-022) the key span names the file that first defined the field and the
//! value span the mod that last wrote it, giving the "after merge by" form.

use std::path::Path;

use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::json5::{PathSeg, Span, SpannedValue, ValueKind};
use crate::merge::MergedItem;
use crate::schema::{KindTag, describe, schema};
use crate::source::Sources;

/// Splits a JSON pointer (`/ranged/accuracy`, `/abilities/1`) into segments.
fn pointer_segments(pointer: &str) -> Vec<String> {
    pointer
        .split('/')
        .skip(1)
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect()
}

/// Walks `value` along the pointer segments, choosing array indices where the
/// current node is an array, and returns the deepest node reached plus how
/// many segments matched.
fn locate<'a>(value: &'a SpannedValue, segments: &[String]) -> (&'a SpannedValue, usize) {
    let mut cur = value;
    for (i, seg) in segments.iter().enumerate() {
        let next = match &cur.kind {
            ValueKind::Array(_) => seg
                .parse::<usize>()
                .ok()
                .and_then(|idx| cur.at_path(&[PathSeg::Index(idx)])),
            ValueKind::Object(_) => cur.get(seg),
            _ => None,
        };
        match next {
            Some(n) => cur = n,
            None => return (cur, i),
        }
    }
    (cur, segments.len())
}

/// `ranged.accuracy`, `abilities[1]`, or `<root>`.
fn field_path(segments: &[String], value: &SpannedValue) -> String {
    if segments.is_empty() {
        return "<root>".to_string();
    }
    let mut out = String::new();
    let mut cur = Some(value);
    for seg in segments {
        let is_index = matches!(cur.map(|c| &c.kind), Some(ValueKind::Array(_)));
        if is_index {
            out.push('[');
            out.push_str(seg);
            out.push(']');
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(seg);
        }
        cur = cur.and_then(|c| match &c.kind {
            ValueKind::Array(items) => seg.parse::<usize>().ok().and_then(|i| items.get(i)),
            ValueKind::Object(_) => c.get(seg),
            _ => None,
        });
    }
    out
}

/// Span of the key that holds the node at `segments` (the last object key on
/// the path), if the last segment is an object member.
fn parent_key_span(value: &SpannedValue, segments: &[String]) -> Option<Span> {
    let (parent_segs, last) = segments.split_at(segments.len() - 1);
    let (parent, matched) = locate(value, parent_segs);
    if matched != parent_segs.len() {
        return None;
    }
    match &parent.kind {
        ValueKind::Object(_) => parent.key_span(&last[0]),
        ValueKind::Array(items) => last[0]
            .parse::<usize>()
            .ok()
            .and_then(|i| items.get(i))
            .map(|v| v.span),
        _ => None,
    }
}

/// One schema problem located in the source tree.
struct Found {
    /// Where the diagnostic points: the field's key (first definition).
    primary: Span,
    /// Where the offending value was written (last writer).
    writer: Span,
    field: String,
    message: String,
    expected: Option<String>,
}

fn collect(kind: KindTag, value: &SpannedValue) -> Vec<Found> {
    let instance = value.to_json();
    let mut found: Vec<Found> = Vec::new();
    for err in schema(kind).validator.iter_errors(&instance) {
        let segments = pointer_segments(err.instance_path().as_str());
        let seg_refs: Vec<&str> = segments.iter().map(String::as_str).collect();
        let (node, matched) = locate(value, &segments);
        let base_field = field_path(&segments[..matched], value);
        for d in describe(kind, &seg_refs, &err) {
            let (primary, writer, field) = match (&d.key, d.index) {
                // Unknown field: point at the key itself.
                (Some(key), _) => {
                    let span = node.key_span(key).unwrap_or(node.span);
                    let field = if base_field == "<root>" {
                        key.clone()
                    } else {
                        format!("{base_field}.{key}")
                    };
                    (span, span, field)
                }
                // Bad list element: point at the element.
                (None, Some(i)) => {
                    let span = node
                        .as_array()
                        .and_then(|a| a.get(i))
                        .map_or(node.span, |v| v.span);
                    (span, span, format!("{base_field}[{i}]"))
                }
                (None, None) => {
                    // The key of a leaf field, so the column lands on the
                    // name as the SDK examples show; objects and arrays point
                    // at their own opening bracket.
                    let primary = if matched == segments.len() && matched > 0 {
                        parent_key_span(value, &segments).unwrap_or(node.span)
                    } else {
                        node.span
                    };
                    (primary, node.span, base_field.clone())
                }
            };
            found.push(Found {
                primary,
                writer,
                field,
                message: d.message,
                expected: d.expected,
            });
        }
    }
    found.sort_by(|a, b| {
        (a.primary.line, a.primary.col, &a.field).cmp(&(b.primary.line, b.primary.col, &b.field))
    });
    found.dedup_by(|a, b| a.primary == b.primary && a.field == b.field && a.message == b.message);
    found
}

fn to_diagnostic(f: Found, file: &Path, message: String) -> Diagnostic {
    let mut diag = Diagnostic::file_level(file, message)
        .at(f.primary.line, f.primary.col)
        .field(f.field);
    if let Some(e) = f.expected {
        diag = diag.expected(e);
    }
    diag
}

/// Validates one parsed object against `kind`'s schema, appending one
/// diagnostic per problem (sorted by position). Returns `true` when valid.
/// `file` is the display path used in the diagnostics.
pub fn validate_value(
    kind: KindTag,
    value: &SpannedValue,
    file: &Path,
    diags: &mut Diagnostics,
) -> bool {
    let found = collect(kind, value);
    let ok = found.is_empty();
    for f in found {
        let message = f.message.clone();
        diags.push(to_diagnostic(f, file, message));
    }
    ok
}

/// Validates a merged item. Diagnostics name the file that defined the field;
/// when another mod wrote the offending value they add
/// `after merge by "<mod>" (<file>:<line>:<col>)` (Modding SDK §3.6).
pub fn validate_merged(
    kind: KindTag,
    item: &MergedItem,
    sources: &Sources,
    diags: &mut Diagnostics,
) -> bool {
    let found = collect(kind, &item.value);
    let ok = found.is_empty();
    for f in found {
        let primary_file = sources.get(f.primary.file);
        let writer_file = sources.get(f.writer.file);
        let message = if writer_file.mod_index != primary_file.mod_index {
            format!(
                "after merge by {:?} ({}:{}:{}): {}",
                writer_file.mod_id, writer_file.rel, f.writer.line, f.writer.col, f.message
            )
        } else {
            f.message.clone()
        };
        let file = sources.display(f.primary.file);
        diags.push(to_diagnostic(f, &file, message));
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json5::{FileId, parse_json5};

    const FILE: &str = "mymod/content/units/peltast.json5";

    fn run(kind: KindTag, src: &str) -> Diagnostics {
        let value = parse_json5(src, FileId(0)).unwrap();
        let mut diags = Diagnostics::new();
        validate_value(kind, &value, Path::new(FILE), &mut diags);
        diags
    }

    const VALID_UNIT: &str = r#"{
  id: "mymod:peltast",
  name_key: "mymod.units.peltast.name",
  category: "skirmisher",
  hp: 80, speed_walk: 1.8, speed_run: 4.5,
  attack: 25, defence: 20, damage: 25,
  formations: ["rome:loose"],
  sprite_set: "sprites/units/peltast",
  cost: 250, upkeep: 40,
}"#;

    #[test]
    fn a_valid_unit_has_no_diagnostics() {
        let diags = run(KindTag::Unit, VALID_UNIT);
        assert!(diags.is_empty(), "{diags}");
    }

    #[test]
    fn three_errors_report_three_diagnostics_with_correct_lines() {
        let src = "{\n  id: \"mymod:peltast\",\n  name_key: \"mymod.units.peltast.name\",\n  category: \"infantree\",\n  hp: 80, speed_walk: 1.8, speed_run: 4.5,\n  attack: 25, defence: 20, damage: 25,\n  armour: -2,\n  formations: [\"rome:loose\"],\n  sprite_set: \"sprites/units/peltast\",\n  amour: 1,\n  cost: 250, upkeep: 40,\n}";
        let diags = run(KindTag::Unit, src);
        let got: Vec<String> = diags.0.iter().map(ToString::to_string).collect();
        assert_eq!(
            got,
            vec![
                format!(
                    "{FILE}:4:3 category: value \"infantree\" is not allowed (expected one of [\"infantry\", \"cavalry\", \"ranged\", \"skirmisher\", \"general\", \"siege\"])"
                ),
                format!("{FILE}:7:3 armour: value -2 out of range (expected 0..=100)"),
                format!("{FILE}:10:3 amour: unknown field \"amour\"; did you mean \"armour\"?"),
            ]
        );
    }

    #[test]
    fn nested_field_out_of_range_matches_the_sdk_example() {
        let src = "{\n  id: \"mymod:peltast\",\n  name_key: \"mymod.units.peltast.name\",\n  category: \"skirmisher\",\n  hp: 80, speed_walk: 1.8, speed_run: 4.5,\n  attack: 25, defence: 20, damage: 25,\n  formations: [\"rome:loose\"],\n  sprite_set: \"sprites/units/peltast\",\n  cost: 250, upkeep: 40,\n  ranged: {\n    range: 40, projectile_speed: 20, reload_ticks: 80, ammo: 8, damage: 30,\n    accuracy: 1.4,\n  },\n}";
        let diags = run(KindTag::Unit, src);
        assert_eq!(
            diags.to_string().trim(),
            format!("{FILE}:12:5 ranged.accuracy: value 1.4 out of range (expected 0..=1)")
        );
    }

    #[test]
    fn missing_required_points_at_the_root() {
        let diags = run(KindTag::Faction, "{\n  id: \"mymod:thrace\",\n}");
        let first = diags.0[0].to_string();
        assert!(
            first.starts_with(&format!("{FILE}:1:1 <root>: missing required field \"")),
            "{first}"
        );
        assert!(
            diags
                .0
                .iter()
                .any(|d| d.message == "missing required field \"colour_primary\""
                    && d.expected.as_deref() == Some("a string matching ^#[0-9a-fA-F]{6}$")),
            "{diags}"
        );
    }

    #[test]
    fn wrong_types_and_array_items_are_positioned() {
        let src = "{\n  id: \"mymod:peltast\",\n  name_key: \"mymod.units.peltast.name\",\n  category: \"skirmisher\",\n  hp: \"lots\", speed_walk: 1.8, speed_run: 4.5,\n  attack: 25, defence: 20, damage: 25,\n  formations: [\"rome:loose\", 7],\n  sprite_set: \"sprites/units/peltast\",\n  cost: 250, upkeep: 40,\n}";
        let diags = run(KindTag::Unit, src);
        let got: Vec<String> = diags.0.iter().map(ToString::to_string).collect();
        assert!(
            got.iter().any(|g| g.starts_with(&format!(
                "{FILE}:5:3 hp: wrong type: found a string (expected"
            ))),
            "{got:?}"
        );
        assert!(
            got.iter().any(|g| g.starts_with(&format!(
                "{FILE}:7:30 formations[1]: wrong type: found an integer"
            ))),
            "{got:?}"
        );
    }

    #[test]
    fn manifests_validate_too() {
        let diags = run(
            KindTag::Manifest,
            "{\n  id: \"My Mod\",\n  name_key: \"x\",\n  version: \"1\",\n  engine_version: \"*\",\n}",
        );
        let lines: Vec<u32> = diags.0.iter().map(|d| d.line).collect();
        assert_eq!(lines, vec![2, 3, 4], "{diags}");
    }
}
