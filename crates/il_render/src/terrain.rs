//! Terrain rendering (T1-053, TDD §10.1 terrain, REQ-RNDR-006).
//!
//! A `TerrainMesh` is built once per battle from the sim's `LoadedMap`: one
//! vertex per height sample with a slope shade from the finite-difference
//! normal, two triangles per cell, an R8 zone-index raster (255 = open
//! water) and a 256-entry linear-colour palette from the zone types. The GPU
//! side projects vertices with the camera uniform, so nothing is rebuilt per
//! frame.

use glam::Vec2;
use il_core::{S, Scalar, V2};
use il_data::Registries;
use il_sim_battle::LoadedMap;

use crate::camera::Camera;
use crate::lines::LineScene;
use crate::scene::side_tint;

/// Palette slot of river cells no crossing covers.
pub const WATER_INDEX: u8 = 255;
/// Contour spacing in metres.
pub const CONTOUR_METRES: f32 = 2.0;
/// Deployment outlines follow the ground: one segment per this many metres.
const OUTLINE_STEP_METRES: f32 = 8.0;

/// Light direction (unit, pointing toward the light) for slope shading:
/// from the north-west and well above the horizon.
const LIGHT: [f32; 3] = [-0.40, 0.45, 0.80];

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TerrainVertex {
    /// World position, metres.
    pub pos: [f32; 2],
    pub height: f32,
    /// Slope shade multiplier around 1.
    pub shade: f32,
}

impl TerrainVertex {
    pub const SIZE: u64 = 16;
}

/// CPU-side terrain data, ready to upload.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainMesh {
    pub width: f32,
    pub height: f32,
    pub vertices: Vec<TerrainVertex>,
    pub indices: Vec<u32>,
    pub zone_cell: f32,
    pub zone_cols: u32,
    pub zone_rows: u32,
    /// Zone index per raster cell, row-major, rows padded to
    /// [`zone_row_bytes`](Self::zone_row_bytes) for the texture upload.
    pub zone_texels: Vec<u8>,
    /// Linear RGBA per zone index; `[WATER_INDEX]` is water.
    pub palette: [[f32; 4]; 256],
}

/// sRGB byte to linear.
pub fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn shade_at(map: &LoadedMap, i: u32, j: u32) -> f32 {
    let cols = map.height_cols as i64;
    let rows = map.height_rows as i64;
    let at = |i: i64, j: i64| -> f32 {
        let i = i.clamp(0, cols - 1);
        let j = j.clamp(0, rows - 1);
        map.heights[(j * cols + i) as usize].to_f32_render()
    };
    let (i, j) = (i64::from(i), i64::from(j));
    let cell = map.height_cell.to_f32_render();
    // Central differences (one-sided at the edges through the clamp).
    let dx = (at(i + 1, j) - at(i - 1, j)) / (2.0 * cell);
    let dy = (at(i, j + 1) - at(i, j - 1)) / (2.0 * cell);
    let n = glam::Vec3::new(-dx, -dy, 1.0).normalize();
    let l = glam::Vec3::from(LIGHT).normalize();
    // Ambient plus diffuse, normalised so flat ground is exactly 1.
    let flat = l.z;
    let diffuse = n.dot(l).max(0.0);
    0.45 + 0.55 * diffuse / flat
}

impl TerrainMesh {
    /// Builds the mesh, zone raster and palette for `map`.
    pub fn build(map: &LoadedMap, regs: &Registries) -> Self {
        let cols = map.height_cols;
        let rows = map.height_rows;
        let cell = map.height_cell.to_f32_render();
        let mut vertices = Vec::with_capacity((cols * rows) as usize);
        for j in 0..rows {
            for i in 0..cols {
                vertices.push(TerrainVertex {
                    pos: [i as f32 * cell, j as f32 * cell],
                    height: map.heights[(j * cols + i) as usize].to_f32_render(),
                    shade: shade_at(map, i, j),
                });
            }
        }
        let mut indices = Vec::with_capacity(((cols - 1) * (rows - 1) * 6) as usize);
        for j in 0..rows - 1 {
            for i in 0..cols - 1 {
                let a = j * cols + i;
                let b = a + 1;
                let c = a + cols;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }

        let mut palette = [[0.0f32; 4]; 256];
        for (k, h) in map.zone_handles.iter().enumerate() {
            let rgb = regs.zones.get(*h).colour.0;
            palette[k] = [
                srgb_to_linear(rgb[0]),
                srgb_to_linear(rgb[1]),
                srgb_to_linear(rgb[2]),
                1.0,
            ];
        }
        palette[usize::from(WATER_INDEX)] = [
            srgb_to_linear(0x3a),
            srgb_to_linear(0x6e),
            srgb_to_linear(0xa5),
            1.0,
        ];

        let row_bytes = Self::padded_row_bytes(map.zone_cols);
        let mut zone_texels = vec![0u8; row_bytes as usize * map.zone_rows as usize];
        for j in 0..map.zone_rows as usize {
            for i in 0..map.zone_cols as usize {
                let slot = j * map.zone_cols as usize + i;
                let zone = map.zones[slot];
                let crossing = map
                    .zone_handles
                    .get(usize::from(zone))
                    .is_some_and(|h| regs.zones.get(*h).crossing);
                zone_texels[j * row_bytes as usize + i] = if map.river[slot] && !crossing {
                    WATER_INDEX
                } else {
                    zone
                };
            }
        }

        Self {
            width: map.width.to_f32_render(),
            height: map.height.to_f32_render(),
            vertices,
            indices,
            zone_cell: map.zone_cell.to_f32_render(),
            zone_cols: map.zone_cols,
            zone_rows: map.zone_rows,
            zone_texels,
            palette,
        }
    }

    /// Texture rows are padded to wgpu's 256-byte alignment.
    pub fn padded_row_bytes(cols: u32) -> u32 {
        cols.div_ceil(256) * 256
    }

    pub fn zone_row_bytes(&self) -> u32 {
        Self::padded_row_bytes(self.zone_cols)
    }
}

/// Camera uniform of the terrain shader (64 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TerrainGlobals {
    pub screen: [f32; 2],
    pub center: [f32; 2],
    pub rot: [f32; 4],
    pub zoom: f32,
    pub pitch: f32,
    pub elevation: f32,
    pub zone_cell: f32,
    pub zone_dims: [u32; 2],
    pub contour: f32,
    pub _pad: f32,
}

/// The 2×2 view rotation of `camera` as the shader reads it
/// (`view = (rot[0..2] · d, rot[2..4] · d)`), matching `rotate_to_view`.
pub fn rotation_rows(camera: &Camera) -> [f32; 4] {
    match camera.rotation & 3 {
        0 => [1.0, 0.0, 0.0, 1.0],
        1 => [0.0, 1.0, -1.0, 0.0],
        2 => [-1.0, 0.0, 0.0, -1.0],
        _ => [0.0, -1.0, 1.0, 0.0],
    }
}

impl TerrainGlobals {
    pub fn new(camera: &Camera, screen: Vec2, zone_cell: f32, zone_dims: [u32; 2]) -> Self {
        Self {
            screen: screen.to_array(),
            center: camera.center.to_array(),
            rot: rotation_rows(camera),
            zoom: camera.zoom,
            pitch: camera.pitch,
            elevation: camera.elevation,
            zone_cell,
            zone_dims,
            contour: CONTOUR_METRES,
            _pad: 0.0,
        }
    }
}

/// Appends the deployment polygon outlines of `map` to `lines`, following
/// the ground and tinted per side.
pub fn deployment_outlines(map: &LoadedMap, camera: &Camera, screen: Vec2, lines: &mut LineScene) {
    for zone in &map.deployment {
        let colour = side_tint(zone.side);
        let n = zone.polygon.len();
        for k in 0..n {
            let a = zone.polygon[k];
            let b = zone.polygon[(k + 1) % n];
            let (ax, ay) = (a.x.to_f32_render(), a.y.to_f32_render());
            let (bx, by) = (b.x.to_f32_render(), b.y.to_f32_render());
            let len = Vec2::new(bx - ax, by - ay).length();
            let steps = (len / OUTLINE_STEP_METRES).ceil().max(1.0) as u32;
            let mut prev = project(map, camera, screen, Vec2::new(ax, ay));
            for s in 1..=steps {
                let t = s as f32 / steps as f32;
                let p = Vec2::new(ax + (bx - ax) * t, ay + (by - ay) * t);
                let cur = project(map, camera, screen, p);
                lines.segment(prev, cur, colour);
                prev = cur;
            }
        }
    }
}

/// Projects a world point sitting on the terrain.
pub fn project(map: &LoadedMap, camera: &Camera, screen: Vec2, p: Vec2) -> Vec2 {
    let h = map.height_at(V2::from_f32_data(p.x, p.y)).to_f32_render();
    camera.world_to_screen(p, h, screen)
}

/// Terrain height at a render-side world position.
pub fn ground_height(map: &LoadedMap, p: Vec2) -> f32 {
    map.height_at(V2::new(S::from_f32_data(p.x), S::from_f32_data(p.y)))
        .to_f32_render()
}

/// Uploaded terrain: vertex and index buffers, the zone raster, the palette
/// and the camera uniform.
pub(crate) struct TerrainGpu {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub index_count: u32,
    pub globals: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub zone_cell: f32,
    pub zone_dims: [u32; 2],
}

pub(crate) struct TerrainPipeline {
    pub pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl TerrainPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("terrain.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("terrain.wgsl").into()),
        });
        let uniform = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("terrain"),
            entries: &[
                uniform(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                uniform(1, wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: TerrainVertex::SIZE,
            step_mode: wgpu::VertexStepMode::Vertex,
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
                    format: wgpu::VertexFormat::Float32,
                    offset: 12,
                    shader_location: 2,
                },
            ],
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&pipeline_layout),
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
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // Drawn first, under everything: no depth write, always passes.
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
        Self { pipeline, layout }
    }

    /// Uploads a mesh: buffers, zone texture, palette, uniform.
    pub fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mesh: &TerrainMesh,
    ) -> TerrainGpu {
        use wgpu::util::DeviceExt;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain vertices"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain globals"),
            size: std::mem::size_of::<TerrainGlobals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let palette = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("terrain palette"),
            contents: bytemuck::cast_slice(&mesh.palette),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let size = wgpu::Extent3d {
            width: mesh.zone_cols,
            height: mesh.zone_rows,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("zone raster"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Uint,
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
            &mesh.zone_texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(mesh.zone_row_bytes()),
                rows_per_image: Some(mesh.zone_rows),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("terrain"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: palette.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        TerrainGpu {
            vertices,
            indices,
            index_count: mesh.indices.len() as u32,
            globals,
            bind_group,
            zone_cell: mesh.zone_cell,
            zone_dims: [mesh.zone_cols, mesh.zone_rows],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_rows_match_the_camera() {
        for k in 0..4u8 {
            let mut cam = Camera::new(Vec2::ZERO);
            cam.rotation = k;
            let r = rotation_rows(&cam);
            for d in [
                Vec2::new(1.0, 0.0),
                Vec2::new(0.3, -2.0),
                Vec2::new(-5.0, 4.0),
            ] {
                let shader = Vec2::new(r[0] * d.x + r[1] * d.y, r[2] * d.x + r[3] * d.y);
                assert_eq!(shader, cam.rotate_to_view(d), "k={k} d={d}");
            }
        }
    }

    #[test]
    fn flat_map_builds_a_grid_with_unit_shade_and_padded_rows() {
        let map = LoadedMap::flat(S::from_i32(100), S::from_i32(50));
        let regs = Registries::default();
        let mesh = TerrainMesh::build(&map, &regs);
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
        assert!(mesh.vertices.iter().all(|v| (v.shade - 1.0).abs() < 1e-5));
        assert_eq!(mesh.zone_row_bytes(), 256);
        assert_eq!(mesh.zone_texels.len(), 256);
        assert_eq!(TerrainMesh::padded_row_bytes(400), 512);
        assert_eq!(TerrainMesh::padded_row_bytes(512), 512);
        assert_eq!(std::mem::size_of::<TerrainGlobals>(), 64);
        assert_eq!(std::mem::size_of::<TerrainVertex>(), 16);
        assert!(mesh.palette[usize::from(WATER_INDEX)][2] > 0.3);
    }

    #[test]
    fn srgb_conversion_hits_the_anchors() {
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
        assert!((srgb_to_linear(128) - 0.2158).abs() < 1e-3);
    }
}
