//! egui context and winit event plumbing (T1-060, REQ-TECH-004).

use egui_winit::winit::event::WindowEvent;
use egui_winit::winit::window::Window;

/// Tessellated egui output for one frame; the renderer paints it after the
/// world (owned data, no borrows into the context).
pub struct UiOutput {
    pub textures_delta: egui::TexturesDelta,
    pub primitives: Vec<egui::ClippedPrimitive>,
    pub pixels_per_point: f32,
}

/// Owns the egui `Context` and the egui-winit input state.
pub struct UiContext {
    ctx: egui::Context,
    state: egui_winit::State,
}

impl UiContext {
    pub fn new(window: &Window) -> Self {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );
        Self { ctx, state }
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// Feeds a window event to egui. Returns `true` when egui consumed it
    /// (pointer over a panel, text field focused), so game input skips it.
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// Runs one egui frame with `ui` building the panels.
    pub fn run(&mut self, window: &Window, ui: impl FnOnce(&egui::Context)) -> UiOutput {
        let raw = self.state.take_egui_input(window);
        self.ctx.begin_pass(raw);
        ui(&self.ctx);
        let out = self.ctx.end_pass();
        self.state
            .handle_platform_output(window, out.platform_output);
        let primitives = self.ctx.tessellate(out.shapes, out.pixels_per_point);
        UiOutput {
            textures_delta: out.textures_delta,
            primitives,
            pixels_per_point: out.pixels_per_point,
        }
    }

    /// Whether egui wants the pointer (a panel is under it).
    pub fn wants_pointer(&self) -> bool {
        self.ctx.egui_wants_pointer_input()
    }

    /// Whether egui wants keyboard input (a text field is focused).
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }
}
