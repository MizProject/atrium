//! Low-poly chibi 3D figure builders for the hidden `3+9` (Miku) and `4+1` (Teto) desk mascots.

use std::f32::consts::{PI, TAU};

use super::{
    Vertex, cross3, cubic_bezier3, dot3, face_normal, lerp3, linear_hex, normalize3,
    push_oriented_frustum, push_quad, sub3,
};

pub const KIND_MIKU: f32 = 7.0;
pub const KIND_TETO: f32 = 8.0;

pub const MIKU_PIVOT: [f32; 3] = [1.48, -0.30, -1.82];
pub const TETO_PIVOT: [f32; 3] = [-1.25, -0.30, -1.82];

const PEDESTAL_H: f32 = 0.06;

struct ChibiBuilder<'a> {
    verts: &'a mut Vec<Vertex>,
    indices: &'a mut Vec<u32>,
    kind: f32,
    pivot: [f32; 3],
    yaw: f32,
}

impl<'a> ChibiBuilder<'a> {
    fn new(
        verts: &'a mut Vec<Vertex>,
        indices: &'a mut Vec<u32>,
        kind: f32,
        pivot: [f32; 3],
        yaw: f32,
    ) -> Self {
        Self {
            verts,
            indices,
            kind,
            pivot,
            yaw,
        }
    }

    fn world_pt(&self, rel: [f32; 3]) -> [f32; 3] {
        let (sy, cy) = self.yaw.sin_cos();
        [
            self.pivot[0] + rel[0] * cy + rel[2] * sy,
            self.pivot[1] + rel[1],
            self.pivot[2] - rel[0] * sy + rel[2] * cy,
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn part(
        &mut self,
        offset: [f32; 3],
        size: [f32; 3],
        top_scale: [f32; 2],
        rot_xyz: [f32; 3],
        hex: u32,
        sway: f32,
        glow: f32,
    ) {
        let color = linear_hex(hex);
        let adjusted = [offset[0], offset[1] + PEDESTAL_H, offset[2]];
        let meta = [self.kind, sway, adjusted[0], glow];
        push_oriented_frustum(
            self.verts,
            self.indices,
            self.pivot,
            self.yaw,
            adjusted,
            size,
            top_scale,
            rot_xyz,
            color,
            meta,
        );
    }

    fn pedestal(&mut self, accent_hex: u32) {
        let base_col = linear_hex(0x131926);
        let rim_col = linear_hex(accent_hex);
        push_oriented_frustum(
            self.verts,
            self.indices,
            self.pivot,
            self.yaw,
            [0.0, PEDESTAL_H * 0.42, 0.0],
            [0.42, PEDESTAL_H * 0.84, 0.36],
            [0.92, 0.92],
            [0.0, 0.0, 0.0],
            base_col,
            [self.kind, 0.0, 0.0, 0.0],
        );
        push_oriented_frustum(
            self.verts,
            self.indices,
            self.pivot,
            self.yaw,
            [0.0, PEDESTAL_H * 0.90, 0.0],
            [0.39, 0.012, 0.33],
            [0.98, 0.98],
            [0.0, 0.0, 0.0],
            rim_col,
            [self.kind, 0.0, 0.0, 0.22],
        );
    }

    /// Build a tapered, corkscrew-fluted low-poly twin drill anchored cleanly from top clip to tip.
    fn helical_drill(&mut self, side: f32, bright_hex: u32, deep_hex: u32, tip_hex: u32) {
        let steps = 26usize;
        let sides = 10usize;
        let turns = 3.5_f32;

        let p0 = [side * 0.222, 0.788 + PEDESTAL_H, -0.010];
        let p1 = [side * 0.375, 0.748 + PEDESTAL_H, -0.004];
        let p2 = [side * 0.405, 0.520 + PEDESTAL_H, 0.014];
        let p3 = [side * 0.285, 0.285 + PEDESTAL_H, 0.044];

        let mut spine = Vec::with_capacity(steps + 1);
        let mut radii = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            spine.push(cubic_bezier3(p0, p1, p2, p3, t));

            // Snug at hair clip -> plump cone barrel beside cheeks -> sharp pointed curl tip.
            let swell = (t / 0.22).min(1.0).powf(0.68);
            let taper = 1.0 - ((t - 0.28).max(0.0) / 0.72).powf(1.22) * 0.94;
            radii.push(0.132 * (0.30 + 0.70 * swell) * taper);
        }

        let mut tangents = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let v = if i == 0 {
                sub3(spine[1], spine[0])
            } else if i == steps {
                sub3(spine[i], spine[i - 1])
            } else {
                sub3(spine[i + 1], spine[i - 1])
            };
            tangents.push(normalize3(v));
        }

        let seed = [0.0, 0.0, 1.0];
        let mut n1 = normalize3(cross3(tangents[0], seed));
        let bright = linear_hex(bright_hex);
        let deep = linear_hex(deep_hex);
        let tip = linear_hex(tip_hex);

        let mut rings: Vec<Vec<[f32; 3]>> = Vec::with_capacity(steps + 1);
        let mut center_pts: Vec<[f32; 3]> = Vec::with_capacity(steps + 1);
        let mut ring_cols: Vec<Vec<[f32; 3]>> = Vec::with_capacity(steps + 1);

        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let ti = tangents[i];
            n1 = normalize3(sub3(
                n1,
                [ti[0] * dot3(n1, ti), ti[1] * dot3(n1, ti), ti[2] * dot3(n1, ti)],
            ));
            let n2 = normalize3(cross3(ti, n1));

            let r_base = radii[i];
            let twist = side * t * turns * TAU;
            let helix_amp = r_base * 0.28 * (t * PI).sin();
            let center_rel = [
                spine[i][0] + (n1[0] * twist.cos() + n2[0] * twist.sin()) * helix_amp,
                spine[i][1] + (n1[1] * twist.cos() + n2[1] * twist.sin()) * helix_amp,
                spine[i][2] + (n1[2] * twist.cos() + n2[2] * twist.sin()) * helix_amp,
            ];
            center_pts.push(self.world_pt(center_rel));

            let mut ring = Vec::with_capacity(sides + 1);
            let mut cols = Vec::with_capacity(sides + 1);
            for j in 0..=sides {
                let frac = j as f32 / sides as f32;
                let theta = frac * TAU + twist;
                // Distinct corkscrew curl edge + recessed inner spiral groove.
                let ridge = if frac < 0.76 {
                    (frac / 0.76).powf(0.62)
                } else {
                    1.0 - ((frac - 0.76) / 0.24)
                };
                let r = r_base * (0.52 + 0.56 * ridge);
                let (st, ct) = theta.sin_cos();
                let off_rel = [
                    center_rel[0] + (n1[0] * ct + n2[0] * st) * r,
                    center_rel[1] + (n1[1] * ct + n2[1] * st) * r,
                    center_rel[2] + (n1[2] * ct + n2[2] * st) * r,
                ];
                ring.push(self.world_pt(off_rel));
                let col_mix = lerp3(deep, bright, ridge.powf(0.70));
                cols.push(lerp3(col_mix, tip, ((t - 0.68).max(0.0) / 0.32) * 0.45));
            }
            rings.push(ring);
            ring_cols.push(cols);
        }

        for i in 0..steps {
            let sway_weight = ((i as f32 / steps as f32) * 1.15).clamp(0.15, 1.0);
            let meta = [self.kind, sway_weight, side * 0.35, 0.16];
            for j in 0..sides {
                let q0 = rings[i][j];
                let q1 = rings[i][j + 1];
                let q2 = rings[i + 1][j + 1];
                let q3 = rings[i + 1][j];

                let quad_mid = [
                    (q0[0] + q1[0] + q2[0] + q3[0]) * 0.25,
                    (q0[1] + q1[1] + q2[1] + q3[1]) * 0.25,
                    (q0[2] + q1[2] + q2[2] + q3[2]) * 0.25,
                ];
                let core_mid = [
                    (center_pts[i][0] + center_pts[i + 1][0]) * 0.5,
                    (center_pts[i][1] + center_pts[i + 1][1]) * 0.5,
                    (center_pts[i][2] + center_pts[i + 1][2]) * 0.5,
                ];
                let outward = sub3(quad_mid, core_mid);
                let n = face_normal(q0, q1, q2);
                let col = ring_cols[i][j];
                if dot3(n, outward) >= 0.0 {
                    push_quad(self.verts, self.indices, q0, q1, q2, q3, n, col, meta);
                } else {
                    push_quad(
                        self.verts,
                        self.indices,
                        q0,
                        q3,
                        q2,
                        q1,
                        [-n[0], -n[1], -n[2]],
                        col,
                        meta,
                    );
                }
            }
        }
    }

    /// Build a continuous, gap-free flared rectangular low-poly twin-tail ribbon for Miku.
    fn smooth_twintail(&mut self, side: f32, main_hex: u32, shade_hex: u32) {
        let steps = 12usize;
        let p0 = [side * 0.245, 0.805 + PEDESTAL_H, -0.055];
        let p1 = [side * 0.365, 0.725 + PEDESTAL_H, -0.095];
        let p2 = [side * 0.420, 0.455 + PEDESTAL_H, -0.125];
        let p3 = [side * 0.445, 0.145 + PEDESTAL_H, -0.145];

        let mut spine = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            spine.push(cubic_bezier3(p0, p1, p2, p3, i as f32 / steps as f32));
        }

        let main_c = linear_hex(main_hex);
        let shade_c = linear_hex(shade_hex);

        let mut rings: Vec<[[f32; 3]; 4]> = Vec::with_capacity(steps + 1);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let tang = if i == 0 {
                sub3(spine[1], spine[0])
            } else if i == steps {
                sub3(spine[i], spine[i - 1])
            } else {
                sub3(spine[i + 1], spine[i - 1])
            };
            let tang = normalize3(tang);
            let fwd = [0.0, 0.0, 1.0];
            let right = normalize3(cross3(tang, fwd));
            let real_fwd = normalize3(cross3(right, tang));

            // Flare wider toward lower middle, taper cleanly at tip.
            let swell = 0.036 + 0.052 * (t * 0.88 * PI).sin().max(0.0);
            let tip_taper = if t > 0.75 { 1.0 - (t - 0.75) / 0.25 * 0.58 } else { 1.0 };
            let hw = swell * tip_taper;
            let hd = hw * 0.78;

            let c = spine[i];
            let mk = |sx: f32, sz: f32| -> [f32; 3] {
                self.world_pt([
                    c[0] + right[0] * hw * sx + real_fwd[0] * hd * sz,
                    c[1] + right[1] * hw * sx + real_fwd[1] * hd * sz,
                    c[2] + right[2] * hw * sx + real_fwd[2] * hd * sz,
                ])
            };
            rings.push([mk(-1.0, 1.0), mk(1.0, 1.0), mk(1.0, -1.0), mk(-1.0, -1.0)]);
        }

        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let sway = (0.25 + t * 0.90).min(1.0);
            let meta = [self.kind, sway, side * 0.35, 0.16];
            let col = lerp3(main_c, shade_c, (t - 0.45).max(0.0) / 0.55 * 0.85);
            let r0 = rings[i];
            let r1 = rings[i + 1];
            let c_mid = [
                (r0[0][0] + r0[2][0] + r1[0][0] + r1[2][0]) * 0.25,
                (r0[0][1] + r0[2][1] + r1[0][1] + r1[2][1]) * 0.25,
                (r0[0][2] + r0[2][2] + r1[0][2] + r1[2][2]) * 0.25,
            ];
            for k in 0..4 {
                let k_next = (k + 1) % 4;
                let q0 = r0[k];
                let q1 = r0[k_next];
                let q2 = r1[k_next];
                let q3 = r1[k];
                let q_mid = [
                    (q0[0] + q1[0] + q2[0] + q3[0]) * 0.25,
                    (q0[1] + q1[1] + q2[1] + q3[1]) * 0.25,
                    (q0[2] + q1[2] + q2[2] + q3[2]) * 0.25,
                ];
                let outward = sub3(q_mid, c_mid);
                let n = face_normal(q0, q1, q2);
                if dot3(n, outward) >= 0.0 {
                    push_quad(self.verts, self.indices, q0, q1, q2, q3, n, col, meta);
                } else {
                    push_quad(
                        self.verts,
                        self.indices,
                        q0,
                        q3,
                        q2,
                        q1,
                        [-n[0], -n[1], -n[2]],
                        col,
                        meta,
                    );
                }
            }
        }
    }
}

/// Append both hidden low-poly chibi meshes (culled in `vs_main` until triggered).
pub fn push_easter_egg_mascots(verts: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    build_chibi_miku(verts, indices);
    build_chibi_teto(verts, indices);
}

fn build_chibi_miku(verts: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    let mut b = ChibiBuilder::new(verts, indices, KIND_MIKU, MIKU_PIVOT, -0.22);

    let skin = 0xFCE3D6;
    let hair = 0x39C5BB;
    let hair_shade = 0x269B96;
    let blouse = 0xD2DAE4;
    let dark_suit = 0x191F2C;
    let magenta = 0xE83E8C;
    let eye_white = 0xFFFFFF;
    let blush = 0xF5989D;
    let leek_white = 0xEEF4EC;
    let leek_green = 0x5CC85A;

    b.pedestal(hair);

    // Legs & thigh-high boots
    for sx in [-1.0_f32, 1.0] {
        b.part([sx * 0.068, 0.015, 0.010], [0.095, 0.030, 0.130], [1.0, 1.0], [0.0, 0.0, 0.0], hair, 0.0, 0.15);
        b.part([sx * 0.068, 0.115, 0.0], [0.088, 0.175, 0.096], [1.05, 1.05], [0.0, 0.0, 0.0], dark_suit, 0.0, 0.0);
        b.part([sx * 0.068, 0.195, 0.0], [0.094, 0.022, 0.102], [1.0, 1.0], [0.0, 0.0, 0.0], hair, 0.0, 0.25);
        b.part([sx * 0.068, 0.228, 0.0], [0.078, 0.048, 0.084], [1.0, 1.0], [0.0, 0.0, 0.0], skin, 0.0, 0.0);
    }

    // Pleated dark skirt with teal hem stripe
    b.part([0.0, 0.250, 0.0], [0.275, 0.024, 0.220], [0.96, 0.96], [0.0, 0.0, 0.0], hair, 0.0, 0.20);
    b.part([0.0, 0.295, 0.0], [0.268, 0.095, 0.214], [0.76, 0.78], [0.0, 0.0, 0.0], dark_suit, 0.0, 0.0);

    // Silver-grey sleeveless vest torso & collar
    b.part([0.0, 0.415, 0.0], [0.198, 0.160, 0.148], [1.06, 1.04], [0.0, 0.0, 0.0], blouse, 0.0, 0.05);
    b.part([0.0, 0.486, 0.008], [0.172, 0.032, 0.142], [1.0, 1.0], [0.0, 0.0, 0.0], blouse, 0.0, 0.05);

    // Signature cyan necktie
    b.part([0.0, 0.478, 0.080], [0.036, 0.028, 0.022], [1.0, 1.0], [0.0, 0.0, 0.0], hair, 0.0, 0.25);
    b.part([0.0, 0.398, 0.086], [0.040, 0.145, 0.020], [0.68, 1.0], [-0.08, 0.0, 0.0], hair, 0.0, 0.25);

    // Detached sleeves + waving right arm holding a chibi leek (negi)
    b.part([-0.145, 0.405, 0.0], [0.070, 0.155, 0.072], [0.85, 0.85], [0.0, 0.0, -0.35], dark_suit, 0.0, 0.0);
    b.part([-0.168, 0.325, 0.0], [0.052, 0.052, 0.052], [0.9, 0.9], [0.0, 0.0, -0.35], skin, 0.0, 0.0);

    b.part([0.150, 0.455, 0.042], [0.068, 0.155, 0.070], [0.88, 0.88], [-0.45, 0.0, 0.65], dark_suit, 1.0, 0.05);
    b.part([0.205, 0.510, 0.085], [0.050, 0.052, 0.050], [0.9, 0.9], [-0.45, 0.0, 0.65], skin, 1.0, 0.0);
    b.part([0.228, 0.568, 0.105], [0.026, 0.125, 0.026], [1.0, 1.0], [-0.20, 0.0, 0.28], leek_white, 1.0, 0.15);
    b.part([0.244, 0.662, 0.118], [0.044, 0.085, 0.032], [1.3, 1.1], [-0.20, 0.0, 0.28], leek_green, 1.0, 0.25);

    // Neck + Chibi Head
    b.part([0.0, 0.512, 0.0], [0.072, 0.055, 0.072], [1.0, 1.0], [0.0, 0.0, 0.0], skin, 0.0, 0.0);
    b.part([0.0, 0.695, 0.0], [0.405, 0.355, 0.365], [0.95, 0.95], [0.0, 0.0, 0.0], skin, 0.0, 0.04);

    // Teal eyes + sparkle highlights + blush
    for sx in [-1.0_f32, 1.0] {
        b.part([sx * 0.098, 0.678, 0.184], [0.076, 0.088, 0.016], [1.0, 1.0], [0.0, 0.0, 0.0], dark_suit, 0.0, 0.0);
        b.part([sx * 0.098, 0.670, 0.189], [0.058, 0.068, 0.014], [1.0, 1.0], [0.0, 0.0, 0.0], hair, 0.0, 0.35);
        b.part([sx * 0.084, 0.694, 0.193], [0.022, 0.022, 0.012], [1.0, 1.0], [0.0, 0.0, 0.0], eye_white, 0.0, 0.55);
        b.part([sx * 0.132, 0.612, 0.184], [0.048, 0.024, 0.010], [1.0, 1.0], [0.0, 0.0, 0.0], blush, 0.0, 0.10);
    }

    // Hair cap, bangs, side locks, headset, twin-tails
    b.part([0.0, 0.845, -0.010], [0.435, 0.105, 0.398], [0.88, 0.88], [0.0, 0.0, 0.0], hair, 0.0, 0.16);
    b.part([0.0, 0.695, -0.175], [0.422, 0.345, 0.070], [0.94, 1.0], [0.0, 0.0, 0.0], hair_shade, 0.0, 0.10);
    for (bx, bw, rotz) in [
        (-0.135_f32, 0.095_f32, 0.08_f32),
        (-0.042, 0.088, 0.02),
        (0.048, 0.088, -0.02),
        (0.138, 0.095, -0.08),
    ] {
        b.part([bx, 0.785, 0.195], [bw, 0.118, 0.045], [0.82, 0.85], [0.08, 0.0, rotz], hair, 0.0, 0.18);
    }
    for sx in [-1.0_f32, 1.0] {
        b.part([sx * 0.212, 0.672, 0.055], [0.055, 0.285, 0.125], [0.90, 0.92], [0.0, 0.0, sx * 0.08], hair, 0.0, 0.14);
        b.part([sx * 0.228, 0.695, -0.010], [0.042, 0.128, 0.128], [0.90, 0.90], [0.0, 0.0, 0.0], dark_suit, 0.0, 0.0);
        b.part([sx * 0.246, 0.695, -0.010], [0.016, 0.072, 0.072], [1.0, 1.0], [0.0, 0.0, 0.0], magenta, 0.0, 0.40);
    }
    b.part([0.0, 0.902, -0.010], [0.430, 0.030, 0.052], [0.92, 0.95], [0.0, 0.0, 0.0], dark_suit, 0.0, 0.0);

    for sx in [-1.0_f32, 1.0] {
        b.part([sx * 0.245, 0.805, -0.055], [0.056, 0.068, 0.068], [1.0, 1.0], [0.0, 0.0, sx * -0.35], dark_suit, 0.5, 0.0);
        b.part([sx * 0.252, 0.805, -0.055], [0.062, 0.036, 0.036], [1.0, 1.0], [0.0, 0.0, sx * -0.35], magenta, 0.5, 0.35);

        b.smooth_twintail(sx, hair, hair_shade);
    }
}

fn build_chibi_teto(verts: &mut Vec<Vertex>, indices: &mut Vec<u32>) {
    let mut b = ChibiBuilder::new(verts, indices, KIND_TETO, TETO_PIVOT, 0.20);

    let skin = 0xFCE3D6;
    let hair = 0xEC3864;
    let hair_dark = 0x7A122C;
    let hair_tip = 0xF55E82;
    let jacket = 0x262B3A;
    let trim_red = 0xD8314E;
    let gold = 0xE6B84D;
    let shirt = 0xECEEF4;
    let eye_white = 0xFFFFFF;
    let blush = 0xF5989D;
    let baguette_crust = 0xD1934A;
    let baguette_score = 0xF0C77C;

    b.pedestal(trim_red);

    // Boots
    for sx in [-1.0_f32, 1.0] {
        b.part([sx * 0.068, 0.015, 0.010], [0.095, 0.030, 0.130], [1.0, 1.0], [0.0, 0.0, 0.0], trim_red, 0.0, 0.15);
        b.part([sx * 0.068, 0.115, 0.0], [0.088, 0.175, 0.096], [1.04, 1.04], [0.0, 0.0, 0.0], jacket, 0.0, 0.0);
        b.part([sx * 0.068, 0.220, 0.0], [0.078, 0.055, 0.084], [1.0, 1.0], [0.0, 0.0, 0.0], skin, 0.0, 0.0);
    }

    // Crimson pleated skirt & belt
    b.part([0.0, 0.248, 0.0], [0.278, 0.024, 0.222], [0.96, 0.96], [0.0, 0.0, 0.0], trim_red, 0.0, 0.20);
    b.part([0.0, 0.295, 0.0], [0.268, 0.096, 0.214], [0.76, 0.78], [0.0, 0.0, 0.0], hair_dark, 0.0, 0.05);

    // Double-breasted charcoal jacket + crimson lapels + waist belt
    b.part([0.0, 0.415, 0.0], [0.205, 0.165, 0.152], [1.06, 1.04], [0.0, 0.0, 0.0], jacket, 0.0, 0.0);
    b.part([0.0, 0.352, 0.0], [0.214, 0.026, 0.158], [1.0, 1.0], [0.0, 0.0, 0.0], trim_red, 0.0, 0.20);
    b.part([0.0, 0.450, 0.076], [0.068, 0.095, 0.018], [1.0, 1.0], [0.0, 0.0, 0.0], shirt, 0.0, 0.05);
    for sx in [-1.0_f32, 1.0] {
        b.part([sx * 0.052, 0.455, 0.079], [0.036, 0.095, 0.018], [0.9, 1.0], [0.0, 0.0, sx * -0.18], trim_red, 0.0, 0.20);
        b.part([sx * 0.108, 0.502, 0.0], [0.052, 0.018, 0.088], [1.0, 1.0], [0.0, 0.0, 0.0], trim_red, 0.0, 0.15);
    }
    b.part([0.0, 0.352, 0.083], [0.032, 0.030, 0.014], [1.0, 1.0], [0.0, 0.0, 0.0], gold, 0.0, 0.25);

    // Arms + French Baguette prop
    b.part([0.145, 0.405, 0.0], [0.070, 0.155, 0.072], [0.85, 0.85], [0.0, 0.0, 0.35], jacket, 0.0, 0.0);
    b.part([0.168, 0.325, 0.0], [0.052, 0.052, 0.052], [0.9, 0.9], [0.0, 0.0, 0.35], skin, 0.0, 0.0);

    b.part([-0.150, 0.455, 0.042], [0.068, 0.155, 0.070], [0.88, 0.88], [-0.45, 0.0, -0.65], jacket, 1.0, 0.05);
    b.part([-0.205, 0.510, 0.085], [0.050, 0.052, 0.050], [0.9, 0.9], [-0.45, 0.0, -0.65], skin, 1.0, 0.0);
    b.part([-0.235, 0.590, 0.110], [0.054, 0.180, 0.054], [0.88, 0.88], [-0.22, 0.0, -0.30], baguette_crust, 1.0, 0.08);
    b.part([-0.235, 0.595, 0.132], [0.038, 0.115, 0.018], [0.9, 0.9], [-0.22, 0.0, -0.30], baguette_score, 1.0, 0.12);

    // Neck + Chibi Head
    b.part([0.0, 0.512, 0.0], [0.072, 0.055, 0.072], [1.0, 1.0], [0.0, 0.0, 0.0], skin, 0.0, 0.0);
    b.part([0.0, 0.695, 0.0], [0.405, 0.355, 0.365], [0.95, 0.95], [0.0, 0.0, 0.0], skin, 0.0, 0.04);

    // Ruby anime eyes + catchlights + blush
    for sx in [-1.0_f32, 1.0] {
        b.part([sx * 0.098, 0.678, 0.184], [0.076, 0.088, 0.016], [1.0, 1.0], [0.0, 0.0, 0.0], jacket, 0.0, 0.0);
        b.part([sx * 0.098, 0.670, 0.189], [0.058, 0.068, 0.014], [1.0, 1.0], [0.0, 0.0, 0.0], trim_red, 0.0, 0.35);
        b.part([sx * 0.084, 0.694, 0.193], [0.022, 0.022, 0.012], [1.0, 1.0], [0.0, 0.0, 0.0], eye_white, 0.0, 0.55);
        b.part([sx * 0.132, 0.612, 0.184], [0.048, 0.024, 0.010], [1.0, 1.0], [0.0, 0.0, 0.0], blush, 0.0, 0.10);
    }

    // Crimson hair cap, ahoge, spiky bangs, side locks
    b.part([0.0, 0.848, -0.010], [0.436, 0.108, 0.400], [0.88, 0.88], [0.0, 0.0, 0.0], hair, 0.0, 0.16);
    b.part([0.015, 0.945, 0.015], [0.036, 0.095, 0.036], [0.45, 0.45], [-0.15, 0.0, -0.25], hair, 0.8, 0.20);
    b.part([0.0, 0.695, -0.175], [0.420, 0.305, 0.070], [0.94, 1.0], [0.0, 0.0, 0.0], hair_dark, 0.0, 0.10);

    for (bx, bw, rotz) in [
        (-0.138_f32, 0.092_f32, 0.12_f32),
        (-0.045, 0.088, -0.04),
        (0.045, 0.088, 0.04),
        (0.138, 0.092, -0.12),
    ] {
        b.part([bx, 0.785, 0.195], [bw, 0.122, 0.046], [0.72, 0.75], [0.10, 0.0, rotz], hair, 0.0, 0.18);
    }

    for sx in [-1.0_f32, 1.0] {
        b.part([sx * 0.212, 0.668, 0.055], [0.054, 0.265, 0.122], [0.88, 0.90], [0.0, 0.0, sx * 0.08], hair, 0.0, 0.14);
        // Dark & gold hair clips anchoring the twin drills
        b.part([sx * 0.228, 0.780, -0.012], [0.052, 0.062, 0.062], [1.0, 1.0], [0.0, 0.0, sx * -0.25], jacket, 0.3, 0.0);
        b.part([sx * 0.234, 0.780, -0.012], [0.058, 0.034, 0.034], [1.0, 1.0], [0.0, 0.0, sx * -0.25], gold, 0.3, 0.30);

        // Iconic tapered corkscrew twin drills draping downward beside her cheeks & shoulders
        b.helical_drill(sx, hair, hair_dark, hair_tip);
    }
}
