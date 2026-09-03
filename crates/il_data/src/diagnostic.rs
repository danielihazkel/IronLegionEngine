//! Load-time diagnostics (TDD §3.2, REQ-MOD-007, Modding SDK §3.6).
//! The loader collects every diagnostic before failing; it never stops at
//! the first error (SAD §9.2). Warnings never fail a load.

use core::fmt;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// One problem in one content file, `file:line:col field: message (expected ...)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub file: PathBuf,
    /// 1-based; `1` when the position is unknown.
    pub line: u32,
    /// 1-based; `1` when the position is unknown.
    pub col: u32,
    /// JSON path of the offending field, empty for file-level problems.
    pub field: String,
    pub message: String,
    pub expected: Option<String>,
}

impl Diagnostic {
    pub fn file_level(file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            file: file.into(),
            line: 1,
            col: 1,
            field: String::new(),
            message: message.into(),
            expected: None,
        }
    }

    #[must_use]
    pub fn at(mut self, line: u32, col: u32) -> Self {
        self.line = line;
        self.col = col;
        self
    }

    #[must_use]
    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.field = field.into();
        self
    }

    #[must_use]
    pub fn expected(mut self, expected: impl Into<String>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    #[must_use]
    pub fn warning(mut self) -> Self {
        self.severity = Severity::Warning;
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.severity == Severity::Warning {
            f.write_str("warning: ")?;
        }
        write!(f, "{}:{}:{}", self.file.display(), self.line, self.col)?;
        if !self.field.is_empty() {
            write!(f, " {}", self.field)?;
        }
        write!(f, ": {}", self.message)?;
        if let Some(e) = &self.expected {
            write!(f, " (expected {e})")?;
        }
        Ok(())
    }
}

/// Every diagnostic from one load, in file order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, d: Diagnostic) {
        self.0.push(d);
    }

    pub fn extend(&mut self, other: Diagnostics) {
        self.0.extend(other.0);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn has_errors(&self) -> bool {
        self.0.iter().any(Diagnostic::is_error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter().filter(|d| d.is_error())
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter().filter(|d| !d.is_error())
    }

    /// `Ok(value)` when there are no errors (warnings allowed), else `Err(self)`.
    pub fn into_result<T>(self, value: T) -> Result<T, Diagnostics> {
        if self.has_errors() {
            Err(self)
        } else {
            Ok(value)
        }
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for d in &self.0 {
            writeln!(f, "{d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_format() {
        let d = Diagnostic::file_level("units/x.json5", "wrong type")
            .at(4, 12)
            .field("speed_walk")
            .expected("number");
        assert_eq!(
            d.to_string(),
            "units/x.json5:4:12 speed_walk: wrong type (expected number)"
        );
        let d = Diagnostic::file_level("a.json5", "parse error");
        assert_eq!(d.to_string(), "a.json5:1:1: parse error");
        let w = Diagnostic::file_level("a.json5", "odd").warning();
        assert_eq!(w.to_string(), "warning: a.json5:1:1: odd");
        let mut ds = Diagnostics::new();
        assert!(ds.clone().into_result(1).is_ok());
        ds.push(w);
        assert!(ds.clone().into_result(1).is_ok(), "warnings do not fail");
        assert_eq!(ds.warnings().count(), 1);
        ds.push(d);
        assert!(ds.has_errors());
        assert!(ds.into_result(1).is_err());
    }
}
