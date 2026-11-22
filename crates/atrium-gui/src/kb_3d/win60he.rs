//! 3D physical chassis, Type-C wiring, and keycap mesh builder for the Aula WIN 60 HE.

use atrium_core::keyboards::win60he;
use atrium_core::keymap::{KEYBOARD_CANVAS_HEIGHT, KEYBOARD_CANVAS_WIDTH, WIN60HE_KEYS};

use super::{
    CameraPreset, Kb3dCamera, KeyboardModel3d, Vertex, apply_board_tilt, cubic_bezier3,
    push_beveled_cap, push_box, push_tube,
};

const BOARD_SCALE: f32 = 0.01;
const BOARD_CX: f32 = KEYBOARD_CANVAS_WIDTH as f32 * 0.5 + 5.0;
const BOARD_CZ: f32 = KEYBOARD_CANVAS_HEIGHT as f32 * 0.5 + 5.0;

pub const MODEL: KeyboardModel3d = KeyboardModel3d {
    id: win60he::SPEC.id,
    label: win60he::SPEC.display_name,
    spec: Some(&win60he::SPEC),
    keys: &WIN60HE_KEYS,
    default_camera: Kb3dCamera {
        yaw: -0.34,
        pitch: 0.72,
        zoom: 6.45,
    },
    build_mesh: build_win60he_parts,
};

pub fn preset_camera(preset: CameraPreset) -> Kb3dCamera {
    match preset {
        CameraPreset::Perspective => MODEL.default_camera,
        CameraPreset::Top => Kb3dCamera {
            yaw: 0.0,
            pitch: 1.46,
            zoom: 6.20,
        },
        CameraPreset::Side => Kb3dCamera {
            yaw: -1.18,
            pitch: 0.44,
            zoom: 6.50,
        },
        CameraPreset::Front => Kb3dCamera {
            yaw: 0.0,
            pitch: 0.30,
            zoom: 6.20,
        },
    }
}

fn build_win60he_parts(verts: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    // Matte black WIN 60 HE aluminum/ABS base chassis + recessed switch plate.
    let case_col = [0.052, 0.056, 0.068];
    let rim_col = [0.066, 0.072, 0.086];
    let plate_col = [0.025, 0.028, 0.036];

    // Base chassis block below switch plate.
    push_box(
        verts,
        indices,
        [-3.06, -0.22, -1.19],
        [3.06, -0.02, 1.19],
        case_col,
        [0.0, -1.0, 0.0, 0.0],
    );
    // Recessed switch plate floor where RGB shines upward.
    push_box(
        verts,
        indices,
        [-2.94, -0.02, -1.07],
        [2.94, 0.002, 1.07],
        plate_col,
        [1.0, -1.0, 0.0, 0.0],
    );

    // Four raised perimeter bezel rails around the recessed plate.
    push_box(
        verts,
        indices,
        [-3.06, -0.02, -1.19],
        [3.06, 0.095, -1.07],
        rim_col,
        [0.0, -1.0, 0.0, 0.2],
    );
    push_box(
        verts,
        indices,
        [-3.06, -0.02, 1.07],
        [3.06, 0.095, 1.19],
        rim_col,
        [0.0, -1.0, 0.0, 0.2],
    );
    push_box(
        verts,
        indices,
        [-3.06, -0.02, -1.07],
        [-2.94, 0.095, 1.07],
        rim_col,
        [0.0, -1.0, 0.0, 0.2],
    );
    push_box(
        verts,
        indices,
        [2.94, -0.02, -1.07],
        [3.06, 0.095, 1.07],
        rim_col,
        [0.0, -1.0, 0.0, 0.2],
    );

    // Signature WIN 60 HE protruding left-side strap / lanyard lug near Esc/Tab.
    push_box(
        verts,
        indices,
        [-3.27, -0.10, -1.08],
        [-3.06, 0.075, -0.97],
        rim_col,
        [0.0, -1.0, 0.0, 0.1],
    );
    push_box(
        verts,
        indices,
        [-3.27, -0.10, -0.45],
        [-3.06, 0.075, -0.34],
        rim_col,
        [0.0, -1.0, 0.0, 0.1],
    );
    push_box(
        verts,
        indices,
        [-3.27, -0.10, -0.97],
        [-3.18, 0.075, -0.45],
        rim_col,
        [0.0, -1.0, 0.0, 0.1],
    );

    // Subtle bottom-right AULA front bezel accent.
    let accent_col = [0.24, 0.27, 0.33];
    for i in 0..4 {
        let x0 = 2.15 + i as f32 * 0.18;
        push_box(
            verts,
            indices,
            [x0, 0.092, 1.11],
            [x0 + 0.12, 0.103, 1.155],
            accent_col,
            [1.0, -1.0, 0.0, 0.0],
        );
    }

    // Back-left USB Type-C port, plug housing, strain relief, and braided desk wiring.
    push_type_c_wiring(verts, indices);

    // Build all 61 beveled keycaps, glowing switch diffusers, and legends/diffuser strip.
    let cap_col = [0.075, 0.082, 0.098];
    let top_col = [0.095, 0.104, 0.122];

    for (slot, k) in WIN60HE_KEYS.iter().enumerate() {
        let slot_f = slot as f32;
        let cx = (k.x as f32 + k.width as f32 * 0.5 - BOARD_CX) * BOARD_SCALE;
        let cz = (k.y as f32 + k.height as f32 * 0.5 - BOARD_CZ) * BOARD_SCALE;
        let bw = (k.width as f32 - 3.6) * BOARD_SCALE;
        let bd = (k.height as f32 - 3.6) * BOARD_SCALE;

        let row_idx = ((k.y - 10) / 41).clamp(0, 4) as f32;
        let row_height_boost = (4.0 - row_idx) * 0.008;

        // Per-key underglow switch halo resting on the plate.
        push_box(
            verts,
            indices,
            [cx - bw * 0.49, 0.002, cz - bd * 0.49],
            [cx + bw * 0.49, 0.022, cz + bd * 0.49],
            [1.0, 1.0, 1.0],
            [3.0, slot_f, 1.0, 0.0],
        );

        let bot_half_w = bw * 0.47;
        let bot_half_d = bd * 0.47;
        let top_half_w = (bw * 0.47 - 0.042).max(0.08);
        let top_half_d = (bd * 0.47 - 0.042).max(0.08);
        let y_bot = 0.028;
        let y_top_front = 0.145 + row_height_boost;
        let y_top_back = 0.162 + row_height_boost;

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

        if k.name == "Space" {
            push_box(
                verts,
                indices,
                [cx - 0.19, y_top_front + 0.006, cz - 0.011],
                [cx + 0.19, y_top_front + 0.012, cz + 0.011],
                [0.95, 0.98, 1.0],
                [4.0, slot_f, 0.85, 0.0],
            );
        } else {
            let leg_w = (top_half_w * 0.55).min(0.08);
            push_box(
                verts,
                indices,
                [cx - top_half_w * 0.72, y_top_back - 0.002, cz - top_half_d * 0.55],
                [
                    cx - top_half_w * 0.72 + leg_w,
                    y_top_back + 0.004,
                    cz - top_half_d * 0.55 + 0.026,
                ],
                [0.72, 0.78, 0.88],
                [4.0, slot_f, 0.45, 0.0],
            );
        }
    }
}

fn push_type_c_wiring(verts: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    // Authentic Aula WIN 60 HE rear-left USB Type-C port (behind Esc / 1!).
    let usb_x = -2.42_f32;
    let usb_y = -0.075_f32;

    let shield_col = [0.16, 0.19, 0.25];
    let cavity_col = [0.014, 0.016, 0.022];
    let chrome_col = [0.30, 0.34, 0.41];
    let plug_col = [0.095, 0.108, 0.135];
    let plug_bevel_col = [0.125, 0.142, 0.175];
    let collar_band_col = [0.14, 0.30, 0.56];
    let boot_col = [0.036, 0.040, 0.050];

    push_box(
        verts,
        indices,
        [usb_x - 0.168, usb_y - 0.068, -1.215],
        [usb_x + 0.168, usb_y + 0.068, -1.185],
        shield_col,
        [1.0, -1.0, 0.0, 0.0],
    );
    push_box(
        verts,
        indices,
        [usb_x - 0.132, usb_y - 0.046, -1.220],
        [usb_x + 0.132, usb_y + 0.046, -1.183],
        cavity_col,
        [0.0, -1.0, 0.0, 0.0],
    );

    push_box(
        verts,
        indices,
        [usb_x - 0.110, usb_y - 0.035, -1.245],
        [usb_x + 0.110, usb_y + 0.035, -1.195],
        chrome_col,
        [1.0, -1.0, 0.0, 0.0],
    );
    push_box(
        verts,
        indices,
        [usb_x - 0.125, usb_y - 0.052, -1.495],
        [usb_x + 0.125, usb_y + 0.052, -1.240],
        plug_col,
        [1.0, -1.0, 0.0, 0.0],
    );
    push_box(
        verts,
        indices,
        [usb_x - 0.105, usb_y - 0.060, -1.490],
        [usb_x + 0.105, usb_y + 0.060, -1.245],
        plug_bevel_col,
        [1.0, -1.0, 0.0, 0.0],
    );
    push_box(
        verts,
        indices,
        [usb_x - 0.128, usb_y - 0.055, -1.455],
        [usb_x + 0.128, usb_y + 0.055, -1.425],
        collar_band_col,
        [1.0, -1.0, 0.0, 0.0],
    );

    let boot_spine = [
        [usb_x, usb_y, -1.490],
        [usb_x, usb_y, -1.522],
        [usb_x, usb_y, -1.536],
        [usb_x, usb_y, -1.564],
        [usb_x, usb_y, -1.578],
        [usb_x, usb_y, -1.608],
        [usb_x, usb_y, -1.622],
        [usb_x, usb_y, -1.680],
    ];
    let boot_radii = [0.056, 0.056, 0.044, 0.052, 0.042, 0.047, 0.039, 0.036];
    push_tube(verts, indices, &boot_spine, &boot_radii, 14, boot_col, 0.0);

    let exit_world = apply_board_tilt([usb_x, usb_y, -1.675]);
    let cable_r = 0.032_f32;
    let desk_y = -0.30 + cable_r;

    let mut cable_pts = Vec::with_capacity(38);
    let c1 = [exit_world[0], exit_world[1] + 0.018, exit_world[2] - 0.28];
    let c2 = [usb_x - 0.02, desk_y, -2.18];
    let join = [usb_x - 0.08, desk_y, -2.56];
    for i in 0..=18 {
        let t = i as f32 / 18.0;
        cable_pts.push(cubic_bezier3(exit_world, c1, c2, join, t));
    }

    let d1 = [usb_x - 0.14, desk_y, -3.05];
    let d2 = [usb_x - 0.40, desk_y, -3.65];
    let end = [usb_x - 0.54, desk_y, -4.18];
    for i in 1..=18 {
        let t = i as f32 / 18.0;
        cable_pts.push(cubic_bezier3(join, d1, d2, end, t));
    }

    let cable_radii = vec![cable_r; cable_pts.len()];
    let braid_col = [0.050, 0.058, 0.078];
    push_tube(verts, indices, &cable_pts, &cable_radii, 14, braid_col, 6.0);

    let cuff_pts = [
        cubic_bezier3(join, d1, d2, end, 0.30),
        cubic_bezier3(join, d1, d2, end, 0.42),
    ];
    let cuff_radii = [0.044, 0.044];
    push_tube(verts, indices, &cuff_pts, &cuff_radii, 14, collar_band_col, 6.5);
}
