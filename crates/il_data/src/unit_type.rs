//! `UnitType`, Phase 0 subset (TDD §3.2, `docs/schemas/unit-type.schema.json`).
//!
//! Only the fields Phase 0 reads are typed; every other field in the file is
//! accepted and ignored until schema validation (T1-021) and the full struct
//! (T1-023) arrive. Numbers are stored as `S` straight from the data, which
//! is the `from_f32_data` conversion point for content.

use il_core::{S, Scalar, impl_hashable_fieldless_enum};
use serde::{Deserialize, Deserializer, Serialize};

use crate::content_id::ContentId;
use crate::registry::ContentKind;

/// Role category (schema `category`).
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum UnitCategory {
    Infantry = 0,
    Cavalry = 1,
    Ranged = 2,
    Skirmisher = 3,
    General = 4,
    Siege = 5,
}
impl_hashable_fieldless_enum!(UnitCategory);

#[derive(Clone, Debug, PartialEq)]
pub struct UnitType {
    pub id: ContentId,
    pub name_key: String,
    pub category: UnitCategory,
    /// Collision radius in world units (default 0.4).
    pub soldier_radius: S,
    /// Push weight (default 1.0).
    pub mass: S,
    pub hp: S,
    pub speed_walk: S,
    pub speed_run: S,
    /// Column and road march speed; defaults to `speed_walk`.
    pub speed_march: S,
    /// Default 60.
    pub morale_base: S,
    /// Default 80.
    pub los_radius: S,
}

/// The on-disk shape: optional fields become `Option` so defaults can be
/// applied after parsing. Unknown fields are ignored in Phase 0.
#[derive(Deserialize)]
struct Raw {
    id: ContentId,
    name_key: String,
    category: UnitCategory,
    soldier_radius: Option<f32>,
    mass: Option<f32>,
    hp: f32,
    speed_walk: f32,
    speed_run: f32,
    speed_march: Option<f32>,
    morale_base: Option<f32>,
    los_radius: Option<f32>,
}

impl<'de> Deserialize<'de> for UnitType {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = Raw::deserialize(d)?;
        let s = S::from_f32_data;
        Ok(UnitType {
            id: r.id,
            name_key: r.name_key,
            category: r.category,
            soldier_radius: s(r.soldier_radius.unwrap_or(0.4)),
            mass: s(r.mass.unwrap_or(1.0)),
            hp: s(r.hp),
            speed_walk: s(r.speed_walk),
            speed_run: s(r.speed_run),
            speed_march: s(r.speed_march.unwrap_or(r.speed_walk)),
            morale_base: s(r.morale_base.unwrap_or(60.0)),
            los_radius: s(r.los_radius.unwrap_or(80.0)),
        })
    }
}

impl ContentKind for UnitType {
    const DIR: &'static str = "units";
    const TAG: crate::schema::KindTag = crate::schema::KindTag::Unit;
    fn id(&self) -> &ContentId {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json5::{FileId, parse_json5};

    fn from_json5<T: serde::de::DeserializeOwned>(src: &str) -> Result<T, serde_json::Error> {
        serde_json::from_value(parse_json5(src, FileId(0)).unwrap().to_json())
    }

    #[test]
    fn defaults_and_unknown_fields() {
        let u: UnitType = from_json5(
            r#"{ id: "rome:x", name_key: "u.x", category: "cavalry", hp: 100,
                speed_walk: 1.5, speed_run: 4, attack: 30, weird_field: [1, 2] }"#,
        )
        .unwrap();
        let s = S::from_f32_data;
        assert_eq!(u.category, UnitCategory::Cavalry);
        assert_eq!(u.soldier_radius, s(0.4));
        assert_eq!(u.mass, s(1.0));
        assert_eq!(u.speed_march, s(1.5));
        assert_eq!(u.morale_base, s(60.0));
        assert_eq!(u.los_radius, s(80.0));
        assert_eq!(u.hp, s(100.0));
    }

    #[test]
    fn missing_required_field_fails() {
        let err =
            from_json5::<UnitType>(r#"{ id: "rome:x", name_key: "u.x", category: "infantry" }"#)
                .unwrap_err();
        assert!(err.to_string().contains("hp"), "{err}");
    }
}
