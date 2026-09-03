//! Override, merge, list operations, `$from` and `$delete` (T1-022,
//! Modding SDK §3.3–§3.5, REQ-MOD-005).
//!
//! Content of one kind accumulates mod by mod into a map keyed by
//! `ContentId`. Every value keeps its spans: a merged leaf keeps the *key*
//! span of the mod that first defined the field and takes the *value* span of
//! the mod that last wrote it, which is exactly what the "after merge by"
//! diagnostic form needs (SDK §3.6). Directives never survive into the
//! accumulator, so the merged result is validated as plain content.

use std::collections::BTreeMap;

use crate::content_id::ContentId;
use crate::diagnostic::{Diagnostic, Diagnostics};
use crate::json5::{Key, Span, SpannedValue, ValueKind};
use crate::schema::KindTag;
use crate::source::Sources;
use crate::text::nearest;

/// Maximum `$from` chain length (SDK §4.1).
pub const MAX_FROM_DEPTH: u8 = 8;

/// One merged content object.
#[derive(Clone, Debug, PartialEq)]
pub struct MergedItem {
    pub id: ContentId,
    /// An object with directives stripped; leaf spans are provenance.
    pub value: SpannedValue,
    /// Object span of the definition that owns the item (first definition,
    /// or the last `$override: "replace"`).
    pub defined_at: Span,
    /// Length of the `$from` chain behind this item.
    pub from_depth: u8,
}

/// Where an item was deleted, for "deleted by mod X" reference diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct Tombstone {
    pub span: Span,
    pub mod_index: usize,
}

/// Accumulated content of one kind across the load order.
#[derive(Debug)]
pub struct KindAccumulator {
    pub kind: KindTag,
    pub items: BTreeMap<ContentId, MergedItem>,
    pub tombstones: BTreeMap<ContentId, Tombstone>,
}

/// The mod whose files are being applied.
pub struct ApplyCtx<'a> {
    pub mod_index: usize,
    pub mod_id: &'a str,
    /// Namespaces this mod may define new content in.
    pub namespaces: &'a [String],
    pub sources: &'a Sources,
}

#[derive(Clone, Debug, PartialEq)]
enum Directive {
    Merge,
    Replace,
    Delete,
    From(ContentId),
}

struct ParsedObject {
    id: ContentId,
    id_span: Span,
    span: Span,
    directive: Directive,
    /// Whether the object spelled out `$override` or `$delete`.
    explicit: bool,
    /// The object without `id`... no: with `id`, without directives.
    body: SpannedValue,
}

const LIST_OPS: [&str; 3] = ["$replace", "$remove", "$append"];

fn diag_at(ctx: &ApplyCtx<'_>, span: Span, message: impl Into<String>) -> Diagnostic {
    Diagnostic::file_level(ctx.sources.display(span.file), message).at(span.line, span.col)
}

fn location(ctx: &ApplyCtx<'_>, span: Span) -> String {
    let f = ctx.sources.get(span.file);
    format!("{}:{}:{}", f.rel, span.line, span.col)
}

/// The per-file checks: object shape, `id`, directive syntax.
fn parse_object(
    obj: SpannedValue,
    ctx: &ApplyCtx<'_>,
    diags: &mut Diagnostics,
) -> Option<ParsedObject> {
    let span = obj.span;
    if obj.as_object().is_none() {
        diags.push(diag_at(
            ctx,
            span,
            format!("expected an object, found {}", obj.type_name()),
        ));
        return None;
    }
    let (id, id_span) = match obj.get("id") {
        None => {
            diags.push(diag_at(ctx, span, "missing required field \"id\"").field("<root>"));
            return None;
        }
        Some(v) => match v.as_str().map(ContentId::new) {
            Some(Ok(id)) => (id, obj.key_span("id").unwrap_or(v.span)),
            Some(Err(e)) => {
                diags.push(diag_at(ctx, v.span, e.to_string()).field("id"));
                return None;
            }
            None => {
                diags.push(
                    diag_at(ctx, v.span, format!("wrong type: found {}", v.type_name()))
                        .field("id")
                        .expected("a string \"modid:item_id\""),
                );
                return None;
            }
        },
    };

    let mut body = obj;
    let mut directive = Directive::Merge;
    let mut explicit = false;
    let mut ok = true;
    let mut from: Option<(ContentId, Span)> = None;
    let mut delete: Option<Span> = None;
    let mut over: Option<(Directive, Span)> = None;
    let root_keys: Vec<Key> = body
        .as_object()
        .map(|e| e.iter().map(|(k, _)| k.clone()).collect())
        .unwrap_or_default();
    for key in root_keys.iter().filter(|k| k.name.starts_with('$')) {
        let (_, value) = body.remove(&key.name).expect("key exists");
        match key.name.as_str() {
            "$override" => match value.as_str() {
                Some("merge") => over = Some((Directive::Merge, key.span)),
                Some("replace") => over = Some((Directive::Replace, key.span)),
                _ => {
                    diags.push(
                        diag_at(ctx, value.span, "invalid $override value")
                            .field("$override")
                            .expected("\"merge\" or \"replace\""),
                    );
                    ok = false;
                }
            },
            "$delete" => {
                if value.as_bool() == Some(true) {
                    delete = Some(key.span);
                } else {
                    diags.push(
                        diag_at(ctx, value.span, "invalid $delete value")
                            .field("$delete")
                            .expected("true"),
                    );
                    ok = false;
                }
            }
            "$from" => match value.as_str().map(ContentId::new) {
                Some(Ok(base)) => from = Some((base, key.span)),
                _ => {
                    diags.push(
                        diag_at(ctx, value.span, "invalid $from value")
                            .field("$from")
                            .expected("a ContentId \"modid:item_id\""),
                    );
                    ok = false;
                }
            },
            other => {
                diags.push(
                    diag_at(ctx, key.span, format!("unknown directive {other:?}"))
                        .field(other)
                        .expected("$override, $delete or $from"),
                );
                ok = false;
            }
        }
    }
    if let Some(dspan) = delete {
        explicit = true;
        directive = Directive::Delete;
        let extra: Vec<&str> = body
            .as_object()
            .map(|e| {
                e.iter()
                    .map(|(k, _)| k.name.as_str())
                    .filter(|n| *n != "id")
                    .collect()
            })
            .unwrap_or_default();
        if !extra.is_empty() || over.is_some() || from.is_some() {
            diags.push(
                diag_at(ctx, dspan, "a $delete object may carry nothing but \"id\"")
                    .field("$delete"),
            );
            ok = false;
        }
    } else if let Some((base, fspan)) = from {
        if let Some((Directive::Replace, _)) = over {
            diags.push(
                diag_at(
                    ctx,
                    fspan,
                    "$from cannot be combined with $override: \"replace\"",
                )
                .field("$from"),
            );
            ok = false;
        }
        directive = Directive::From(base);
    } else if let Some((d, _)) = over {
        explicit = true;
        directive = d;
    }
    if !ok {
        return None;
    }
    // List-operation objects anywhere in the body must be well formed.
    if !check_list_ops(&body, ctx, diags) {
        return None;
    }
    Some(ParsedObject {
        id,
        id_span,
        span,
        directive,
        explicit,
        body,
    })
}

/// A `{ $append/$remove/$replace }` object, or an error, or `None` for a
/// plain object.
fn classify_list_op(v: &SpannedValue) -> Option<Result<(), String>> {
    let entries = v.as_object()?;
    let dollar: Vec<&str> = entries
        .iter()
        .map(|(k, _)| k.name.as_str())
        .filter(|n| n.starts_with('$'))
        .collect();
    if dollar.is_empty() {
        return None;
    }
    if dollar.len() != entries.len() {
        return Some(Err(
            "a list-operation object may only contain $append, $remove and $replace".to_string(),
        ));
    }
    for (k, val) in entries {
        if !LIST_OPS.contains(&k.name.as_str()) {
            return Some(Err(format!("unknown list operation {:?}", k.name)));
        }
        if val.as_array().is_none() {
            return Some(Err(format!(
                "{} takes an array, found {}",
                k.name,
                val.type_name()
            )));
        }
    }
    Some(Ok(()))
}

fn check_list_ops(v: &SpannedValue, ctx: &ApplyCtx<'_>, diags: &mut Diagnostics) -> bool {
    match &v.kind {
        ValueKind::Object(entries) => {
            if let Some(Err(msg)) = classify_list_op(v) {
                diags.push(diag_at(ctx, v.span, msg));
                return false;
            }
            entries
                .iter()
                .all(|(_, child)| check_list_ops(child, ctx, diags))
        }
        ValueKind::Array(items) => items.iter().all(|c| check_list_ops(c, ctx, diags)),
        _ => true,
    }
}

fn same_element(a: &SpannedValue, b: &SpannedValue) -> bool {
    match (a.get("id"), b.get("id")) {
        (Some(x), Some(y)) if a.as_object().is_some() && b.as_object().is_some() => {
            x.to_json() == y.to_json()
        }
        _ => a.to_json() == b.to_json(),
    }
}

/// Applies `$replace`, `$remove`, `$append` (in that order) from `op` to
/// `existing`. A missing list is empty; a non-list is an error.
fn apply_list_ops(
    existing: Option<&SpannedValue>,
    op: &SpannedValue,
    ctx: &ApplyCtx<'_>,
    diags: &mut Diagnostics,
) -> Option<SpannedValue> {
    let mut items: Vec<SpannedValue> = match existing {
        None => Vec::new(),
        Some(v) => match &v.kind {
            ValueKind::Array(items) => items.clone(),
            _ => {
                diags.push(diag_at(
                    ctx,
                    op.span,
                    format!("list operation on a field that holds {}", v.type_name()),
                ));
                return None;
            }
        },
    };
    for name in LIST_OPS {
        let Some(arg) = op.get(name) else { continue };
        let arg_items = arg.as_array().expect("checked by check_list_ops");
        match name {
            "$replace" => items = arg_items.to_vec(),
            "$remove" => items.retain(|e| !arg_items.iter().any(|r| same_element(e, r))),
            _ => items.extend(arg_items.iter().cloned()),
        }
    }
    Some(SpannedValue {
        span: op.span,
        kind: ValueKind::Array(items),
    })
}

/// SDK §3.4.1 deep merge of `over` into `existing`.
fn deep_merge(
    existing: &mut SpannedValue,
    over: SpannedValue,
    ctx: &ApplyCtx<'_>,
    diags: &mut Diagnostics,
) {
    let ValueKind::Object(over_entries) = over.kind else {
        *existing = over;
        return;
    };
    for (okey, oval) in over_entries {
        let is_list_op = matches!(classify_list_op(&oval), Some(Ok(())));
        let slot = existing
            .as_object_mut()
            .expect("deep_merge targets objects")
            .iter_mut()
            .find(|(k, _)| k.name == okey.name);
        match slot {
            Some((_, cur)) => {
                if is_list_op {
                    if let Some(list) = apply_list_ops(Some(cur), &oval, ctx, diags) {
                        *cur = list;
                    }
                } else if matches!(oval.kind, ValueKind::Null) {
                    existing.remove(&okey.name);
                } else if cur.as_object().is_some() && oval.as_object().is_some() {
                    deep_merge(cur, oval, ctx, diags);
                } else {
                    // Keep the original key span (first definition); the
                    // value span now names the last writer.
                    *cur = oval;
                }
            }
            None => {
                if is_list_op {
                    if let Some(list) = apply_list_ops(None, &oval, ctx, diags) {
                        existing.as_object_mut().expect("object").push((okey, list));
                    }
                } else if matches!(oval.kind, ValueKind::Null) {
                    // Resetting an absent field: nothing to do.
                } else if let Some(mut inner) = oval.as_object().map(|_| SpannedValue {
                    span: oval.span,
                    kind: ValueKind::Object(Vec::new()),
                }) {
                    // A new nested object may itself hold list operations.
                    deep_merge(&mut inner, oval, ctx, diags);
                    existing
                        .as_object_mut()
                        .expect("object")
                        .push((okey, inner));
                } else {
                    existing.as_object_mut().expect("object").push((okey, oval));
                }
            }
        }
    }
}

impl KindAccumulator {
    pub fn new(kind: KindTag) -> Self {
        Self {
            kind,
            items: BTreeMap::new(),
            tombstones: BTreeMap::new(),
        }
    }

    /// Applies every object of one mod, in the given (file, array) order.
    /// `$from` may point at an object later in the same mod: such bases are
    /// applied first.
    pub fn apply_mod(
        &mut self,
        objects: Vec<SpannedValue>,
        ctx: &ApplyCtx<'_>,
        diags: &mut Diagnostics,
    ) {
        let parsed: Vec<ParsedObject> = objects
            .into_iter()
            .filter_map(|o| parse_object(o, ctx, diags))
            .collect();

        // Duplicate ids within one mod: found up front, reported in source
        // order below so diagnostics read top to bottom.
        let mut first_seen: BTreeMap<ContentId, Span> = BTreeMap::new();
        let mut duplicate_of: Vec<Option<Span>> = Vec::with_capacity(parsed.len());
        for p in &parsed {
            match first_seen.get(&p.id) {
                Some(first) => duplicate_of.push(Some(*first)),
                None => {
                    first_seen.insert(p.id.clone(), p.id_span);
                    duplicate_of.push(None);
                }
            }
        }
        let keep: Vec<bool> = duplicate_of.iter().map(Option::is_none).collect();

        #[derive(Clone, Copy, PartialEq)]
        enum State {
            Pending,
            Visiting,
            Done,
        }
        let mut state: Vec<State> = keep
            .iter()
            .map(|k| if *k { State::Pending } else { State::Done })
            .collect();
        let index_of = |id: &ContentId| parsed.iter().position(|p| &p.id == id);

        fn apply_one(
            acc: &mut KindAccumulator,
            parsed: &[ParsedObject],
            state: &mut [State],
            i: usize,
            ctx: &ApplyCtx<'_>,
            diags: &mut Diagnostics,
            index_of: &dyn Fn(&ContentId) -> Option<usize>,
        ) {
            if state[i] != State::Pending {
                return;
            }
            state[i] = State::Visiting;
            let p = &parsed[i];
            if let Directive::From(base) = &p.directive
                && let Some(j) = index_of(base)
                && j != i
            {
                match state[j] {
                    State::Pending => apply_one(acc, parsed, state, j, ctx, diags, index_of),
                    State::Visiting => {
                        diags.push(
                            diag_at(
                                ctx,
                                p.span,
                                format!("$from cycle: {} -> {} -> {}", p.id, base, p.id),
                            )
                            .field("$from"),
                        );
                        state[i] = State::Done;
                        return;
                    }
                    State::Done => {}
                }
            }
            acc.apply_object(p, ctx, diags);
            state[i] = State::Done;
        }

        for i in 0..parsed.len() {
            if let Some(first) = duplicate_of[i] {
                let p = &parsed[i];
                diags.push(
                    diag_at(
                        ctx,
                        p.id_span,
                        format!(
                            "duplicate {:?} (first defined in {})",
                            p.id.as_str(),
                            location(ctx, first)
                        ),
                    )
                    .field("id"),
                );
                continue;
            }
            apply_one(self, &parsed, &mut state, i, ctx, diags, &index_of);
        }
    }

    fn apply_object(&mut self, p: &ParsedObject, ctx: &ApplyCtx<'_>, diags: &mut Diagnostics) {
        let own = ctx.namespaces.iter().any(|ns| ns == p.id.namespace());
        let exists = self.items.contains_key(&p.id);
        match &p.directive {
            Directive::Delete => {
                if self.items.remove(&p.id).is_some() {
                    self.tombstones.insert(
                        p.id.clone(),
                        Tombstone {
                            span: p.span,
                            mod_index: ctx.mod_index,
                        },
                    );
                } else {
                    diags.push(
                        diag_at(
                            ctx,
                            p.id_span,
                            format!(
                                "{:?} is not defined by any enabled mod; the $delete is ignored",
                                p.id.as_str()
                            ),
                        )
                        .field("id")
                        .warning(),
                    );
                }
            }
            Directive::Replace => {
                if !exists && !own {
                    self.push_namespace_problem(p, ctx, diags);
                    return;
                }
                self.items.insert(
                    p.id.clone(),
                    MergedItem {
                        id: p.id.clone(),
                        value: fresh_object(&p.body, ctx, diags),
                        defined_at: p.span,
                        from_depth: 0,
                    },
                );
            }
            Directive::Merge => {
                if let Some(item) = self.items.get_mut(&p.id) {
                    deep_merge(&mut item.value, p.body.clone(), ctx, diags);
                } else if own {
                    self.items.insert(
                        p.id.clone(),
                        MergedItem {
                            id: p.id.clone(),
                            value: fresh_object(&p.body, ctx, diags),
                            defined_at: p.span,
                            from_depth: 0,
                        },
                    );
                } else {
                    self.push_namespace_problem(p, ctx, diags);
                }
            }
            Directive::From(base_id) => {
                if exists {
                    diags.push(
                        diag_at(ctx, p.id_span, format!("{:?} already exists; use $override to change it, $from only derives new content", p.id.as_str()))
                            .field("$from"),
                    );
                    return;
                }
                if !own {
                    self.push_namespace_problem(p, ctx, diags);
                    return;
                }
                let Some(base) = self.items.get(base_id) else {
                    let hint = nearest(base_id.as_str(), self.items.keys().map(ContentId::as_str))
                        .map_or(String::new(), |n| format!("; nearest: {n:?}"));
                    let deleted = self
                        .tombstones
                        .get(base_id)
                        .map(|t| {
                            format!(
                                "; deleted by {} ({})",
                                mod_name(ctx, t.mod_index),
                                location(ctx, t.span)
                            )
                        })
                        .unwrap_or_default();
                    diags.push(
                        diag_at(
                            ctx,
                            p.span,
                            format!(
                                "$from: unknown {} {:?}{hint}{deleted}",
                                self.kind.label(),
                                base_id.as_str()
                            ),
                        )
                        .field("$from")
                        .expected(format!("an existing {} ContentId", self.kind.label())),
                    );
                    return;
                };
                let depth = base.from_depth + 1;
                if depth > MAX_FROM_DEPTH {
                    diags.push(
                        diag_at(
                            ctx,
                            p.span,
                            format!("$from chain deeper than {MAX_FROM_DEPTH}"),
                        )
                        .field("$from"),
                    );
                    return;
                }
                let mut value = base.value.clone();
                // The derived item is its own object: the id must be replaced,
                // not inherited, and the rest merges on top of the copy.
                deep_merge(&mut value, p.body.clone(), ctx, diags);
                self.items.insert(
                    p.id.clone(),
                    MergedItem {
                        id: p.id.clone(),
                        value,
                        defined_at: p.span,
                        from_depth: depth,
                    },
                );
            }
        }
    }

    fn push_namespace_problem(
        &self,
        p: &ParsedObject,
        ctx: &ApplyCtx<'_>,
        diags: &mut Diagnostics,
    ) {
        if p.explicit {
            diags.push(
                diag_at(
                    ctx,
                    p.id_span,
                    format!(
                        "{:?} is not defined by any enabled mod; the override is ignored",
                        p.id.as_str()
                    ),
                )
                .field("id")
                .warning(),
            );
        } else {
            diags.push(
                diag_at(
                    ctx,
                    p.id_span,
                    format!(
                        "{:?} is not defined by mod {:?}; new content must use the {:?} namespace",
                        p.id.as_str(),
                        p.id.namespace(),
                        format!("{}:", ctx.mod_id)
                    ),
                )
                .field("id"),
            );
        }
    }
}

/// Merges one mod's singleton object (a rules file or the bindings file)
/// into the accumulated one: the first mod's object is the base, later mods
/// deep-merge. Directives other than list operations are errors.
pub fn merge_singleton(
    existing: &mut Option<SpannedValue>,
    obj: SpannedValue,
    ctx: &ApplyCtx<'_>,
    diags: &mut Diagnostics,
) {
    if obj.as_object().is_none() {
        diags.push(diag_at(
            ctx,
            obj.span,
            format!("expected an object, found {}", obj.type_name()),
        ));
        return;
    }
    let mut ok = true;
    for key in ["$override", "$delete", "$from", "id"] {
        if let Some(span) = obj.key_span(key) {
            diags.push(
                diag_at(ctx, span, format!("{key:?} is not allowed in a rules or bindings file; the whole file is one merged object"))
                    .field(key),
            );
            ok = false;
        }
    }
    if !ok || !check_list_ops(&obj, ctx, diags) {
        return;
    }
    match existing {
        Some(base) => deep_merge(base, obj, ctx, diags),
        None => *existing = Some(fresh_object(&obj, ctx, diags)),
    }
}

fn mod_name(ctx: &ApplyCtx<'_>, mod_index: usize) -> String {
    if mod_index == ctx.mod_index {
        format!("{:?}", ctx.mod_id)
    } else {
        format!("mod #{mod_index}")
    }
}

/// A new definition: list operations inside it apply against empty lists.
fn fresh_object(body: &SpannedValue, ctx: &ApplyCtx<'_>, diags: &mut Diagnostics) -> SpannedValue {
    let mut value = SpannedValue {
        span: body.span,
        kind: ValueKind::Object(Vec::new()),
    };
    deep_merge(&mut value, body.clone(), ctx, diags);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json5::{FileId, parse_json5};
    use std::path::Path;

    struct Fixture {
        sources: Sources,
    }

    impl Fixture {
        fn new(mods: &[&str]) -> Self {
            let mut sources = Sources::new();
            for (i, m) in mods.iter().enumerate() {
                sources.add(
                    i,
                    m,
                    Path::new("content/units/a.json5"),
                    Path::new(&format!("/mods/{m}/content/units/a.json5")),
                );
            }
            Self { sources }
        }

        fn apply(
            &self,
            acc: &mut KindAccumulator,
            mod_index: usize,
            mod_id: &str,
            namespaces: &[&str],
            src: &str,
            diags: &mut Diagnostics,
        ) {
            let value = parse_json5(src, FileId(mod_index as u32)).unwrap();
            let objects = match value.kind {
                ValueKind::Array(items) => items,
                _ => vec![value],
            };
            let ns: Vec<String> = namespaces.iter().map(|s| (*s).to_string()).collect();
            let ctx = ApplyCtx {
                mod_index,
                mod_id,
                namespaces: &ns,
                sources: &self.sources,
            };
            acc.apply_mod(objects, &ctx, diags);
        }
    }

    fn json(acc: &KindAccumulator, id: &str) -> serde_json::Value {
        acc.items[&ContentId::new(id).unwrap()].value.to_json()
    }

    #[test]
    fn merge_replace_delete_and_list_ops() {
        let fx = Fixture::new(&["rome", "mymod"]);
        let mut acc = KindAccumulator::new(KindTag::Unit);
        let mut diags = Diagnostics::new();
        fx.apply(&mut acc, 0, "rome", &["rome"], r#"[
            { id: "rome:a", hp: 10, ranged: { accuracy: 0.5, range: 40 }, formations: ["rome:line", "rome:loose"], tags: [{ id: "x", v: 1 }, { id: "y", v: 2 }] },
            { id: "rome:b", hp: 5 },
            { id: "rome:c", hp: 7 },
        ]"#, &mut diags);
        fx.apply(&mut acc, 1, "mymod", &["mymod"], r#"[
            { id: "rome:a", hp: 11, ranged: { accuracy: 0.6 }, formations: { $append: ["rome:column"], $remove: ["rome:loose"] }, tags: { $remove: [{ id: "x" }] }, extra: { $append: [1] } },
            { $override: "replace", id: "rome:b", hp: 99 },
            { id: "rome:c", $delete: true },
            { id: "rome:zzz", $delete: true },
        ]"#, &mut diags);
        assert_eq!(diags.errors().count(), 0, "{diags}");
        assert_eq!(
            diags.warnings().count(),
            1,
            "deleting an unknown id warns: {diags}"
        );
        assert_eq!(
            json(&acc, "rome:a"),
            serde_json::json!({ "id": "rome:a", "hp": 11, "ranged": { "accuracy": 0.6, "range": 40 },
                "formations": ["rome:line", "rome:column"], "tags": [{ "id": "y", "v": 2 }], "extra": [1] })
        );
        assert_eq!(
            json(&acc, "rome:b"),
            serde_json::json!({ "id": "rome:b", "hp": 99 })
        );
        assert!(!acc.items.contains_key(&ContentId::new("rome:c").unwrap()));
        assert!(
            acc.tombstones
                .contains_key(&ContentId::new("rome:c").unwrap())
        );

        // Provenance: hp's key span is rome's (file 0), its value span mymod's (file 1).
        let a = &acc.items[&ContentId::new("rome:a").unwrap()];
        assert_eq!(a.value.key_span("hp").unwrap().file, FileId(0));
        assert_eq!(a.value.get("hp").unwrap().span.file, FileId(1));
        assert_eq!(a.value.key_span("extra").unwrap().file, FileId(1));
        assert_eq!(a.defined_at.file, FileId(0));
        let b = &acc.items[&ContentId::new("rome:b").unwrap()];
        assert_eq!(b.defined_at.file, FileId(1), "replace transfers ownership");
    }

    #[test]
    fn null_removes_and_plain_lists_replace() {
        let fx = Fixture::new(&["rome", "mymod"]);
        let mut acc = KindAccumulator::new(KindTag::Unit);
        let mut diags = Diagnostics::new();
        fx.apply(&mut acc, 0, "rome", &["rome"], r#"{ id: "rome:a", armour: 8, formations: ["rome:line"], sounds: { select: "s", move: "m" } }"#, &mut diags);
        fx.apply(
            &mut acc,
            1,
            "mymod",
            &["mymod"],
            r#"{ id: "rome:a", armour: null, formations: ["rome:loose"], sounds: { move: null } }"#,
            &mut diags,
        );
        assert!(diags.is_empty(), "{diags}");
        assert_eq!(
            json(&acc, "rome:a"),
            serde_json::json!({ "id": "rome:a", "formations": ["rome:loose"], "sounds": { "select": "s" } })
        );
    }

    #[test]
    fn from_derives_forward_references_and_limits_depth() {
        let fx = Fixture::new(&["rome", "mymod"]);
        let mut acc = KindAccumulator::new(KindTag::Unit);
        let mut diags = Diagnostics::new();
        fx.apply(&mut acc, 0, "rome", &["rome"], r#"{ id: "rome:velites", hp: 80, armour: 2, ranged: { accuracy: 0.5, damage: 30 }, abilities: [], formations: ["a", "b", "c"] }"#, &mut diags);
        fx.apply(&mut acc, 1, "mymod", &["mymod"], r#"[
            { id: "mymod:c", $from: "mymod:b", hp: 3 },
            { id: "mymod:b", $from: "mymod:a", hp: 2 },
            { id: "mymod:a", $from: "rome:velites", hp: 1, ranged: { damage: 14 }, abilities: { $append: ["mymod:x"] }, formations: { $replace: ["a"] } },
        ]"#, &mut diags);
        assert!(diags.is_empty(), "{diags}");
        assert_eq!(
            json(&acc, "mymod:a"),
            serde_json::json!({ "id": "mymod:a", "hp": 1, "armour": 2, "ranged": { "accuracy": 0.5, "damage": 14 }, "abilities": ["mymod:x"], "formations": ["a"] })
        );
        assert_eq!(json(&acc, "mymod:c")["hp"], 3);
        assert_eq!(json(&acc, "mymod:c")["armour"], 2);
        assert_eq!(acc.items[&ContentId::new("mymod:c").unwrap()].from_depth, 3);
        let base = &acc.items[&ContentId::new("rome:velites").unwrap()];
        assert_eq!(
            base.value.get("hp").unwrap().to_json(),
            80,
            "the base is untouched"
        );

        // Depth limit: chain of 9 derived items.
        let mut acc2 = KindAccumulator::new(KindTag::Unit);
        let mut diags2 = Diagnostics::new();
        fx.apply(
            &mut acc2,
            0,
            "rome",
            &["rome"],
            r#"{ id: "rome:base", hp: 1 }"#,
            &mut diags2,
        );
        let mut chain = String::from("[");
        let mut prev = "rome:base".to_string();
        for i in 0..9 {
            chain.push_str(&format!("{{ id: \"mymod:d{i}\", $from: \"{prev}\" }},"));
            prev = format!("mymod:d{i}");
        }
        chain.push(']');
        fx.apply(&mut acc2, 1, "mymod", &["mymod"], &chain, &mut diags2);
        assert_eq!(diags2.errors().count(), 1, "{diags2}");
        assert!(diags2.0[0].message.contains("deeper than 8"));
        assert!(
            acc2.items
                .contains_key(&ContentId::new("mymod:d7").unwrap())
        );
        assert!(
            !acc2
                .items
                .contains_key(&ContentId::new("mymod:d8").unwrap())
        );
    }

    #[test]
    fn from_errors_cycle_unknown_and_existing() {
        let fx = Fixture::new(&["rome", "mymod"]);
        let mut acc = KindAccumulator::new(KindTag::Unit);
        let mut diags = Diagnostics::new();
        fx.apply(
            &mut acc,
            0,
            "rome",
            &["rome"],
            r#"{ id: "rome:velites", hp: 80 }"#,
            &mut diags,
        );
        fx.apply(
            &mut acc,
            1,
            "mymod",
            &["mymod"],
            r#"[
            { id: "mymod:p", $from: "mymod:q" },
            { id: "mymod:q", $from: "mymod:p" },
            { id: "mymod:r", $from: "rome:velite" },
            { id: "rome:velites", $from: "rome:velites" },
        ]"#,
            &mut diags,
        );
        let msgs: Vec<&str> = diags.0.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.starts_with("$from cycle:")),
            "{diags}"
        );
        assert!(msgs.iter().any(|m| m.contains("unknown unit type \"rome:velite\"; nearest: \"rome:velites\"")), "{diags}");
        assert!(msgs.iter().any(|m| m.contains("already exists")), "{diags}");
    }

    #[test]
    fn namespace_rule_and_duplicate_ids() {
        let fx = Fixture::new(&["rome", "mymod"]);
        let mut acc = KindAccumulator::new(KindTag::Unit);
        let mut diags = Diagnostics::new();
        fx.apply(
            &mut acc,
            0,
            "rome",
            &["rome", "greece"],
            r#"[{ id: "greece:hoplite", hp: 1 }, { id: "rome:a", hp: 1 }]"#,
            &mut diags,
        );
        assert!(
            diags.is_empty(),
            "the game may use its declared namespaces: {diags}"
        );
        fx.apply(&mut acc, 1, "mymod", &["mymod"], "[\n  { id: \"rome:legionary_v2\", hp: 1 },\n  { id: \"rome:zz\", $override: \"merge\", hp: 1 },\n  { id: \"mymod:x\", hp: 1 },\n  { id: \"mymod:x\", hp: 2 },\n]", &mut diags);
        let errors: Vec<String> = diags.errors().map(ToString::to_string).collect();
        assert_eq!(errors.len(), 2, "{diags}");
        assert_eq!(
            errors[0],
            "mymod/content/units/a.json5:2:5 id: \"rome:legionary_v2\" is not defined by mod \"rome\"; new content must use the \"mymod:\" namespace"
        );
        assert_eq!(
            errors[1],
            "mymod/content/units/a.json5:5:5 id: duplicate \"mymod:x\" (first defined in content/units/a.json5:4:5)"
        );
        assert_eq!(
            diags.warnings().count(),
            1,
            "explicit override of an undefined id warns: {diags}"
        );
        assert_eq!(json(&acc, "mymod:x")["hp"], 1, "the duplicate is skipped");
    }

    #[test]
    fn directive_syntax_errors() {
        let fx = Fixture::new(&["mymod"]);
        let mut acc = KindAccumulator::new(KindTag::Unit);
        let mut diags = Diagnostics::new();
        fx.apply(
            &mut acc,
            0,
            "mymod",
            &["mymod"],
            r#"[
            { id: "mymod:a", $override: "sometimes" },
            { id: "mymod:b", $delete: true, hp: 1 },
            { id: "mymod:c", $bogus: 1 },
            { id: "mymod:d", formations: { $append: 1 } },
            { id: "mymod:e", formations: { $append: [], extra: 1 } },
            { id: 12 },
            { hp: 1 },
            { id: "mymod:f", $from: "mymod:g", $override: "replace" },
        ]"#,
            &mut diags,
        );
        assert_eq!(diags.errors().count(), 8, "{diags}");
        assert!(acc.items.is_empty());
    }
}
