//! Key bindings: engine actions mapped to chords parsed from
//! `content/input/bindings.json5` (T1-061, REQ-INP-005, TDD §11, Modding
//! SDK §4 "Input bindings").
//!
//! A chord is `[Ctrl+][Shift+][Alt+]Key`. `Key` is a letter, a digit, a
//! winit `KeyCode` name (`ArrowUp`, `Equal`, `F1`, `Space`, `NumpadAdd`) or
//! a mouse token (`LeftClick`, `DoubleLeftClick`, `LeftDrag`, `MouseWheelUp`
//! ...). A chord that is only modifiers (`Alt`) matches while they are held.
//! Modifiers match exactly: `W` does not fire while Shift is down, so `W`
//! and `Shift+W` can be different actions.

use std::collections::BTreeMap;
use std::fmt;

use egui_winit::winit::keyboard::KeyCode;
use il_data::InputBindings;

/// Modifier keys held; compared exactly against a chord's modifiers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Mods {
    pub const NONE: Self = Self {
        ctrl: false,
        shift: false,
        alt: false,
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };

    fn any(self) -> bool {
        self.ctrl || self.shift || self.alt
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// What a chord fires on, besides its modifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// A physical key (layout independent: `W` is the key at W on QWERTY).
    Key(KeyCode),
    Click(Button),
    DoubleClick(Button),
    Drag(Button),
    WheelUp,
    WheelDown,
    /// Modifiers only; "active" while they are held.
    ModifierOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub mods: Mods,
    pub trigger: Trigger,
}

/// Every action the engine understands. Names in the bindings file are the
/// snake_case of the variant; `group_set_3` / `group_recall_3` carry the
/// group index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Action {
    CameraPanUp,
    CameraPanDown,
    CameraPanLeft,
    CameraPanRight,
    CameraRotateLeft,
    CameraRotateRight,
    CameraZoomIn,
    CameraZoomOut,
    CameraDrag,
    Select,
    SelectAdd,
    BoxSelect,
    BoxSelectAdd,
    SelectType,
    SelectAll,
    /// Ctrl+n: store the selection as control group `n` (0..=9).
    GroupSet(u8),
    /// n: recall control group `n`.
    GroupRecall(u8),
    OrderMove,
    OrderDragFormation,
    OrderFlipFacing,
    OrderHalt,
    ToggleRun,
    /// Flips the selected ranged regiments between fire-at-will and hold
    /// (T2-030).
    ToggleFire,
    /// The unit type's n-th formation template (1-based).
    Formation(u8),
    Pause,
    SpeedUp,
    SpeedDown,
    ToggleProfiler,
    DebugNavGrid,
    DebugSlots,
    DebugPaths,
    DebugAnchors,
    DebugSpatial,
    QuitToMenu,
}

const FIXED_ACTIONS: &[(&str, Action)] = &[
    ("camera_pan_up", Action::CameraPanUp),
    ("camera_pan_down", Action::CameraPanDown),
    ("camera_pan_left", Action::CameraPanLeft),
    ("camera_pan_right", Action::CameraPanRight),
    ("camera_rotate_left", Action::CameraRotateLeft),
    ("camera_rotate_right", Action::CameraRotateRight),
    ("camera_zoom_in", Action::CameraZoomIn),
    ("camera_zoom_out", Action::CameraZoomOut),
    ("camera_drag", Action::CameraDrag),
    ("select", Action::Select),
    ("select_add", Action::SelectAdd),
    ("box_select", Action::BoxSelect),
    ("box_select_add", Action::BoxSelectAdd),
    ("select_type", Action::SelectType),
    ("select_all", Action::SelectAll),
    ("order_move", Action::OrderMove),
    ("order_drag_formation", Action::OrderDragFormation),
    ("order_flip_facing", Action::OrderFlipFacing),
    ("order_halt", Action::OrderHalt),
    ("toggle_run", Action::ToggleRun),
    ("toggle_fire", Action::ToggleFire),
    ("pause", Action::Pause),
    ("speed_up", Action::SpeedUp),
    ("speed_down", Action::SpeedDown),
    ("toggle_profiler", Action::ToggleProfiler),
    ("debug_nav_grid", Action::DebugNavGrid),
    ("debug_slots", Action::DebugSlots),
    ("debug_paths", Action::DebugPaths),
    ("debug_anchors", Action::DebugAnchors),
    ("debug_spatial", Action::DebugSpatial),
    ("quit_to_menu", Action::QuitToMenu),
];

impl Action {
    /// Parses an action name from the bindings file.
    pub fn from_name(name: &str) -> Option<Self> {
        if let Some((_, a)) = FIXED_ACTIONS.iter().find(|(n, _)| *n == name) {
            return Some(*a);
        }
        let digit = |prefix: &str, max: u8| -> Option<u8> {
            let rest = name.strip_prefix(prefix)?;
            let n: u8 = rest.parse().ok()?;
            (rest.len() == 1 && n <= max).then_some(n)
        };
        if let Some(n) = digit("group_set_", 9) {
            return Some(Action::GroupSet(n));
        }
        if let Some(n) = digit("group_recall_", 9) {
            return Some(Action::GroupRecall(n));
        }
        if let Some(n) = digit("formation_", 9)
            && n >= 1
        {
            return Some(Action::Formation(n));
        }
        None
    }

    /// The bindings-file name.
    pub fn name(self) -> String {
        match self {
            Action::GroupSet(n) => format!("group_set_{n}"),
            Action::GroupRecall(n) => format!("group_recall_{n}"),
            Action::Formation(n) => format!("formation_{n}"),
            other => FIXED_ACTIONS
                .iter()
                .find(|(_, a)| *a == other)
                .map_or("?", |(n, _)| n)
                .to_string(),
        }
    }
}

/// Why a binding entry was skipped; the file still loads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingError {
    UnknownAction(String),
    UnknownKey {
        action: String,
        chord: String,
    },
    /// A chord with two key tokens (`A+B`) or an empty token (`Ctrl+`).
    Malformed {
        action: String,
        chord: String,
    },
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::UnknownAction(a) => write!(f, "unknown action {a:?}"),
            BindingError::UnknownKey { action, chord } => {
                write!(f, "{action}: unknown key in chord {chord:?}")
            }
            BindingError::Malformed { action, chord } => {
                write!(f, "{action}: malformed chord {chord:?}")
            }
        }
    }
}

/// Parsed bindings: each action's chords in file order.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    map: BTreeMap<Action, Vec<Chord>>,
}

impl Bindings {
    /// Parses the merged content. Bad entries are reported and skipped, so a
    /// mod's typo disables one action rather than every one.
    pub fn from_content(content: &InputBindings) -> (Self, Vec<BindingError>) {
        let mut map: BTreeMap<Action, Vec<Chord>> = BTreeMap::new();
        let mut errors = Vec::new();
        for b in &content.bindings {
            let Some(action) = Action::from_name(&b.action) else {
                errors.push(BindingError::UnknownAction(b.action.clone()));
                continue;
            };
            let chords = map.entry(action).or_default();
            chords.clear();
            for text in &b.keys {
                match parse_chord(text) {
                    Ok(chord) => chords.push(chord),
                    Err(ChordError::UnknownKey) => errors.push(BindingError::UnknownKey {
                        action: b.action.clone(),
                        chord: text.clone(),
                    }),
                    Err(ChordError::Malformed) => errors.push(BindingError::Malformed {
                        action: b.action.clone(),
                        chord: text.clone(),
                    }),
                }
            }
        }
        (Self { map }, errors)
    }

    /// The chords bound to `action`; empty when unbound.
    pub fn chords(&self, action: Action) -> &[Chord] {
        self.map.get(&action).map_or(&[], Vec::as_slice)
    }

    /// Every bound action, for the settings panel and diagnostics.
    pub fn actions(&self) -> impl Iterator<Item = (Action, &[Chord])> {
        self.map.iter().map(|(a, c)| (*a, c.as_slice()))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChordError {
    UnknownKey,
    Malformed,
}

/// Parses `Ctrl+Shift+LeftClick` and friends.
pub fn parse_chord(text: &str) -> Result<Chord, ChordError> {
    let mut mods = Mods::NONE;
    let mut trigger = None;
    let tokens: Vec<&str> = text.split('+').map(str::trim).collect();
    if tokens.is_empty() || tokens.iter().any(|t| t.is_empty()) {
        return Err(ChordError::Malformed);
    }
    for (i, token) in tokens.iter().enumerate() {
        let last = i + 1 == tokens.len();
        match token.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "shift" => mods.shift = true,
            "alt" => mods.alt = true,
            _ => {
                if !last || trigger.is_some() {
                    return Err(ChordError::Malformed);
                }
                trigger = Some(parse_trigger(token).ok_or(ChordError::UnknownKey)?);
            }
        }
    }
    match trigger {
        Some(trigger) => Ok(Chord { mods, trigger }),
        None if mods.any() => Ok(Chord {
            mods,
            trigger: Trigger::ModifierOnly,
        }),
        None => Err(ChordError::Malformed),
    }
}

fn parse_trigger(token: &str) -> Option<Trigger> {
    let mouse = match token {
        "LeftClick" => Some(Trigger::Click(Button::Left)),
        "RightClick" => Some(Trigger::Click(Button::Right)),
        "MiddleClick" => Some(Trigger::Click(Button::Middle)),
        "DoubleLeftClick" => Some(Trigger::DoubleClick(Button::Left)),
        "DoubleRightClick" => Some(Trigger::DoubleClick(Button::Right)),
        "DoubleMiddleClick" => Some(Trigger::DoubleClick(Button::Middle)),
        "LeftDrag" => Some(Trigger::Drag(Button::Left)),
        "RightDrag" => Some(Trigger::Drag(Button::Right)),
        "MiddleDrag" => Some(Trigger::Drag(Button::Middle)),
        "MouseWheelUp" => Some(Trigger::WheelUp),
        "MouseWheelDown" => Some(Trigger::WheelDown),
        _ => None,
    };
    if mouse.is_some() {
        return mouse;
    }
    parse_key_code(token).map(Trigger::Key)
}

/// `A`..`Z`, `0`..`9`, or a winit `KeyCode` name.
pub fn parse_key_code(token: &str) -> Option<KeyCode> {
    let mut chars = token.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphabetic() {
            return letter(c.to_ascii_uppercase());
        }
        if c.is_ascii_digit() {
            return digit(c);
        }
    }
    if let Some(n) = token.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        return function_key(n);
    }
    if let Some(d) = token
        .strip_prefix("Numpad")
        .and_then(|d| d.chars().next())
        .filter(|d| d.is_ascii_digit() && token.len() == 7)
    {
        return numpad_digit(d);
    }
    if let Some(d) = token
        .strip_prefix("Digit")
        .and_then(|d| d.chars().next())
        .filter(|d| d.is_ascii_digit() && token.len() == 6)
    {
        return digit(d);
    }
    if let Some(c) = token
        .strip_prefix("Key")
        .and_then(|c| c.chars().next())
        .filter(|c| c.is_ascii_uppercase() && token.len() == 4)
    {
        return letter(c);
    }
    Some(match token {
        "ArrowUp" => KeyCode::ArrowUp,
        "ArrowDown" => KeyCode::ArrowDown,
        "ArrowLeft" => KeyCode::ArrowLeft,
        "ArrowRight" => KeyCode::ArrowRight,
        "Equal" => KeyCode::Equal,
        "Minus" => KeyCode::Minus,
        "Space" => KeyCode::Space,
        "Escape" => KeyCode::Escape,
        "Enter" => KeyCode::Enter,
        "Tab" => KeyCode::Tab,
        "Backspace" => KeyCode::Backspace,
        "Delete" => KeyCode::Delete,
        "Insert" => KeyCode::Insert,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "CapsLock" => KeyCode::CapsLock,
        "Backquote" => KeyCode::Backquote,
        "Comma" => KeyCode::Comma,
        "Period" => KeyCode::Period,
        "Slash" => KeyCode::Slash,
        "Backslash" => KeyCode::Backslash,
        "BracketLeft" => KeyCode::BracketLeft,
        "BracketRight" => KeyCode::BracketRight,
        "Semicolon" => KeyCode::Semicolon,
        "Quote" => KeyCode::Quote,
        "NumpadAdd" => KeyCode::NumpadAdd,
        "NumpadSubtract" => KeyCode::NumpadSubtract,
        "NumpadMultiply" => KeyCode::NumpadMultiply,
        "NumpadDivide" => KeyCode::NumpadDivide,
        "NumpadDecimal" => KeyCode::NumpadDecimal,
        "NumpadEnter" => KeyCode::NumpadEnter,
        _ => return None,
    })
}

fn letter(c: char) -> Option<KeyCode> {
    Some(match c {
        'A' => KeyCode::KeyA,
        'B' => KeyCode::KeyB,
        'C' => KeyCode::KeyC,
        'D' => KeyCode::KeyD,
        'E' => KeyCode::KeyE,
        'F' => KeyCode::KeyF,
        'G' => KeyCode::KeyG,
        'H' => KeyCode::KeyH,
        'I' => KeyCode::KeyI,
        'J' => KeyCode::KeyJ,
        'K' => KeyCode::KeyK,
        'L' => KeyCode::KeyL,
        'M' => KeyCode::KeyM,
        'N' => KeyCode::KeyN,
        'O' => KeyCode::KeyO,
        'P' => KeyCode::KeyP,
        'Q' => KeyCode::KeyQ,
        'R' => KeyCode::KeyR,
        'S' => KeyCode::KeyS,
        'T' => KeyCode::KeyT,
        'U' => KeyCode::KeyU,
        'V' => KeyCode::KeyV,
        'W' => KeyCode::KeyW,
        'X' => KeyCode::KeyX,
        'Y' => KeyCode::KeyY,
        'Z' => KeyCode::KeyZ,
        _ => return None,
    })
}

fn digit(c: char) -> Option<KeyCode> {
    Some(match c {
        '0' => KeyCode::Digit0,
        '1' => KeyCode::Digit1,
        '2' => KeyCode::Digit2,
        '3' => KeyCode::Digit3,
        '4' => KeyCode::Digit4,
        '5' => KeyCode::Digit5,
        '6' => KeyCode::Digit6,
        '7' => KeyCode::Digit7,
        '8' => KeyCode::Digit8,
        '9' => KeyCode::Digit9,
        _ => return None,
    })
}

fn numpad_digit(c: char) -> Option<KeyCode> {
    Some(match c {
        '0' => KeyCode::Numpad0,
        '1' => KeyCode::Numpad1,
        '2' => KeyCode::Numpad2,
        '3' => KeyCode::Numpad3,
        '4' => KeyCode::Numpad4,
        '5' => KeyCode::Numpad5,
        '6' => KeyCode::Numpad6,
        '7' => KeyCode::Numpad7,
        '8' => KeyCode::Numpad8,
        '9' => KeyCode::Numpad9,
        _ => return None,
    })
}

fn function_key(n: u8) -> Option<KeyCode> {
    Some(match n {
        1 => KeyCode::F1,
        2 => KeyCode::F2,
        3 => KeyCode::F3,
        4 => KeyCode::F4,
        5 => KeyCode::F5,
        6 => KeyCode::F6,
        7 => KeyCode::F7,
        8 => KeyCode::F8,
        9 => KeyCode::F9,
        10 => KeyCode::F10,
        11 => KeyCode::F11,
        12 => KeyCode::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use il_data::Binding;

    fn content(entries: &[(&str, &[&str])]) -> InputBindings {
        InputBindings {
            bindings: entries
                .iter()
                .map(|(a, k)| Binding {
                    action: (*a).to_string(),
                    keys: k.iter().map(|s| (*s).to_string()).collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn parses_modifiers_keys_and_mouse_tokens() {
        assert_eq!(
            parse_chord("Ctrl+Shift+A"),
            Ok(Chord {
                mods: Mods {
                    ctrl: true,
                    shift: true,
                    alt: false
                },
                trigger: Trigger::Key(KeyCode::KeyA)
            })
        );
        assert_eq!(
            parse_chord("Shift+LeftDrag").unwrap().trigger,
            Trigger::Drag(Button::Left)
        );
        assert_eq!(
            parse_chord("DoubleLeftClick").unwrap().trigger,
            Trigger::DoubleClick(Button::Left)
        );
        assert_eq!(
            parse_chord("MouseWheelUp").unwrap().trigger,
            Trigger::WheelUp
        );
        assert_eq!(
            parse_chord("Alt"),
            Ok(Chord {
                mods: Mods::ALT,
                trigger: Trigger::ModifierOnly
            })
        );
        assert_eq!(
            parse_chord("NumpadAdd").unwrap().trigger,
            Trigger::Key(KeyCode::NumpadAdd)
        );
        assert_eq!(
            parse_chord("7").unwrap().trigger,
            Trigger::Key(KeyCode::Digit7)
        );
        assert_eq!(
            parse_chord("F12").unwrap().trigger,
            Trigger::Key(KeyCode::F12)
        );
        assert_eq!(
            parse_chord("Numpad3").unwrap().trigger,
            Trigger::Key(KeyCode::Numpad3)
        );
        assert_eq!(
            parse_chord("KeyQ").unwrap().trigger,
            Trigger::Key(KeyCode::KeyQ)
        );
    }

    #[test]
    fn rejects_malformed_and_unknown_chords() {
        assert_eq!(parse_chord("A+B"), Err(ChordError::Malformed));
        assert_eq!(parse_chord("Ctrl+"), Err(ChordError::Malformed));
        assert_eq!(parse_chord(""), Err(ChordError::Malformed));
        assert_eq!(parse_chord("Hyper"), Err(ChordError::UnknownKey));
        assert_eq!(parse_chord("F13"), Err(ChordError::UnknownKey));
    }

    #[test]
    fn action_names_round_trip() {
        for (name, action) in FIXED_ACTIONS {
            assert_eq!(Action::from_name(name), Some(*action));
            assert_eq!(action.name(), *name);
        }
        assert_eq!(Action::from_name("group_set_9"), Some(Action::GroupSet(9)));
        assert_eq!(
            Action::from_name("group_recall_0"),
            Some(Action::GroupRecall(0))
        );
        assert_eq!(Action::from_name("formation_4"), Some(Action::Formation(4)));
        assert_eq!(Action::from_name("formation_0"), None);
        assert_eq!(Action::from_name("group_set_10"), None);
        assert_eq!(Action::GroupSet(3).name(), "group_set_3");
    }

    #[test]
    fn later_entries_replace_earlier_ones_and_errors_are_reported() {
        let c = content(&[
            ("box_select", &["LeftDrag"]),
            ("box_select", &["Alt+LeftDrag"]),
            ("fly", &["F"]),
            ("select", &["LeftClick", "Ctrl+Hyper"]),
        ]);
        let (b, errors) = Bindings::from_content(&c);
        assert_eq!(
            b.chords(Action::BoxSelect),
            &[Chord {
                mods: Mods::ALT,
                trigger: Trigger::Drag(Button::Left)
            }]
        );
        assert_eq!(b.chords(Action::Select).len(), 1);
        assert!(b.chords(Action::Pause).is_empty());
        assert_eq!(errors.len(), 2);
        assert!(matches!(&errors[0], BindingError::UnknownAction(a) if a == "fly"));
        assert!(
            matches!(&errors[1], BindingError::UnknownKey { chord, .. } if chord == "Ctrl+Hyper")
        );
    }
}
