//! egui paint pass (T1-060, REQ-TECH-004): draws tessellated egui output
//! over the resolved frame in a second, non-multisampled pass.

use egui_wgpu::ScreenDescriptor;

/// One frame of egui output, borrowed from `il_ui::UiOutput`. The texture
/// delta is taken mutably because egui insists every delta is applied (or
/// cleared) before it is dropped, even on a skipped frame.
pub struct EguiPaint<'a> {
    pub textures_delta: &'a mut egui::TexturesDelta,
    pub primitives: &'a [egui::ClippedPrimitive],
    pub pixels_per_point: f32,
}

pub(crate) struct EguiPass {
    renderer: egui_wgpu::Renderer,
}

impl EguiPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let renderer = egui_wgpu::Renderer::new(
            device,
            format,
            egui_wgpu::RendererOptions {
                msaa_samples: 1,
                depth_stencil_format: None,
                ..Default::default()
            },
        );
        Self { renderer }
    }

    /// Applies (and clears) the frame's texture updates. Called on every
    /// frame, painted or skipped, so no delta is ever dropped unapplied.
    pub fn apply_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        delta: &mut egui::TexturesDelta,
    ) {
        for (id, deltas) in &delta.set {
            for image_delta in deltas {
                self.renderer
                    .update_texture(device, queue, *id, image_delta);
            }
        }
        for id in &delta.free {
            self.renderer.free_texture(id);
        }
        delta.clear();
    }

    /// Uploads textures and buffers, records the paint pass onto `target`
    /// (loading what is already there) and returns command buffers that must
    /// be submitted before `encoder`'s.
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size_in_pixels: [u32; 2],
        paint: &mut EguiPaint<'_>,
    ) -> Vec<wgpu::CommandBuffer> {
        // Frees are deferred until after the pass so this frame can still
        // sample textures egui just released.
        let frees = std::mem::take(&mut paint.textures_delta.free);
        self.apply_textures(device, queue, paint.textures_delta);
        let desc = ScreenDescriptor {
            size_in_pixels,
            pixels_per_point: paint.pixels_per_point,
        };
        let uploads = self
            .renderer
            .update_buffers(device, queue, encoder, paint.primitives, &desc);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            self.renderer.render(&mut pass, paint.primitives, &desc);
        }
        for id in &frees {
            self.renderer.free_texture(id);
        }
        uploads
    }
}
