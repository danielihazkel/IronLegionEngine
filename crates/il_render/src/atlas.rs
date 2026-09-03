//! GPU atlases built from `il_data::SpriteSet` frame tables (T1-051, T1-023).

use std::path::{Path, PathBuf};

use il_data::SpriteSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AtlasError {
    #[error("reading {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("decoding PNG {path}: {source}")]
    Png {
        path: PathBuf,
        source: png::DecodingError,
    },
    #[error("PNG {path} has colour type {colour:?}; RGBA or RGB 8-bit expected")]
    ColourType {
        path: PathBuf,
        colour: png::ColorType,
    },
    #[error("atlas {path} is {w}x{h} but the frame table needs {need_w}x{need_h}")]
    Size {
        path: PathBuf,
        w: u32,
        h: u32,
        need_w: u32,
        need_h: u32,
    },
}

/// Absolute path of a sprite set's PNG.
pub fn atlas_path(set: &SpriteSet, assets_root: &Path) -> PathBuf {
    assets_root.join(&set.atlas)
}

/// Column of animation `name` at `time` seconds (looping); column 0 if the
/// animation is unknown.
pub fn anim_column(set: &SpriteSet, name: &str, time: f32) -> u32 {
    match set.anims.get(name) {
        Some(a) if a.count > 0 => {
            let step = (time * a.fps).max(0.0) as u32;
            a.first + step % a.count
        }
        _ => 0,
    }
}

/// Decoded RGBA8 pixels.
pub struct Rgba8Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rgba8Image {
    pub fn load_png(path: &Path) -> Result<Self, AtlasError> {
        let file = std::fs::File::open(path).map_err(|source| AtlasError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = decoder.read_info().map_err(|source| AtlasError::Png {
            path: path.to_path_buf(),
            source,
        })?;
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|source| AtlasError::Png {
                path: path.to_path_buf(),
                source,
            })?;
        buf.truncate(info.buffer_size());
        let pixels = match (info.color_type, info.bit_depth) {
            (png::ColorType::Rgba, png::BitDepth::Eight) => buf,
            (png::ColorType::Rgb, png::BitDepth::Eight) => buf
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            (colour, _) => {
                return Err(AtlasError::ColourType {
                    path: path.to_path_buf(),
                    colour,
                });
            }
        };
        Ok(Self {
            width: info.width,
            height: info.height,
            pixels,
        })
    }
}

/// Per-atlas uniform read by the sprite shader (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct AtlasUniform {
    pub inv_size: [f32; 2],
    pub frame: [f32; 2],
    pub origin: [f32; 2],
    pub _pad: [f32; 2],
}

/// A sprite sheet uploaded to the GPU.
pub struct Atlas {
    pub set: SpriteSet,
    pub(crate) bind_group: wgpu::BindGroup,
}

/// Index of an atlas inside the renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AtlasId(pub u32);

impl Atlas {
    pub(crate) fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        set: &SpriteSet,
        image: &Rgba8Image,
        path: &Path,
    ) -> Result<Self, AtlasError> {
        let need_w = set.columns * set.frame_w;
        let need_h = set.facings * set.frame_h;
        if image.width < need_w || image.height < need_h {
            return Err(AtlasError::Size {
                path: path.to_path_buf(),
                w: image.width,
                h: image.height,
                need_w,
                need_h,
            });
        }
        let size = wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(set.id.as_str()),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(image.width * 4),
                rows_per_image: Some(image.height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let uniform = AtlasUniform {
            inv_size: [1.0 / image.width as f32, 1.0 / image.height as f32],
            frame: [set.frame_w as f32, set.frame_h as f32],
            origin: set.origin,
            _pad: [0.0; 2],
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas uniform"),
            size: std::mem::size_of::<AtlasUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::bytes_of(&uniform));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(set.id.as_str()),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        Ok(Self {
            set: set.clone(),
            bind_group,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_data::Anim;
    use std::collections::BTreeMap;

    fn set() -> SpriteSet {
        SpriteSet {
            id: il_data::ContentId::new("rome:sprites_infantry").unwrap(),
            atlas: "sprites/units/infantry.png".to_string(),
            frame_w: 64,
            frame_h: 64,
            facings: 8,
            columns: 5,
            origin: [32.0, 52.0],
            anims: BTreeMap::from([
                (
                    "idle".to_string(),
                    Anim {
                        first: 0,
                        count: 1,
                        fps: 1.0,
                    },
                ),
                (
                    "walk".to_string(),
                    Anim {
                        first: 1,
                        count: 4,
                        fps: 8.0,
                    },
                ),
            ]),
            deprecated: None,
        }
    }

    #[test]
    fn animation_columns_loop_and_paths_resolve() {
        let s = set();
        assert_eq!(anim_column(&s, "idle", 10.0), 0);
        assert_eq!(anim_column(&s, "walk", 0.0), 1);
        assert_eq!(anim_column(&s, "walk", 0.25), 3);
        assert_eq!(anim_column(&s, "walk", 0.5), 1, "loops after count frames");
        assert_eq!(anim_column(&s, "missing", 1.0), 0);
        assert_eq!(
            atlas_path(&s, Path::new("game/assets")),
            PathBuf::from("game/assets/sprites/units/infantry.png")
        );
    }

    #[test]
    fn atlas_uniform_is_32_bytes() {
        assert_eq!(std::mem::size_of::<AtlasUniform>(), 32);
    }
}
