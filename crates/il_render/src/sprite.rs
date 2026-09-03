//! Instanced sprite pipeline: 32-byte instances, a ring of three instance
//! buffers, one draw per atlas (T1-051, TDD §10.1).

use std::ops::Range;

use crate::atlas::AtlasId;

/// One sprite. 32 bytes, `Pod`, written straight into the instance buffer.
///
/// `frame_facing` packs the atlas column in bits 0..16 and the facing row in
/// bits 16..24. `depth` is the depth-buffer value in `[0, 1]` (smaller wins).
/// `flags` bit 0 = selected, bit 1 = hovered; the rest are reserved.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteInstance {
    pub pos: [f32; 2],
    pub depth: f32,
    pub frame_facing: u32,
    pub tint: [u8; 4],
    pub scale: f32,
    pub flags: u32,
    pub _reserved: u32,
}

impl SpriteInstance {
    pub const SIZE: u64 = 32;

    pub fn pack_frame_facing(frame: u32, facing: u8) -> u32 {
        (frame & 0xffff) | (u32::from(facing) << 16)
    }
}

/// A run of consecutive instances drawn with one atlas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpriteBatch {
    pub atlas: AtlasId,
    pub range: Range<u32>,
}

/// Everything the sprite pass draws in one frame.
#[derive(Clone, Debug, Default)]
pub struct SpriteScene {
    pub instances: Vec<SpriteInstance>,
    pub batches: Vec<SpriteBatch>,
}

impl SpriteScene {
    pub fn clear(&mut self) {
        self.instances.clear();
        self.batches.clear();
    }

    /// Appends `instances` as one batch for `atlas`.
    pub fn push_batch(
        &mut self,
        atlas: AtlasId,
        instances: impl IntoIterator<Item = SpriteInstance>,
    ) {
        let start = self.instances.len() as u32;
        self.instances.extend(instances);
        let end = self.instances.len() as u32;
        if end > start {
            self.batches.push(SpriteBatch {
                atlas,
                range: start..end,
            });
        }
    }
}

/// Screen-size uniform (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Globals {
    pub screen: [f32; 2],
    pub _pad: [f32; 2],
}

const RING: usize = 3;
const INITIAL_CAPACITY: u64 = 4096;

pub(crate) struct SpritePipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub atlas_layout: wgpu::BindGroupLayout,
    pub globals_buffer: wgpu::Buffer,
    pub globals_bind_group: wgpu::BindGroup,
    ring: [wgpu::Buffer; RING],
    capacity: u64,
    frame: usize,
}

impl SpritePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sprite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sprite.wgsl").into()),
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite"),
            bind_group_layouts: &[Some(&globals_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });
        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: SpriteInstance::SIZE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 8,
                    shader_location: 1,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 12,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Unorm8x4,
                    offset: 16,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32,
                    offset: 20,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Uint32,
                    offset: 24,
                    shader_location: 5,
                },
            ],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprite"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(instance_layout)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::renderer::DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: true,
            },
            multiview_mask: None,
            cache: None,
        });
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let ring = std::array::from_fn(|i| Self::instance_buffer(device, INITIAL_CAPACITY, i));
        Self {
            pipeline,
            atlas_layout,
            globals_buffer,
            globals_bind_group,
            ring,
            capacity: INITIAL_CAPACITY,
            frame: 0,
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: u64, i: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("sprite instances {i}")),
            size: capacity * SpriteInstance::SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uploads the scene's instances into the next ring buffer and returns it
    /// (a `wgpu::Buffer` is a cheap handle clone).
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: [f32; 2],
        scene: &SpriteScene,
    ) -> wgpu::Buffer {
        let needed = scene.instances.len() as u64;
        if needed > self.capacity {
            let mut cap = self.capacity;
            while cap < needed {
                cap = cap * 3 / 2;
            }
            self.ring = std::array::from_fn(|i| Self::instance_buffer(device, cap, i));
            self.capacity = cap;
        }
        queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen,
                _pad: [0.0; 2],
            }),
        );
        self.frame = (self.frame + 1) % RING;
        let buffer = self.ring[self.frame].clone();
        if !scene.instances.is_empty() {
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&scene.instances));
        }
        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;

    #[test]
    fn instance_is_32_bytes_and_packs_frame_and_facing() {
        assert_eq!(std::mem::size_of::<SpriteInstance>(), 32);
        assert_eq!(SpriteInstance::SIZE, 32);
        let packed = SpriteInstance::pack_frame_facing(3, 7);
        assert_eq!(packed & 0xffff, 3);
        assert_eq!((packed >> 16) & 0xff, 7);
    }

    #[test]
    fn push_batch_records_ranges_and_skips_empty_batches() {
        let mut scene = SpriteScene::default();
        let inst = SpriteInstance::zeroed();
        scene.push_batch(AtlasId(0), [inst, inst]);
        scene.push_batch(AtlasId(1), []);
        scene.push_batch(AtlasId(2), [inst]);
        assert_eq!(scene.instances.len(), 3);
        assert_eq!(scene.batches.len(), 2);
        assert_eq!(scene.batches[1].range, 2..3);
        scene.clear();
        assert!(scene.instances.is_empty() && scene.batches.is_empty());
    }
}
