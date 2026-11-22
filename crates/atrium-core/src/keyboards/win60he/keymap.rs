//! Key definitions, physical geometry, and matrix lookup for the Aula WIN 60 HE (61-key ANSI).
//!
//! The firmware organizes keys in a 6x22 matrix (`slot = row * 22 + col`,
//! `0..132`). These 61 slots match the vendor `SI2825KZHEARGB` / `SI2825HEARGB`
//! keymap and produce the hardware-verified all-keys bitmask:
//! `3e 26 3e 1e 1e 1e 3e 1e 1e 3e 3e 3e 3e 00 0e 00 00 00 00 00 00 00`.

pub use crate::keyboards::KeyDef;

/// All 61 physical keys of the Aula WIN 60 HE in physical row-major order.
pub const WIN60HE_KEYS: [KeyDef; 61] = [
    KeyDef::new(22, "Esc", "Escape", 0x29, 10, 15, 35, 35),
    KeyDef::new(23, "1!", "Digit1", 0x1e, 48, 15, 35, 35),
    KeyDef::new(24, "2@", "Digit2", 0x1f, 86, 15, 35, 35),
    KeyDef::new(25, "3#", "Digit3", 0x20, 124, 15, 35, 35),
    KeyDef::new(26, "4$", "Digit4", 0x21, 162, 15, 35, 35),
    KeyDef::new(27, "5%", "Digit5", 0x22, 200, 15, 35, 35),
    KeyDef::new(28, "6^", "Digit6", 0x23, 238, 15, 35, 35),
    KeyDef::new(29, "7&", "Digit7", 0x24, 276, 15, 35, 35),
    KeyDef::new(30, "8*", "Digit8", 0x25, 314, 15, 35, 35),
    KeyDef::new(31, "9(", "Digit9", 0x26, 352, 15, 35, 35),
    KeyDef::new(32, "0)", "Digit0", 0x27, 390, 15, 35, 35),
    KeyDef::new(33, "-_", "Minus", 0x2d, 428, 15, 35, 35),
    KeyDef::new(34, "=+", "Equal", 0x2e, 466, 15, 35, 35),
    KeyDef::new(36, "Back", "Backspace", 0x2a, 504, 15, 73, 35),
    KeyDef::new(44, "Tab", "Tab", 0x2b, 10, 53, 54, 35),
    KeyDef::new(45, "Q", "KeyQ", 0x14, 67, 53, 35, 35),
    KeyDef::new(46, "W", "KeyW", 0x1a, 105, 53, 35, 35),
    KeyDef::new(47, "E", "KeyE", 0x08, 143, 53, 35, 35),
    KeyDef::new(48, "R", "KeyR", 0x15, 181, 53, 35, 35),
    KeyDef::new(49, "T", "KeyT", 0x17, 219, 53, 35, 35),
    KeyDef::new(50, "Y", "KeyY", 0x1c, 257, 53, 35, 35),
    KeyDef::new(51, "U", "KeyU", 0x18, 295, 53, 35, 35),
    KeyDef::new(52, "I", "KeyI", 0x0c, 333, 53, 35, 35),
    KeyDef::new(53, "O", "KeyO", 0x12, 371, 53, 35, 35),
    KeyDef::new(54, "P", "KeyP", 0x13, 409, 53, 35, 35),
    KeyDef::new(55, "[{", "BracketLeft", 0x2f, 447, 53, 35, 35),
    KeyDef::new(56, "]}", "BracketRight", 0x30, 485, 53, 35, 35),
    KeyDef::new(58, "\\|", "Backslash", 0x31, 523, 53, 54, 35),
    KeyDef::new(66, "Caps", "CapsLock", 0x39, 10, 91, 63, 35),
    KeyDef::new(68, "A", "KeyA", 0x04, 76, 91, 35, 35),
    KeyDef::new(69, "S", "KeyS", 0x16, 114, 91, 35, 35),
    KeyDef::new(70, "D", "KeyD", 0x07, 152, 91, 35, 35),
    KeyDef::new(71, "F", "KeyF", 0x09, 190, 91, 35, 35),
    KeyDef::new(72, "G", "KeyG", 0x0a, 228, 91, 35, 35),
    KeyDef::new(73, "H", "KeyH", 0x0b, 266, 91, 35, 35),
    KeyDef::new(74, "J", "KeyJ", 0x0d, 304, 91, 35, 35),
    KeyDef::new(75, "K", "KeyK", 0x0e, 342, 91, 35, 35),
    KeyDef::new(76, "L", "KeyL", 0x0f, 380, 91, 35, 35),
    KeyDef::new(77, ";:", "Semicolon", 0x33, 418, 91, 35, 35),
    KeyDef::new(78, "\" '", "Quote", 0x34, 456, 91, 35, 35),
    KeyDef::new(80, "Enter", "Enter", 0x28, 494, 91, 83, 35),
    KeyDef::new(88, "L-Shift", "ShiftLeft", 0xe1, 10, 129, 83, 35),
    KeyDef::new(90, "Z", "KeyZ", 0x1d, 96, 129, 35, 35),
    KeyDef::new(91, "X", "KeyX", 0x1b, 134, 129, 35, 35),
    KeyDef::new(92, "C", "KeyC", 0x06, 172, 129, 35, 35),
    KeyDef::new(93, "V", "KeyV", 0x19, 210, 129, 35, 35),
    KeyDef::new(94, "B", "KeyB", 0x05, 248, 129, 35, 35),
    KeyDef::new(95, "N", "KeyN", 0x11, 286, 129, 35, 35),
    KeyDef::new(96, "M", "KeyM", 0x10, 324, 129, 35, 35),
    KeyDef::new(97, ",<", "Comma", 0x36, 362, 129, 35, 35),
    KeyDef::new(98, ".>", "Period", 0x37, 400, 129, 35, 35),
    KeyDef::new(99, "/?", "Slash", 0x38, 438, 129, 35, 35),
    KeyDef::new(100, "R-Shift", "ShiftRight", 0xe5, 476, 129, 101, 35),
    KeyDef::new(110, "L-Ctrl", "ControlLeft", 0xe0, 10, 167, 47, 35),
    KeyDef::new(111, "L-Win", "MetaLeft", 0xe3, 60, 167, 47, 35),
    KeyDef::new(112, "L-Alt", "AltLeft", 0xe2, 110, 167, 47, 35),
    KeyDef::new(116, "Space", "Space", 0x2c, 160, 167, 217, 35),
    KeyDef::new(119, "R-Alt", "AltRight", 0xe6, 380, 167, 47, 35),
    KeyDef::new(120, "Menu", "ContextMenu", 0x65, 430, 167, 47, 35),
    KeyDef::new(121, "R-Ctrl", "ControlRight", 0xe4, 480, 167, 47, 35),
    KeyDef::new(122, "Fn", "KeyFn", 0xfa, 530, 167, 47, 35),
];

/// Logical keyboard canvas dimensions matching the vendor coordinates.
pub const KEYBOARD_CANVAS_WIDTH: u16 = 587;
pub const KEYBOARD_CANVAS_HEIGHT: u16 = 217;

/// Symbolic matrix slot constants for common WIN 60 HE keys.
pub mod idx {
    pub const ESC: u8 = 22;
    pub const TAB: u8 = 44;
    pub const W: u8 = 46;
    pub const R: u8 = 48;
    pub const T: u8 = 49;
    pub const CAPS: u8 = 66;
    pub const A: u8 = 68;
    pub const S: u8 = 69;
    pub const D: u8 = 70;
    pub const F: u8 = 71;
    pub const G: u8 = 72;
    pub const H: u8 = 73;
    pub const L_CTRL: u8 = 110;
    pub const L_WIN: u8 = 111;
    pub const L_ALT: u8 = 112;
    pub const SPACE: u8 = 116;
    pub const R_ALT: u8 = 119;
    pub const MENU: u8 = 120;
    pub const R_CTRL: u8 = 121;
    pub const FN: u8 = 122;
}

/// Return all 61 matrix indices in layout order.
pub fn all_indices() -> Vec<u8> {
    WIN60HE_KEYS.iter().map(|k| k.index).collect()
}

/// Look up a physical key by matrix slot index.
pub fn by_index(index: u8) -> Option<&'static KeyDef> {
    WIN60HE_KEYS.iter().find(|k| k.index == index)
}

/// Look up a single key by name, web code, common alias, or numeric index.
pub fn resolve_key(token: &str) -> Option<&'static KeyDef> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(idx) = trimmed.parse::<u8>()
        && let Some(k) = by_index(idx)
    {
        return Some(k);
    }

    let upper = trimmed.to_ascii_uppercase();
    let canonical = match upper.as_str() {
        "ESC" => "Escape",
        "1" => "Digit1",
        "2" => "Digit2",
        "3" => "Digit3",
        "4" => "Digit4",
        "5" => "Digit5",
        "6" => "Digit6",
        "7" => "Digit7",
        "8" => "Digit8",
        "9" => "Digit9",
        "0" => "Digit0",
        "-" | "MINUS" => "Minus",
        "=" | "EQUAL" | "EQUALS" => "Equal",
        "BKSP" | "BACKSPACE" => "Backspace",
        "[" | "LBRACKET" => "BracketLeft",
        "]" | "RBRACKET" => "BracketRight",
        "\\" | "BSLASH" => "Backslash",
        "CAPS" | "CAPSLOCK" => "CapsLock",
        ";" | "SEMI" => "Semicolon",
        "'" | "QUOTE" | "APOSTROPHE" => "Quote",
        "RETURN" => "Enter",
        "LSHIFT" | "L-SHIFT" | "SHIFT" => "ShiftLeft",
        "RSHIFT" | "R-SHIFT" => "ShiftRight",
        "," | "COMMA" => "Comma",
        "." | "DOT" | "PERIOD" => "Period",
        "/" | "SLASH" => "Slash",
        "LCTRL" | "L-CTRL" | "CTRL" => "ControlLeft",
        "RCTRL" | "R-CTRL" => "ControlRight",
        "LWIN" | "L-WIN" | "WIN" | "SUPER" | "META" => "MetaLeft",
        "LALT" | "L-ALT" | "ALT" => "AltLeft",
        "RALT" | "R-ALT" => "AltRight",
        "APP" | "MENU" => "ContextMenu",
        "FN" => "KeyFn",
        _ => "",
    };

    if !canonical.is_empty() {
        return WIN60HE_KEYS.iter().find(|k| k.code == canonical);
    }

    WIN60HE_KEYS.iter().find(|k| {
        k.name.eq_ignore_ascii_case(trimmed) || k.code.eq_ignore_ascii_case(trimmed)
    })
}

/// Parse a comma-separated key selector (`all`, `wasd`, or key names/codes/indices).
pub fn parse_key_selector(expr: &str) -> Result<Vec<u8>, String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("all") {
        return Ok(all_indices());
    }

    let mut indices = Vec::new();
    for part in trimmed.split(',') {
        let token = part.trim();
        if token.is_empty() {
            continue;
        }
        if token.eq_ignore_ascii_case("wasd") {
            for k in ["W", "A", "S", "D"] {
                let idx = resolve_key(k)
                    .expect("W/A/S/D must exist in layout")
                    .index;
                if !indices.contains(&idx) {
                    indices.push(idx);
                }
            }
            continue;
        }
        let key = resolve_key(token)
            .ok_or_else(|| format!("unknown key '{token}' (try e.g. W, Esc, Space, L-Shift, or index 46)"))?;
        if !indices.contains(&key.index) {
            indices.push(key.index);
        }
    }

    if indices.is_empty() {
        return Err("empty key selection".into());
    }
    Ok(indices)
}

const VK_HID_MAP: &[(crate::ipc::VirtualKey, u8)] = &[
    (crate::ipc::VirtualKey::Up, 0x52),
    (crate::ipc::VirtualKey::Down, 0x51),
    (crate::ipc::VirtualKey::Left, 0x50),
    (crate::ipc::VirtualKey::Right, 0x4F),
    (crate::ipc::VirtualKey::Home, 0x4A),
    (crate::ipc::VirtualKey::End, 0x4D),
    (crate::ipc::VirtualKey::PageUp, 0x4B),
    (crate::ipc::VirtualKey::PageDown, 0x4E),
    (crate::ipc::VirtualKey::Insert, 0x49),
    (crate::ipc::VirtualKey::Delete, 0x4C),
    (crate::ipc::VirtualKey::Escape, 0x29),
    (crate::ipc::VirtualKey::Grave, 0x35),
    (crate::ipc::VirtualKey::PrintScreen, 0x46),
    (crate::ipc::VirtualKey::F1, 0x3A),
    (crate::ipc::VirtualKey::F2, 0x3B),
    (crate::ipc::VirtualKey::F3, 0x3C),
    (crate::ipc::VirtualKey::F4, 0x3D),
    (crate::ipc::VirtualKey::F5, 0x3E),
    (crate::ipc::VirtualKey::F6, 0x3F),
    (crate::ipc::VirtualKey::F7, 0x40),
    (crate::ipc::VirtualKey::F8, 0x41),
    (crate::ipc::VirtualKey::F9, 0x42),
    (crate::ipc::VirtualKey::F10, 0x43),
    (crate::ipc::VirtualKey::F11, 0x44),
    (crate::ipc::VirtualKey::F12, 0x45),
];

/// Map a standard [`VirtualKey`](crate::ipc::VirtualKey) to its USB HID Keyboard usage (`0x07`).
///
/// Consumer-control keys (`Mute`, `VolumeUp`, etc.) cannot be encoded as plain HID `0x07` keymap
/// entries and return `None` so callers keep them handled via `atriumd` `uinput`.
pub fn virtual_key_to_hid_usage(vk: crate::ipc::VirtualKey) -> Option<u8> {
    VK_HID_MAP
        .iter()
        .find(|(k, _)| *k == vk)
        .map(|(_, hid)| *hid)
}

/// Map a USB HID Keyboard usage (`0x07`) to a [`VirtualKey`](crate::ipc::VirtualKey), if represented.
pub fn hid_usage_to_virtual_key(hid: u8) -> Option<crate::ipc::VirtualKey> {
    VK_HID_MAP
        .iter()
        .find(|(_, code)| *code == hid)
        .map(|(k, _)| *k)
}

/// Human-readable name for a USB HID Keyboard usage (`0x07`).
pub fn usb_hid_name(hid: u8) -> String {
    if let Some(vk) = hid_usage_to_virtual_key(hid) {
        return vk.label().to_string();
    }
    if let Some(key) = WIN60HE_KEYS.iter().find(|k| k.hid_code == hid) {
        return key.name.to_string();
    }
    format!("HID 0x{hid:02X}")
}

/// Map a USB HID Keyboard usage (`0x07`) to a Linux `<linux/input-event-codes.h>` `KEY_*` code.
pub fn hid_usage_to_linux_ev_key(hid: u8) -> Option<u16> {
    const ALPHA: [u16; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17,
        45, 21, 44,
    ];
    const NUM_ROW: [u16; 10] = [2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
    const FKEYS: [u16; 12] = [59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88];

    let code = match hid {
        0x04..=0x1D => ALPHA[usize::from(hid - 0x04)],
        0x1E..=0x27 => NUM_ROW[usize::from(hid - 0x1E)],
        0x28 => 28,
        0x29 => 1,
        0x2A => 14,
        0x2B => 15,
        0x2C => 57,
        0x2D => 12,
        0x2E => 13,
        0x2F => 26,
        0x30 => 27,
        0x31 | 0x32 => 43,
        0x33 => 39,
        0x34 => 40,
        0x35 => 41,
        0x36 => 51,
        0x37 => 52,
        0x38 => 53,
        0x39 => 58,
        0x3A..=0x45 => FKEYS[usize::from(hid - 0x3A)],
        0x46 => 99,
        0x47 => 70,
        0x48 => 119,
        0x49 => 110,
        0x4A => 102,
        0x4B => 104,
        0x4C => 111,
        0x4D => 107,
        0x4E => 109,
        0x4F => 106,
        0x50 => 105,
        0x51 => 108,
        0x52 => 103,
        0x65 => 127,
        0xE0 => 29,
        0xE1 => 42,
        0xE2 => 56,
        0xE3 => 125,
        0xE4 => 97,
        0xE5 => 54,
        0xE6 => 100,
        0xE7 => 126,
        _ => return None,
    };
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn unique_61_keys() {
        let set: HashSet<u8> = WIN60HE_KEYS.iter().map(|k| k.index).collect();
        assert_eq!(set.len(), 61);
    }

    #[test]
    fn rows_align_to_577() {
        // Every row in a 60% ANSI board ends at x+width = 577.
        for row_end_idx in [36u8, 58, 80, 100, 122] {
            let k = by_index(row_end_idx).unwrap();
            assert_eq!(k.x + k.width, 577);
        }
    }

    #[test]
    fn resolve_common_names() {
        assert_eq!(resolve_key("w").unwrap().index, 46);
        assert_eq!(resolve_key("Esc").unwrap().index, 22);
        assert_eq!(resolve_key("ShiftLeft").unwrap().index, 88);
        assert_eq!(resolve_key("fn").unwrap().index, 122);
        assert_eq!(resolve_key("116").unwrap().code, "Space");
    }

    #[test]
    fn parse_wasd_and_all() {
        assert_eq!(parse_key_selector("wasd").unwrap(), vec![46, 68, 69, 70]);
        assert_eq!(parse_key_selector("all").unwrap().len(), 61);
    }

    #[test]
    fn all_non_fn_win60_keys_map_to_linux_ev_key() {
        for key in WIN60HE_KEYS {
            if key.is_fn() {
                assert_eq!(hid_usage_to_linux_ev_key(key.hid_code), None);
            } else {
                assert!(
                    hid_usage_to_linux_ev_key(key.hid_code).is_some(),
                    "missing Linux EV_KEY mapping for {} (hid=0x{:02X})",
                    key.name,
                    key.hid_code
                );
            }
        }
        assert_eq!(virtual_key_to_hid_usage(crate::ipc::VirtualKey::Up), Some(0x52));
        assert_eq!(virtual_key_to_hid_usage(crate::ipc::VirtualKey::Mute), None);
    }
}

