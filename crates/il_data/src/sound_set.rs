//! `SoundSet`: which sample plays for which battle event, with its rate
//! limits, the battle-roar loop and the zoom curve (`sound-set.schema.json`,
//! TDD §12, T2-100). Audio-only, so it is neither hashed nor read by the
//! sim; the sample paths are resolved against the mod's `assets_root` by
//! `il_audio` at load, missing files being warnings there.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::content_id::ContentId;
use crate::registry::{ContentKind, Lookup, ResolveError};
use crate::schema::KindTag;

/// The closed list of sounds a set may name (plan I2). Every `BattleEvent`
/// the router voices maps onto one of these; unknown keys fail validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundEvent {
    /// `Charge` (a unit's `sounds.charge` wins; cavalry fall back to
    /// `cavalry_charge`).
    Charge,
    CavalryCharge,
    /// `Engaged`.
    Clash,
    /// `VolleyFired`.
    Volley,
    /// `ProjectileLanded { hit: true }`.
    ArrowHit,
    /// `SoldierDied` (a unit's `sounds.die` wins; cavalry fall back to
    /// `cavalry_death`).
    Death,
    CavalryDeath,
    /// `MoraleChanged { to: Routing }`.
    Rout,
    Rally,
    Shatter,
    GeneralDied,
    /// `AbilityUsed`.
    Ability,
    /// `PhaseChanged`, and `Ended` without a winner.
    Phase,
    /// `Ended` won by the observer's side.
    Victory,
    /// `Ended` won by another side.
    Defeat,
}

impl SoundEvent {
    pub const ALL: [SoundEvent; 15] = [
        SoundEvent::Charge,
        SoundEvent::CavalryCharge,
        SoundEvent::Clash,
        SoundEvent::Volley,
        SoundEvent::ArrowHit,
        SoundEvent::Death,
        SoundEvent::CavalryDeath,
        SoundEvent::Rout,
        SoundEvent::Rally,
        SoundEvent::Shatter,
        SoundEvent::GeneralDied,
        SoundEvent::Ability,
        SoundEvent::Phase,
        SoundEvent::Victory,
        SoundEvent::Defeat,
    ];

    /// The key as written in the file.
    pub fn key(self) -> &'static str {
        match self {
            SoundEvent::Charge => "charge",
            SoundEvent::CavalryCharge => "cavalry_charge",
            SoundEvent::Clash => "clash",
            SoundEvent::Volley => "volley",
            SoundEvent::ArrowHit => "arrow_hit",
            SoundEvent::Death => "death",
            SoundEvent::CavalryDeath => "cavalry_death",
            SoundEvent::Rout => "rout",
            SoundEvent::Rally => "rally",
            SoundEvent::Shatter => "shatter",
            SoundEvent::GeneralDied => "general_died",
            SoundEvent::Ability => "ability",
            SoundEvent::Phase => "phase",
            SoundEvent::Victory => "victory",
            SoundEvent::Defeat => "defeat",
        }
    }
}

/// One event's samples and limits.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct EventSounds {
    /// WAV paths relative to the mod's assets root; one is picked per play.
    pub samples: Vec<String>,
    /// Plays of this event closer together than this are dropped.
    #[serde(default = "d_interval")]
    pub min_interval_ms: u32,
    /// Concurrent plays of this event.
    #[serde(default = "d_voices")]
    pub max_voices: u8,
    /// Linear gain applied to every play.
    #[serde(default = "d_one")]
    pub gain: f32,
}

/// The far-zoom battle roar (REQ-AUD-002).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Roar {
    /// A seamless loop, relative to the assets root.
    pub sample: String,
    /// Engaged soldiers at which the roar reaches full gain.
    pub ref_engaged: u32,
}

/// The zoom curve: camera pixels per metre at which the mix is all roar
/// (`far`) and all individual effects (`near`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Zoom {
    pub far: f32,
    pub near: f32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SoundSet {
    pub id: ContentId,
    pub events: BTreeMap<SoundEvent, EventSounds>,
    pub roar: Roar,
    pub zoom: Zoom,
    /// Concurrent plays over every event.
    #[serde(default = "d_max_voices")]
    pub max_voices: u16,
    /// Metres beyond the camera's visible bounds inside which a placed
    /// event still plays.
    #[serde(default = "d_cull_pad")]
    pub cull_pad_m: f32,
    #[serde(default)]
    pub deprecated: Option<String>,
}

fn d_interval() -> u32 {
    100
}
fn d_voices() -> u8 {
    4
}
fn d_one() -> f32 {
    1.0
}
fn d_max_voices() -> u16 {
    24
}
fn d_cull_pad() -> f32 {
    40.0
}

impl ContentKind for SoundSet {
    const DIR: &'static str = "sounds";
    const TAG: KindTag = KindTag::SoundSet;

    fn id(&self) -> &ContentId {
        &self.id
    }

    /// Cross-field checks the schema cannot express.
    fn resolve(&mut self, _lookup: &Lookup, errors: &mut Vec<ResolveError>) {
        if self.zoom.far >= self.zoom.near {
            errors.push(
                ResolveError::new("zoom.far", self.id.clone(), KindTag::SoundSet)
                    .with_message(format!(
                        "zoom.far ({}) must be below zoom.near ({})",
                        self.zoom.far, self.zoom.near
                    ))
                    .with_expected("far < near"),
            );
        }
        for (event, sounds) in &self.events {
            if sounds.samples.is_empty() {
                errors.push(
                    ResolveError::new(
                        format!("events.{}.samples", event.key()),
                        self.id.clone(),
                        KindTag::SoundSet,
                    )
                    .with_message("an event needs at least one sample")
                    .with_expected("a non-empty list of paths"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_key_round_trips() {
        for e in SoundEvent::ALL {
            let parsed: SoundEvent =
                serde_json::from_str(&format!("\"{}\"", e.key())).expect("key parses");
            assert_eq!(parsed, e);
        }
    }
}
