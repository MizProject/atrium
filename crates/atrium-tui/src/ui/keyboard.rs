//! Interactive 61-key WIN 60 HE hardware matrix renderer with live travel heat,
//! RGB swatch painting preview, Calibration status, and `[FW]` / `[DM]` keymap badges.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use atrium_core::keymap::{KEYBOARD_CANVAS_WIDTH, WIN60HE_KEYS};
use atrium_core::protocol;

use crate::state::{App, HitRegion, HitTarget, KeymapLayerView};
use crate::theme::{self, contrast_ink, depth_matte_rgb};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MatrixMode {
    Main,
    Calibration,
    CustomSlot(u8),
    AudioPreview,
    LightingPreview,
    Keymaps,
}

pub fn draw_keyboard_matrix(
    f: &mut Frame<'_>,
    area: Rect,
    app: &App,
    mode: MatrixMode,
    focused: bool,
    title: &str,
    hits: &mut Vec<HitRegion>,
) {
    let outer = theme::panel_block(title, focused);
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    if inner.width < 12 || inner.height < 5 {
        return;
    }

    // Fall back to compact multi-column list on ultra-narrow SSH panes.
    if inner.width < 62 || inner.height < 10 {
        draw_compact_key_grid(f, inner, app, mode, hits);
        return;
    }

    let (dm_active, dm_pulsing) = app.active_daemon_indicators();
    let fn_held = app.is_fn_held();

    let rows_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
            Constraint::Ratio(1, 5),
        ])
        .split(inner);

    for key in WIN60HE_KEYS {
        let row_idx = usize::from(key.row().saturating_sub(1));
        if row_idx >= 5 {
            continue;
        }
        let row_area = rows_layout[row_idx];
        let total_w = u32::from(KEYBOARD_CANVAS_WIDTH.saturating_sub(20)).max(1);
        let x0 = row_area.x
            + ((u32::from(key.x.saturating_sub(10)) * u32::from(row_area.width)) / total_w) as u16;
        let x1 = row_area.x
            + ((u32::from(key.x.saturating_sub(10) + key.width) * u32::from(row_area.width))
                / total_w) as u16;
        if x1 <= x0 || row_area.height < 2 {
            continue;
        }
        let w = (x1 - x0).max(2).min(row_area.x + row_area.width - x0);
        let rect = Rect::new(x0, row_area.y, w, row_area.height);
        hits.push(HitRegion {
            rect,
            action: HitTarget::SelectKey(key.index),
        });

        let is_cursor = key.index == app.current_key().index;
        let is_selected = app.selected_indices.contains(&key.index);
        let depth = app.live_depths.get(&key.index).map_or(0.0, |(d, _)| *d);

        let (bg, fg, border_fg, sub_label) = match mode {
            MatrixMode::Main => {
                let bg = if depth > 0.08 {
                    depth_matte_rgb(depth, app.max_mm)
                } else if is_selected {
                    theme::SHADE_REGAL
                } else {
                    theme::SHADE_DEEP
                };
                let bfg = if is_cursor {
                    theme::CYAN_HIGHLIGHT
                } else if depth > 0.15 {
                    theme::INK_OK
                } else if is_selected {
                    theme::BORDER_FOCUS
                } else {
                    theme::BORDER_SUBTLE
                };
                let sub = if depth > 0.02 {
                    format!("{depth:.1}")
                } else if let Some(cfg) = app.actuations.get(&key.index) {
                    format!("{:.1}", f64::from(cfg.travel_raw) * app.unit_mm)
                } else {
                    String::new()
                };
                (bg, theme::INK_PRIMARY, bfg, sub)
            }
            MatrixMode::Calibration => {
                let done = app.cal_calibrated.contains(&key.index);
                let bg = if done {
                    Color::Rgb(16, 62, 52)
                } else if depth > 0.10 {
                    depth_matte_rgb(depth, app.max_mm)
                } else {
                    theme::SHADE_DEEP
                };
                let bfg = if is_cursor {
                    theme::CYAN_HIGHLIGHT
                } else if done {
                    theme::INK_OK
                } else {
                    theme::BORDER_SUBTLE
                };
                let sub = if done {
                    "CAL".into()
                } else if depth > 0.05 {
                    format!("{depth:.1}")
                } else {
                    "..".into()
                };
                (bg, theme::INK_PRIMARY, bfg, sub)
            }
            MatrixMode::CustomSlot(slot) => {
                let rgb = app.custom_slots[usize::from(slot)]
                    .get(usize::from(key.index))
                    .copied()
                    .unwrap_or([0, 0, 0]);
                let bg = if rgb == [0, 0, 0] {
                    theme::SHADE_DEEP
                } else {
                    Color::Rgb(rgb[0], rgb[1], rgb[2])
                };
                let ink = if rgb == [0, 0, 0] {
                    theme::INK_MUTED
                } else {
                    contrast_ink(rgb)
                };
                let bfg = if is_cursor {
                    theme::CYAN_HIGHLIGHT
                } else if is_selected {
                    theme::BORDER_FOCUS
                } else {
                    theme::BORDER_SUBTLE
                };
                let sub = if rgb == [0, 0, 0] {
                    "off".into()
                } else {
                    format!("{:02X}{:02X}", rgb[0], rgb[1])
                };
                (bg, ink, bfg, sub)
            }
            MatrixMode::AudioPreview => {
                let rgb = if app.audio_status.active {
                    app.audio_status
                        .frame
                        .get(usize::from(key.index))
                        .copied()
                        .unwrap_or([0, 0, 0])
                } else {
                    [0, 0, 0]
                };
                let bg = if rgb == [0, 0, 0] {
                    theme::SHADE_DEEP
                } else {
                    Color::Rgb(rgb[0], rgb[1], rgb[2])
                };
                let ink = if rgb == [0, 0, 0] {
                    theme::INK_MUTED
                } else {
                    contrast_ink(rgb)
                };
                let bfg = if is_cursor {
                    theme::CYAN_HIGHLIGHT
                } else {
                    theme::BORDER_SUBTLE
                };
                (bg, ink, bfg, String::new())
            }
            MatrixMode::LightingPreview => {
                let ls = app.light_state;
                let bg = if !ls.power_on {
                    theme::SHADE_DEEP
                } else if ls.full_color {
                    let phase = ((u16::from(key.index) * 19 + u16::from(ls.mode) * 31) % 255) as u8;
                    let hsv_rgb = crate::theme::hsv_to_rgb_u8(f32::from(phase) / 255.0, 0.78, 0.65);
                    Color::Rgb(hsv_rgb[0], hsv_rgb[1], hsv_rgb[2])
                } else {
                    Color::Rgb(
                        ((u16::from(ls.fg[0]) * u16::from(ls.brightness.max(1))) / 4) as u8,
                        ((u16::from(ls.fg[1]) * u16::from(ls.brightness.max(1))) / 4) as u8,
                        ((u16::from(ls.fg[2]) * u16::from(ls.brightness.max(1))) / 4) as u8,
                    )
                };
                let ink = match bg {
                    Color::Rgb(r, g, b) if ls.power_on => contrast_ink([r, g, b]),
                    _ => theme::INK_MUTED,
                };
                let bfg = if is_cursor {
                    theme::CYAN_HIGHLIGHT
                } else {
                    theme::BORDER_SUBTLE
                };
                (bg, ink, bfg, String::new())
            }
            MatrixMode::Keymaps => {
                let show_fw = matches!(app.keymap_view, KeymapLayerView::Both | KeymapLayerView::FirmwareOnly);
                let show_dm = matches!(app.keymap_view, KeymapLayerView::Both | KeymapLayerView::DaemonOnly);
                let has_fw = show_fw && (key.index == atrium_core::keymap::idx::FN || app.keyboard_fn_map.contains_key(&key.index));
                let has_dm = show_dm && app.shortcut_rules.iter().any(|r| {
                    r.enabled && (r.trigger_key == key.index || r.modifier_key == Some(key.index))
                });
                let is_dm_pulse = show_dm && dm_pulsing.contains(&key.index);
                let is_dm_active = show_dm && dm_active.contains(&key.index);
                let is_fw_lit = show_fw && fn_held && has_fw;

                let (bg, bfg, ink) = if is_dm_pulse {
                    (theme::DM_AQUA_PULSE, theme::DM_AQUA_PULSE_BORDER, theme::INK_PRIMARY)
                } else if is_dm_active {
                    (theme::DM_AQUA_ACTIVE, theme::DM_AQUA_BORDER, theme::DM_AQUA_INK)
                } else if is_fw_lit {
                    (Color::Rgb(0x45, 0x30, 0x12), theme::FW_AMBER_BORDER, theme::FW_AMBER_INK)
                } else if has_fw && has_dm {
                    (theme::OVERLAP_BG, theme::OVERLAP_BORDER, theme::OVERLAP_INK)
                } else if has_fw {
                    (theme::FW_AMBER_BG, theme::FW_AMBER_BORDER, theme::FW_AMBER_INK)
                } else if has_dm {
                    (theme::DM_AQUA_BG, theme::DM_AQUA_BORDER, theme::DM_AQUA_INK)
                } else {
                    (theme::SHADE_DEEP, theme::BORDER_SUBTLE, theme::INK_SECONDARY)
                };

                let tag = if has_fw && has_dm {
                    "F+D".into()
                } else if has_fw {
                    if key.index == atrium_core::keymap::idx::FN {
                        "FN".into()
                    } else {
                        "FW".into()
                    }
                } else if has_dm {
                    "DM".into()
                } else {
                    String::new()
                };
                let final_bfg = if is_cursor { theme::CYAN_HIGHLIGHT } else { bfg };
                (bg, ink, final_bfg, tag)
            }
        };

        let mut lines = vec![Line::from(Span::styled(
            trim_label(key.name, usize::from(w.saturating_sub(2))),
            Style::default().fg(fg).add_modifier(Modifier::BOLD),
        ))];
        if rect.height >= 3 && !sub_label.is_empty() {
            lines.push(Line::from(Span::styled(
                trim_label(&sub_label, usize::from(w.saturating_sub(2))),
                Style::default().fg(if matches!(mode, MatrixMode::CustomSlot(_)) {
                    fg
                } else {
                    theme::INK_MUTED
                }),
            )));
        }

        let btype = if is_cursor {
            BorderType::Thick
        } else {
            BorderType::Plain
        };
        let widget = Paragraph::new(lines)
            .style(Style::default().bg(bg))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(btype)
                    .border_style(Style::default().fg(border_fg)),
            );
        f.render_widget(widget, rect);
    }
}

fn draw_compact_key_grid(
    f: &mut Frame<'_>,
    area: Rect,
    app: &App,
    mode: MatrixMode,
    hits: &mut Vec<HitRegion>,
) {
    let cols = (area.width / 8).max(1) as usize;
    let mut row_items: Vec<Line<'_>> = Vec::new();
    let cur = app.current_key().index;

    for (y, chunk) in (area.y..).zip(WIN60HE_KEYS.chunks(cols)) {
        if y >= area.y + area.height {
            break;
        }
        let mut spans = Vec::new();
        let mut x = area.x;
        for key in chunk {
            let cell_w = 8u16.min(area.x + area.width - x);
            if cell_w < 3 {
                break;
            }
            let rect = Rect::new(x, y, cell_w, 1);
            hits.push(HitRegion {
                rect,
                action: HitTarget::SelectKey(key.index),
            });
            let sel = app.selected_indices.contains(&key.index);
            let is_cur = key.index == cur;
            let depth = app.live_depths.get(&key.index).map_or(0.0, |(d, _)| *d);
            let st = match mode {
                MatrixMode::Keymaps => {
                    let has_fw = app.keyboard_fn_map.contains_key(&key.index);
                    let has_dm = app
                        .shortcut_rules
                        .iter()
                        .any(|r| r.trigger_key == key.index || r.modifier_key == Some(key.index));
                    if is_cur {
                        Style::default().bg(theme::SHADE_REGAL).fg(theme::INK_PRIMARY).add_modifier(Modifier::BOLD)
                    } else if has_fw && has_dm {
                        Style::default().bg(theme::OVERLAP_BG).fg(theme::OVERLAP_INK)
                    } else if has_fw {
                        Style::default().bg(theme::FW_AMBER_BG).fg(theme::FW_AMBER_INK)
                    } else if has_dm {
                        Style::default().bg(theme::DM_AQUA_BG).fg(theme::DM_AQUA_INK)
                    } else {
                        Style::default().fg(theme::INK_SECONDARY)
                    }
                }
                _ if is_cur => Style::default()
                    .bg(theme::CYAN_HIGHLIGHT)
                    .fg(theme::SHADE_DEEP)
                    .add_modifier(Modifier::BOLD),
                _ if depth > 0.1 => Style::default().bg(depth_matte_rgb(depth, app.max_mm)).fg(theme::INK_PRIMARY),
                _ if sel => Style::default().bg(theme::SHADE_REGAL).fg(theme::INK_PRIMARY),
                _ => Style::default().fg(theme::INK_SECONDARY),
            };
            let txt = format!("[{:^5}] ", trim_label(key.name, 5));
            spans.push(Span::styled(txt, st));
            x += cell_w;
        }
        row_items.push(Line::from(spans));
    }

    let note = Line::from(Span::styled(
        format!(
            "Focused: {} ({}) | Click key or resize >=62 cols for 60% visual map",
            app.current_key().name,
            protocol::switch_profile_name(
                app.switches.get(&app.current_key().index).copied().unwrap_or(1)
            )
        ),
        Style::default().fg(theme::INK_MUTED),
    ));
    row_items.push(note);
    f.render_widget(Paragraph::new(row_items), area);
}

fn trim_label(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    s.chars().take(max_chars).collect()
}
