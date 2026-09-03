//! `SpriteSet`: the frame table of one sprite sheet (`sprite-set.schema.json`,
//! TDD §10.1). Render-only, so it is neither hashed nor read by the sim.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::content_id::ContentId;
use crate::registry::ContentKind;
use crate::schema::KindTag;

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Anim {
    pub first: u32,
    pub count: u32,
    pub fps: f32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SpriteSet {
    pub id: ContentId,
    /// PNG path relative to the mod's assets root.
    pub atlas: String,
    pub frame_w: u32,
    pub frame_h: u32,
    #[serde(default = "d_facings")]
    pub facings: u32,
    pub columns: u32,
    /// Pixel of a frame that sits on the ground position.
    pub origin: [f32; 2],
    pub anims: BTreeMap<String, Anim>,
    #[serde(default)]
    pub deprecated: Option<String>,
}

fn d_facings() -> u32 {
    8
}

impl ContentKind for SpriteSet {
    const DIR: &'static str = "sprites";
    const TAG: KindTag = KindTag::SpriteSet;

    fn id(&self) -> &ContentId {
        &self.id
    }
}
