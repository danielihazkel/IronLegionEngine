//! Embedded JSON Schemas (draft 2020-12) and error wording (T1-021,
//! REQ-MOD-007). `docs/schemas/*.json` is the source of truth; the files are
//! compiled into the binary with `include_str!` so a shipped engine and its
//! documentation can never disagree.

use std::sync::LazyLock;

use jsonschema::error::ValidationErrorKind;
use jsonschema::{Draft, ValidationError, Validator};

use crate::text::nearest;

/// The content kinds that have a schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KindTag {
    Manifest,
    Unit,
    Formation,
    GroupFormation,
    Faction,
    Zone,
    Map,
    SpriteSet,
    RulesMovement,
    RulesFormation,
    InputBindings,
}

impl KindTag {
    pub const ALL: [KindTag; 11] = [
        KindTag::Manifest,
        KindTag::Unit,
        KindTag::Formation,
        KindTag::GroupFormation,
        KindTag::Faction,
        KindTag::Zone,
        KindTag::Map,
        KindTag::SpriteSet,
        KindTag::RulesMovement,
        KindTag::RulesFormation,
        KindTag::InputBindings,
    ];

    /// Human name used in messages.
    pub fn label(self) -> &'static str {
        match self {
            KindTag::Manifest => "mod manifest",
            KindTag::Unit => "unit type",
            KindTag::Formation => "formation template",
            KindTag::GroupFormation => "group formation",
            KindTag::Faction => "faction",
            KindTag::Zone => "zone type",
            KindTag::Map => "map",
            KindTag::SpriteSet => "sprite set",
            KindTag::RulesMovement => "movement rules",
            KindTag::RulesFormation => "formation rules",
            KindTag::InputBindings => "input bindings",
        }
    }

    fn source(self) -> &'static str {
        match self {
            KindTag::Manifest => include_str!("../../../docs/schemas/mod-manifest.schema.json"),
            KindTag::Unit => include_str!("../../../docs/schemas/unit-type.schema.json"),
            KindTag::Formation => {
                include_str!("../../../docs/schemas/formation-template.schema.json")
            }
            KindTag::GroupFormation => {
                include_str!("../../../docs/schemas/group-formation.schema.json")
            }
            KindTag::Faction => include_str!("../../../docs/schemas/faction.schema.json"),
            KindTag::Zone => include_str!("../../../docs/schemas/zone-type.schema.json"),
            KindTag::Map => include_str!("../../../docs/schemas/map-def.schema.json"),
            KindTag::SpriteSet => include_str!("../../../docs/schemas/sprite-set.schema.json"),
            KindTag::RulesMovement => {
                include_str!("../../../docs/schemas/rules-movement.schema.json")
            }
            KindTag::RulesFormation => {
                include_str!("../../../docs/schemas/rules-formation.schema.json")
            }
            KindTag::InputBindings => {
                include_str!("../../../docs/schemas/input-bindings.schema.json")
            }
        }
    }
}

pub struct CompiledSchema {
    pub raw: serde_json::Value,
    pub validator: Validator,
}

fn build(raw: &serde_json::Value) -> Result<Validator, ValidationError<'static>> {
    jsonschema::options()
        .with_draft(Draft::Draft202012)
        .build(raw)
}

static SCHEMAS: LazyLock<Vec<CompiledSchema>> = LazyLock::new(|| {
    KindTag::ALL
        .iter()
        .map(|kind| {
            let raw: serde_json::Value = serde_json::from_str(kind.source()).unwrap_or_else(|e| {
                panic!("embedded schema for {} is not JSON: {e}", kind.label())
            });
            let validator = build(&raw)
                .unwrap_or_else(|e| panic!("embedded schema for {} is invalid: {e}", kind.label()));
            CompiledSchema { raw, validator }
        })
        .collect()
});

/// The compiled schema of a kind.
pub fn schema(kind: KindTag) -> &'static CompiledSchema {
    &SCHEMAS[kind as usize]
}

/// Resolves a local `$ref` (`#/$defs/x`) inside `root`.
fn deref<'a>(root: &'a serde_json::Value, node: &'a serde_json::Value) -> &'a serde_json::Value {
    if let Some(r) = node.get("$ref").and_then(|r| r.as_str())
        && let Some(path) = r.strip_prefix("#/")
    {
        let mut cur = root;
        for seg in path.split('/') {
            match cur.get(seg) {
                Some(next) => cur = next,
                None => return node,
            }
        }
        return cur;
    }
    node
}

/// The schema node describing the instance at `path` (JSON pointer segments),
/// following `properties`, `items`, local `$ref`s and the array branch of a
/// `oneOf [array, listOp]` list field.
pub fn node_at<'a>(kind: KindTag, path: &[&str]) -> Option<&'a serde_json::Value> {
    let root = &schema(kind).raw;
    let mut cur = deref(root, root);
    for seg in path {
        let next = if seg.parse::<usize>().is_ok() && cur.get("items").is_some() {
            cur.get("items")?
        } else if let Some(p) = cur.get("properties").and_then(|p| p.get(seg)) {
            p
        } else {
            let one_of = cur.get("oneOf").and_then(|v| v.as_array())?;
            let arr = one_of
                .iter()
                .map(|b| deref(root, b))
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("array"))?;
            arr.get("items")?
        };
        cur = deref(root, next);
    }
    Some(cur)
}

/// Property names accepted by the object at `path`.
pub fn property_names(kind: KindTag, path: &[&str]) -> Vec<&'static str> {
    node_at(kind, path)
        .and_then(|n| n.get("properties"))
        .and_then(|p| p.as_object())
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

fn fmt_number(v: &serde_json::Value) -> String {
    match v.as_f64() {
        Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
        Some(f) => format!("{f}"),
        None => v.to_string(),
    }
}

/// What a schema node accepts, for the `(expected ...)` suffix.
pub fn expected_of(node: Option<&serde_json::Value>) -> Option<String> {
    let node = node?;
    if let Some(e) = node.get("enum").and_then(|e| e.as_array()) {
        let opts: Vec<String> = e.iter().map(ToString::to_string).collect();
        return Some(format!("one of [{}]", opts.join(", ")));
    }
    if let Some(c) = node.get("const") {
        return Some(format!("exactly {c}"));
    }
    let ty = node.get("type").and_then(|t| t.as_str());
    let min = node
        .get("minimum")
        .map(|v| (fmt_number(v), true))
        .or_else(|| node.get("exclusiveMinimum").map(|v| (fmt_number(v), false)));
    let max = node
        .get("maximum")
        .map(|v| (fmt_number(v), true))
        .or_else(|| node.get("exclusiveMaximum").map(|v| (fmt_number(v), false)));
    match (min, max) {
        (Some((lo, lo_inc)), Some((hi, hi_inc))) => {
            return Some(format!(
                "{}{}..{}{}",
                if lo_inc { "" } else { ">" },
                lo,
                if hi_inc { "=" } else { "" },
                hi
            ));
        }
        (Some((lo, inc)), None) => return Some(format!("{} {lo}", if inc { ">=" } else { ">" })),
        (None, Some((hi, inc))) => return Some(format!("{} {hi}", if inc { "<=" } else { "<" })),
        (None, None) => {}
    }
    if let Some(p) = node.get("pattern").and_then(|p| p.as_str()) {
        return Some(format!("a string matching {p}"));
    }
    if let Some(d) = node.get("description").and_then(|d| d.as_str())
        && ty.is_none()
    {
        return Some(d.to_string());
    }
    ty.map(|t| match t {
        "integer" => "an integer".to_string(),
        "number" => "a number".to_string(),
        "string" => "a string".to_string(),
        "boolean" => "a boolean".to_string(),
        "array" => "an array".to_string(),
        "object" => "an object".to_string(),
        other => other.to_string(),
    })
}

/// A described validation error: where it points inside the instance (an
/// unknown key, or a list element), the message and the expected text.
pub struct Described {
    /// For `additionalProperties`: the offending key name.
    pub key: Option<String>,
    /// For an invalid element inside a list: its index.
    pub index: Option<usize>,
    pub message: String,
    pub expected: Option<String>,
}

impl Described {
    fn new(message: String, expected: Option<String>) -> Self {
        Self {
            key: None,
            index: None,
            message,
            expected,
        }
    }
}

/// Turns one validation error into user-facing wording. `path` is the
/// instance path as JSON pointer segments.
pub fn describe(kind: KindTag, path: &[&str], err: &ValidationError<'_>) -> Vec<Described> {
    let node = node_at(kind, path);
    describe_node(kind, path, node, err)
}

/// A list field may be `oneOf [array, listOp]`; the validator then reports
/// the whole list. Re-check each element against the array branch's item
/// schema so the diagnostic lands on the bad element.
fn describe_list_items(
    kind: KindTag,
    path: &[&str],
    items: &[serde_json::Value],
) -> Option<Vec<Described>> {
    let mut item_path: Vec<&str> = path.to_vec();
    item_path.push("0");
    let item_schema = node_at(kind, &item_path)?;
    let validator = build(item_schema).ok()?;
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if let Some(e) = validator.iter_errors(item).next() {
            for mut d in describe_node(kind, &item_path, Some(item_schema), &e) {
                d.index = Some(i);
                out.push(d);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

fn describe_node(
    kind: KindTag,
    path: &[&str],
    node: Option<&serde_json::Value>,
    err: &ValidationError<'_>,
) -> Vec<Described> {
    let instance = err.instance();
    match err.kind() {
        ValidationErrorKind::Minimum { .. }
        | ValidationErrorKind::Maximum { .. }
        | ValidationErrorKind::ExclusiveMinimum { .. }
        | ValidationErrorKind::ExclusiveMaximum { .. } => vec![Described::new(
            format!("value {} out of range", fmt_number(instance)),
            expected_of(node),
        )],
        ValidationErrorKind::Required { property } => {
            let name = property.as_str().unwrap_or("?");
            let mut sub: Vec<&str> = path.to_vec();
            sub.push(name);
            vec![Described::new(
                format!("missing required field {name:?}"),
                expected_of(node_at(kind, &sub)),
            )]
        }
        ValidationErrorKind::AdditionalProperties { unexpected } => unexpected
            .iter()
            .map(|name| {
                let names = property_names(kind, path);
                let hint = nearest(name, names.iter().copied())
                    .map_or(String::new(), |n| format!("; did you mean {n:?}?"));
                Described {
                    key: Some(name.to_string()),
                    index: None,
                    message: format!("unknown field {name:?}{hint}"),
                    expected: None,
                }
            })
            .collect(),
        ValidationErrorKind::Enum { options } => {
            let opts = options
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            vec![Described::new(
                format!("value {instance} is not allowed"),
                Some(format!("one of [{opts}]")),
            )]
        }
        ValidationErrorKind::Constant { expected_value } => vec![Described::new(
            format!("value {instance} is not allowed"),
            Some(format!("exactly {expected_value}")),
        )],
        ValidationErrorKind::Type { .. } => vec![Described::new(
            format!("wrong type: found {}", json_type_name(instance)),
            expected_of(node),
        )],
        ValidationErrorKind::Pattern { pattern } => vec![Described::new(
            format!("value {instance} does not match the required pattern"),
            Some(format!("a string matching {pattern}")),
        )],
        ValidationErrorKind::MinLength { limit } => vec![Described::new(
            "string too short".to_string(),
            Some(format!("at least {limit} characters")),
        )],
        ValidationErrorKind::MaxLength { limit } => vec![Described::new(
            "string too long".to_string(),
            Some(format!("at most {limit} characters")),
        )],
        ValidationErrorKind::MinItems { limit } => vec![Described::new(
            "too few items".to_string(),
            Some(format!("at least {limit}")),
        )],
        ValidationErrorKind::MaxItems { limit } => vec![Described::new(
            "too many items".to_string(),
            Some(format!("at most {limit}")),
        )],
        ValidationErrorKind::OneOfNotValid { .. } | ValidationErrorKind::AnyOf { .. } => {
            if let Some(items) = instance.as_array()
                && let Some(described) = describe_list_items(kind, path, items)
            {
                return described;
            }
            vec![Described::new(
                format!("value is not valid for this field: {err}"),
                expected_of(node),
            )]
        }
        _ => vec![Described::new(err.to_string(), None)],
    }
}

fn json_type_name(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => "an integer",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_schema_compiles() {
        for kind in KindTag::ALL {
            assert!(
                schema(kind).raw.get("properties").is_some(),
                "{}",
                kind.label()
            );
        }
    }

    #[test]
    fn node_lookup_follows_refs_items_and_one_of() {
        let acc = node_at(KindTag::Unit, &["ranged", "accuracy"]).unwrap();
        assert_eq!(expected_of(Some(acc)).as_deref(), Some("0..=1"));
        let ability = node_at(KindTag::Unit, &["abilities", "1"]).unwrap();
        assert!(
            ability.get("pattern").is_some(),
            "contentId def via oneOf/array/items/$ref"
        );
        let tier_xp = node_at(KindTag::Unit, &["experience_tiers", "0", "xp"]).unwrap();
        assert_eq!(expected_of(Some(tier_xp)).as_deref(), Some(">= 0"));
        assert!(property_names(KindTag::Unit, &[]).contains(&"armour"));
        assert!(property_names(KindTag::Unit, &["ranged"]).contains(&"accuracy"));
        assert_eq!(
            expected_of(node_at(KindTag::Unit, &["category"])).unwrap(),
            "one of [\"infantry\", \"cavalry\", \"ranged\", \"skirmisher\", \"general\", \"siege\"]"
        );
    }
}
