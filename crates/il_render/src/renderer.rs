//! Device, surface and the per-frame skeleton (T1-050).

use thiserror::Error;

/// Errors while bringing up the GPU.
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

/// Owns the wgpu device, queue and window surface.
pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
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

        Ok(Self {
            surface,
            device,
            queue,
            config,
        })
    }

    /// Reconfigures the surface after the window changed size. Zero sizes
    /// (minimised window) are clamped to one pixel so the surface stays valid.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.config.width = width.max(1);
        self.config.height = height.max(1);
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

    /// Clears the next frame to `colour` and presents it. An outdated or
    /// suboptimal surface is reconfigured and the frame skipped; a timeout or
    /// an occluded window skips the frame; a lost surface is an error.
    pub fn render_clear(&mut self, colour: ClearColour) -> Result<(), RenderError> {
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
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("il_render frame"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: colour.r,
                            g: colour.g,
                            b: colour.b,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}
