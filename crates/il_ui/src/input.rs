//! `InputState`: winit events accumulated per frame and matched against
//! [`Bindings`] (T1-061, REQ-INP-001, TDD §11).
//!
//! Raw events come in through [`InputState::on_window_event`] (or the typed
//! methods it dispatches to, which tests drive directly). Clicks, double
//! clicks and drags are recognised here so every consumer sees the same
//! thresholds: a press that moves under [`DRAG_THRESHOLD_PX`] is a click, a
//! second click of the same button within [`DOUBLE_CLICK_SECONDS`] and
//! [`DOUBLE_CLICK_PX`] is a double click. Time is handed in by the app in
//! seconds; this crate reads no clock.

use std::collections::BTreeSet;

use egui_winit::winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use egui_winit::winit::keyboard::{KeyCode, PhysicalKey};
use glam::Vec2;

use crate::bindings::{Action, Bindings, Button, Mods, Trigger};

/// A press that moves further than this before release is a drag.
pub const DRAG_THRESHOLD_PX: f32 = 4.0;
/// Two clicks closer than this in time and space are a double click.
pub const DOUBLE_CLICK_SECONDS: f64 = 0.35;
pub const DOUBLE_CLICK_PX: f32 = 6.0;
/// Wheel pixel deltas (touchpads) are converted to lines at this rate.
const WHEEL_PIXELS_PER_LINE: f32 = 40.0;

/// A completed mouse gesture, reported in the frame it ends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    Click {
        button: Button,
        pos: Vec2,
        mods: Mods,
        /// Second click of a pair; the first was reported as a plain click.
        double: bool,
    },
    /// The press crossed the drag threshold this frame.
    DragStart {
        button: Button,
        from: Vec2,
        mods: Mods,
    },
    DragEnd {
        button: Button,
        from: Vec2,
        to: Vec2,
        mods: Mods,
    },
}

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drag {
    pub button: Button,
    pub from: Vec2,
    pub to: Vec2,
    pub mods: Mods,
}

#[derive(Clone, Copy, Debug, Default)]
struct ButtonState {
    /// Press position while the button is down.
    press: Option<Vec2>,
    dragging: bool,
}

#[derive(Debug)]
pub struct InputState {
    held: BTreeSet<KeyCode>,
    /// Keys pressed this frame (no repeats) with the modifiers at the time.
    pressed: Vec<(KeyCode, Mods)>,
    mods: Mods,
    cursor: Option<Vec2>,
    /// Last known cursor even after it left the window (drags continue).
    last_cursor: Vec2,
    cursor_prev_frame: Option<Vec2>,
    wheel_lines: f32,
    buttons: [ButtonState; 3],
    gestures: Vec<Gesture>,
    last_click: Option<(Button, f64, Vec2)>,
    time: f64,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

fn index(b: Button) -> usize {
    match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    }
}

impl InputState {
    pub fn new() -> Self {
        Self {
            held: BTreeSet::new(),
            pressed: Vec::new(),
            mods: Mods::NONE,
            cursor: None,
            last_cursor: Vec2::ZERO,
            cursor_prev_frame: None,
            wheel_lines: 0.0,
            buttons: [ButtonState::default(); 3],
            gestures: Vec::new(),
            last_click: None,
            time: 0.0,
        }
    }

    /// Feeds one window event. `consumed` is egui's verdict: a consumed
    /// press or wheel is ignored, releases and cursor motion always count so
    /// no key or button sticks when a panel takes the pointer mid-gesture.
    pub fn on_window_event(&mut self, event: &WindowEvent, consumed: bool) {
        match event {
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.set_modifiers(Mods {
                    ctrl: s.control_key(),
                    shift: s.shift_key(),
                    alt: s.alt_key(),
                });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    if pressed && consumed {
                        return;
                    }
                    self.key(code, pressed, event.repeat);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_moved(Vec2::new(position.x as f32, position.y as f32));
            }
            WindowEvent::CursorLeft { .. } => self.cursor_left(),
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    MouseButton::Middle => Button::Middle,
                    _ => return,
                };
                let pressed = *state == ElementState::Pressed;
                if pressed && consumed {
                    return;
                }
                self.button(button, pressed);
            }
            WindowEvent::MouseWheel { delta, .. } if !consumed => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / WHEEL_PIXELS_PER_LINE,
                };
                self.wheel(lines);
            }
            _ => {}
        }
    }

    // Typed event entry points (what `on_window_event` dispatches to).

    pub fn set_modifiers(&mut self, mods: Mods) {
        self.mods = mods;
    }

    pub fn key(&mut self, code: KeyCode, pressed: bool, repeat: bool) {
        if pressed {
            if !repeat {
                self.pressed.push((code, self.mods));
            }
            self.held.insert(code);
        } else {
            self.held.remove(&code);
        }
    }

    pub fn cursor_moved(&mut self, p: Vec2) {
        self.cursor = Some(p);
        self.last_cursor = p;
        for (i, b) in self.buttons.iter_mut().enumerate() {
            if let Some(from) = b.press
                && !b.dragging
                && (p - from).length() > DRAG_THRESHOLD_PX
            {
                b.dragging = true;
                self.gestures.push(Gesture::DragStart {
                    button: button_at(i),
                    from,
                    mods: self.mods,
                });
            }
        }
    }

    pub fn cursor_left(&mut self) {
        self.cursor = None;
    }

    pub fn button(&mut self, button: Button, pressed: bool) {
        let pos = self.last_cursor;
        let state = &mut self.buttons[index(button)];
        if pressed {
            state.press = Some(pos);
            state.dragging = false;
            return;
        }
        let Some(from) = state.press.take() else {
            return;
        };
        if state.dragging {
            state.dragging = false;
            self.gestures.push(Gesture::DragEnd {
                button,
                from,
                to: pos,
                mods: self.mods,
            });
            return;
        }
        let double = self.last_click.is_some_and(|(b, t, p)| {
            b == button
                && self.time - t <= DOUBLE_CLICK_SECONDS
                && (pos - p).length() <= DOUBLE_CLICK_PX
        });
        self.last_click = if double {
            None
        } else {
            Some((button, self.time, pos))
        };
        self.gestures.push(Gesture::Click {
            button,
            pos,
            mods: self.mods,
            double,
        });
    }

    /// Positive lines scroll up (away from the user).
    pub fn wheel(&mut self, lines: f32) {
        self.wheel_lines += lines;
    }

    /// Call once per frame before polling, with wall time in seconds.
    pub fn begin_frame(&mut self, time_seconds: f64) {
        self.time = time_seconds;
    }

    /// Clears the per-frame sets; call after the frame's polling.
    pub fn end_frame(&mut self) {
        self.pressed.clear();
        self.gestures.clear();
        self.wheel_lines = 0.0;
        self.cursor_prev_frame = self.cursor;
    }

    // Queries.

    pub fn mods(&self) -> Mods {
        self.mods
    }

    /// Cursor position while it is inside the window.
    pub fn cursor(&self) -> Option<Vec2> {
        self.cursor
    }

    /// Cursor motion since the previous frame's `end_frame`.
    pub fn cursor_delta(&self) -> Vec2 {
        match (self.cursor, self.cursor_prev_frame) {
            (Some(a), Some(b)) => a - b,
            _ => Vec2::ZERO,
        }
    }

    pub fn key_held(&self, code: KeyCode) -> bool {
        self.held.contains(&code)
    }

    /// Whether a button is down and past the drag threshold.
    pub fn button_dragging(&self, button: Button) -> bool {
        self.buttons[index(button)].dragging
    }

    /// The gestures completed this frame.
    pub fn gestures(&self) -> &[Gesture] {
        &self.gestures
    }

    /// A key chord of `action` went down this frame.
    pub fn pressed(&self, bindings: &Bindings, action: Action) -> bool {
        bindings.chords(action).iter().any(|c| match c.trigger {
            Trigger::Key(code) => self.pressed.iter().any(|(k, m)| *k == code && *m == c.mods),
            _ => false,
        })
    }

    /// A key chord of `action` is down (exact modifiers), or, for a bare
    /// modifier chord, its modifiers are among those held.
    pub fn held(&self, bindings: &Bindings, action: Action) -> bool {
        bindings.chords(action).iter().any(|c| match c.trigger {
            Trigger::Key(code) => self.held.contains(&code) && self.mods == c.mods,
            Trigger::ModifierOnly => covers(self.mods, c.mods),
            _ => false,
        })
    }

    /// Wheel lines this frame in the direction `action` is bound to
    /// (always non-negative), zero when unbound or scrolling the other way.
    pub fn wheel_for(&self, bindings: &Bindings, action: Action) -> f32 {
        bindings
            .chords(action)
            .iter()
            .filter(|c| c.mods == self.mods)
            .map(|c| match c.trigger {
                Trigger::WheelUp => self.wheel_lines.max(0.0),
                Trigger::WheelDown => (-self.wheel_lines).max(0.0),
                _ => 0.0,
            })
            .fold(0.0, f32::max)
    }

    /// The first completed gesture this frame that `action` is bound to.
    pub fn gesture(&self, bindings: &Bindings, action: Action) -> Option<Gesture> {
        self.gestures
            .iter()
            .copied()
            .find(|g| gesture_matches(bindings, action, g))
    }

    /// The drag in progress that `action` is bound to, with the current
    /// cursor as its end point.
    pub fn drag(&self, bindings: &Bindings, action: Action) -> Option<Drag> {
        bindings.chords(action).iter().find_map(|c| {
            let Trigger::Drag(button) = c.trigger else {
                return None;
            };
            let s = self.buttons[index(button)];
            (s.dragging && c.mods == self.mods).then(|| Drag {
                button,
                from: s.press.unwrap_or(self.last_cursor),
                to: self.last_cursor,
                mods: self.mods,
            })
        })
    }
}

fn button_at(i: usize) -> Button {
    match i {
        0 => Button::Left,
        1 => Button::Right,
        _ => Button::Middle,
    }
}

/// Every modifier set in `wanted` is set in `held`.
fn covers(held: Mods, wanted: Mods) -> bool {
    (!wanted.ctrl || held.ctrl) && (!wanted.shift || held.shift) && (!wanted.alt || held.alt)
}

/// Whether `gesture` is one of `action`'s chords (exact modifiers).
pub fn gesture_matches(bindings: &Bindings, action: Action, gesture: &Gesture) -> bool {
    bindings
        .chords(action)
        .iter()
        .any(|c| match (c.trigger, gesture) {
            (
                Trigger::Click(b),
                Gesture::Click {
                    button,
                    mods,
                    double,
                    ..
                },
            ) => b == *button && !*double && c.mods == *mods,
            (
                Trigger::DoubleClick(b),
                Gesture::Click {
                    button,
                    mods,
                    double,
                    ..
                },
            ) => b == *button && *double && c.mods == *mods,
            (Trigger::Drag(b), Gesture::DragEnd { button, mods, .. }) => {
                b == *button && c.mods == *mods
            }
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_data::{Binding, InputBindings};

    fn bindings(entries: &[(&str, &str)]) -> Bindings {
        let content = InputBindings {
            bindings: entries
                .iter()
                .map(|(a, k)| Binding {
                    action: (*a).to_string(),
                    keys: vec![(*k).to_string()],
                })
                .collect(),
        };
        let (b, errors) = Bindings::from_content(&content);
        assert!(errors.is_empty(), "{errors:?}");
        b
    }

    fn drag(input: &mut InputState, button: Button, from: Vec2, to: Vec2) {
        input.cursor_moved(from);
        input.button(button, true);
        input.cursor_moved(to);
        input.button(button, false);
    }

    #[test]
    fn short_press_is_a_click_and_long_press_is_a_drag() {
        let mut input = InputState::new();
        drag(
            &mut input,
            Button::Left,
            Vec2::new(10.0, 10.0),
            Vec2::new(12.0, 11.0),
        );
        assert!(matches!(
            input.gestures(),
            [Gesture::Click {
                button: Button::Left,
                double: false,
                ..
            }]
        ));
        input.end_frame();
        drag(
            &mut input,
            Button::Left,
            Vec2::new(10.0, 10.0),
            Vec2::new(40.0, 10.0),
        );
        assert!(matches!(
            input.gestures(),
            [
                Gesture::DragStart { button: Button::Left, .. },
                Gesture::DragEnd { button: Button::Left, to, .. }
            ] if *to == Vec2::new(40.0, 10.0)
        ));
    }

    #[test]
    fn double_click_needs_the_same_button_close_in_time_and_space() {
        let mut input = InputState::new();
        let p = Vec2::new(100.0, 100.0);
        input.begin_frame(1.0);
        drag(&mut input, Button::Left, p, p);
        input.end_frame();
        input.begin_frame(1.2);
        drag(
            &mut input,
            Button::Left,
            p + Vec2::new(2.0, 0.0),
            p + Vec2::new(2.0, 0.0),
        );
        assert!(matches!(
            input.gestures(),
            [Gesture::Click { double: true, .. }]
        ));
        input.end_frame();
        // A third click starts a new pair.
        input.begin_frame(1.3);
        drag(&mut input, Button::Left, p, p);
        assert!(matches!(
            input.gestures(),
            [Gesture::Click { double: false, .. }]
        ));
        input.end_frame();
        // Too late.
        input.begin_frame(2.0);
        drag(&mut input, Button::Left, p, p);
        assert!(matches!(
            input.gestures(),
            [Gesture::Click { double: false, .. }]
        ));
        input.end_frame();
        // Other button.
        input.begin_frame(2.1);
        drag(&mut input, Button::Right, p, p);
        assert!(matches!(
            input.gestures(),
            [Gesture::Click { double: false, .. }]
        ));
    }

    #[test]
    fn the_bindings_file_rebinds_box_select_to_another_modifier() {
        // T1-061 done-when: the same drag maps to box_select under whichever
        // modifier the file names, with no code change.
        let stock = bindings(&[
            ("box_select", "LeftDrag"),
            ("box_select_add", "Shift+LeftDrag"),
        ]);
        let rebound = bindings(&[
            ("box_select", "Alt+LeftDrag"),
            ("box_select_add", "Shift+LeftDrag"),
        ]);

        let mut plain = InputState::new();
        drag(&mut plain, Button::Left, Vec2::ZERO, Vec2::new(50.0, 50.0));
        assert!(plain.gesture(&stock, Action::BoxSelect).is_some());
        assert!(plain.gesture(&rebound, Action::BoxSelect).is_none());

        let mut with_alt = InputState::new();
        with_alt.set_modifiers(Mods::ALT);
        drag(
            &mut with_alt,
            Button::Left,
            Vec2::ZERO,
            Vec2::new(50.0, 50.0),
        );
        assert!(with_alt.gesture(&stock, Action::BoxSelect).is_none());
        assert!(with_alt.gesture(&rebound, Action::BoxSelect).is_some());
        assert!(with_alt.gesture(&rebound, Action::BoxSelectAdd).is_none());
    }

    #[test]
    fn key_chords_match_modifiers_exactly_and_ignore_repeats() {
        let b = bindings(&[
            ("select_all", "Ctrl+A"),
            ("camera_pan_up", "W"),
            ("order_flip_facing", "Alt"),
        ]);
        let mut input = InputState::new();
        input.key(KeyCode::KeyA, true, false);
        assert!(!input.pressed(&b, Action::SelectAll));
        input.key(KeyCode::KeyA, false, false);
        input.set_modifiers(Mods::CTRL);
        input.key(KeyCode::KeyA, true, false);
        assert!(input.pressed(&b, Action::SelectAll));
        input.end_frame();
        input.key(KeyCode::KeyA, true, true);
        assert!(
            !input.pressed(&b, Action::SelectAll),
            "repeats do not re-fire"
        );

        input.set_modifiers(Mods::NONE);
        input.key(KeyCode::KeyW, true, false);
        assert!(input.held(&b, Action::CameraPanUp));
        input.set_modifiers(Mods::SHIFT);
        assert!(!input.held(&b, Action::CameraPanUp));
        input.set_modifiers(Mods {
            alt: true,
            shift: true,
            ctrl: false,
        });
        assert!(
            input.held(&b, Action::OrderFlipFacing),
            "a bare modifier chord tolerates extras"
        );
    }

    #[test]
    fn wheel_and_drag_in_progress_queries() {
        let b = bindings(&[
            ("camera_zoom_in", "MouseWheelUp"),
            ("camera_zoom_out", "MouseWheelDown"),
            ("camera_drag", "MiddleDrag"),
        ]);
        let mut input = InputState::new();
        input.wheel(2.0);
        assert_eq!(input.wheel_for(&b, Action::CameraZoomIn), 2.0);
        assert_eq!(input.wheel_for(&b, Action::CameraZoomOut), 0.0);
        input.end_frame();
        input.wheel(-1.5);
        assert_eq!(input.wheel_for(&b, Action::CameraZoomOut), 1.5);

        input.cursor_moved(Vec2::new(5.0, 5.0));
        input.button(Button::Middle, true);
        assert!(input.drag(&b, Action::CameraDrag).is_none());
        input.cursor_moved(Vec2::new(30.0, 5.0));
        let d = input.drag(&b, Action::CameraDrag).expect("dragging");
        assert_eq!((d.from, d.to), (Vec2::new(5.0, 5.0), Vec2::new(30.0, 5.0)));
        input.button(Button::Middle, false);
        assert!(input.drag(&b, Action::CameraDrag).is_none());
    }
}
