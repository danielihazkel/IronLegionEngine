//! The winit application handler: window, renderer, frame loop (T1-050).

use std::sync::Arc;
use std::time::Instant;

use il_render::{ClearColour, Renderer};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

use crate::session::BattleSession;

/// Frames between title refreshes (the title shows the tick and sim cost).
const TITLE_EVERY_FRAMES: u32 = 15;

pub struct App {
    session: BattleSession,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    last_frame: Option<Instant>,
    frames: u32,
    /// Wall time spent inside `BattleWorld::step` since the last title refresh.
    step_seconds: f64,
    ticks_since_title: u32,
}

impl App {
    pub fn new(session: BattleSession) -> Self {
        Self {
            session,
            window: None,
            renderer: None,
            last_frame: None,
            frames: 0,
            step_seconds: 0.0,
            ticks_since_title: 0,
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last_frame = Some(now);

        let before = Instant::now();
        let stepped = self.session.advance(dt).len() as u32;
        self.step_seconds += before.elapsed().as_secs_f64();
        self.ticks_since_title += stepped;

        if let Some(renderer) = self.renderer.as_mut()
            && let Err(e) = renderer.render_clear(ClearColour::FIELD)
        {
            eprintln!("fatal render error: {e}");
            std::process::exit(1);
        }

        self.frames += 1;
        if self.frames.is_multiple_of(TITLE_EVERY_FRAMES) {
            self.refresh_title();
        }
    }

    /// Temporary keyboard handling until bindings arrive (T1-061): Space
    /// pauses, `+`/`-` change speed.
    fn key(&mut self, event: &KeyEvent) {
        if event.state != ElementState::Pressed || event.repeat {
            return;
        }
        match &event.logical_key {
            Key::Named(NamedKey::Space) => {
                let paused = self.session.paused();
                self.session.set_paused(!paused);
            }
            Key::Character(c) if c == "+" || c == "=" => {
                let speed = (self.session.speed() * 2.0).min(8.0);
                self.session.set_speed(speed);
            }
            Key::Character(c) if c == "-" => {
                let speed = (self.session.speed() * 0.5).max(0.125);
                self.session.set_speed(speed);
            }
            _ => {}
        }
        self.refresh_title();
    }

    fn refresh_title(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let per_tick_ms = if self.ticks_since_title > 0 {
            self.step_seconds * 1000.0 / f64::from(self.ticks_since_title)
        } else {
            0.0
        };
        window.set_title(&format!(
            "Iron Legion — tick {} — {} soldiers — sim {:.2} ms/tick — speed x{:.2} — {} commands{}",
            self.session.world.tick().0,
            self.session.world.soldier_count(),
            per_tick_ms,
            self.session.speed(),
            self.session.command_log().len(),
            if self.session.paused() { " — paused" } else { "" }
        ));
        self.step_seconds = 0.0;
        self.ticks_since_title = 0;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Iron Legion")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 800.0));
        let window = match event_loop.create_window(attributes) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("cannot create the window: {e}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        match Renderer::new(window.clone(), size.width, size.height) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                eprintln!("cannot initialise the renderer: {e}");
                event_loop.exit();
                return;
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(&event),
            WindowEvent::RedrawRequested => self.frame(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = self.window.as_ref() {
            w.request_redraw();
        }
    }
}
