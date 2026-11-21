//! Regal Navy (`#203C62`) three-shade paper-matte design system and procedural
//! archival grain texture for `reaula-gui`.
//!
//! Surface colors are strictly constrained to three related shades of `#203C62`:
//! - [`SHADE_DEEP`] (`#101D30`): window canvas, recessed trays, input wells, footer
//! - [`SHADE_BASE`] (`#172B46`): cards, sidebar rail, top bar, default controls
//! - [`SHADE_REGAL`] (`#203C62`): active states, primary actions, selected controls

use std::f32::consts::TAU;
use std::sync::Arc;

use gpui::{Bounds, Corners, Element, Hsla, Pixels, RenderImage, canvas, point, prelude::*, px, size};
use image::{Frame, RgbaImage};
use smallvec::SmallVec;

// ---------------------------------------------------------------------------
// Strict 3-Shade Regal Navy Surface Palette (Hue ~214.5°, Matte S ~50%)
// ---------------------------------------------------------------------------

/// Shade 1 — Deep Inkwell Navy (`#101D30`): canvas backdrop & recessed wells.
pub const SHADE_DEEP: u32 = 0x101D30;
/// Shade 2 — Matte Midnight Navy (`#172B46`): primary cards, sidebar & default controls.
pub const SHADE_BASE: u32 = 0x172B46;
/// Shade 3 — Regal Navy (`#203C62`): exact anchor swatch for active/selected surfaces & CTAs.
pub const SHADE_REGAL: u32 = 0x203C62;

// ---------------------------------------------------------------------------
// Etched Matte Hairline Borders (Same 214.5° Regal Navy Tonal Ramp)
// ---------------------------------------------------------------------------

/// Recessed panel seam on `SHADE_DEEP` / `SHADE_BASE`.
pub const BORDER_SUBTLE: u32 = 0x1F3656;
/// Standard paper-card & button border.
pub const BORDER_MATTE: u32 = 0x2A4770;
/// Active / raised edge on `SHADE_REGAL` surfaces.
pub const BORDER_REGAL: u32 = 0x3D6294;
/// Crisp focus ring & active navigation indicator.
pub const BORDER_FOCUS: u32 = 0x7BA1CE;

// ---------------------------------------------------------------------------
// Paper Ink Typography (Matte Chalk & Cool Slate derived from #203C62)
// ---------------------------------------------------------------------------

/// Primary matte chalk ink (headings, active labels, key metrics).
pub const INK_PRIMARY: u32 = 0xEEF3F8;
/// Secondary body ink (descriptions, inactive controls, legends).
pub const INK_SECONDARY: u32 = 0xB4C6DC;
/// Quiet metadata & caption ink (subtitles, slot numbers, telemetry).
pub const INK_MUTED: u32 = 0x768FAD;
/// Ultra-quiet embossed watermark / disabled ink.
pub const INK_DIM: u32 = 0x4E6B90;
/// Luminous frost-navy ink for emphasized measurements & active readouts.
pub const INK_ACCENT: u32 = 0xC6DCF5;

// ---------------------------------------------------------------------------
// Restrained Mineral Status Hairlines & Ink (Never loud neon boxes)
// ---------------------------------------------------------------------------

/// Muted sage-frost ink for confirmed OK/pass telemetry.
pub const INK_OK: u32 = 0x9BC7BC;
/// Muted warm parchment ink for recording/calibration/warning cues.
pub const INK_WARN: u32 = 0xDEC69A;
/// Subtle warm-matte border for active recording / calibration states.
pub const BORDER_WARN: u32 = 0x6C5E46;
/// Muted rose-clay ink for errors & destructive actions.
pub const INK_DANGER: u32 = 0xDB9CA6;
/// Subtle muted-rose hairline for error banners & destructive buttons.
pub const BORDER_DANGER: u32 = 0x5E3B46;

// ---------------------------------------------------------------------------
// Keymap Category Visual Taxonomy ([FW] Amber Flash vs. [DM] Aqua Daemon)
// ---------------------------------------------------------------------------

/// Firmware-level MCU flash (`cmd 0x18`) Amber Gold category tokens.
pub const FW_AMBER_BORDER: u32 = 0xF59E0B;
pub const FW_AMBER_DIM_BORDER: u32 = 0x855814;
pub const FW_AMBER_BG: u32 = 0x291E0D;
pub const FW_AMBER_GLOW: u32 = 0x453012;
pub const FW_AMBER_INK: u32 = 0xFCD34D;

/// Daemon-level (`reaulad` / `uinput` / `EVIOCGRAB`) Aqua Blue category tokens.
pub const DM_AQUA_BORDER: u32 = 0x00D7FF;
pub const DM_AQUA_DIM_BORDER: u32 = 0x0E7490;
pub const DM_AQUA_BG: u32 = 0x072938;
pub const DM_AQUA_ACTIVE: u32 = 0x083B4D;
pub const DM_AQUA_PULSE: u32 = 0x0F5E75;
pub const DM_AQUA_PULSE_BORDER: u32 = 0x7EF9FF;
pub const DM_AQUA_INK: u32 = 0x6EFCFF;
pub const DM_AQUA_PULSE_INK: u32 = 0xC4FEFF;

/// Shared Firmware + Daemon overlap slot highlight.
pub const OVERLAP_BORDER: u32 = 0xC084FC;
pub const OVERLAP_BG: u32 = 0x221F38;
pub const OVERLAP_INK: u32 = 0xE9D5FF;

/// Linear RGB equivalents for the `wgpu` 3D studio horizon (`SHADE_DEEP`) and desk (`SHADE_BASE`).
pub const STUDIO_CLEAR_LINEAR: [f64; 3] = [0.0052, 0.0123, 0.0296];
pub const STUDIO_DESK_LINEAR: [f32; 3] = [0.0108, 0.0242, 0.0612];

pub const GRAIN_TILE_SIZE: u32 = 256;

/// Monochromatic Regal Navy pressure ramp for live Hall-effect travel depth (`0.0..=max_mm`).
pub fn depth_matte_hsla(mm: f64, max_mm: f64) -> Hsla {
    let ratio = (mm / max_mm.max(0.5)).clamp(0.0, 1.0) as f32;
    Hsla {
        h: 214.5 / 360.0,
        s: 0.50 - ratio * 0.06,
        l: 0.21 + ratio * 0.23,
        a: 1.0,
    }
}

/// Generate a seamless `256×256` straight-BGRA archival cotton paper-matte texture.
///
/// Combines deterministic high-frequency mineral grain, subtle horizontal & vertical
/// wrapped cotton fibers, and a low-frequency periodic paper tooth so tiling at 1:1
/// produces zero seams and gives every flat surface a uniform dark paper-matte finish.
pub fn create_paper_grain_image() -> Arc<RenderImage> {
    let size_u32 = GRAIN_TILE_SIZE;
    let n = size_u32 as usize;

    // Precompute deterministic fine hash lattice wrapped cleanly mod N.
    let mut base_noise = vec![0.0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            base_noise[y * n + x] = hash_signed(x as u32, y as u32, 0xA24B_AED5);
        }
    }

    let mut bgra = vec![0u8; n * n * 4];
    let inv_n = 1.0 / (n as f32);

    for y in 0..n {
        let yf = y as f32 * inv_n * TAU;
        for x in 0..n {
            let xf = x as f32 * inv_n * TAU;
            let xm1 = (x + n - 1) % n;
            let xp1 = (x + 1) % n;
            let ym1 = (y + n - 1) % n;
            let yp1 = (y + 1) % n;

            // Fine crisp mineral grain.
            let grain = base_noise[y * n + x];
            // Soft directional cotton fibers (wrapped seamlessly).
            let fiber_h = 0.5 * (base_noise[y * n + xm1] + base_noise[y * n + xp1]);
            let fiber_v = 0.5 * (base_noise[ym1 * n + x] + base_noise[yp1 * n + x]);
            // Gentle low-frequency periodic matte paper tooth.
            let tooth = 0.5
                * ((xf * 2.0 + yf * 1.0).sin() * (yf * 3.0 - xf * 1.0).cos()
                    + (xf * 5.0 - yf * 4.0).sin() * 0.45);

            let combined = grain * 0.56 + fiber_h * 0.22 + fiber_v * 0.14 + tooth * 0.16;

            let idx = (y * n + x) * 4;
            if combined >= 0.0 {
                // Subtle cool chalk paper speck (straight BGRA).
                let alpha = (combined * 13.5).clamp(0.0, 14.0).round() as u8;
                bgra[idx] = 238;     // B
                bgra[idx + 1] = 224; // G
                bgra[idx + 2] = 208; // R
                bgra[idx + 3] = alpha;
            } else {
                // Subtle deep inkwell matte pore (straight BGRA).
                let alpha = (-combined * 16.0).clamp(0.0, 16.0).round() as u8;
                bgra[idx] = 28;      // B
                bgra[idx + 1] = 15;  // G
                bgra[idx + 2] = 8;   // R
                bgra[idx + 3] = alpha;
            }
        }
    }

    let img = RgbaImage::from_raw(size_u32, size_u32, bgra)
        .expect("paper grain buffer matches tile dimensions");
    let frames = SmallVec::from_elem(Frame::new(img), 1);
    Arc::new(RenderImage::new(frames))
}

/// Non-interactive full-window tiled paper-matte grain overlay.
///
/// Uses `gpui::canvas` directly without inserting any hitbox so pointer clicks,
/// drags, hovers, and wheel scrolling pass through unobstructed.
pub fn paper_grain_overlay(grain: Arc<RenderImage>) -> impl Element {
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, (), window, _| {
            let tile = px(GRAIN_TILE_SIZE as f32);
            let tile_f = GRAIN_TILE_SIZE as f32;
            let origin_x = f32::from(bounds.origin.x);
            let origin_y = f32::from(bounds.origin.y);
            let width = f32::from(bounds.size.width).max(0.0);
            let height = f32::from(bounds.size.height).max(0.0);

            let cols = (width / tile_f).ceil() as usize;
            let rows = (height / tile_f).ceil() as usize;

            for r in 0..rows {
                for c in 0..cols {
                    let tile_bounds = Bounds {
                        origin: point(
                            px(origin_x + (c as f32) * tile_f),
                            px(origin_y + (r as f32) * tile_f),
                        ),
                        size: size(tile, tile),
                    };
                    let _ = window.paint_image(
                        tile_bounds,
                        Corners::default(),
                        Arc::clone(&grain),
                        0,
                        false,
                    );
                }
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full()
}

fn hash_signed(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed;
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    let unit = (h as f32) / (u32::MAX as f32);
    unit * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_shade_palette_is_anchored_to_regal_navy() {
        assert_eq!(SHADE_REGAL, 0x203C62);
        assert_eq!(SHADE_BASE, 0x172B46);
        assert_eq!(SHADE_DEEP, 0x101D30);
    }

    #[test]
    fn paper_grain_image_has_subtle_matte_alpha() {
        let img = create_paper_grain_image();
        let sz = img.size(0);
        assert_eq!(sz.width.0, GRAIN_TILE_SIZE as i32);
        assert_eq!(sz.height.0, GRAIN_TILE_SIZE as i32);
        let bytes = img.as_bytes(0).expect("grain frame bytes");
        let max_alpha = bytes.chunks_exact(4).map(|px| px[3]).max().unwrap_or(0);
        assert!(max_alpha > 0 && max_alpha <= 20, "max_alpha={max_alpha}");
    }
}
