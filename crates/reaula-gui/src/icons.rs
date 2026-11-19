//! Embedded Google Material Icons (`24×24` monochrome SVG assets) for `reaula-gui`.
//!
//! GPUI rasterizes SVGs into monochrome alpha sprites tinted by `.text_color(...)`,
//! so each icon uses `fill="#000000"` for full-opacity mask coverage.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString, Svg, prelude::*, px, rgb, svg};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialIcon {
    Keyboard,
    MenuBook,
    Tune,
    Palette,
    Bolt,
    MonitorHeart,
    GridView,
    ViewInAr,
    ExpandMore,
    ExpandLess,
    Check,
    Refresh,
    Memory,
    Save,
    Brush,
    PowerSettings,
    DeleteOutline,
    PlayArrow,
    Pause,
    Sensors,
    FiberManualRecord,
    RestartAlt,
    Videocam,
    GraphicEq,
    Warning,
    VolumeUp,
}

impl MaterialIcon {
    pub const ALL: [Self; 26] = [
        Self::Keyboard,
        Self::MenuBook,
        Self::Tune,
        Self::Palette,
        Self::Bolt,
        Self::MonitorHeart,
        Self::GridView,
        Self::ViewInAr,
        Self::ExpandMore,
        Self::ExpandLess,
        Self::Check,
        Self::Refresh,
        Self::Memory,
        Self::Save,
        Self::Brush,
        Self::PowerSettings,
        Self::DeleteOutline,
        Self::PlayArrow,
        Self::Pause,
        Self::Sensors,
        Self::FiberManualRecord,
        Self::RestartAlt,
        Self::Videocam,
        Self::GraphicEq,
        Self::Warning,
        Self::VolumeUp,
    ];

    pub const fn asset_path(self) -> &'static str {
        match self {
            Self::Keyboard => "icons/keyboard.svg",
            Self::MenuBook => "icons/menu_book.svg",
            Self::Tune => "icons/tune.svg",
            Self::Palette => "icons/palette.svg",
            Self::Bolt => "icons/bolt.svg",
            Self::MonitorHeart => "icons/monitor_heart.svg",
            Self::GridView => "icons/grid_view.svg",
            Self::ViewInAr => "icons/view_in_ar.svg",
            Self::ExpandMore => "icons/expand_more.svg",
            Self::ExpandLess => "icons/expand_less.svg",
            Self::Check => "icons/check.svg",
            Self::Refresh => "icons/refresh.svg",
            Self::Memory => "icons/memory.svg",
            Self::Save => "icons/save.svg",
            Self::Brush => "icons/brush.svg",
            Self::PowerSettings => "icons/power_settings_new.svg",
            Self::DeleteOutline => "icons/delete_outline.svg",
            Self::PlayArrow => "icons/play_arrow.svg",
            Self::Pause => "icons/pause.svg",
            Self::Sensors => "icons/sensors.svg",
            Self::FiberManualRecord => "icons/fiber_manual_record.svg",
            Self::RestartAlt => "icons/restart_alt.svg",
            Self::Videocam => "icons/videocam.svg",
            Self::GraphicEq => "icons/graphic_eq.svg",
            Self::Warning => "icons/warning.svg",
            Self::VolumeUp => "icons/volume_up.svg",
        }
    }

    const fn svg_bytes(self) -> &'static [u8] {
        match self {
            Self::Keyboard => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M20 5H4c-1.1 0-1.99.9-1.99 2L2 17c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V7c0-1.1-.9-2-2-2zm-9 3h2v2h-2V8zm0 3h2v2h-2v-2zM8 8h2v2H8V8zm0 3h2v2H8v-2zm-1 2H5v-2h2v2zm0-3H5V8h2v2zm9 7H8v-2h8v2zm0-4h-2v-2h2v2zm0-3h-2V8h2v2zm3 3h-2v-2h2v2zm0-3h-2V8h2v2z"/></svg>"##,
            Self::MenuBook => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M21 5c-1.11-.35-2.33-.5-3.5-.5-1.95 0-4.05.4-5.5 1.5-1.45-1.1-3.55-1.5-5.5-1.5S2.45 4.9 1 6v14.65c0 .25.25.5.5.5.1 0 .15-.05.25-.05C3.1 20.45 5.05 20 6.5 20c1.95 0 4.05.4 5.5 1.5 1.35-.85 3.8-1.5 5.5-1.5 1.65 0 3.35.3 4.75 1.05.1.05.15.05.25.05.25 0 .5-.25.5-.5V6c-.6-.45-1.25-.75-2-1zm0 13.5c-1.1-.35-2.3-.5-3.5-.5-1.7 0-4.15.65-5.5 1.5V8c1.35-.85 3.8-1.5 5.5-1.5 1.2 0 2.4.15 3.5.5v11.5z"/></svg>"##,
            Self::Tune => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M3 17v2h6v-2H3zM3 5v2h10V5H3zm10 16v-2h8v-2h-8v-2h-2v6h2zM7 9v2H3v2h4v2h2V9H7zm14 4v-2H11v2h10zm-6-4h2V7h4V5h-4V3h-2v6z"/></svg>"##,
            Self::Palette => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M12 3c-4.97 0-9 4.03-9 9s4.03 9 9 9c.83 0 1.5-.67 1.5-1.5 0-.39-.15-.74-.39-1.01-.23-.26-.38-.61-.38-.99 0-.83.67-1.5 1.5-1.5H16c2.76 0 5-2.24 5-5 0-4.42-4.03-8-9-8zm-5.5 9c-.83 0-1.5-.67-1.5-1.5S5.67 9 6.5 9 8 9.67 8 10.5 7.33 12 6.5 12zm3-4C8.67 8 8 7.33 8 6.5S8.67 5 9.5 5s1.5.67 1.5 1.5S10.33 8 9.5 8zm5 0c-.83 0-1.5-.67-1.5-1.5S13.67 5 14.5 5s1.5.67 1.5 1.5S15.33 8 14.5 8zm3 4c-.83 0-1.5-.67-1.5-1.5S16.67 9 17.5 9s1.5.67 1.5 1.5-.67 1.5-1.5 1.5z"/></svg>"##,
            Self::Bolt => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M11 21h-1l1-7H7.5c-.58 0-.57-.32-.38-.66.19-.34.05-.08.07-.12C8.48 10.94 10.42 7.54 13 3h1l-1 7h3.5c.49 0 .56.33.47.51l-.07.15C12.96 17.55 11 21 11 21z"/></svg>"##,
            Self::MonitorHeart => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M15.11 12.45L14 10.24l-3.11 6.21c-.16.34-.51.55-.89.55s-.73-.21-.89-.55L7.38 13H2v2h4.15l1.96 3.92c.36.71 1.43.71 1.79 0L14 10.76l1.11 2.21c.17.34.51.55.89.55h6v-2h-5.38l-1.51-3.07zM20 4H4c-1.1 0-2 .9-2 2v4h2V6h16v4h2V6c0-1.1-.9-2-2-2zm0 14H4v-2H2v2c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2v-2h-2v2z"/></svg>"##,
            Self::GridView => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M3 3v8h8V3H3zm6 6H5V5h4v4zm-6 4v8h8v-8H3zm6 6H5v-4h4v4zm4-16v8h8V3h-8zm6 6h-4V5h4v4zm-6 4v8h8v-8h-8zm6 6h-4v-4h4v4z"/></svg>"##,
            Self::ViewInAr => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M18.25 7.6l-5.5-3.18c-.46-.27-1.04-.27-1.5 0L5.75 7.6c-.46.27-.75.76-.75 1.3v6.35c0 .54.29 1.03.75 1.3l5.5 3.18c.46.27 1.04.27 1.5 0l5.5-3.18c.46-.27.75-.76.75-1.3V8.9c0-.54-.29-1.03-.75-1.3zM7 14.96v-4.62l4 2.32v4.61l-4-2.31zm5-4.03L8 8.61l4-2.31 4 2.31-4 2.32zm1 6.34v-4.61l4-2.32v4.62l-4 2.31z"/></svg>"##,
            Self::ExpandMore => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M16.59 8.59L12 13.17 7.41 8.59 6 10l6 6 6-6z"/></svg>"##,
            Self::ExpandLess => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M12 8l-6 6 1.41 1.41L12 10.83l4.59 4.58L18 14z"/></svg>"##,
            Self::Check => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M9 16.17L4.83 12l-1.42 1.41L9 19 21 7l-1.41-1.41z"/></svg>"##,
            Self::Refresh => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M17.65 6.35A7.958 7.958 0 0012 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0112 18c-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35z"/></svg>"##,
            Self::Memory => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M15 9H9v6h6V9zm-2 4h-2v-2h2v2zm8-2V9h-2V7c0-1.1-.9-2-2-2h-2V3h-2v2h-2V3H9v2H7c-1.1 0-2 .9-2 2v2H3v2h2v2H3v2h2v2c0 1.1.9 2 2 2h2v2h2v-2h2v2h2v-2h2c1.1 0 2-.9 2-2v-2h2v-2h-2v-2h2zm-4 6H7V7h10v10z"/></svg>"##,
            Self::Save => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M17 3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V7l-4-4zm-5 16c-1.66 0-3-1.34-3-3s1.34-3 3-3 3 1.34 3 3-1.34 3-3 3zm3-10H5V5h10v4z"/></svg>"##,
            Self::Brush => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M7 14c-1.66 0-3 1.34-3 3 0 1.31-1.16 2-2 2 .92 1.22 2.49 2 4 2 2.21 0 4-1.79 4-4 0-1.66-1.34-3-3-3zm13.71-9.37l-1.34-1.34a.996.996 0 00-1.41 0L9 12.25 11.75 15l8.96-8.96a.996.996 0 000-1.41z"/></svg>"##,
            Self::PowerSettings => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M13 3h-2v10h2V3zm4.83 2.17l-1.42 1.42A6.92 6.92 0 0119 12c0 3.87-3.13 7-7 7s-7-3.13-7-7c0-2.19 1.01-4.14 2.58-5.42L6.17 5.17A8.932 8.932 0 003 12a9 9 0 0018 0c0-2.74-1.23-5.18-3.17-6.83z"/></svg>"##,
            Self::DeleteOutline => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM8 9h8v10H8V9zm7.5-5l-1-1h-5l-1 1H5v2h14V4z"/></svg>"##,
            Self::PlayArrow => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M8 5v14l11-7z"/></svg>"##,
            Self::Pause => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/></svg>"##,
            Self::Sensors => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M7.76 16.24l-1.41 1.41C4.78 16.1 4 14.05 4 12s.78-4.1 2.34-5.66l1.41 1.41C6.59 8.93 6 10.46 6 12s.59 3.07 1.76 4.24zm9.89 1.41l-1.41-1.41C17.41 15.07 18 13.54 18 12s-.59-3.07-1.76-4.24l1.41-1.41C19.22 7.9 20 9.95 20 12s-.78 4.1-2.35 5.65zM12 10c-1.1 0-2 .9-2 2s.9 2 2 2 2-.9 2-2-.9-2-2-2zm-2.83 4.83l-1.41 1.41C6.67 15.16 6 13.66 6 12s.67-3.16 1.76-4.24l1.41 1.41C8.45 9.9 8 10.9 8 12s.45 2.1 1.17 2.83zm5.66 0c.72-.73 1.17-1.73 1.17-2.83s-.45-2.1-1.17-2.83l1.41-1.41C17.33 8.84 18 10.34 18 12s-.67 3.16-1.76 4.24l-1.41-1.41z"/></svg>"##,
            Self::FiberManualRecord => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle fill="#000000" cx="12" cy="12" r="7"/></svg>"##,
            Self::RestartAlt => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M12 5V2L8 6l4 4V7c3.31 0 6 2.69 6 6 0 2.97-2.17 5.43-5 5.91v2.02c3.95-.49 7-3.85 7-7.93 0-4.42-3.58-8-8-8zm-6 8c0-1.65.67-3.15 1.76-4.24L6.34 7.34A8.014 8.014 0 004 13c0 4.08 3.05 7.44 7 7.93v-2.02c-2.83-.48-5-2.94-5-5.91z"/></svg>"##,
            Self::Videocam => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M17 10.5V7c0-.55-.45-1-1-1H4c-.55 0-1 .45-1 1v10c0 .55.45 1 1 1h12c.55 0 1-.45 1-1v-3.5l4 4v-11l-4 4z"/></svg>"##,
            Self::GraphicEq => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M7 18h2V6H7v12zm4 4h2V2h-2v20zm-8-8h2v-4H3v4zm12 4h2V6h-2v12zm4-8v4h2v-4h-2z"/></svg>"##,
            Self::Warning => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z"/></svg>"##,
            Self::VolumeUp => br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill="#000000" d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02zM14 3.23v2.06c2.89.86 5 3.54 5 6.71s-2.11 5.85-5 6.71v2.06c4.01-.91 7-4.49 7-8.77s-2.99-7.86-7-8.77z"/></svg>"##,
        }
    }
}

/// Static embedded GPUI `AssetSource` serving Material Icons SVGs zero-copy.
pub struct IconAssets;

impl AssetSource for IconAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        for ic in MaterialIcon::ALL {
            if ic.asset_path() == path {
                return Ok(Some(Cow::Borrowed(ic.svg_bytes())));
            }
        }
        Ok(None)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let prefix = if path.is_empty() { "icons/" } else { path };
        Ok(MaterialIcon::ALL
            .iter()
            .filter(|ic| ic.asset_path().starts_with(prefix))
            .map(|ic| SharedString::from(ic.asset_path()))
            .collect())
    }
}

/// Render a monochrome Material Icon tinted cleanly to any `theme::*` ink color.
pub fn icon(ic: MaterialIcon, size_px: f32, color: u32) -> Svg {
    svg()
        .path(ic.asset_path())
        .size(px(size_px))
        .text_color(rgb(color))
        .flex_shrink_0()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_material_icons_resolve_as_valid_svg() {
        let assets = IconAssets;
        for ic in MaterialIcon::ALL {
            let loaded = assets
                .load(ic.asset_path())
                .unwrap()
                .expect("icon asset must resolve");
            let text = std::str::from_utf8(&loaded).unwrap();
            assert!(text.contains("<svg"), "missing <svg for {:?}", ic);
            assert!(
                text.contains("viewBox=\"0 0 24 24\""),
                "missing 24x24 viewBox for {:?}",
                ic
            );
        }
    }
}
