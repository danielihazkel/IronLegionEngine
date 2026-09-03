//! Serde helpers shared by the typed content structs: data floats enter the
//! simulation only through `Scalar::from_f32_data` (REQ-TECH-009), points are
//! `[x, y]` pairs, colours are `#rrggbb`.

use il_core::{S, Scalar, V2};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

pub fn de_s<'de, D: Deserializer<'de>>(d: D) -> Result<S, D::Error> {
    f32::deserialize(d).map(S::from_f32_data)
}

pub fn de_opt_s<'de, D: Deserializer<'de>>(d: D) -> Result<Option<S>, D::Error> {
    Option::<f32>::deserialize(d).map(|o| o.map(S::from_f32_data))
}

pub fn de_point<'de, D: Deserializer<'de>>(d: D) -> Result<V2, D::Error> {
    let [x, y] = <[f32; 2]>::deserialize(d)?;
    Ok(V2::from_f32_data(x, y))
}

pub fn de_points<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<V2>, D::Error> {
    let pts = Vec::<[f32; 2]>::deserialize(d)?;
    Ok(pts
        .into_iter()
        .map(|[x, y]| V2::from_f32_data(x, y))
        .collect())
}

/// `{x, y}` objects (formation custom slots).
pub fn de_xy_points<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<V2>, D::Error> {
    #[derive(Deserialize)]
    struct Xy {
        x: f32,
        y: f32,
    }
    let pts = Vec::<Xy>::deserialize(d)?;
    Ok(pts
        .into_iter()
        .map(|p| V2::from_f32_data(p.x, p.y))
        .collect())
}

/// An sRGB colour from `#rrggbb`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb(pub [u8; 3]);

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let hex = s
            .strip_prefix('#')
            .filter(|h| h.len() == 6)
            .ok_or_else(|| D::Error::custom(format!("expected #rrggbb, found {s:?}")))?;
        let byte = |i: usize| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| D::Error::custom(format!("expected #rrggbb, found {s:?}")))
        };
        Ok(Rgb([byte(0)?, byte(2)?, byte(4)?]))
    }
}

impl Rgb {
    pub fn rgba(self) -> [u8; 4] {
        [self.0[0], self.0[1], self.0[2], 255]
    }
}

pub fn s(v: f32) -> S {
    S::from_f32_data(v)
}

pub fn d_zero() -> S {
    s(0.0)
}
pub fn d_one() -> S {
    s(1.0)
}
pub fn d_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse() {
        let c: Rgb = serde_json::from_str("\"#7a1F1f\"").unwrap();
        assert_eq!(c, Rgb([0x7a, 0x1f, 0x1f]));
        assert!(serde_json::from_str::<Rgb>("\"7a1f1f\"").is_err());
        assert!(serde_json::from_str::<Rgb>("\"#7a1f\"").is_err());
    }
}
