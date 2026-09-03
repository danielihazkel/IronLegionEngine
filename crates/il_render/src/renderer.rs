//! Device, surface, frame targets and the per-frame sprite pass (T1-050,
//! T1-051).

use std::path::Path;

use thiserror::Error;

use crate::atlas::{Atlas, AtlasError, AtlasId, Rgba8Image, SpriteSheet};
use crate::sprite::{SpritePipeline, SpriteScene};

/// Depth attachment format shared by every pipeline.
pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// MSAA sample count; alpha-to-coverage needs multisampling.
pub(crate) const SAMPLES: u32 = 4;

/// Errors while bringing up the GPU or presenting.
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("creating the window surface: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("no compatible GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("requesting the GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    #[error("the surface reports no supported texture format")]
    NoSurfaceFormat,
    #[error("the window surface was lost; the window must be recreated")]
    SurfaceLost,
    #[error("the surface configuration failed validation")]
    SurfaceConfig,
    #[error(transparent)]
    Atlas(#[from] AtlasError),
}

/// Linear-space clear colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClearColour {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl ClearColour {
    /// The battlefield ground tone used until terrain rendering lands (T1-053).
    pub const FIELD: ClearColour = ClearColour {
        r: 0.16,
        g: 0.20,
        b: 0.11,
    };
}

/// MSAA colour and depth attachments, recreated on resize.
struct Targets {
    width: u32,
    height: u32,
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
}

impl Targets {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, width: u32, height: u32) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let make = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: SAMPLES,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        Self {
            width,
            height,
            msaa: make("msaa colour", format),
            depth: make("depth", DEPTH_FORMAT),
        }
    }
}

/// Owns the wgpu device, queue, window surface, pipelines and atlases.
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    targets: Targets,
    sprites: SpritePipeline,
    atlases: Vec<Atlas>,
}

impl Renderer {
    /// Creates the instance, adapter, device and a configured surface for
    /// `target` (an `Arc<winit::window::Window>` in the app). `width` and
    /// `height` are the current inner size in physical pixels.
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        pollster::block_on(Self::new_async(target.into(), width, height))
    }

    async fn new_async(
        target: wgpu::SurfaceTarget<'static>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        // No display handle: only Wayland/X11 need one and Windows is the
        // sole MVP platform (REQ-PLAT-001). Pass winit's OwnedDisplayHandle
        // here when Linux support arrives.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(target)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("il_render device"),
                ..Default::default()
            })
            .await?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .or_else(|| caps.formats.first().copied())
            .ok_or(RenderError::NoSurfaceFormat)?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: vec![],
        };
        surface.configure(&device, &config);
        let targets = Targets::new(&device, format, config.width, config.height);
        let sprites = SpritePipeline::new(&device, format, SAMPLES);

        Ok(Self {
            surface,
            device,
            queue,
            config,
            targets,
            sprites,
            atlases: Vec::new(),
        })
    }

    /// Reconfigures the surface after the window changed size. Zero sizes
    /// (minimised window) are clamped to one pixel so the surface stays valid.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    /// Vsync on (`AutoVsync`) or off (`AutoNoVsync`, used by the sprite bench).
    pub fn set_vsync(&mut self, on: bool) {
        self.config.present_mode = if on {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::AutoNoVsync
        };
        self.surface.configure(&self.device, &self.config);
    }

    /// Current surface size in physical pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Loads a sheet's PNG from `assets_root` and uploads it. Returns the id
    /// batches refer to.
    pub fn load_atlas(
        &mut self,
        sheet: SpriteSheet,
        assets_root: &Path,
    ) -> Result<AtlasId, RenderError> {
        let path = sheet.atlas_path(assets_root);
        let image = Rgba8Image::load_png(&path)?;
        let atlas = Atlas::upload(
            &self.device,
            &self.queue,
            &self.sprites.atlas_layout,
            sheet,
            &image,
            &path,
        )?;
        self.atlases.push(atlas);
        Ok(AtlasId(self.atlases.len() as u32 - 1))
    }

    pub fn atlas(&self, id: AtlasId) -> Option<&Atlas> {
        self.atlases.get(id.0 as usize)
    }

    /// Clears to `colour`, draws the sprite scene and presents. An outdated or
    /// suboptimal surface is reconfigured and the frame skipped; a timeout or
    /// an occluded window skips the frame; a lost surface is an error.
    pub fn render(&mut self, colour: ClearColour, scene: &SpriteScene) -> Result<(), RenderError> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(_) | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => return Err(RenderError::SurfaceLost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(RenderError::SurfaceConfig),
        };
        if self.targets.width != self.config.width || self.targets.height != self.config.height {
            self.targets = Targets::new(
                &self.device,
                self.config.format,
                self.config.width,
                self.config.height,
            );
        }
        let screen = [self.config.width as f32, self.config.height as f32];
        let instances = self
            .sprites
            .upload(&self.device, &self.queue, screen, scene);

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("il_render frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sprites"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(&view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: colour.r,
                            g: colour.g,
                            b: colour.b,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.sprites.pipeline);
            pass.set_bind_group(0, &self.sprites.globals_bind_group, &[]);
            pass.set_vertex_buffer(0, instances.slice(..));
            for batch in &scene.batches {
                let Some(atlas) = self.atlases.get(batch.atlas.0 as usize) else {
                    continue;
                };
                pass.set_bind_group(1, &atlas.bind_group, &[]);
                pass.draw(0..6, batch.range.clone());
            }
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}
