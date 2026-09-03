//! Line-list pipeline for outlines and debug overlays (T1-053, T1-054,
//! TDD §10.1 debug). Vertices are projected on the CPU (screen pixels) and
//! uploaded into a ring of three buffers like the sprites.

use glam::Vec2;

use crate::sprite::Globals;

/// One line vertex: screen position and straight-alpha colour. 12 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineVertex {
    pub pos: [f32; 2],
    pub colour: [u8; 4],
}

impl LineVertex {
    pub const SIZE: u64 = 12;
}

/// Every line segment drawn in one frame, as a line list (two vertices per
/// segment).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineScene {
    pub vertices: Vec<LineVertex>,
}

impl LineScene {
    pub fn clear(&mut self) {
        self.vertices.clear();
    }

    pub fn segment(&mut self, a: Vec2, b: Vec2, colour: [u8; 4]) {
        self.vertices.push(LineVertex {
            pos: a.to_array(),
            colour,
        });
        self.vertices.push(LineVertex {
            pos: b.to_array(),
            colour,
        });
    }

    /// Consecutive points joined by segments; `closed` adds the last → first
    /// edge.
    pub fn polyline(&mut self, points: &[Vec2], colour: [u8; 4], closed: bool) {
        for w in points.windows(2) {
            self.segment(w[0], w[1], colour);
        }
        if closed && points.len() > 2 {
            self.segment(points[points.len() - 1], points[0], colour);
        }
    }

    /// A circle approximated by `segments` chords.
    pub fn circle(&mut self, center: Vec2, radius: f32, segments: u32, colour: [u8; 4]) {
        let n = segments.max(3);
        let mut prev = center + Vec2::new(radius, 0.0);
        for k in 1..=n {
            let a = k as f32 / n as f32 * std::f32::consts::TAU;
            let p = center + Vec2::new(a.cos(), a.sin()) * radius;
            self.segment(prev, p, colour);
            prev = p;
        }
    }

    pub fn segment_count(&self) -> usize {
        self.vertices.len() / 2
    }
}

const RING: usize = 3;
const INITIAL_CAPACITY: u64 = 4096;

pub(crate) struct LinePipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub globals_buffer: wgpu::Buffer,
    pub globals_bind_group: wgpu::BindGroup,
    ring: [wgpu::Buffer; RING],
    capacity: u64,
    frame: usize,
}

impl LinePipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lines.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("lines.wgsl").into()),
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("line globals"),
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lines"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: LineVertex::SIZE,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Unorm8x4,
                    offset: 8,
                    shader_location: 1,
                },
            ],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lines"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: crate::renderer::DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });
        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("line globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("line globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });
        let ring = std::array::from_fn(|i| Self::vertex_buffer(device, INITIAL_CAPACITY, i));
        Self {
            pipeline,
            globals_buffer,
            globals_bind_group,
            ring,
            capacity: INITIAL_CAPACITY,
            frame: 0,
        }
    }

    fn vertex_buffer(device: &wgpu::Device, capacity: u64, i: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("line vertices {i}")),
            size: capacity * LineVertex::SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Uploads the scene into the next ring buffer and returns it.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: [f32; 2],
        scene: &LineScene,
    ) -> wgpu::Buffer {
        let needed = scene.vertices.len() as u64;
        if needed > self.capacity {
            let mut cap = self.capacity;
            while cap < needed {
                cap = cap * 3 / 2;
            }
            self.ring = std::array::from_fn(|i| Self::vertex_buffer(device, cap, i));
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
        if !scene.vertices.is_empty() {
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&scene.vertices));
        }
        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_builders_emit_two_vertices_per_segment() {
        let mut s = LineScene::default();
        s.segment(Vec2::ZERO, Vec2::X, [1, 2, 3, 4]);
        assert_eq!(s.segment_count(), 1);
        s.polyline(&[Vec2::ZERO, Vec2::X, Vec2::Y], [0; 4], true);
        assert_eq!(s.segment_count(), 4);
        s.polyline(&[Vec2::ZERO, Vec2::X], [0; 4], true);
        assert_eq!(
            s.segment_count(),
            5,
            "a closed two-point line is one segment"
        );
        s.circle(Vec2::ZERO, 2.0, 8, [0; 4]);
        assert_eq!(s.segment_count(), 13);
        assert_eq!(std::mem::size_of::<LineVertex>(), 12);
        s.clear();
        assert!(s.vertices.is_empty());
    }
}
