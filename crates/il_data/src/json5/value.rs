//! `SpannedValue`: a JSON5 document tree where every key and value keeps its
//! source position, so validation and merge diagnostics can point at the
//! file that wrote a field (Modding SDK §3.6).

use std::path::Path;

/// Index of a source file in the loader's file table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

/// A 1-based position in a source file; `col` counts characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub file: FileId,
    pub line: u32,
    pub col: u32,
}

/// An object key with the position of its first character.
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub name: String,
    pub span: Span,
}

/// A number as written: an integer when the lexeme has no fraction or
/// exponent and fits `i64` (hex included), otherwise a finite float.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    pub fn as_f64(self) -> f64 {
        match self {
            Num::Int(i) => i as f64,
            Num::Float(f) => f,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ValueKind {
    Null,
    Bool(bool),
    Number(Num),
    String(String),
    Array(Vec<SpannedValue>),
    /// Keys in source order; duplicates are a parse error.
    Object(Vec<(Key, SpannedValue)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpannedValue {
    pub span: Span,
    pub kind: ValueKind,
}

/// One step of a path into a document, for span lookups by instance path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathSeg<'a> {
    Key(&'a str),
    Index(usize),
}

impl SpannedValue {
    pub fn type_name(&self) -> &'static str {
        match self.kind {
            ValueKind::Null => "null",
            ValueKind::Bool(_) => "a boolean",
            ValueKind::Number(_) => "a number",
            ValueKind::String(_) => "a string",
            ValueKind::Array(_) => "an array",
            ValueKind::Object(_) => "an object",
        }
    }

    pub fn as_object(&self) -> Option<&[(Key, SpannedValue)]> {
        match &self.kind {
            ValueKind::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Vec<(Key, SpannedValue)>> {
        match &mut self.kind {
            ValueKind::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[SpannedValue]> {
        match &self.kind {
            ValueKind::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match &self.kind {
            ValueKind::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self.kind {
            ValueKind::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// Field `key` of an object.
    pub fn get(&self, key: &str) -> Option<&SpannedValue> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k.name == key)
            .map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut SpannedValue> {
        self.as_object_mut()?
            .iter_mut()
            .find(|(k, _)| k.name == key)
            .map(|(_, v)| v)
    }

    /// Position of the key `key` in an object.
    pub fn key_span(&self, key: &str) -> Option<Span> {
        self.as_object()?
            .iter()
            .find(|(k, _)| k.name == key)
            .map(|(k, _)| k.span)
    }

    /// Removes and returns field `key` of an object.
    pub fn remove(&mut self, key: &str) -> Option<(Key, SpannedValue)> {
        let entries = self.as_object_mut()?;
        let i = entries.iter().position(|(k, _)| k.name == key)?;
        Some(entries.remove(i))
    }

    /// Follows `path`; `None` if any step is missing or of the wrong shape.
    pub fn at_path(&self, path: &[PathSeg<'_>]) -> Option<&SpannedValue> {
        let mut cur = self;
        for seg in path {
            cur = match seg {
                PathSeg::Key(k) => cur.get(k)?,
                PathSeg::Index(i) => cur.as_array()?.get(*i)?,
            };
        }
        Some(cur)
    }

    /// The plain JSON value, positions dropped. Floats are finite by
    /// construction, so every number converts.
    pub fn to_json(&self) -> serde_json::Value {
        match &self.kind {
            ValueKind::Null => serde_json::Value::Null,
            ValueKind::Bool(b) => serde_json::Value::Bool(*b),
            ValueKind::Number(Num::Int(i)) => serde_json::Value::Number((*i).into()),
            ValueKind::Number(Num::Float(f)) => serde_json::Number::from_f64(*f)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            ValueKind::String(s) => serde_json::Value::String(s.clone()),
            ValueKind::Array(items) => {
                serde_json::Value::Array(items.iter().map(SpannedValue::to_json).collect())
            }
            ValueKind::Object(entries) => serde_json::Value::Object(
                entries
                    .iter()
                    .map(|(k, v)| (k.name.clone(), v.to_json()))
                    .collect(),
            ),
        }
    }
}

/// `file:line:col` for messages; `file` is whatever display path the caller
/// associates with the span's `FileId`.
pub fn span_display(file: &Path, span: Span) -> String {
    format!("{}:{}:{}", file.display(), span.line, span.col)
}
