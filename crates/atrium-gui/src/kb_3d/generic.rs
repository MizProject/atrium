//! Generic 3D procedural keyboard rendition for unsupported or probing boards.

use atrium_core::keyboards::KeyDef;

use super::{
    CameraPreset, Kb3dCamera, KeyboardModel3d, Vertex, apply_board_tilt, cubic_bezier3,
    push_beveled_cap, push_box, push_tube,
};

const GENERIC_CX: f32 = 312.5;
const GENERIC_CZ: f32 = 127.5;
const GENERIC_SCALE: f32 = 0.0098;

pub const MODEL: KeyboardModel3d = KeyboardModel3d {
    id: "generic-unsupported",
    label: "Generic Keyboard",
    spec: None,
    keys: &GENERIC_KEYS,
    default_camera: Kb3dCamera {
        yaw: -0.36,
        pitch: 0.70,
        zoom: 7.05,
    },
    build_mesh: build_generic_parts,
};

pub fn preset_camera(preset: CameraPreset) -> Kb3dCamera {
    match preset {
        CameraPreset::Perspective => MODEL.default_camera,
        CameraPreset::Top => Kb3dCamera {
            yaw: 0.0,
            pitch: 1.46,
            zoom: 6.75,
        },
        CameraPreset::Side => Kb3dCamera {
            yaw: -1.18,
            pitch: 0.44,
            zoom: 7.10,
        },
        CameraPreset::Front => Kb3dCamera {
            yaw: 0.0,
            pitch: 0.30,
            zoom: 6.75,
        },
    }
}

/// 83-key compact 75% layout used as an anatomical reference when probing unsupported boards.
pub const GENERIC_KEYS: [KeyDef; 83] = [
    // Row 0: Esc + F1..F12 + Prt + Del (15 keys)
    KeyDef::new(0, "Esc", "Escape", 0x29, 10, 15, 35, 35),
    KeyDef::new(1, "F1", "F1", 0x3a, 56, 15, 35, 35),
    KeyDef::new(2, "F2", "F2", 0x3b, 94, 15, 35, 35),
    KeyDef::new(3, "F3", "F3", 0x3c, 132, 15, 35, 35),
    KeyDef::new(4, "F4", "F4", 0x3d, 170, 15, 35, 35),
    KeyDef::new(5, "F5", "F5", 0x3e, 216, 15, 35, 35),
    KeyDef::new(6, "F6", "F6", 0x3f, 254, 15, 35, 35),
    KeyDef::new(7, "F7", "F7", 0x40, 292, 15, 35, 35),
    KeyDef::new(8, "F8", "F8", 0x41, 330, 15, 35, 35),
    KeyDef::new(9, "F9", "F9", 0x42, 376, 15, 35, 35),
    KeyDef::new(10, "F10", "F10", 0x43, 414, 15, 35, 35),
    KeyDef::new(11, "F11", "F11", 0x44, 452, 15, 35, 35),
    KeyDef::new(12, "F12", "F12", 0x45, 490, 15, 35, 35),
    KeyDef::new(13, "Prt", "PrintScreen", 0x46, 542, 15, 35, 35),
    KeyDef::new(14, "Del", "Delete", 0x4c, 580, 15, 35, 35),
    // Row 1: Number row + Backspace + Home (15 keys)
    KeyDef::new(15, "`~", "Backquote", 0x35, 10, 53, 35, 35),
    KeyDef::new(16, "1!", "Digit1", 0x1e, 48, 53, 35, 35),
    KeyDef::new(17, "2@", "Digit2", 0x1f, 86, 53, 35, 35),
    KeyDef::new(18, "3#", "Digit3", 0x20, 124, 53, 35, 35),
    KeyDef::new(19, "4$", "Digit4", 0x21, 162, 53, 35, 35),
    KeyDef::new(20, "5%", "Digit5", 0x22, 200, 53, 35, 35),
    KeyDef::new(21, "6^", "Digit6", 0x23, 238, 53, 35, 35),
    KeyDef::new(22, "7&", "Digit7", 0x24, 276, 53, 35, 35),
    KeyDef::new(23, "8*", "Digit8", 0x25, 314, 53, 35, 35),
    KeyDef::new(24, "9(", "Digit9", 0x26, 352, 53, 35, 35),
    KeyDef::new(25, "0)", "Digit0", 0x27, 390, 53, 35, 35),
    KeyDef::new(26, "-_", "Minus", 0x2d, 428, 53, 35, 35),
    KeyDef::new(27, "=+", "Equal", 0x2e, 466, 53, 35, 35),
    KeyDef::new(28, "Back", "Backspace", 0x2a, 504, 53, 73, 35),
    KeyDef::new(29, "Home", "Home", 0x4a, 580, 53, 35, 35),
    // Row 2: Tab + QWERTY + PgUp (15 keys)
    KeyDef::new(30, "Tab", "Tab", 0x2b, 10, 91, 54, 35),
    KeyDef::new(31, "Q", "KeyQ", 0x14, 67, 91, 35, 35),
    KeyDef::new(32, "W", "KeyW", 0x1a, 105, 91, 35, 35),
    KeyDef::new(33, "E", "KeyE", 0x08, 143, 91, 35, 35),
    KeyDef::new(34, "R", "KeyR", 0x15, 181, 91, 35, 35),
    KeyDef::new(35, "T", "KeyT", 0x17, 219, 91, 35, 35),
    KeyDef::new(36, "Y", "KeyY", 0x1c, 257, 91, 35, 35),
    KeyDef::new(37, "U", "KeyU", 0x18, 295, 91, 35, 35),
    KeyDef::new(38, "I", "KeyI", 0x0c, 333, 91, 35, 35),
    KeyDef::new(39, "O", "KeyO", 0x12, 371, 91, 35, 35),
    KeyDef::new(40, "P", "KeyP", 0x13, 409, 91, 35, 35),
    KeyDef::new(41, "[{", "BracketLeft", 0x2f, 447, 91, 35, 35),
    KeyDef::new(42, "]}", "BracketRight", 0x30, 485, 91, 35, 35),
    KeyDef::new(43, "\\|", "Backslash", 0x31, 523, 91, 54, 35),
    KeyDef::new(44, "PgUp", "PageUp", 0x4b, 580, 91, 35, 35),
    // Row 3: Caps + ASDF + Enter + PgDn (14 keys)
    KeyDef::new(45, "Caps", "CapsLock", 0x39, 10, 129, 63, 35),
    KeyDef::new(46, "A", "KeyA", 0x04, 76, 129, 35, 35),
    KeyDef::new(47, "S", "KeyS", 0x16, 114, 129, 35, 35),
    KeyDef::new(48, "D", "KeyD", 0x07, 152, 129, 35, 35),
    KeyDef::new(49, "F", "KeyF", 0x09, 190, 129, 35, 35),
    KeyDef::new(50, "G", "KeyG", 0x0a, 228, 129, 35, 35),
    KeyDef::new(51, "H", "KeyH", 0x0b, 266, 129, 35, 35),
    KeyDef::new(52, "J", "KeyJ", 0x0d, 304, 129, 35, 35),
    KeyDef::new(53, "K", "KeyK", 0x0e, 342, 129, 35, 35),
    KeyDef::new(54, "L", "KeyL", 0x0f, 380, 129, 35, 35),
    KeyDef::new(55, ";:", "Semicolon", 0x33, 418, 129, 35, 35),
    KeyDef::new(56, "'\"", "Quote", 0x34, 456, 129, 35, 35),
    KeyDef::new(57, "Enter", "Enter", 0x28, 494, 129, 83, 35),
    KeyDef::new(58, "PgDn", "PageDown", 0x4e, 580, 129, 35, 35),
    // Row 4: Shifts + ZXCV + Up + End (14 keys)
    KeyDef::new(59, "LShift", "ShiftLeft", 0xe1, 10, 167, 82, 35),
    KeyDef::new(60, "Z", "KeyZ", 0x1d, 95, 167, 35, 35),
    KeyDef::new(61, "X", "KeyX", 0x1b, 133, 167, 35, 35),
    KeyDef::new(62, "C", "KeyC", 0x06, 171, 167, 35, 35),
    KeyDef::new(63, "V", "KeyV", 0x19, 209, 167, 35, 35),
    KeyDef::new(64, "B", "KeyB", 0x05, 247, 167, 35, 35),
    KeyDef::new(65, "N", "KeyN", 0x11, 285, 167, 35, 35),
    KeyDef::new(66, "M", "KeyM", 0x10, 323, 167, 35, 35),
    KeyDef::new(67, ",<", "Comma", 0x36, 361, 167, 35, 35),
    KeyDef::new(68, ".>", "Period", 0x37, 399, 167, 35, 35),
    KeyDef::new(69, "/?", "Slash", 0x38, 437, 167, 35, 35),
    KeyDef::new(70, "RShift", "ShiftRight", 0xe5, 475, 167, 64, 35),
    KeyDef::new(71, "Up", "ArrowUp", 0x52, 542, 167, 35, 35),
    KeyDef::new(72, "End", "End", 0x4d, 580, 167, 35, 35),
    // Row 5: Modifiers + Space + Arrow cluster (10 keys)
    KeyDef::new(73, "LCtrl", "ControlLeft", 0xe0, 10, 205, 44, 35),
    KeyDef::new(74, "LWin", "MetaLeft", 0xe3, 57, 205, 44, 35),
    KeyDef::new(75, "LAlt", "AltLeft", 0xe2, 104, 205, 44, 35),
    KeyDef::new(76, "Space", "Space", 0x2c, 151, 205, 232, 35),
    KeyDef::new(77, "RAlt", "AltRight", 0xe6, 386, 205, 35, 35),
    KeyDef::new(78, "Fn", "Fn", 0xfa, 424, 205, 35, 35),
    KeyDef::new(79, "RCtrl", "ControlRight", 0xe4, 462, 205, 38, 35),
    KeyDef::new(80, "Left", "ArrowLeft", 0x50, 504, 205, 35, 35),
    KeyDef::new(81, "Down", "ArrowDown", 0x51, 542, 205, 35, 35),
    KeyDef::new(82, "Right", "ArrowRight", 0x4f, 580, 205, 35, 35),
];

fn build_generic_parts(verts: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    // Prototype anodized titanium-slate top bezel + dark graphite recessed plate.
    let base_col = [0.068, 0.078, 0.098];
    let rim_col = [0.125, 0.145, 0.180];
    let plate_col = [0.028, 0.032, 0.042];

    // Base chassis tray.
    push_box(
        verts,
        indices,
        [-3.18, -0.23, -1.32],
        [3.18, -0.02, 1.32],
        base_col,
        [0.0, -1.0, 0.0, 0.0],
    );
    // Switch plate floor.
    push_box(
        verts,
        indices,
        [-3.04, -0.02, -1.20],
        [3.04, 0.002, 1.20],
        plate_col,
        [1.0, -1.0, 0.0, 0.0],
    );

    // Anodized bezel frame rails around the 75% deck.
    push_box(
        verts,
        indices,
        [-3.18, -0.02, -1.32],
        [3.18, 0.095, -1.20],
        rim_col,
        [0.0, -1.0, 0.0, 0.0],
    );
    push_box(
        verts,
        indices,
        [-3.18, -0.02, 1.20],
        [3.18, 0.095, 1.32],
        rim_col,
        [0.0, -1.0, 0.0, 0.0],
    );
    push_box(
        verts,
        indices,
        [-3.18, -0.02, -1.20],
        [-3.04, 0.095, 1.20],
        rim_col,
        [0.0, -1.0, 0.0, 0.0],
    );
    push_box(
        verts,
        indices,
        [3.04, -0.02, -1.20],
        [3.18, 0.095, 1.20],
        rim_col,
        [0.0, -1.0, 0.0, 0.0],
    );

    // Subtle brushed top bezel accent strip (metallic kind 1.0, slot -1 so it stays static).
    push_box(
        verts,
        indices,
        [2.25, 0.092, -1.275],
        [2.92, 0.104, -1.245],
        [0.18, 0.21, 0.25],
        [1.0, -1.0, 0.0, 0.0],
    );

    push_generic_cable(verts, indices);

    for (slot, k) in GENERIC_KEYS.iter().enumerate() {
        let slot_f = slot as f32;
        let cx = (k.x as f32 + k.width as f32 * 0.5 - GENERIC_CX) * GENERIC_SCALE;
        let cz = (k.y as f32 + k.height as f32 * 0.5 - GENERIC_CZ) * GENERIC_SCALE;
        let bw = (k.width as f32 - 3.4) * GENERIC_SCALE;
        let bd = (k.height as f32 - 3.4) * GENERIC_SCALE;

        let row_idx = ((k.y - 10) / 38).clamp(0, 5) as f32;
        let row_height_boost = (5.0 - row_idx) * 0.007;

        // Distinct prototype keycap colors: cyan/aqua accent Esc/Enter, slate mods, cool graphite alphas.
        let is_accent = matches!(k.name, "Esc" | "Enter");
        let is_mod = matches!(
            k.name,
            "Tab"
                | "Caps"
                | "LShift"
                | "RShift"
                | "LCtrl"
                | "LWin"
                | "LAlt"
                | "RAlt"
                | "Fn"
                | "RCtrl"
                | "Back"
                | "Del"
                | "Home"
                | "PgUp"
                | "PgDn"
                | "End"
                | "Prt"
        );
        let (cap_col, top_col) = if is_accent {
            ([0.08, 0.34, 0.44], [0.11, 0.44, 0.56])
        } else if is_mod {
            ([0.095, 0.110, 0.135], [0.125, 0.145, 0.175])
        } else {
            ([0.135, 0.150, 0.175], [0.175, 0.195, 0.225])
        };

        // Switch underglow halo resting on the plate.
        push_box(
            verts,
            indices,
            [cx - bw * 0.48, 0.002, cz - bd * 0.48],
            [cx + bw * 0.48, 0.022, cz + bd * 0.48],
            [1.0, 1.0, 1.0],
            [3.0, slot_f, 1.0, 0.0],
        );

        let bot_half_w = bw * 0.47;
        let bot_half_d = bd * 0.47;
        let top_half_w = (bw * 0.47 - 0.040).max(0.075);
        let top_half_d = (bd * 0.47 - 0.040).max(0.075);
        let y_bot = 0.028;
        let y_top_front = 0.142 + row_height_boost;
        let y_top_back = 0.158 + row_height_boost;

        push_beveled_cap(
            verts,
            indices,
            cx,
            cz,
            bot_half_w,
            bot_half_d,
            top_half_w,
            top_half_d,
            y_bot,
            y_top_front,
            y_top_back,
            cap_col,
            top_col,
            slot_f,
        );

        // Subtle legend window across the top face.
        let leg_w = (top_half_w * 0.55).min(0.10);
        push_box(
            verts,
            indices,
            [cx - leg_w, y_top_front + 0.005, cz - 0.012],
            [cx + leg_w, y_top_front + 0.011, cz + 0.012],
            [0.86, 0.94, 0.98],
            [4.0, slot_f, 0.72, 0.0],
        );
    }
}

fn push_generic_cable(verts: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    // Port, plug housing, and strain-relief boot use board-local kinds (< 4.5) so
    // `vs_main` tilts them locked to the rear chassis wall (`z = -1.32`).
    let port_x = -2.18_f32;
    let port_y = -0.095_f32;

    let shield_col = [0.16, 0.19, 0.24];
    let cavity_col = [0.014, 0.016, 0.022];
    let chrome_col = [0.28, 0.32, 0.38];
    let plug_col = [0.095, 0.108, 0.135];
    let plug_bevel_col = [0.125, 0.142, 0.175];
    let boot_col = [0.042, 0.046, 0.056];

    // Recessed Type-C receptacle bezel seated flush in rear chassis face (`z = -1.32`).
    push_box(
        verts,
        indices,
        [port_x - 0.165, port_y - 0.064, -1.342],
        [port_x + 0.165, port_y + 0.064, -1.314],
        shield_col,
        [1.0, -1.0, 0.0, 0.0],
    );
    // Recessed dark receptacle cavity.
    push_box(
        verts,
        indices,
        [port_x - 0.130, port_y - 0.042, -1.346],
        [port_x + 0.130, port_y + 0.042, -1.312],
        cavity_col,
        [0.0, -1.0, 0.0, 0.0],
    );
    // Metallic Type-C connector tongue bridging cavity and plug body.
    push_box(
        verts,
        indices,
        [port_x - 0.105, port_y - 0.032, -1.375],
        [port_x + 0.105, port_y + 0.032, -1.325],
        chrome_col,
        [1.0, -1.0, 0.0, 0.0],
    );
    // Molded USB-C plug housing connected directly to metallic collar.
    push_box(
        verts,
        indices,
        [port_x - 0.122, port_y - 0.050, -1.615],
        [port_x + 0.122, port_y + 0.050, -1.370],
        plug_col,
        [1.0, -1.0, 0.0, 0.0],
    );
    // Top/bottom bevel rib on plug housing.
    push_box(
        verts,
        indices,
        [port_x - 0.102, port_y - 0.058, -1.610],
        [port_x + 0.102, port_y + 0.058, -1.375],
        plug_bevel_col,
        [1.0, -1.0, 0.0, 0.0],
    );

    // Tapered strain-relief boot extending seamlessly from plug rear face (`z = -1.610..-1.775`).
    let boot_spine = [
        [port_x, port_y, -1.608],
        [port_x, port_y, -1.655],
        [port_x, port_y, -1.705],
        [port_x, port_y, -1.745],
        [port_x, port_y, -1.775],
    ];
    let boot_radii = [0.054, 0.048, 0.044, 0.038, 0.033];
    push_tube(verts, indices, &boot_spine, &boot_radii, 12, boot_col, 0.0);

    // Flexible desk cable starts in tilted world space (`kind = 6.0 >= 4.5`) at the exact boot exit
    // and drapes gently down onto the studio desk pad (`y = -0.30 + radius`).
    let exit_world = apply_board_tilt([port_x, port_y, -1.770]);
    let cable_r = 0.032_f32;
    let desk_y = -0.30 + cable_r;

    let c1 = [exit_world[0], exit_world[1] + 0.015, exit_world[2] - 0.28];
    let c2 = [port_x - 0.02, desk_y, -2.30];
    let join = [port_x - 0.06, desk_y, -2.68];

    let d1 = [port_x - 0.12, desk_y, -3.15];
    let d2 = [port_x - 0.28, desk_y, -3.65];
    let end = [port_x - 0.42, desk_y, -4.12];

    let mut spine = Vec::with_capacity(34);
    for i in 0..=16 {
        let t = i as f32 / 16.0;
        let mut p = cubic_bezier3(exit_world, c1, c2, join, t);
        p[1] = p[1].max(desk_y);
        spine.push(p);
    }
    for i in 1..=16 {
        let t = i as f32 / 16.0;
        spine.push(cubic_bezier3(join, d1, d2, end, t));
    }
    let radii = vec![cable_r; spine.len()];
    push_tube(verts, indices, &spine, &radii, 12, [0.10, 0.12, 0.16], 6.0);
}
