//! Clinical studio monochrome design system built on `gpui-kit` theme tokens,
//! with accessible hardware layer semantics (`[FW]` Amber Flash vs. `[DM]` Aqua).

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, Hsla, hsla, px, rgb};

pub const STUDIO_CLEAR_LINEAR: [f64; 3] = [0.0080, 0.0095, 0.0130];
pub const STUDIO_DESK_LINEAR: [f32; 3] = [0.0155, 0.0185, 0.0250];

pub fn apply_clinical_theme(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        theme.radius = px(6.);
        theme.radius_lg = px(8.);
        theme.shadow = false;
        theme.focus_ring = true;

        if mode.is_dark() {
            theme.colors.background = rgb(0x0B0D10).into();
            theme.colors.sidebar = rgb(0x101318).into();
            theme.colors.sidebar_border = rgb(0x232833).into();
            theme.colors.sidebar_accent = rgb(0x1E242E).into();
            theme.colors.sidebar_accent_foreground = rgb(0xF3F4F6).into();
            theme.colors.sidebar_primary = rgb(0xF3F4F6).into();
            theme.colors.sidebar_primary_foreground = rgb(0x0B0D10).into();
            theme.colors.group_box = rgb(0x12151B).into();
            theme.colors.group_box_foreground = rgb(0xF3F4F6).into();
            theme.colors.popover = rgb(0x141820).into();
            theme.colors.popover_foreground = rgb(0xF3F4F6).into();
            theme.colors.border = rgb(0x232833).into();
            theme.colors.input = rgb(0x2B313E).into();
            theme.colors.ring = rgb(0x9CA3AF).into();
            theme.colors.foreground = rgb(0xF3F4F6).into();
            theme.colors.muted = rgb(0x181C24).into();
            theme.colors.muted_foreground = rgb(0x9CA3AF).into();
            theme.colors.secondary = rgb(0x1B2029).into();
            theme.colors.secondary_hover = rgb(0x242A36).into();
            theme.colors.secondary_active = rgb(0x2D3544).into();
            theme.colors.secondary_foreground = rgb(0xE5E7EB).into();
            theme.colors.accent = rgb(0x1F2430).into();
            theme.colors.accent_foreground = rgb(0xF3F4F6).into();
            theme.colors.primary = rgb(0xF3F4F6).into();
            theme.colors.primary_hover = rgb(0xFFFFFF).into();
            theme.colors.primary_active = rgb(0xD1D5DB).into();
            theme.colors.primary_foreground = rgb(0x0B0D10).into();
            theme.colors.success = rgb(0x34D399).into();
            theme.colors.warning = rgb(0xFBBF24).into();
            theme.colors.danger = rgb(0xF87171).into();
            theme.colors.info = rgb(0x38BDF8).into();
            theme.colors.status_bar = rgb(0x101318).into();
            theme.colors.status_bar_border = rgb(0x232833).into();
        } else {
            theme.colors.background = rgb(0xFAFAFB).into();
            theme.colors.sidebar = rgb(0xF3F4F6).into();
            theme.colors.sidebar_border = rgb(0xE5E7EB).into();
            theme.colors.sidebar_accent = rgb(0xE5E7EB).into();
            theme.colors.sidebar_accent_foreground = rgb(0x111827).into();
            theme.colors.group_box = rgb(0xFFFFFF).into();
            theme.colors.group_box_foreground = rgb(0x111827).into();
            theme.colors.border = rgb(0xD1D5DB).into();
            theme.colors.input = rgb(0xD1D5DB).into();
            theme.colors.ring = rgb(0x4B5563).into();
            theme.colors.foreground = rgb(0x111827).into();
            theme.colors.muted = rgb(0xF3F4F6).into();
            theme.colors.muted_foreground = rgb(0x4B5563).into();
            theme.colors.secondary = rgb(0xE5E7EB).into();
            theme.colors.secondary_hover = rgb(0xD1D5DB).into();
            theme.colors.secondary_foreground = rgb(0x111827).into();
            theme.colors.primary = rgb(0x111827).into();
            theme.colors.primary_hover = rgb(0x1F2937).into();
            theme.colors.primary_foreground = rgb(0xFFFFFF).into();
            theme.colors.success = rgb(0x047857).into();
            theme.colors.warning = rgb(0xB45309).into();
            theme.colors.danger = rgb(0xB91C1C).into();
            theme.colors.info = rgb(0x0369A1).into();
            theme.colors.status_bar = rgb(0xF3F4F6).into();
            theme.colors.status_bar_border = rgb(0xE5E7EB).into();
        }
    });
}

#[derive(Clone, Copy)]
pub struct HardwareColors {
    pub is_dark: bool,
    pub deck_bg: u32,
    pub deck_border: u32,
    pub key_bg: u32,
    pub key_selected_bg: u32,
    pub key_border: u32,
    pub key_border_selected: u32,
    pub key_border_focus: u32,
    pub key_ink: u32,
    pub key_sub_ink: u32,
    pub key_dim_ink: u32,
    pub ok_bg: u32,
    pub ok_ink: u32,
    pub ok_border: u32,
    pub warn_bg: u32,
    pub warn_ink: u32,
    pub warn_border: u32,
    pub fw_bg: u32,
    pub fw_glow: u32,
    pub fw_border: u32,
    pub fw_dim_border: u32,
    pub fw_ink: u32,
    pub dm_bg: u32,
    pub dm_active: u32,
    pub dm_pulse: u32,
    pub dm_border: u32,
    pub dm_dim_border: u32,
    pub dm_pulse_border: u32,
    pub dm_ink: u32,
    pub dm_pulse_ink: u32,
    pub dm_muted_bg: u32,
    pub dm_muted_border: u32,
    pub dm_muted_ink: u32,
    pub dm_mod_bg: u32,
    pub dm_mod_border: u32,
    pub dm_mod_ink: u32,
    pub overlap_bg: u32,
    pub overlap_border: u32,
    pub overlap_ink: u32,
    pub track_bg: u32,
}

impl HardwareColors {
    pub fn from_app(cx: &App) -> Self {
        Self::for_mode(cx.theme().is_dark())
    }

    pub fn for_mode(is_dark: bool) -> Self {
        if is_dark {
            Self {
                is_dark: true,
                deck_bg: 0x0D1015,
                deck_border: 0x232833,
                key_bg: 0x151922,
                key_selected_bg: 0x262D3B,
                key_border: 0x282E3A,
                key_border_selected: 0x6B7280,
                key_border_focus: 0xE5E7EB,
                key_ink: 0xF3F4F6,
                key_sub_ink: 0x9CA3AF,
                key_dim_ink: 0x6B7280,
                ok_bg: 0x0F2922,
                ok_ink: 0x34D399,
                ok_border: 0x059669,
                warn_bg: 0x282112,
                warn_ink: 0xFBBF24,
                warn_border: 0xD97706,
                fw_bg: 0x271C0B,
                fw_glow: 0x3D290D,
                fw_border: 0xF59E0B,
                fw_dim_border: 0x92600E,
                fw_ink: 0xFCD34D,
                dm_bg: 0x072430,
                dm_active: 0x0A3546,
                dm_pulse: 0x0D4C61,
                dm_border: 0x00D7FF,
                dm_dim_border: 0x0E7490,
                dm_pulse_border: 0x7EF9FF,
                dm_ink: 0x6EFCFF,
                dm_pulse_ink: 0xE0FEFF,
                dm_muted_bg: 0x101C24,
                dm_muted_border: 0x264A5C,
                dm_muted_ink: 0x7DA7B8,
                dm_mod_bg: 0x0D2230,
                dm_mod_border: 0x0284C7,
                dm_mod_ink: 0x7DD3FC,
                overlap_bg: 0x201B36,
                overlap_border: 0xC084FC,
                overlap_ink: 0xE9D5FF,
                track_bg: 0x0B0D10,
            }
        } else {
            Self {
                is_dark: false,
                deck_bg: 0xE5E7EB,
                deck_border: 0xCBD5E1,
                key_bg: 0xFFFFFF,
                key_selected_bg: 0xE2E8F0,
                key_border: 0xCBD5E1,
                key_border_selected: 0x64748B,
                key_border_focus: 0x111827,
                key_ink: 0x111827,
                key_sub_ink: 0x4B5563,
                key_dim_ink: 0x6B7280,
                ok_bg: 0xECFDF5,
                ok_ink: 0x047857,
                ok_border: 0x10B981,
                warn_bg: 0xFFFBEB,
                warn_ink: 0xB45309,
                warn_border: 0xD97706,
                fw_bg: 0xFEF3C7,
                fw_glow: 0xFDE68A,
                fw_border: 0xD97706,
                fw_dim_border: 0xF59E0B,
                fw_ink: 0x92400E,
                dm_bg: 0xECFEFF,
                dm_active: 0xCFFAFE,
                dm_pulse: 0xA5F3FC,
                dm_border: 0x0891B2,
                dm_dim_border: 0x06B6D4,
                dm_pulse_border: 0x00D7FF,
                dm_ink: 0x155E75,
                dm_pulse_ink: 0x083344,
                dm_muted_bg: 0xF1F5F9,
                dm_muted_border: 0x94A3B8,
                dm_muted_ink: 0x475569,
                dm_mod_bg: 0xE0F2FE,
                dm_mod_border: 0x0284C7,
                dm_mod_ink: 0x075985,
                overlap_bg: 0xF3E8FF,
                overlap_border: 0x9333EA,
                overlap_ink: 0x6B21A8,
                track_bg: 0xCBD5E1,
            }
        }
    }
}

/// Mini surface preview palette `(canvas_bg, card_bg, border_col, fg_col)` for Setup theme cards.
pub const fn setup_theme_swatch(dark: bool) -> (u32, u32, u32, u32) {
    if dark {
        (0x0B0D10, 0x12151B, 0x232833, 0xF3F4F6)
    } else {
        (0xFAFAFB, 0xFFFFFF, 0xD1D5DB, 0x111827)
    }
}

/// Depth fill curve for live Hall-effect travel telemetry (`0.0..=max_mm`).
pub fn depth_fill_hsla(mm: f64, max_mm: f64, is_dark: bool) -> Hsla {
    let ratio = (mm / max_mm.max(0.5)).clamp(0.0, 1.0) as f32;
    if is_dark {
        hsla(210.0 / 360.0, 0.25 + ratio * 0.35, 0.16 + ratio * 0.24, 1.0)
    } else {
        hsla(205.0 / 360.0, 0.35 + ratio * 0.35, 0.90 - ratio * 0.22, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_and_light_hardware_palettes_stay_distinct() {
        let dark = HardwareColors::for_mode(true);
        let light = HardwareColors::for_mode(false);
        assert_ne!(dark.deck_bg, light.deck_bg);
        assert_ne!(dark.dm_ink, light.dm_ink);
        assert_eq!(dark.dm_border, 0x00D7FF);
    }
}
