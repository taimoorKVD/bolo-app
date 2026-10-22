//! Push-to-talk chord matching.
//!
//! The default chord is Ctrl+Win with no extra key: recording starts when both
//! modifiers are down and stops when either one is released. A chord with a
//! trigger key (Ctrl+Space) starts on the trigger and stops when it is released.
//! Modifier-only chords require two modifiers so a lone Ctrl or Shift cannot
//! start a recording while the user is typing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Canonical key name (`space`, `f8`, `a`). `None` is a modifier-only chord.
    pub key: Option<String>,
}

impl Default for Hotkey {
    fn default() -> Self {
        Self {
            ctrl: true,
            alt: false,
            shift: false,
            win: true,
            key: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEdge {
    Pressed,
    Released,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyKind {
    Ctrl,
    Alt,
    Shift,
    Win,
    Named(String),
}

/// Keys currently held, after left/right modifiers have been folded together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeySnapshot {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub keys: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct EdgeListener {
    was_held: bool,
}

impl Default for EdgeListener {
    fn default() -> Self {
        Self { was_held: false }
    }
}

impl EdgeListener {
    pub fn holding(&self) -> bool {
        self.was_held
    }

    /// Update hold state. While `paused` is true the tracker stays in sync but
    /// does not emit, so finishing hotkey capture does not start a recording.
    pub fn observe(&mut self, held: bool, paused: bool) -> Option<HotkeyEdge> {
        if paused {
            self.was_held = held;
            return None;
        }
        let edge = match (self.was_held, held) {
            (false, true) => Some(HotkeyEdge::Pressed),
            (true, false) => Some(HotkeyEdge::Released),
            _ => None,
        };
        self.was_held = held;
        edge
    }
}

pub fn validate_hotkey(hotkey: &Hotkey) -> Result<(), String> {
    let modifiers = modifier_count(hotkey);
    if modifiers == 0 {
        return Err("Use at least one modifier (Ctrl, Alt, Shift, or Win).".into());
    }
    match &hotkey.key {
        None => {
            if modifiers < 2 {
                return Err(
                    "A modifier-only hotkey needs two modifiers, for example Ctrl + Win."
                        .into(),
                );
            }
        }
        Some(key) => {
            if !is_allowed_key(key) {
                return Err(format!("\"{key}\" can't be used as the hotkey trigger."));
            }
        }
    }
    Ok(())
}

pub fn is_held(snap: &KeySnapshot, hotkey: &Hotkey) -> bool {
    if !modifiers_match(snap, hotkey) {
        return false;
    }
    match &hotkey.key {
        None => snap.keys.is_empty(),
        Some(key) => snap.keys.len() == 1 && snap.keys.iter().next() == Some(key),
    }
}

pub fn modifiers_match(snap: &KeySnapshot, hotkey: &Hotkey) -> bool {
    snap.ctrl == hotkey.ctrl
        && snap.alt == hotkey.alt
        && snap.shift == hotkey.shift
        && snap.win == hotkey.win
}

pub fn is_participant(kind: &KeyKind, hotkey: &Hotkey) -> bool {
    match kind {
        KeyKind::Ctrl => hotkey.ctrl,
        KeyKind::Alt => hotkey.alt,
        KeyKind::Shift => hotkey.shift,
        KeyKind::Win => hotkey.win,
        KeyKind::Named(name) => hotkey.key.as_ref() == Some(name),
    }
}

/// Whether a key-down should be eaten so it does not land in the focused app.
pub fn should_swallow_keydown(kind: &KeyKind, snap_after: &KeySnapshot, hotkey: &Hotkey) -> bool {
    if !is_participant(kind, hotkey) {
        return false;
    }
    match &hotkey.key {
        Some(trigger) => {
            matches!(kind, KeyKind::Named(name) if name == trigger)
                && modifiers_match(snap_after, hotkey)
                && snap_after.keys.len() == 1
        }
        None => {
            is_held(snap_after, hotkey)
                && matches!(
                    kind,
                    KeyKind::Ctrl | KeyKind::Alt | KeyKind::Shift | KeyKind::Win
                )
        }
    }
}

pub fn display(hotkey: &Hotkey) -> String {
    let mut parts = Vec::new();
    if hotkey.ctrl {
        parts.push("Ctrl".to_string());
    }
    if hotkey.alt {
        parts.push("Alt".to_string());
    }
    if hotkey.shift {
        parts.push("Shift".to_string());
    }
    if hotkey.win {
        parts.push("Win".to_string());
    }
    if let Some(key) = &hotkey.key {
        parts.push(pretty_key(key));
    }
    if parts.is_empty() {
        "Not set".to_string()
    } else {
        parts.join(" + ")
    }
}

pub fn pretty_key(key: &str) -> String {
    match key {
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        "enter" => "Enter".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        "insert" => "Insert".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "Page Up".into(),
        "pagedown" => "Page Down".into(),
        "left" => "Left".into(),
        "right" => "Right".into(),
        "up" => "Up".into(),
        "down" => "Down".into(),
        "esc" => "Esc".into(),
        other => {
            if other.len() == 1 {
                other.to_ascii_uppercase()
            } else if other.starts_with('f') && other[1..].chars().all(|c| c.is_ascii_digit()) {
                format!("F{}", &other[1..])
            } else {
                other.to_string()
            }
        }
    }
}

pub fn is_allowed_key(key: &str) -> bool {
    matches!(
        key,
        "space"
            | "tab"
            | "enter"
            | "backspace"
            | "delete"
            | "insert"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "left"
            | "right"
            | "up"
            | "down"
            | "esc"
    ) || (key.len() == 1 && key.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()))
        || is_function_key(key)
}

fn is_function_key(key: &str) -> bool {
    let Some(rest) = key.strip_prefix('f') else {
        return false;
    };
    matches!(rest.parse::<u8>(), Ok(n) if (1..=12).contains(&n))
}

fn modifier_count(hotkey: &Hotkey) -> usize {
    usize::from(hotkey.ctrl)
        + usize::from(hotkey.alt)
        + usize::from(hotkey.shift)
        + usize::from(hotkey.win)
}

/// Fold a virtual-key code into a chord participant.
/// Unknown keys become `Named("vk:N")` so they still break a modifier-only chord.
pub fn kind_from_vk(vk: u16) -> KeyKind {
    match vk {
        0x10 | 0xA0 | 0xA1 => KeyKind::Shift,
        0x11 | 0xA2 | 0xA3 => KeyKind::Ctrl,
        0x12 | 0xA4 | 0xA5 => KeyKind::Alt,
        0x5B | 0x5C => KeyKind::Win,
        0x20 => KeyKind::Named("space".into()),
        0x09 => KeyKind::Named("tab".into()),
        0x0D => KeyKind::Named("enter".into()),
        0x08 => KeyKind::Named("backspace".into()),
        0x2E => KeyKind::Named("delete".into()),
        0x2D => KeyKind::Named("insert".into()),
        0x24 => KeyKind::Named("home".into()),
        0x23 => KeyKind::Named("end".into()),
        0x21 => KeyKind::Named("pageup".into()),
        0x22 => KeyKind::Named("pagedown".into()),
        0x25 => KeyKind::Named("left".into()),
        0x26 => KeyKind::Named("up".into()),
        0x27 => KeyKind::Named("right".into()),
        0x28 => KeyKind::Named("down".into()),
        0x1B => KeyKind::Named("esc".into()),
        code if (0x30..=0x39).contains(&code) => {
            KeyKind::Named(char::from_u32(u32::from(code)).unwrap_or('?').to_string())
        }
        code if (0x41..=0x5A).contains(&code) => KeyKind::Named(
            char::from_u32(u32::from(code))
                .unwrap_or('?')
                .to_ascii_lowercase()
                .to_string(),
        ),
        code if (0x70..=0x7B).contains(&code) => KeyKind::Named(format!("f{}", code - 0x70 + 1)),
        other => KeyKind::Named(format!("vk:{other}")),
    }
}

pub fn apply_kind(snap: &mut KeySnapshot, kind: &KeyKind, down: bool) {
    match kind {
        KeyKind::Ctrl => snap.ctrl = down,
        KeyKind::Alt => snap.alt = down,
        KeyKind::Shift => snap.shift = down,
        KeyKind::Win => snap.win = down,
        KeyKind::Named(name) => {
            if down {
                snap.keys.insert(name.clone());
            } else {
                snap.keys.remove(name);
            }
        }
    }
}

pub fn snapshot_from_kinds<'a>(kinds: impl IntoIterator<Item = &'a KeyKind>) -> KeySnapshot {
    let mut snap = KeySnapshot::default();
    for kind in kinds {
        apply_kind(&mut snap, kind, true);
    }
    snap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_space() -> Hotkey {
        Hotkey {
            ctrl: true,
            alt: false,
            shift: false,
            win: false,
            key: Some("space".into()),
        }
    }

    #[test]
    fn default_hotkey_is_ctrl_win() {
        let hotkey = Hotkey::default();
        assert!(validate_hotkey(&hotkey).is_ok());
        assert_eq!(display(&hotkey), "Ctrl + Win");
    }

    #[test]
    fn modifier_only_requires_two_modifiers() {
        let only_ctrl = Hotkey {
            ctrl: true,
            ..Hotkey {
                win: false,
                ..Hotkey::default()
            }
        };
        assert!(validate_hotkey(&only_ctrl).is_err());
    }

    #[test]
    fn ctrl_win_hold_and_release() {
        let hotkey = Hotkey::default();
        let mut snap = KeySnapshot::default();
        snap.ctrl = true;
        assert!(!is_held(&snap, &hotkey));
        snap.win = true;
        assert!(is_held(&snap, &hotkey));
        snap.keys.insert("a".into());
        assert!(!is_held(&snap, &hotkey));
    }

    #[test]
    fn ctrl_space_requires_exact_trigger() {
        let hotkey = ctrl_space();
        let mut snap = KeySnapshot {
            ctrl: true,
            ..KeySnapshot::default()
        };
        assert!(!is_held(&snap, &hotkey));
        snap.keys.insert("space".into());
        assert!(is_held(&snap, &hotkey));
        snap.keys.insert("a".into());
        assert!(!is_held(&snap, &hotkey));
        snap.shift = true;
        snap.keys.remove("a");
        assert!(!is_held(&snap, &hotkey));
    }

    #[test]
    fn edges_ignore_pause_and_repeats() {
        let mut listener = EdgeListener::default();
        assert_eq!(listener.observe(true, false), Some(HotkeyEdge::Pressed));
        assert_eq!(listener.observe(true, false), None);
        assert_eq!(listener.observe(true, true), None);
        assert_eq!(listener.observe(false, true), None);
        assert_eq!(listener.observe(true, false), Some(HotkeyEdge::Pressed));
        assert_eq!(listener.observe(false, false), Some(HotkeyEdge::Released));
    }

    #[test]
    fn swallows_trigger_and_completing_modifier_only() {
        let space = ctrl_space();
        let snap = KeySnapshot {
            ctrl: true,
            keys: BTreeSet::from(["space".into()]),
            ..KeySnapshot::default()
        };
        assert!(should_swallow_keydown(
            &KeyKind::Named("space".into()),
            &snap,
            &space
        ));
        assert!(!should_swallow_keydown(&KeyKind::Ctrl, &snap, &space));

        let bare = KeySnapshot {
            keys: BTreeSet::from(["space".into()]),
            ..KeySnapshot::default()
        };
        assert!(!should_swallow_keydown(
            &KeyKind::Named("space".into()),
            &bare,
            &space
        ));

        let chord = Hotkey::default();
        let partial = KeySnapshot {
            ctrl: true,
            ..KeySnapshot::default()
        };
        assert!(!should_swallow_keydown(&KeyKind::Ctrl, &partial, &chord));
        let full = KeySnapshot {
            ctrl: true,
            win: true,
            ..KeySnapshot::default()
        };
        assert!(should_swallow_keydown(&KeyKind::Win, &full, &chord));
    }

    #[test]
    fn vk_letters_and_win() {
        assert_eq!(kind_from_vk(0x41), KeyKind::Named("a".into()));
        assert_eq!(kind_from_vk(0x5B), KeyKind::Win);
        assert_eq!(kind_from_vk(0x20), KeyKind::Named("space".into()));
        assert_eq!(kind_from_vk(0x70), KeyKind::Named("f1".into()));
    }
}
