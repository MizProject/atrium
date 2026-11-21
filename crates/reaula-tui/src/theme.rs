//! Regal Navy three-shade palette, contrast-aware RGB/HSV color utilities,
//! and shared Ratatui styling primitives matching `reaula-gui`.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders};

// ---------------------------------------------------------------------------
// Strict 3-Shade Regal Navy Surface Palette (#101D30 / #172B46 / #203C62)
// ---------------------------------------------------------------------------

pub const SHADE_DEEP: Color = Color::Rgb(0x10, 0x1D, 0x30);
pub const SHADE_BASE: Color = Color::Rgb(0x17, 0x2B, 0x46);
pub const SHADE_REGAL: Color = Color::Rgb(0x20, 0x3C, 0x62);

// Borders
pub const BORDER_SUBTLE: Color = Color::Rgb(0x1F, 0x36, 0x56);
pub const BORDER_MATTE: Color = Color::Rgb(0x2A, 0x47, 0x70);
pub const BORDER_FOCUS: Color = Color::Rgb(0x7B, 0xA1, 0xCE);

// Ink
pub const INK_PRIMARY: Color = Color::Rgb(0xEE, 0xF3, 0xF8);
pub const INK_SECONDARY: Color = Color::Rgb(0xB4, 0xC6, 0xDC);
pub const INK_MUTED: Color = Color::Rgb(0x76, 0x8F, 0xAD);
pub const INK_DIM: Color = Color::Rgb(0x4E, 0x6B, 0x90);
pub const INK_ACCENT: Color = Color::Rgb(0xC6, 0xDC, 0xF5);

// Status
pub const INK_OK: Color = Color::Rgb(0x9B, 0xC7, 0xBC);
pub const INK_WARN: Color = Color::Rgb(0xDE, 0xC6, 0x9A);
pub const BORDER_WARN: Color = Color::Rgb(0x6C, 0x5E, 0x46);
pub const INK_DANGER: Color = Color::Rgb(0xDB, 0x9C, 0xA6);

// Keymap Layer Taxonomy ([FW] Amber Flash vs. [DM] Aqua Daemon vs. Shared)
pub const FW_AMBER_BORDER: Color = Color::Rgb(0xF5, 0x9E, 0x0B);
pub const FW_AMBER_BG: Color = Color::Rgb(0x29, 0x1E, 0x0D);
pub const FW_AMBER_INK: Color = Color::Rgb(0xFC, 0xD3, 0x4D);

pub const DM_AQUA_BORDER: Color = Color::Rgb(0x00, 0xD7, 0xFF);
pub const DM_AQUA_BG: Color = Color::Rgb(0x07, 0x29, 0x38);
pub const DM_AQUA_ACTIVE: Color = Color::Rgb(0x08, 0x3B, 0x4D);
pub const DM_AQUA_PULSE: Color = Color::Rgb(0x0F, 0x5E, 0x75);
pub const DM_AQUA_PULSE_BORDER: Color = Color::Rgb(0x7E, 0xF9, 0xFF);
pub const DM_AQUA_INK: Color = Color::Rgb(0x6E, 0xFC, 0xFF);

pub const OVERLAP_BORDER: Color = Color::Rgb(0xC0, 0x84, 0xFC);
pub const OVERLAP_BG: Color = Color::Rgb(0x22, 0x1F, 0x38);
pub const OVERLAP_INK: Color = Color::Rgb(0xE9, 0xD5, 0xFF);

pub const CYAN_HIGHLIGHT: Color = Color::Rgb(0x38, 0xBD, 0xF8);

pub const HUMAN_COLOR_PRESETS: [(&str, [u8; 3]); 10] = [
    ("Aqua Blue", [0, 215, 255]),
    ("Sky Blue", [56, 189, 248]),
    ("Mint Teal", [0, 255, 170]),
    ("Emerald", [16, 185, 129]),
    ("Amber Gold", [245, 158, 11]),
    ("Crimson", [239, 68, 68]),
    ("Hot Pink", [244, 63, 160]),
    ("Electric Violet", [168, 85, 247]),
    ("Pure White", [255, 255, 255]),
    ("Off / Black", [0, 0, 0]),
];

pub fn panel_block<'a>(title: impl Into<String>, focused: bool) -> Block<'a> {
    let border_fg = if focused { BORDER_FOCUS } else { BORDER_MATTE };
    let title_style = if focused {
        Style::default()
            .fg(CYAN_HIGHLIGHT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(INK_ACCENT)
            .add_modifier(Modifier::BOLD)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(Style::default().fg(border_fg))
        .style(Style::default().bg(SHADE_BASE).fg(INK_PRIMARY))
        .title(Span::styled(format!(" {} ", title.into()), title_style))
}

pub fn well_block<'a>(title: impl Into<String>, border_fg: Color) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_fg))
        .style(Style::default().bg(SHADE_DEEP).fg(INK_PRIMARY))
        .title(Span::styled(
            format!(" {} ", title.into()),
            Style::default()
                .fg(INK_SECONDARY)
                .add_modifier(Modifier::BOLD),
        ))
}

/// Monochromatic Regal Navy travel pressure color for keycaps (`0.0..=max_mm`).
pub fn depth_matte_rgb(mm: f64, max_mm: f64) -> Color {
    let t = (mm / max_mm.max(0.5)).clamp(0.0, 1.0);
    let r = (24.0 + t * 56.0).round() as u8;
    let g = (48.0 + t * 145.0).round() as u8;
    let b = (78.0 + t * 165.0).round() as u8;
    Color::Rgb(r, g, b)
}

pub fn contrast_ink(rgb: [u8; 3]) -> Color {
    let luma = 0.2126 * f32::from(rgb[0]) + 0.7152 * f32::from(rgb[1]) + 0.0722 * f32::from(rgb[2]);
    if luma > 142.0 {
        Color::Rgb(16, 29, 48)
    } else {
        INK_PRIMARY
    }
}

pub fn format_rgb_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

/// Convert sRGB `[0..255]` into HSV `(hue in 0..1, sat in 0..1, val in 0..1)`.
pub fn rgb_to_hsv(rgb: [u8; 3]) -> [f32; 3] {
    let r = f32::from(rgb[0]) / 255.0;
    let g = f32::from(rgb[1]) / 255.0;
    let b = f32::from(rgb[2]) / 255.0;
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let delta = max_c - min_c;

    let hue = if delta <= 1e-5 {
        0.0
    } else if (max_c - r).abs() <= 1e-5 {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if (max_c - g).abs() <= 1e-5 {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    let sat = if max_c <= 1e-5 { 0.0 } else { delta / max_c };
    [hue.rem_euclid(1.0), sat.clamp(0.0, 1.0), max_c.clamp(0.0, 1.0)]
}

/// Convert HSV `(hue in 0..1, sat in 0..1, val in 0..1)` back to sRGB `[0..255]`.
pub fn hsv_to_rgb_u8(hue: f32, sat: f32, val: f32) -> [u8; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let s = sat.clamp(0.0, 1.0);
    let v = val.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

pub fn describe_rgb_human(rgb: [u8; 3]) -> &'static str {
    let [h, s, v] = rgb_to_hsv(rgb);
    if v < 0.08 {
        return "Off / Black";
    }
    if s < 0.14 {
        return if v > 0.88 {
            "Pure White"
        } else if v > 0.45 {
            "Silver Gray"
        } else {
            "Charcoal"
        };
    }
    let deg = h * 360.0;
    match deg {
        d if !(14.0..344.0).contains(&d) => "Crimson Red",
        d if d < 28.0 => "Coral Orange",
        d if d < 46.0 => "Warm Amber",
        d if d < 66.0 => "Golden Yellow",
        d if d < 105.0 => "Lime Green",
        d if d < 152.0 => "Emerald Green",
        d if d < 176.0 => "Mint Teal",
        d if d < 200.0 => "Aqua Blue",
        d if d < 220.0 => "Sky Blue",
        d if d < 252.0 => "Cobalt Blue",
        d if d < 286.0 => "Electric Violet",
        d if d < 322.0 => "Magenta",
        _ => "Hot Pink",
    }
}

pub fn parse_hex_or_rgb(raw: &str) -> Option<[u8; 3]> {
    let trimmed = raw.trim();
    if let Some(hex) = trimmed.strip_prefix('#').or_else(|| {
        trimmed
            .chars()
            .all(|c| c.is_ascii_hexdigit())
            .then_some(trimmed)
    }) {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some([r, g, b]);
        }
        if hex.len() == 3 {
            let d = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| v * 17);
            return Some([d(&hex[0..1])?, d(&hex[1..2])?, d(&hex[2..3])?]);
        }
    }
    let parts: Vec<&str> = trimmed.split([',', ' ']).filter(|s| !s.is_empty()).collect();
    if parts.len() == 3 {
        return Some([
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_and_hex_helpers_roundtrip_cleanly() {
        let aqua = [0, 215, 255];
        let hsv = rgb_to_hsv(aqua);
        let back = hsv_to_rgb_u8(hsv[0], hsv[1], hsv[2]);
        assert!((i16::from(back[0]) - i16::from(aqua[0])).abs() <= 2);
        assert!((i16::from(back[1]) - i16::from(aqua[1])).abs() <= 2);
        assert!((i16::from(back[2]) - i16::from(aqua[2])).abs() <= 2);
        assert_eq!(describe_rgb_human(aqua), "Aqua Blue");
        assert_eq!(parse_hex_or_rgb("#00D7FF"), Some(aqua));
        assert_eq!(parse_hex_or_rgb("0, 215, 255"), Some(aqua));
    }
}
