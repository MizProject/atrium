//! Color & Light workspace: Firmware Effects, Custom Slots 1/2 RGB painter,
//! HSV/preset/hex Human Color Picker, and Daemon-hosted Audio Equalizer.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use reaula_core::audio_fx::{AudioEffectStyle, AudioPalette};
use reaula_core::protocol;

use crate::state::{
    App, FocusPane, HitRegion, HitTarget, LIGHT_MODE_OPTIONS, LightEditorTab, PromptPurpose,
    TextPromptModal,
};
use crate::theme::{self, HUMAN_COLOR_PRESETS, contrast_ink, format_rgb_hex, hsv_to_rgb_u8, rgb_to_hsv};
use crate::ui::keyboard::{MatrixMode, draw_keyboard_matrix};
use crate::ui::{ActionChip, render_chip_bar};

pub fn draw(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(if area.height < 28 { 11 } else { 15 }),
            Constraint::Min(10),
        ])
        .split(area);

    // Sub-tab bar
    render_chip_bar(
        f,
        split[0],
        &[
            ActionChip::new(
                "Firmware Effects",
                app.light_tab == LightEditorTab::FirmwareEffects,
                HitTarget::LightTab(LightEditorTab::FirmwareEffects),
            ),
            ActionChip::new(
                "Custom Slot 1",
                app.light_tab == LightEditorTab::CustomSlot1,
                HitTarget::LightTab(LightEditorTab::CustomSlot1),
            ),
            ActionChip::new(
                "Custom Slot 2",
                app.light_tab == LightEditorTab::CustomSlot2,
                HitTarget::LightTab(LightEditorTab::CustomSlot2),
            ),
            ActionChip::new(
                if app.audio_status.active {
                    "Audio Sync (LIVE)"
                } else {
                    "Audio Sync"
                },
                app.light_tab == LightEditorTab::AudioSync,
                HitTarget::LightTab(LightEditorTab::AudioSync),
            ),
        ],
        hits,
    );

    let matrix_mode = match app.light_tab {
        LightEditorTab::FirmwareEffects => MatrixMode::LightingPreview,
        LightEditorTab::CustomSlot1 => MatrixMode::CustomSlot(0),
        LightEditorTab::CustomSlot2 => MatrixMode::CustomSlot(1),
        LightEditorTab::AudioSync => MatrixMode::AudioPreview,
    };
    let matrix_title = match app.light_tab {
        LightEditorTab::FirmwareEffects => "WIN 60 HE Firmware RGB Preview",
        LightEditorTab::CustomSlot1 => {
            if app.custom_paint_mode {
                "Custom Slot 1 Painter (Click or Space on key to PAINT brush color)"
            } else {
                "Custom Slot 1 Preview (Toggle Paint Mode to click/Space-paint keys)"
            }
        }
        LightEditorTab::CustomSlot2 => {
            if app.custom_paint_mode {
                "Custom Slot 2 Painter (Click or Space on key to PAINT brush color)"
            } else {
                "Custom Slot 2 Preview (Toggle Paint Mode to click/Space-paint keys)"
            }
        }
        LightEditorTab::AudioSync => "Daemon Audio Sync Live Per-Key Preview",
    };
    draw_keyboard_matrix(
        f,
        split[1],
        app,
        matrix_mode,
        app.focus == FocusPane::LightKeyboard,
        matrix_title,
        hits,
    );

    match app.light_tab {
        LightEditorTab::FirmwareEffects => draw_firmware_panel(f, split[2], app, hits),
        LightEditorTab::CustomSlot1 | LightEditorTab::CustomSlot2 => {
            draw_custom_panel(f, split[2], app, hits);
        }
        LightEditorTab::AudioSync => draw_audio_panel(f, split[2], app, hits),
    }
}

fn draw_firmware_panel(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(area);

    let list_focused = app.focus == FocusPane::LightEffectsList;
    let items: Vec<ListItem> = LIGHT_MODE_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, &m)| {
            let mark = if i == app.light_mode_cursor { "▶ " } else { "  " };
            let st = if i == app.light_mode_cursor {
                Style::default()
                    .fg(theme::CYAN_HIGHLIGHT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::INK_SECONDARY)
            };
            ListItem::new(format!("{mark}{:>3} · {}", m, protocol::light_mode_name(m))).style(st)
        })
        .collect();
    let list_blk = theme::panel_block("Firmware Effects (Up/Down)", list_focused);
    let list_inner = list_blk.inner(cols[0]);
    f.render_widget(List::new(items).block(list_blk), cols[0]);
    for (i, _) in LIGHT_MODE_OPTIONS.iter().enumerate() {
        let y = list_inner.y + i as u16;
        if y < list_inner.y + list_inner.height {
            hits.push(HitRegion {
                rect: Rect::new(list_inner.x, y, list_inner.width, 1),
                action: HitTarget::LightEffectPick(i),
            });
        }
    }

    let ctrl_focused = app.focus == FocusPane::LightControls;
    let ctrl_blk = theme::panel_block("Parameters & Human Color Picker", ctrl_focused);
    let inner = ctrl_blk.inner(cols[1]);
    f.render_widget(ctrl_blk, cols[1]);
    hits.push(HitRegion {
        rect: cols[1],
        action: HitTarget::Focus(FocusPane::LightControls),
    });

    if inner.height < 8 || inner.width < 22 {
        return;
    }

    let ls = app.light_state;
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // brightness
            Constraint::Length(1), // speed
            Constraint::Length(1), // dir / full_color / power
            Constraint::Length(1), // target Fg/Bg + presets
            Constraint::Length(1), // H stepper
            Constraint::Length(1), // S stepper
            Constraint::Length(1), // V stepper
            Constraint::Length(1), // Apply / Reload
            Constraint::Min(0),
        ])
        .split(inner);

    // Row 0: Brightness
    render_chip_bar(
        f,
        rows[0],
        &[
            ActionChip::new("Bri:", false, HitTarget::Focus(FocusPane::LightControls))
                .focus_mark(ctrl_focused && app.light_ctrl_cursor == 0),
            ActionChip::new("0", ls.brightness == 0, HitTarget::LightBri(0)),
            ActionChip::new("1", ls.brightness == 1, HitTarget::LightBri(1)),
            ActionChip::new("2", ls.brightness == 2, HitTarget::LightBri(2)),
            ActionChip::new("3", ls.brightness == 3, HitTarget::LightBri(3)),
            ActionChip::new("4", ls.brightness == 4, HitTarget::LightBri(4)),
        ],
        hits,
    );

    // Row 1: Speed
    render_chip_bar(
        f,
        rows[1],
        &[
            ActionChip::new("Spd:", false, HitTarget::Focus(FocusPane::LightControls))
                .focus_mark(ctrl_focused && app.light_ctrl_cursor == 1),
            ActionChip::new("0", ls.speed == 0, HitTarget::LightSpd(0)),
            ActionChip::new("1", ls.speed == 1, HitTarget::LightSpd(1)),
            ActionChip::new("2", ls.speed == 2, HitTarget::LightSpd(2)),
            ActionChip::new("3", ls.speed == 3, HitTarget::LightSpd(3)),
            ActionChip::new("4", ls.speed == 4, HitTarget::LightSpd(4)),
        ],
        hits,
    );

    // Row 2: Direction, Rainbow, LED Power
    render_chip_bar(
        f,
        rows[2],
        &[
            ActionChip::new(
                format!("Dir: {}", protocol::direction_name(ls.direction)),
                false,
                HitTarget::LightDirNext,
            )
            .focus_mark(ctrl_focused && app.light_ctrl_cursor == 2),
            ActionChip::new(
                if ls.full_color { "Rainbow:ON" } else { "Rainbow:OFF" },
                ls.full_color,
                HitTarget::LightFullColor,
            ),
            ActionChip::new(
                if ls.power_on { "LED:ON" } else { "LED:OFF" },
                ls.power_on,
                HitTarget::LightPower,
            ),
        ],
        hits,
    );

    // Row 3: Fg vs Bg color picker swatch + quick human presets
    let active_rgb = if app.light_edit_bg { ls.bg } else { ls.fg };
    let mut preset_chips = vec![
        ActionChip::new(
            format!("FG {}", format_rgb_hex(ls.fg)),
            !app.light_edit_bg,
            HitTarget::LightHexPrompt(false),
        )
        .focus_mark(ctrl_focused && app.light_ctrl_cursor == 3),
        ActionChip::new(
            format!("BG {}", format_rgb_hex(ls.bg)),
            app.light_edit_bg,
            HitTarget::LightHexPrompt(true),
        ),
    ];
    for (i, (name, rgb)) in HUMAN_COLOR_PRESETS.iter().take(5).enumerate() {
        preset_chips.push(ActionChip::new(
            *name,
            active_rgb == *rgb,
            HitTarget::LightPresetPick(app.light_edit_bg, i),
        ));
    }
    render_chip_bar(f, rows[3], &preset_chips, hits);

    // Row 4..6: HSV intuitive human color sliders
    let hsv = rgb_to_hsv(active_rgb);
    let hue_deg = (hsv[0] * 360.0).round();
    let sat_pct = (hsv[1] * 100.0).round();
    let val_pct = (hsv[2] * 100.0).round();
    let target_lbl = if app.light_edit_bg { "BG" } else { "FG" };
    let swatch = Span::styled(
        format!(" {target_lbl}: {} ({}) ", format_rgb_hex(active_rgb), App::preset_color_name(active_rgb)),
        Style::default()
            .bg(Color::Rgb(active_rgb[0], active_rgb[1], active_rgb[2]))
            .fg(contrast_ink(active_rgb))
            .add_modifier(Modifier::BOLD),
    );

    let r4_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(28)])
        .split(rows[4]);
    render_hsv_line(
        f,
        r4_split[0],
        HsvRowSpec {
            text: &format!("Hue        : {hue_deg:>3.0}°"),
            row_focused: ctrl_focused && app.light_ctrl_cursor == 4,
            is_bg: app.light_edit_bg,
            channel: 0,
            step: 0.04,
        },
        hits,
    );
    f.render_widget(Paragraph::new(Line::from(swatch)), r4_split[1]);

    render_hsv_line(
        f,
        rows[5],
        HsvRowSpec {
            text: &format!("Saturation : {sat_pct:>3.0}%"),
            row_focused: ctrl_focused && app.light_ctrl_cursor == 5,
            is_bg: app.light_edit_bg,
            channel: 1,
            step: 0.08,
        },
        hits,
    );
    render_hsv_line(
        f,
        rows[6],
        HsvRowSpec {
            text: &format!("BrightnessV: {val_pct:>3.0}%"),
            row_focused: ctrl_focused && app.light_ctrl_cursor == 6,
            is_bg: app.light_edit_bg,
            channel: 2,
            step: 0.08,
        },
        hits,
    );

    render_chip_bar(
        f,
        rows[7],
        &[
            ActionChip::new(
                "Apply Lighting (Enter)",
                ctrl_focused && app.light_ctrl_cursor == 7,
                HitTarget::LightApply,
            )
            .focus_mark(ctrl_focused && app.light_ctrl_cursor == 7),
            ActionChip::new("Read Current Lighting", false, HitTarget::LightReload),
            ActionChip::new("Type #Hex..", false, HitTarget::LightHexPrompt(app.light_edit_bg)),
        ],
        hits,
    );
}

struct HsvRowSpec<'a> {
    text: &'a str,
    row_focused: bool,
    is_bg: bool,
    channel: usize,
    step: f32,
}

fn render_hsv_line(
    f: &mut Frame<'_>,
    area: Rect,
    spec: HsvRowSpec<'_>,
    hits: &mut Vec<HitRegion>,
) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(14), Constraint::Length(14)])
        .split(area);
    let prefix = if spec.row_focused { "▶ " } else { "  " };
    f.render_widget(
        Paragraph::new(Span::styled(
            format!("{prefix}{}", spec.text),
            Style::default().fg(if spec.row_focused {
                theme::CYAN_HIGHLIGHT
            } else {
                theme::INK_SECONDARY
            }),
        )),
        split[0],
    );
    render_chip_bar(
        f,
        split[1],
        &[
            ActionChip::new(
                " - ",
                false,
                HitTarget::LightHsvStep(spec.is_bg, spec.channel, -spec.step),
            ),
            ActionChip::new(
                " + ",
                false,
                HitTarget::LightHsvStep(spec.is_bg, spec.channel, spec.step),
            ),
        ],
        hits,
    );
}

fn draw_custom_panel(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::LightControls;
    let slot = app.light_tab.custom_slot_index().unwrap_or(0);
    let blk = theme::panel_block(format!("Custom Slot {} Per-Key RGB Painter", slot + 1), focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::LightControls),
    });

    if inner.height < 5 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    render_chip_bar(
        f,
        rows[0],
        &[
            ActionChip::new(
                if app.custom_paint_mode {
                    "Brush Paint: ACTIVE (click key)"
                } else {
                    "Brush Paint: OFF"
                },
                app.custom_paint_mode,
                HitTarget::CustomBrushToggle,
            ),
            ActionChip::new(
                format!("Brush {} ({})", format_rgb_hex(app.custom_brush), App::preset_color_name(app.custom_brush)),
                true,
                HitTarget::CustomBrushPrompt,
            ),
            ActionChip::new("Type #Hex..", false, HitTarget::CustomBrushPrompt),
        ],
        hits,
    );

    let mut swatch_chips = Vec::new();
    for (i, (name, rgb)) in HUMAN_COLOR_PRESETS.iter().take(7).enumerate() {
        swatch_chips.push(ActionChip::new(
            *name,
            app.custom_brush == *rgb,
            HitTarget::CustomBrushPreset(i),
        ));
    }
    render_chip_bar(f, rows[1], &swatch_chips, hits);

    render_chip_bar(
        f,
        rows[2],
        &[
            ActionChip::new("Fill All Keys", false, HitTarget::CustomFillAll),
            ActionChip::new("Clear All (Off)", false, HitTarget::CustomClearAll),
            ActionChip::new("Bri 1", app.custom_brightness == 1, HitTarget::CustomBri(1)),
            ActionChip::new("2", app.custom_brightness == 2, HitTarget::CustomBri(2)),
            ActionChip::new("3", app.custom_brightness == 3, HitTarget::CustomBri(3)),
            ActionChip::new("4", app.custom_brightness == 4, HitTarget::CustomBri(4)),
        ],
        hits,
    );

    render_chip_bar(
        f,
        rows[3],
        &[
            ActionChip::new(
                format!("Upload Slot {} to WIN 60 HE", slot + 1),
                true,
                HitTarget::CustomUpload,
            ),
            ActionChip::new("Reload Slot from KB", false, HitTarget::CustomReload),
        ],
        hits,
    );

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Tip: Focus keyboard (Tab) or click any key above while Brush Paint is ACTIVE to color keys individually.",
            Style::default().fg(theme::INK_MUTED),
        ))),
        rows[4],
    );
}

fn draw_audio_panel(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::LightControls;
    let blk = theme::panel_block("Daemon-Hosted Audio Equalizer & 14-Band FFT", focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::LightControls),
    });

    if inner.height < 6 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(0),
        ])
        .split(inner);

    // Row 0: Start/Stop + Source cycle
    let next_src = (app.audio_source_idx + 1) % app.audio_sources.len().max(1);
    render_chip_bar(
        f,
        rows[0],
        &[
            ActionChip::new(
                if app.audio_status.active {
                    "STOP Audio EQ (Running in reaulad)"
                } else {
                    "START Daemon Audio EQ"
                },
                app.audio_status.active,
                HitTarget::AudioToggle,
            )
            .focus_mark(focused && app.audio_ctrl_cursor == 0),
            ActionChip::new(
                format!("Source: {}", app.selected_audio_source().label()),
                false,
                HitTarget::AudioSourcePick(next_src),
            )
            .focus_mark(focused && app.audio_ctrl_cursor == 1),
        ],
        hits,
    );

    // Row 1: Effect Styles
    let mut style_chips = Vec::new();
    for st in AudioEffectStyle::ALL {
        style_chips.push(ActionChip::new(
            st.label(),
            app.audio_config.style == st,
            HitTarget::AudioStylePick(st),
        ));
    }
    render_chip_bar(f, rows[1], &style_chips, hits);

    // Row 2: Palettes
    let mut pal_chips = Vec::new();
    for pal in AudioPalette::ALL {
        pal_chips.push(ActionChip::new(
            pal.label(),
            app.audio_config.palette == pal,
            HitTarget::AudioPalettePick(pal),
        ));
    }
    render_chip_bar(f, rows[2], &pal_chips, hits);

    // Row 3: Gain / FPS / Brightness steppers
    render_chip_bar(
        f,
        rows[3],
        &[
            ActionChip::new("Gain-", false, HitTarget::AudioGainStep(-0.2)),
            ActionChip::new(format!("Gain {:.1}x", app.audio_config.gain), true, HitTarget::AudioGainStep(0.0)),
            ActionChip::new("Gain+", false, HitTarget::AudioGainStep(0.2)),
            ActionChip::new("FPS-", false, HitTarget::AudioFpsStep(-5)),
            ActionChip::new(format!("{} FPS", app.audio_config.fps), true, HitTarget::AudioFpsStep(0)),
            ActionChip::new("FPS+", false, HitTarget::AudioFpsStep(5)),
            ActionChip::new("Bri-", false, HitTarget::AudioBriStep(-1)),
            ActionChip::new(format!("Bri {}/4", app.audio_config.brightness), true, HitTarget::AudioBriStep(0)),
            ActionChip::new("Bri+", false, HitTarget::AudioBriStep(1)),
        ],
        hits,
    );

    // Row 4: Live 14-Band FFT spectrum + RMS/Beat readout
    const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let mut spans = vec![Span::styled("14-Band FFT: ", Style::default().fg(theme::INK_SECONDARY))];
    for &lvl_raw in &app.audio_status.metrics.bands {
        let lvl = lvl_raw.clamp(0.0, 1.0);
        let idx = (lvl * 8.0).round() as usize;
        let ch = BARS[idx.min(8)];
        let col = if lvl > 0.72 {
            theme::CYAN_HIGHLIGHT
        } else if lvl > 0.25 {
            Color::Rgb(0x3B, 0x82, 0xF6)
        } else {
            theme::INK_DIM
        };
        spans.push(Span::styled(format!("{ch}{ch} "), Style::default().fg(col)));
    }
    let m = &app.audio_status.metrics;
    let stat_line = Line::from(format!(
        "RMS {:.1} dBFS | Peak {:.1} dBFS | Bass {:.2} Mid {:.2} Treble {:.2} | {:.1} fps | {} HID pkts {}",
        m.rms_dbfs,
        m.peak_dbfs,
        m.bass,
        m.mid,
        m.treble,
        app.audio_status.actual_fps,
        app.audio_status.packets_total,
        if m.beat || m.beat_env > 0.35 { "[BEAT]" } else { "" }
    ));
    f.render_widget(
        Paragraph::new(vec![Line::from(spans), stat_line]).wrap(Wrap { trim: false }),
        rows[4],
    );

    if let Some(err) = &app.audio_status.last_error {
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("Error: {err}"),
                Style::default().fg(theme::INK_DANGER),
            )),
            rows[5],
        );
    } else {
        f.render_widget(
            Paragraph::new(Span::styled(
                "Persists inside reaulad after exiting TUI. Requires accepting 10s hardware notice.",
                Style::default().fg(theme::INK_MUTED),
            )),
            rows[5],
        );
    }
}

pub fn handle_key(app: &mut App, code: KeyCode) {
    // Common sub-tab switching on '[' / ']' inside Lighting view
    if matches!(code, KeyCode::Char('[')) {
        let pos = LightEditorTab::ALL.iter().position(|&t| t == app.light_tab).unwrap_or(0);
        app.light_tab = LightEditorTab::ALL[(pos + 3) % 4];
        return;
    }
    if matches!(code, KeyCode::Char(']')) {
        let pos = LightEditorTab::ALL.iter().position(|&t| t == app.light_tab).unwrap_or(0);
        app.light_tab = LightEditorTab::ALL[(pos + 1) % 4];
        return;
    }

    if app.focus == FocusPane::LightKeyboard {
        match code {
            KeyCode::Left | KeyCode::Char('h') => app.move_key_cursor(0, -1),
            KeyCode::Right | KeyCode::Char('l') => app.move_key_cursor(0, 1),
            KeyCode::Up | KeyCode::Char('k') => app.move_key_cursor(-1, 0),
            KeyCode::Down | KeyCode::Char('j') => app.move_key_cursor(1, 0),
            KeyCode::Char(' ') | KeyCode::Enter => {
                if app.light_tab.custom_slot_index().is_some() {
                    app.paint_custom_key(app.current_key().index, false);
                }
            }
            KeyCode::Char('c') if app.light_tab.custom_slot_index().is_some() => {
                app.paint_custom_key(app.current_key().index, true);
            }
            _ => {}
        }
        return;
    }

    match app.light_tab {
        LightEditorTab::FirmwareEffects => {
            if app.focus == FocusPane::LightEffectsList {
                match code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.light_mode_cursor = app.light_mode_cursor.saturating_sub(1);
                        app.light_state.mode = LIGHT_MODE_OPTIONS[app.light_mode_cursor];
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if app.light_mode_cursor + 1 < LIGHT_MODE_OPTIONS.len() {
                            app.light_mode_cursor += 1;
                        }
                        app.light_state.mode = LIGHT_MODE_OPTIONS[app.light_mode_cursor];
                    }
                    KeyCode::Enter => app.apply_lighting(),
                    _ => {}
                }
            } else {
                match code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.light_ctrl_cursor = app.light_ctrl_cursor.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.light_ctrl_cursor = (app.light_ctrl_cursor + 1).min(7);
                    }
                    KeyCode::Left | KeyCode::Char('h') => adjust_light_ctrl(app, -1),
                    KeyCode::Right | KeyCode::Char('l') => adjust_light_ctrl(app, 1),
                    KeyCode::Enter => app.apply_lighting(),
                    KeyCode::Char('r') => app.reload_lighting(),
                    KeyCode::Char('o') => app.light_state.power_on = !app.light_state.power_on,
                    KeyCode::Char('f') => app.light_edit_bg = !app.light_edit_bg,
                    KeyCode::Char('#') => {
                        let rgb = if app.light_edit_bg {
                            app.light_state.bg
                        } else {
                            app.light_state.fg
                        };
                        app.prompt_modal = Some(TextPromptModal::new(
                            if app.light_edit_bg {
                                PromptPurpose::LightBgHex
                            } else {
                                PromptPurpose::LightFgHex
                            },
                            if app.light_edit_bg {
                                "Background Hex / RGB"
                            } else {
                                "Foreground Hex / RGB"
                            },
                            "Enter #RRGGBB or R,G,B (e.g. #00D7FF or 0,215,255)",
                            &format_rgb_hex(rgb),
                        ));
                    }
                    _ => {}
                }
            }
        }
        LightEditorTab::CustomSlot1 | LightEditorTab::CustomSlot2 => match code {
            KeyCode::Char('p') => {
                app.custom_paint_mode = !app.custom_paint_mode;
                if app.custom_paint_mode {
                    app.focus = FocusPane::LightKeyboard;
                }
            }
            KeyCode::Char('f') => app.fill_custom_slot(app.custom_brush),
            KeyCode::Char('c') => app.fill_custom_slot([0, 0, 0]),
            KeyCode::Char('#') => {
                app.prompt_modal = Some(TextPromptModal::new(
                    PromptPurpose::CustomBrushHex,
                    "Brush Hex / RGB Color",
                    "Enter #RRGGBB or R,G,B",
                    &format_rgb_hex(app.custom_brush),
                ));
            }
            KeyCode::Enter => app.upload_custom_slot(),
            _ => {}
        },
        LightEditorTab::AudioSync => match code {
            KeyCode::Char('a') | KeyCode::Enter | KeyCode::Char(' ') => app.toggle_audio_sync(),
            KeyCode::Char('A') => {
                app.audio_source_idx = (app.audio_source_idx + 1) % app.audio_sources.len().max(1);
                let label = app.selected_audio_source().label();
                app.push_audio_live_update(&format!("Audio source -> {label}"));
            }
            KeyCode::Char('s') => {
                let pos = AudioEffectStyle::ALL
                    .iter()
                    .position(|&s| s == app.audio_config.style)
                    .unwrap_or(0);
                app.audio_config.style =
                    AudioEffectStyle::ALL[(pos + 1) % AudioEffectStyle::ALL.len()];
                app.push_audio_live_update(&format!(
                    "Audio style -> {}",
                    app.audio_config.style.label()
                ));
            }
            KeyCode::Char('p') => {
                let pos = AudioPalette::ALL
                    .iter()
                    .position(|&p| p == app.audio_config.palette)
                    .unwrap_or(0);
                app.audio_config.palette =
                    AudioPalette::ALL[(pos + 1) % AudioPalette::ALL.len()];
                app.push_audio_live_update(&format!(
                    "Audio palette -> {}",
                    app.audio_config.palette.label()
                ));
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                app.audio_config.gain = (app.audio_config.gain + 0.2).clamp(0.1, 10.0);
                app.push_audio_live_update(&format!("Gain -> {:.1}x", app.audio_config.gain));
            }
            KeyCode::Char('-') => {
                app.audio_config.gain = (app.audio_config.gain - 0.2).clamp(0.1, 10.0);
                app.push_audio_live_update(&format!("Gain -> {:.1}x", app.audio_config.gain));
            }
            _ => {}
        },
    }
}

fn adjust_light_ctrl(app: &mut App, dir: isize) {
    let target = if app.light_edit_bg {
        &mut app.light_state.bg
    } else {
        &mut app.light_state.fg
    };
    let mut hsv = rgb_to_hsv(*target);
    match app.light_ctrl_cursor {
        0 => {
            if dir >= 0 {
                app.light_state.brightness = (app.light_state.brightness + 1).min(4);
            } else {
                app.light_state.brightness = app.light_state.brightness.saturating_sub(1);
            }
        }
        1 => {
            if dir >= 0 {
                app.light_state.speed = (app.light_state.speed + 1).min(4);
            } else {
                app.light_state.speed = app.light_state.speed.saturating_sub(1);
            }
        }
        2 => {
            app.light_state.direction = if dir >= 0 {
                (app.light_state.direction + 1) % 6
            } else {
                (app.light_state.direction + 5) % 6
            };
        }
        3 => {
            app.light_edit_bg = !app.light_edit_bg;
        }
        4 => {
            hsv[0] = (hsv[0] + if dir >= 0 { 0.04 } else { -0.04 }).rem_euclid(1.0);
            *target = hsv_to_rgb_u8(hsv[0], hsv[1], hsv[2]);
            if !app.light_edit_bg {
                app.light_state.full_color = false;
            }
        }
        5 => {
            hsv[1] = (hsv[1] + if dir >= 0 { 0.08 } else { -0.08 }).clamp(0.0, 1.0);
            *target = hsv_to_rgb_u8(hsv[0], hsv[1], hsv[2]);
            if !app.light_edit_bg {
                app.light_state.full_color = false;
            }
        }
        6 => {
            hsv[2] = (hsv[2] + if dir >= 0 { 0.08 } else { -0.08 }).clamp(0.0, 1.0);
            *target = hsv_to_rgb_u8(hsv[0], hsv[1], hsv[2]);
            if !app.light_edit_bg {
                app.light_state.full_color = false;
            }
        }
        _ => {}
    }
}
