//! `ContentId`: an interned `"modid:item_id"` string (TDD §3.2).

use core::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize};

/// A namespaced content id, `^[a-z0-9_]+:[a-z0-9_]+$` (schemas `contentId`).
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ContentId(Arc<str>);

/// Error for a string that is not a valid content id.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid content id {0:?}: expected \"modid:item_id\" using [a-z0-9_]")]
pub struct InvalidContentId(pub String);

fn is_segment(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl ContentId {
    /// Validates and interns `s`.
    pub fn new(s: &str) -> Result<Self, InvalidContentId> {
        match s.split_once(':') {
            Some((ns, item)) if is_segment(ns) && is_segment(item) => Ok(Self(Arc::from(s))),
            _ => Err(InvalidContentId(s.to_string())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part before the colon (the mod id or, inside the flagship game, a
    /// culture namespace such as `greece`).
    pub fn namespace(&self) -> &str {
        self.0.split_once(':').map_or("", |(ns, _)| ns)
    }

    /// The part after the colon.
    pub fn item(&self) -> &str {
        self.0.split_once(':').map_or("", |(_, item)| item)
    }
}

impl fmt::Debug for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentId({:?})", &*self.0)
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ContentId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ContentId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        ContentId::new(&s).map_err(serde::de::Error::custom)
    }
}

impl core::str::FromStr for ContentId {
    type Err = InvalidContentId;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_shape() {
        let id = ContentId::new("rome:hastati").unwrap();
        assert_eq!(id.namespace(), "rome");
        assert_eq!(id.item(), "hastati");
        assert_eq!(id.to_string(), "rome:hastati");
        for bad in [
            "hastati",
            "Rome:hastati",
            "rome:",
            ":x",
            "rome:has-tati",
            "a:b:c",
            "",
        ] {
            assert!(ContentId::new(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn serde_round_trip_validates() {
        let id: ContentId = serde_json::from_str("\"greece:hoplite\"").unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"greece:hoplite\"");
        assert!(serde_json::from_str::<ContentId>("\"bad id\"").is_err());
    }
}
