//! Responsive TUI shell, sidebar/topbar navigation, clickable chip engine,
//! floating modals (Audio 10s Disclaimer, Text Prompt, Help), and mouse/key router.

pub mod keyboard;
pub mod views_cal;
pub mod views_diag;
pub mod views_guide;
pub mod views_keymaps;
pub mod views_light;
pub mod views_main;

use std::time::Instant;

use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Gauge, Paragraph, Wrap};

use reaula_core::keymap;
use reaula_core::protocol::PrcsPair;

use crate::state::{
    AUDIO_DISCLAIMER_COOLDOWN_SECS, App, FocusPane, HitRegion, HitTarget, KeymapCategory,
    LIGHT_MODE_OPTIONS, LightEditorTab, NavSection, PromptPurpose,
};
use crate::theme::{self, HUMAN_COLOR_PRESETS, hsv_to_rgb_u8, rgb_to_hsv};

pub struct ActionChip {
    pub label: String,
    pub active: bool,
    pub focused: bool,
    pub target: HitTarget,
}

impl ActionChip {
    pub fn new(label: impl Into<String>, active: bool, target: HitTarget) -> Self {
        Self {
            label: label.into(),
            active,
            focused: false,
            target,
        }
    }

    pub fn focus_mark(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }
}

pub fn render_chip_bar(
    f: &mut Frame<'_>,
    area: Rect,
    chips: &[ActionChip],
    hits: &mut Vec<HitRegion>,
) {
    if area.height == 0 || area.width < 4 {
        return;
    }
    let mut x = area.x;
    let max_x = area.x + area.width;
    let mut spans: Vec<Span<'_>> = Vec::new();

    for chip in chips {
        let txt = if chip.focused {
            format!(" [▶ {}] ", chip.label)
        } else if chip.active {
            format!(" [● {}] ", chip.label)
        } else {
            format!(" [{}] ", chip.label)
        };
        let char_w = txt.chars().count() as u16;
        if x >= max_x {
            break;
        }
        let clipped_w = char_w.min(max_x - x);
        let style = if chip.focused {
            Style::default()
                .bg(theme::SHADE_REGAL)
                .fg(theme::CYAN_HIGHLIGHT)
                .add_modifier(Modifier::BOLD)
        } else if chip.active {
            Style::default()
                .bg(theme::SHADE_REGAL)
                .fg(theme::INK_PRIMARY)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .bg(theme::SHADE_DEEP)
                .fg(theme::INK_SECONDARY)
        };
        let display: String = txt.chars().take(usize::from(clipped_w)).collect();
        spans.push(Span::styled(display, style));
        hits.push(HitRegion {
            rect: Rect::new(x, area.y, clipped_w, 1),
            action: chip.target.clone(),
        });
        x = x.saturating_add(clipped_w);
        if x < max_x {
            spans.push(Span::raw(" "));
            x += 1;
        }
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub fn draw(f: &mut Frame<'_>, app: &mut App) {
    app.hit_regions.clear();
    let mut hits = std::mem::take(&mut app.hit_regions);

    let full = f.area();
    f.render_widget(
        Paragraph::new("").style(Style::default().bg(theme::SHADE_DEEP)),
        full,
    );

    let shell = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(2),
        ])
        .split(full);

    draw_top_bar(f, shell[0], app, &mut hits);

    let use_sidebar = full.width >= 102 && full.height >= 26;
    if use_sidebar {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(26), Constraint::Min(60)])
            .split(shell[1]);
        draw_sidebar(f, body[0], app, &mut hits);
        draw_active_section(f, body[1], app, &mut hits);
    } else {
        let body = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(10)])
            .split(shell[1]);
        draw_compact_tabs(f, body[0], app, &mut hits);
        draw_active_section(f, body[1], app, &mut hits);
    }

    draw_footer(f, shell[2], app, &mut hits);

    if app.mascots.any_visible() {
        draw_mascot_overlay(f, full, app, &mut hits);
    }
    if app.audio_disclaimer_modal_open {
        draw_audio_disclaimer_modal(f, full, app, &mut hits);
    }
    if let Some(prompt) = &app.prompt_modal {
        draw_prompt_modal(f, full, prompt, &mut hits);
    }
    if app.show_help {
        draw_help_modal(f, full, &mut hits);
    }

    app.hit_regions = hits;
}

fn draw_top_bar(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let model = app
        .daemon_status
        .keyboard_name
        .as_deref()
        .or(app.probe.model_name.as_deref())
        .unwrap_or("AULA WIN 60 HE");
    let fw = app.probe.firmware_string.as_deref().unwrap_or("USB HID");
    let mgr = if app.daemon_status.systemd_managed {
        "systemd"
    } else {
        "standalone"
    };
    let travel = if app.travel_enabled { "Travel:ON" } else { "Travel:OFF" };
    let eq = if app.audio_status.active {
        format!("AudioEQ:{:.0}fps", app.audio_status.actual_fps)
    } else {
        "AudioEQ:OFF".into()
    };

    let blk = theme::panel_block(
        format!(
            "REAULA TUI [EXPERIMENTAL: INCOMPLETE / HAS BUGS] · {} · {} · {} kHz · pid={} ({}) · {} · {}",
            model, fw, app.poll_rate_khz, app.daemon_status.daemon_pid, mgr, travel, eq
        ),
        false,
    );
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    if inner.height >= 1 {
        let usb_status = if app.daemon_status.device_connected {
            format!("USB ONLINE (max {:.2}mm)", app.max_mm)
        } else {
            "USB WAITING (connect keyboard)".into()
        };
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(24), Constraint::Length(56)])
            .split(inner);
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", app.section.label()),
                    Style::default()
                        .bg(theme::SHADE_REGAL)
                        .fg(theme::CYAN_HIGHLIGHT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(app.section.subtitle(), Style::default().fg(theme::INK_MUTED)),
                Span::raw(" · "),
                Span::styled(
                    usb_status,
                    Style::default().fg(if app.daemon_status.device_connected {
                        theme::INK_OK
                    } else {
                        theme::INK_WARN
                    }),
                ),
            ])),
            cols[0],
        );
        render_chip_bar(
            f,
            cols[1],
            &[
                ActionChip::new("Travel (t)", app.travel_enabled, HitTarget::TravelToggle),
                ActionChip::new("Zero (z)", false, HitTarget::ZeroStuck),
                ActionChip::new("Refresh (R)", false, HitTarget::RefreshAll),
                ActionChip::new("Help (?)", app.show_help, HitTarget::HelpToggle),
                ActionChip::new("Quit (q)", false, HitTarget::Quit),
            ],
            hits,
        );
    }
}

fn draw_sidebar(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let blk = theme::panel_block("Navigation (1-6)", false);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    if inner.height < 8 {
        return;
    }

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(6)])
        .split(inner);

    let mut y = split[0].y;
    for (idx, &sec) in NavSection::ALL.iter().enumerate() {
        if y + 1 >= split[0].y + split[0].height {
            break;
        }
        let active = app.section == sec;
        let rect = Rect::new(split[0].x, y, split[0].width, 2);
        hits.push(HitRegion {
            rect,
            action: HitTarget::Nav(sec),
        });
        let bg = if active { theme::SHADE_REGAL } else { theme::SHADE_BASE };
        let fg = if active { theme::CYAN_HIGHLIGHT } else { theme::INK_PRIMARY };
        let sub_fg = if active { theme::INK_ACCENT } else { theme::INK_MUTED };
        let lines = vec![
            Line::from(Span::styled(
                format!(" {} {} ({})", if active { "▶" } else { " " }, sec.label(), idx + 1),
                Style::default().fg(fg).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("   {}", sec.subtitle()),
                Style::default().fg(sub_fg),
            )),
        ];
        f.render_widget(Paragraph::new(lines).style(Style::default().bg(bg)), rect);
        y += 2;
    }

    let info_lines = vec![
        Line::from(Span::styled(
            format!("UID: {} (SO_PEERCRED)", app.server_cred.uid),
            Style::default().fg(theme::INK_MUTED),
        )),
        Line::from(Span::styled(
            format!("Sock: {}", trim_tail(&app.daemon_status.socket_path, 22)),
            Style::default().fg(theme::INK_MUTED),
        )),
        Line::from(Span::styled(
            "Tab: Cycle Focus Pane",
            Style::default().fg(theme::INK_ACCENT),
        )),
        Line::from(Span::styled(
            "Mouse: Click & Scroll",
            Style::default().fg(theme::INK_ACCENT),
        )),
    ];
    f.render_widget(
        Paragraph::new(info_lines).block(theme::well_block("Session", theme::BORDER_SUBTLE)),
        split[1],
    );
}

fn draw_compact_tabs(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let chips: Vec<ActionChip> = NavSection::ALL
        .iter()
        .map(|&s| ActionChip::new(s.short_label(), app.section == s, HitTarget::Nav(s)))
        .collect();
    render_chip_bar(f, area, &chips, hits);
}

fn draw_active_section(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    match app.section {
        NavSection::Main => views_main::draw(f, area, app, hits),
        NavSection::Guide => views_guide::draw(f, area, app, hits),
        NavSection::Calibration => views_cal::draw(f, area, app, hits),
        NavSection::ColorLight => views_light::draw(f, area, app, hits),
        NavSection::Keymaps => views_keymaps::draw(f, area, app, hits),
        NavSection::Diagnostics => views_diag::draw(f, area, app, hits),
    }
}

fn draw_footer(f: &mut Frame<'_>, area: Rect, app: &App, _hits: &mut Vec<HitRegion>) {
    let hints = match app.section {
        NavSection::Main => "Tab:Switch Pane · Arrows/hjkl:Navigate/Adjust · Space:Toggle · Enter:Apply · a:All · w:WASD",
        NavSection::Guide => "Up/Down/Scroll:Read · 1-6:Jump Workspace",
        NavSection::Calibration => "Tab:Map/Controls · a:All-Key · k:Any-Key · s:Disarm",
        NavSection::ColorLight => "[/]:Subtab · Tab:List/Controls/Keys · o:Power · #:Hex · a:Audio EQ · Enter:Apply",
        NavSection::Keymaps => "Tab:Composer/Matrix/FW/DM · r:Capture HW · Space:Toggle · Enter:Edit/Save · f:Test Fire · d:Del",
        NavSection::Diagnostics => "Tab:Bar/Cutaway/Table/Log · D:4-Layer Diag · E:Echo RTT · Space:Sweep",
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(" KEYBINDS: ", Style::default().fg(theme::CYAN_HIGHLIGHT).add_modifier(Modifier::BOLD)),
            Span::styled(hints, Style::default().fg(theme::INK_MUTED)),
        ]),
        Line::from(vec![
            Span::styled(" NOTE: ", Style::default().fg(theme::INK_WARN).add_modifier(Modifier::BOLD)),
            Span::styled("TUI is not yet properly developed and still has bugs (prefer GUI).  ", Style::default().fg(theme::INK_WARN)),
            Span::styled("STATUS: ", Style::default().fg(theme::INK_OK).add_modifier(Modifier::BOLD)),
            Span::styled(&app.status_msg, Style::default().fg(theme::INK_PRIMARY)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::SHADE_BASE)),
        area,
    );
}

fn draw_mascot_overlay(f: &mut Frame<'_>, full: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    if full.width < 34 || full.height < 12 {
        return;
    }
    let w = 28u16.min(full.width);
    let h = 6u16;
    let rect = Rect::new(full.x + full.width - w - 1, full.y + 3, w, h);
    f.render_widget(Clear, rect);
    let mut lines = Vec::new();
    if app.mascots.miku_visible {
        lines.push(Line::from(Span::styled(
            " [39] <( Miku Online! )",
            Style::default().fg(Color::Rgb(0x39, 0xC5, 0xBB)).add_modifier(Modifier::BOLD),
        )));
    }
    if app.mascots.teto_visible {
        lines.push(Line::from(Span::styled(
            " [41] <( Teto Drills! )",
            Style::default().fg(Color::Rgb(0xE1, 0x1D, 0x48)).add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(Span::styled(
        " Click or repeat code to hide",
        Style::default().fg(theme::INK_MUTED),
    )));
    f.render_widget(
        Paragraph::new(lines).block(theme::well_block("!", theme::CYAN_HIGHLIGHT)),
        rect,
    );
    hits.push(HitRegion {
        rect,
        action: HitTarget::MascotDismiss,
    });
}

fn draw_audio_disclaimer_modal(
    f: &mut Frame<'_>,
    full: Rect,
    app: &App,
    hits: &mut Vec<HitRegion>,
) {
    hits.push(HitRegion {
        rect: full,
        action: HitTarget::DisclaimerCancel,
    });
    let rect = centered_rect(72, 15, full);
    f.render_widget(Clear, rect);
    let blk = theme::panel_block("Experimental Audio Sync — Hardware Disclaimer", true);
    let inner = blk.inner(rect);
    f.render_widget(blk, rect);
    if inner.height < 8 {
        return;
    }

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    let copy = Paragraph::new(vec![
        Line::from(Span::styled(
            "WARNING: AULA states WIN 60 HE does NOT officially support Audio Sync lighting.",
            Style::default().fg(theme::INK_WARN).add_modifier(Modifier::BOLD),
        )),
        Line::from("Continuous per-key cmd 0x09 streaming runs inside reaulad even after TUI exit."),
        Line::from("High-frequency LED modulation is experimental and may reduce LED lifespan."),
        Line::from("Use strictly at your own risk."),
    ])
    .wrap(Wrap { trim: false });
    f.render_widget(copy, v[0]);

    let rem = app.disclaimer_remaining_secs();
    let ratio = ((AUDIO_DISCLAIMER_COOLDOWN_SECS - rem) / AUDIO_DISCLAIMER_COOLDOWN_SECS).clamp(0.0, 1.0);
    f.render_widget(
        Gauge::default()
            .block(theme::well_block("Mandatory Read Timer", theme::BORDER_WARN))
            .gauge_style(Style::default().fg(if rem <= 0.0 {
                theme::INK_OK
            } else {
                theme::FW_AMBER_INK
            }))
            .ratio(ratio)
            .label(if rem <= 0.0 {
                "Cooldown Complete".into()
            } else {
                format!("Please wait {:.0}s...", rem.ceil())
            }),
        v[1],
    );

    let chk_label = if app.audio_disclaimer_checked {
        "[x] I Agree — I accept full hardware risk"
    } else {
        "[ ] I Agree — Press Space or Click to check"
    };
    render_chip_bar(
        f,
        v[2],
        &[ActionChip::new(
            chk_label,
            app.audio_disclaimer_checked,
            HitTarget::DisclaimerCheckToggle,
        )],
        hits,
    );

    render_chip_bar(
        f,
        v[3],
        &[
            ActionChip::new(
                if rem <= 0.0 && app.audio_disclaimer_checked {
                    "Confirm & Start Audio EQ (Enter)"
                } else {
                    "Locked until timer + Agree"
                },
                rem <= 0.0 && app.audio_disclaimer_checked,
                HitTarget::DisclaimerConfirm,
            ),
            ActionChip::new("Cancel (Esc)", false, HitTarget::DisclaimerCancel),
        ],
        hits,
    );
}

fn draw_prompt_modal(
    f: &mut Frame<'_>,
    full: Rect,
    prompt: &crate::state::TextPromptModal,
    hits: &mut Vec<HitRegion>,
) {
    hits.push(HitRegion {
        rect: full,
        action: HitTarget::ModalClose,
    });
    let rect = centered_rect(72, 10, full);
    f.render_widget(Clear, rect);
    let blk = theme::panel_block(&prompt.title, true);
    let inner = blk.inner(rect);
    f.render_widget(blk, rect);
    if inner.height < 5 {
        return;
    }

    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(
        Paragraph::new(Span::styled(&prompt.hint, Style::default().fg(theme::INK_SECONDARY))),
        v[0],
    );

    // Build visible line with block cursor marker
    let chars: Vec<char> = prompt.buffer.chars().collect();
    let before: String = chars[..prompt.cursor.min(chars.len())].iter().collect();
    let cur_ch = chars.get(prompt.cursor).copied().unwrap_or(' ');
    let after: String = if prompt.cursor < chars.len() {
        chars[prompt.cursor + 1..].iter().collect()
    } else {
        String::new()
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(before),
            Span::styled(
                cur_ch.to_string(),
                Style::default()
                    .bg(theme::CYAN_HIGHLIGHT)
                    .fg(theme::SHADE_DEEP)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(after),
        ]))
        .block(theme::well_block("Input", theme::BORDER_FOCUS)),
        v[1],
    );

    if let Some(err) = &prompt.error {
        f.render_widget(
            Paragraph::new(Span::styled(err, Style::default().fg(theme::INK_DANGER))),
            v[2],
        );
    }

    render_chip_bar(
        f,
        v[3],
        &[
            ActionChip::new("Apply / Save (Enter)", true, HitTarget::PromptSubmit),
            ActionChip::new("Cancel (Esc)", false, HitTarget::ModalClose),
        ],
        hits,
    );
}

fn draw_help_modal(f: &mut Frame<'_>, full: Rect, hits: &mut Vec<HitRegion>) {
    hits.push(HitRegion {
        rect: full,
        action: HitTarget::ModalClose,
    });
    let rect = centered_rect(76, 20, full);
    f.render_widget(Clear, rect);
    let blk = theme::panel_block("reaula-tui Help & Controls (? / Esc to close)", true);
    let inner = blk.inner(rect);
    f.render_widget(blk, rect);
    hits.push(HitRegion {
        rect,
        action: HitTarget::ModalClose,
    });

    let help = vec![
        Line::from(Span::styled(
            "Global Navigation & Controls",
            Style::default().fg(theme::CYAN_HIGHLIGHT).add_modifier(Modifier::BOLD),
        )),
        Line::from("  1..6 / F1..F6  Switch workspace (Main, Guide, Calibration, Color & Light, Keymaps, Diagnostics)"),
        Line::from("  Tab / Shift+Tab Cycle active pane inside the current workspace"),
        Line::from("  t / z / R      Toggle Live Travel · Zero Stuck Keys · Full Refresh & Reconnect"),
        Line::from("  Mouse          Click any keycap, tab, sidebar item, list row, stepper [-]/[+] or button"),
        Line::from("  Scroll Wheel   Scroll tables, logs, guide, effect lists, or adjust hovered list"),
        Line::from("  ? / q / Ctrl+C Toggle this Help overlay · Quit cleanly restoring terminal"),
        Line::from(""),
        Line::from(Span::styled(
            "Workspace Highlights (Full GUI Parity)",
            Style::default().fg(theme::INK_ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from("  Main           61-key selection, Actuation/RT/RT-Split, Switch Profile, Deadbands, SOCD, Poll, Win-Lock"),
        Line::from("  Calibration    Arm All-Key / Any-Key calibration with live 61-key progress matrix"),
        Line::from("  Color & Light  16 Firmware effects, HSV/Preset/#Hex color picker, Custom Slots 1-2 painter, Audio EQ"),
        Line::from("  Keymaps        Dual-layer [FW] Amber Flash (cmd 0x18) + [DM] Aqua Daemon rules + Test Fire"),
        Line::from("  Diagnostics    4-Layer Fault Isolation, 5x HID Echo RTT, Switch Cutaway, 61-Key Depth & Return Check"),
        Line::from(""),
        Line::from(Span::styled(
            "Click anywhere or press Esc / ? to dismiss",
            Style::default().fg(theme::INK_OK),
        )),
    ];
    f.render_widget(
        Paragraph::new(help).alignment(Alignment::Left).wrap(Wrap { trim: false }),
        inner,
    );
}

fn centered_rect(max_w: u16, max_h: u16, area: Rect) -> Rect {
    let w = max_w.min(area.width.saturating_sub(2)).max(20);
    let h = max_h.min(area.height.saturating_sub(2)).max(6);
    Rect::new(
        area.x + area.width.saturating_sub(w) / 2,
        area.y + area.height.saturating_sub(h) / 2,
        w,
        h,
    )
}

fn trim_tail(s: &str, max_c: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_c {
        s.to_string()
    } else {
        let tail: String = chars[chars.len() - (max_c.saturating_sub(2))..].iter().collect();
        format!("..{tail}")
    }
}

pub fn handle_key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
    if matches!(code, KeyCode::Char('c')) && modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }

    if let KeyCode::Char(ch) = code
        && matches!(ch, '1' | '3' | '4' | '9')
        && app.prompt_modal.is_none()
    {
        let s = ch.to_string();
        app.mascots.record_code(&s, Instant::now());
    }

    // Modal priority routing
    if let Some(modal) = &mut app.prompt_modal {
        match code {
            KeyCode::Esc => {
                app.prompt_modal = None;
            }
            KeyCode::Enter => app.submit_prompt(),
            KeyCode::Backspace => modal.backspace(),
            KeyCode::Delete => modal.delete(),
            KeyCode::Left => modal.move_left(),
            KeyCode::Right => modal.move_right(),
            KeyCode::Home => modal.move_home(),
            KeyCode::End => modal.move_end(),
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                let mut buf = [0u8; 4];
                modal.insert_str(c.encode_utf8(&mut buf));
            }
            _ => {}
        }
        return false;
    }

    if app.audio_disclaimer_modal_open {
        match code {
            KeyCode::Esc => app.audio_disclaimer_modal_open = false,
            KeyCode::Char(' ') => app.audio_disclaimer_checked = !app.audio_disclaimer_checked,
            KeyCode::Enter => app.confirm_audio_disclaimer(),
            _ => {}
        }
        return false;
    }

    if app.show_help {
        if matches!(code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
            app.show_help = false;
        }
        return false;
    }

    if matches!(code, KeyCode::Char('q')) {
        return true;
    }

    if matches!(code, KeyCode::Esc) && app.shortcut_recording {
        app.shortcut_recording = false;
        app.record_first_key = None;
        app.status_msg = "Cancelled hardware key capture.".into();
        return false;
    }

    // Global navigation & utility bindings
    match code {
        KeyCode::F(1) => {
            app.switch_section(NavSection::Main);
            return false;
        }
        KeyCode::F(2) => {
            app.switch_section(NavSection::Guide);
            return false;
        }
        KeyCode::F(3) => {
            app.switch_section(NavSection::Calibration);
            return false;
        }
        KeyCode::F(4) => {
            app.switch_section(NavSection::ColorLight);
            return false;
        }
        KeyCode::F(5) => {
            app.switch_section(NavSection::Keymaps);
            return false;
        }
        KeyCode::F(6) => {
            app.switch_section(NavSection::Diagnostics);
            return false;
        }
        KeyCode::Char(c) if NavSection::from_digit(c).is_some() => {
            if let Some(sec) = NavSection::from_digit(c) {
                app.switch_section(sec);
            }
            return false;
        }
        KeyCode::Tab => {
            app.cycle_focus(true);
            return false;
        }
        KeyCode::BackTab => {
            app.cycle_focus(false);
            return false;
        }
        KeyCode::Char('t') | KeyCode::Char('T') => {
            app.set_travel_streaming(!app.travel_enabled);
            return false;
        }
        KeyCode::Char('z') | KeyCode::Char('Z') => {
            app.clear_stuck_travel();
            return false;
        }
        KeyCode::Char('R') => {
            app.refresh_all();
            return false;
        }
        KeyCode::Char('?') => {
            app.show_help = !app.show_help;
            return false;
        }
        _ => {}
    }

    match app.section {
        NavSection::Main => views_main::handle_key(app, code),
        NavSection::Guide => views_guide::handle_key(app, code),
        NavSection::Calibration => views_cal::handle_key(app, code),
        NavSection::ColorLight => views_light::handle_key(app, code),
        NavSection::Keymaps => views_keymaps::handle_key(app, code),
        NavSection::Diagnostics => views_diag::handle_key(app, code),
    }
    false
}

pub fn handle_mouse(app: &mut App, me: MouseEvent) -> bool {
    match me.kind {
        MouseEventKind::ScrollUp => {
            match app.section {
                NavSection::Guide => app.guide_scroll = app.guide_scroll.saturating_sub(2),
                NavSection::Diagnostics if app.focus == FocusPane::DiagFeedbackLog => {
                    app.diag_log_scroll = app.diag_log_scroll.saturating_sub(2);
                }
                NavSection::Diagnostics => {
                    app.diag_table_cursor = app.diag_table_cursor.saturating_sub(2);
                }
                NavSection::ColorLight if app.light_tab == LightEditorTab::FirmwareEffects => {
                    app.light_mode_cursor = app.light_mode_cursor.saturating_sub(1);
                    app.light_state.mode = LIGHT_MODE_OPTIONS[app.light_mode_cursor];
                }
                NavSection::Keymaps => {
                    app.km_dm_cursor = app.km_dm_cursor.saturating_sub(1);
                }
                _ => {}
            }
            return false;
        }
        MouseEventKind::ScrollDown => {
            match app.section {
                NavSection::Guide => app.guide_scroll = app.guide_scroll.saturating_add(2),
                NavSection::Diagnostics if app.focus == FocusPane::DiagFeedbackLog => {
                    app.diag_log_scroll = app.diag_log_scroll.saturating_add(2);
                }
                NavSection::Diagnostics => {
                    app.diag_table_cursor =
                        (app.diag_table_cursor + 2).min(keymap::WIN60HE_KEYS.len() - 1);
                }
                NavSection::ColorLight if app.light_tab == LightEditorTab::FirmwareEffects => {
                    if app.light_mode_cursor + 1 < LIGHT_MODE_OPTIONS.len() {
                        app.light_mode_cursor += 1;
                    }
                    app.light_state.mode = LIGHT_MODE_OPTIONS[app.light_mode_cursor];
                }
                NavSection::Keymaps if app.km_dm_cursor + 1 < app.shortcut_rules.len() => {
                    app.km_dm_cursor += 1;
                }
                _ => {}
            }
            return false;
        }
        MouseEventKind::Down(MouseButton::Left) => {}
        MouseEventKind::Down(MouseButton::Right) => {
            // Right-click on custom slot key erases LED to black
            if app.section == NavSection::ColorLight
                && app.light_tab.custom_slot_index().is_some()
                && let Some(HitTarget::SelectKey(idx)) = find_hit(&app.hit_regions, me.column, me.row)
            {
                app.focus_key_index(idx);
                app.paint_custom_key(idx, true);
            }
            return false;
        }
        _ => return false,
    }

    let Some(target) = find_hit(&app.hit_regions, me.column, me.row) else {
        return false;
    };
    dispatch_hit(app, target, me.modifiers)
}

fn find_hit(regions: &[HitRegion], col: u16, row: u16) -> Option<HitTarget> {
    regions
        .iter()
        .rev()
        .find(|r| {
            col >= r.rect.x
                && col < r.rect.x.saturating_add(r.rect.width)
                && row >= r.rect.y
                && row < r.rect.y.saturating_add(r.rect.height)
        })
        .map(|r| r.action.clone())
}

pub fn dispatch_hit(app: &mut App, target: HitTarget, modifiers: KeyModifiers) -> bool {
    match target {
        HitTarget::Quit => return true,
        HitTarget::Nav(sec) => app.switch_section(sec),
        HitTarget::Focus(pane) => app.focus = pane,
        HitTarget::SelectKey(idx) => {
            app.focus_key_index(idx);
            match app.section {
                NavSection::Main => {
                    app.focus = FocusPane::MainKeyboard;
                    if modifiers.contains(KeyModifiers::CONTROL)
                        || modifiers.contains(KeyModifiers::SHIFT)
                    {
                        if !app.selected_indices.remove(&idx) {
                            app.selected_indices.insert(idx);
                        }
                    } else {
                        app.selected_indices.clear();
                        app.selected_indices.insert(idx);
                    }
                }
                NavSection::ColorLight => {
                    app.focus = FocusPane::LightKeyboard;
                    if app.light_tab.custom_slot_index().is_some() && app.custom_paint_mode {
                        app.paint_custom_key(idx, false);
                    }
                }
                NavSection::Keymaps if idx != keymap::idx::FN => {
                    app.draft_rule_key = idx;
                    app.status_msg = format!(
                        "Set keymap trigger key -> {} (#{idx}).",
                        app.current_key().name
                    );
                }
                _ => {}
            }
        }
        HitTarget::SelectAll => app.selected_indices = keymap::all_indices().into_iter().collect(),
        HitTarget::SelectWasd => app.selected_indices = [46, 68, 69, 70].into_iter().collect(),
        HitTarget::SelectFocusOnly => {
            app.selected_indices.clear();
            app.selected_indices.insert(app.current_key().index);
        }
        HitTarget::TravelToggle => app.set_travel_streaming(!app.travel_enabled),
        HitTarget::ZeroStuck => app.clear_stuck_travel(),
        HitTarget::RefreshAll => app.refresh_all(),
        HitTarget::HelpToggle => app.show_help = !app.show_help,
        HitTarget::ActField(r) => {
            app.focus = FocusPane::MainActuation;
            app.main_act_row = r;
        }
        HitTarget::ActMode(m) => {
            app.focus = FocusPane::MainActuation;
            app.main_act_row = 0;
            app.draft_mode = m;
        }
        HitTarget::ActTravelStep(d) => {
            app.focus = FocusPane::MainActuation;
            app.main_act_row = 1;
            app.draft_travel_mm = (app.draft_travel_mm + d).clamp(app.min_mm, app.max_mm);
        }
        HitTarget::ActRtPressStep(d) => {
            app.focus = FocusPane::MainActuation;
            app.main_act_row = 2;
            app.draft_rt_press_mm = (app.draft_rt_press_mm + d).clamp(0.05, 2.00);
        }
        HitTarget::ActRtRelStep(d) => {
            app.focus = FocusPane::MainActuation;
            app.main_act_row = 3;
            app.draft_rt_release_mm = (app.draft_rt_release_mm + d).clamp(0.05, 2.00);
        }
        HitTarget::ActApply => app.apply_actuation(),
        HitTarget::SwPick(sw) => {
            app.focus = FocusPane::MainSwitchDeadband;
            app.main_sw_row = 0;
            app.draft_switch_id = sw;
        }
        HitTarget::SwApply => app.apply_switch(),
        HitTarget::DbField(r) => {
            app.focus = FocusPane::MainSwitchDeadband;
            app.main_sw_row = r;
        }
        HitTarget::DbTopStep(d) => {
            app.focus = FocusPane::MainSwitchDeadband;
            app.main_sw_row = 2;
            app.draft_db_top_mm = (app.draft_db_top_mm + d).clamp(0.0, 1.0);
        }
        HitTarget::DbBottomStep(d) => {
            app.focus = FocusPane::MainSwitchDeadband;
            app.main_sw_row = 3;
            app.draft_db_bottom_mm = (app.draft_db_bottom_mm + d).clamp(0.0, 1.0);
        }
        HitTarget::DbApply => app.apply_deadband(),
        HitTarget::PollPick(khz) => app.apply_poll_rate(khz),
        HitTarget::SocdToggle => app.socd_enabled = !app.socd_enabled,
        HitTarget::SocdPresetClear => {
            app.socd_pairs.clear();
            app.socd_enabled = false;
        }
        HitTarget::SocdPresetAd => {
            app.socd_enabled = true;
            app.socd_pairs = vec![PrcsPair {
                key1_hid: 0x04,
                key2_hid: 0x07,
                model: 1,
            }];
        }
        HitTarget::SocdPresetAdWs => {
            app.socd_enabled = true;
            app.socd_pairs = vec![
                PrcsPair {
                    key1_hid: 0x04,
                    key2_hid: 0x07,
                    model: 1,
                },
                PrcsPair {
                    key1_hid: 0x1A,
                    key2_hid: 0x16,
                    model: 1,
                },
            ];
        }
        HitTarget::SocdPromptPair => {
            app.prompt_modal = Some(crate::state::TextPromptModal::new(
                PromptPurpose::SocdCustomPair,
                "Custom PRCS / SOCD Pairs",
                "Format: KEY1:KEY2:MODEL (e.g. A:D:2, W:S:1)",
                "A:D:2",
            ));
        }
        HitTarget::SocdApply => app.apply_socd(),
        HitTarget::WinLockFlag(i) => match i {
            0 => app.win_lock.disable_win = !app.win_lock.disable_win,
            1 => app.win_lock.disable_shift_tab = !app.win_lock.disable_shift_tab,
            2 => app.win_lock.disable_alt_tab = !app.win_lock.disable_alt_tab,
            3 => app.win_lock.disable_alt_f4 = !app.win_lock.disable_alt_f4,
            _ => app.win_lock.win_lock_on = !app.win_lock.win_lock_on,
        },
        HitTarget::WinLockApply => app.apply_win_lock(),
        HitTarget::GuideScroll(d) => {
            if d < 0 {
                app.guide_scroll = app.guide_scroll.saturating_sub((-d) as u16);
            } else {
                app.guide_scroll = app.guide_scroll.saturating_add(d as u16);
            }
        }
        HitTarget::CalArmAll => app.start_calibration(false),
        HitTarget::CalArmAny => app.start_calibration(true),
        HitTarget::CalStop => app.stop_calibration(),
        HitTarget::LightTab(t) => app.light_tab = t,
        HitTarget::LightEffectPick(i) => {
            app.focus = FocusPane::LightEffectsList;
            if i < LIGHT_MODE_OPTIONS.len() {
                app.light_mode_cursor = i;
                app.light_state.mode = LIGHT_MODE_OPTIONS[i];
            }
        }
        HitTarget::LightBri(b) => app.light_state.brightness = b.min(4),
        HitTarget::LightSpd(s) => app.light_state.speed = s.min(4),
        HitTarget::LightDirNext => app.light_state.direction = (app.light_state.direction + 1) % 6,
        HitTarget::LightFullColor => app.light_state.full_color = !app.light_state.full_color,
        HitTarget::LightPower => app.light_state.power_on = !app.light_state.power_on,
        HitTarget::LightPresetPick(is_bg, i) => {
            if let Some(&(_, rgb)) = HUMAN_COLOR_PRESETS.get(i) {
                if is_bg {
                    app.light_state.bg = rgb;
                } else {
                    app.light_state.fg = rgb;
                    app.light_state.full_color = false;
                }
            }
        }
        HitTarget::LightHsvStep(is_bg, ch, delta) => {
            let slot = if is_bg {
                &mut app.light_state.bg
            } else {
                &mut app.light_state.fg
            };
            let mut hsv = rgb_to_hsv(*slot);
            if ch == 0 {
                hsv[0] = (hsv[0] + delta).rem_euclid(1.0);
            } else {
                hsv[ch] = (hsv[ch] + delta).clamp(0.0, 1.0);
            }
            *slot = hsv_to_rgb_u8(hsv[0], hsv[1], hsv[2]);
            if !is_bg {
                app.light_state.full_color = false;
            }
        }
        HitTarget::LightHexPrompt(is_bg) => {
            app.light_edit_bg = is_bg;
            let cur = if is_bg {
                app.light_state.bg
            } else {
                app.light_state.fg
            };
            app.prompt_modal = Some(crate::state::TextPromptModal::new(
                if is_bg {
                    PromptPurpose::LightBgHex
                } else {
                    PromptPurpose::LightFgHex
                },
                if is_bg { "Background Hex / RGB" } else { "Foreground Hex / RGB" },
                "Enter #RRGGBB or R,G,B",
                &theme::format_rgb_hex(cur),
            ));
        }
        HitTarget::LightApply => app.apply_lighting(),
        HitTarget::LightReload => app.reload_lighting(),
        HitTarget::CustomBrushToggle => {
            app.custom_paint_mode = !app.custom_paint_mode;
            if app.custom_paint_mode {
                app.focus = FocusPane::LightKeyboard;
            }
        }
        HitTarget::CustomBrushPreset(i) => {
            if let Some(&(_, rgb)) = HUMAN_COLOR_PRESETS.get(i) {
                app.custom_brush = rgb;
                app.custom_paint_mode = true;
            }
        }
        HitTarget::CustomBrushPrompt => {
            app.prompt_modal = Some(crate::state::TextPromptModal::new(
                PromptPurpose::CustomBrushHex,
                "Brush Color (#RRGGBB or R,G,B)",
                "Example: #00D7FF or 0,215,255",
                &theme::format_rgb_hex(app.custom_brush),
            ));
        }
        HitTarget::CustomFillAll => app.fill_custom_slot(app.custom_brush),
        HitTarget::CustomClearAll => app.fill_custom_slot([0, 0, 0]),
        HitTarget::CustomBri(b) => app.custom_brightness = b.clamp(1, 4),
        HitTarget::CustomUpload => app.upload_custom_slot(),
        HitTarget::CustomReload => app.reload_custom_slot(),
        HitTarget::AudioSourcePick(i) => {
            if i < app.audio_sources.len() {
                app.audio_source_idx = i;
                let lbl = app.selected_audio_source().label();
                app.push_audio_live_update(&format!("Audio source -> {lbl}"));
            }
        }
        HitTarget::AudioStylePick(st) => {
            app.audio_config.style = st;
            app.push_audio_live_update(&format!("Audio style -> {}", st.label()));
        }
        HitTarget::AudioPalettePick(p) => {
            app.audio_config.palette = p;
            app.push_audio_live_update(&format!("Audio palette -> {}", p.label()));
        }
        HitTarget::AudioGainStep(d) => {
            app.audio_config.gain = (app.audio_config.gain + d).clamp(0.1, 10.0);
            app.push_audio_live_update(&format!("Gain -> {:.1}x", app.audio_config.gain));
        }
        HitTarget::AudioFpsStep(d) => {
            app.audio_config.fps = (i16::from(app.audio_config.fps) + d).clamp(5, 60) as u8;
            app.push_audio_live_update(&format!("FPS -> {}", app.audio_config.fps));
        }
        HitTarget::AudioBriStep(d) => {
            app.audio_config.brightness =
                (i16::from(app.audio_config.brightness) + d).clamp(1, 4) as u8;
            app.push_audio_live_update(&format!("Audio bri -> {}/4", app.audio_config.brightness));
        }
        HitTarget::AudioToggle => app.toggle_audio_sync(),
        HitTarget::DisclaimerCheckToggle => {
            app.audio_disclaimer_checked = !app.audio_disclaimer_checked;
        }
        HitTarget::DisclaimerConfirm => app.confirm_audio_disclaimer(),
        HitTarget::DisclaimerCancel => {
            app.audio_disclaimer_modal_open = false;
            app.audio_disclaimer_pending_start = false;
        }
        HitTarget::KmLayerView(v) => app.keymap_view = v,
        HitTarget::KmCategory(c) => app.set_keymap_draft_category(c),
        HitTarget::KmField(r) => {
            app.focus = FocusPane::KeymapsComposer;
            app.km_composer_row = r;
        }
        HitTarget::KmPresetFw(p) => app.save_arrow_cluster_to_keyboard(p),
        HitTarget::KmPresetDm(p) => {
            app.add_daemon_arrow_cluster(p);
        }
        HitTarget::KmRecordToggle => {
            app.shortcut_recording = !app.shortcut_recording;
            app.record_first_key = None;
        }
        HitTarget::KmWatcherToggle => {
            app.shortcut_watcher_active = !app.shortcut_watcher_active;
            let _ = app.sync_shortcuts();
        }
        HitTarget::KmModCycle(d) => views_keymaps::cycle_modifier(app, d),
        HitTarget::KmTriggerCycle(d) => views_keymaps::cycle_trigger_key(app, d),
        HitTarget::KmDepthStep(d) => {
            app.draft_rule_depth_mm = (app.draft_rule_depth_mm + d).clamp(0.10, 3.30);
        }
        HitTarget::KmPresetCycle(d) => views_keymaps::cycle_preset(app, d),
        HitTarget::KmVirtualCycle(d) => views_keymaps::cycle_virtual_key(app, d),
        HitTarget::KmShellPrompt => views_keymaps::open_shell_prompt(app),
        HitTarget::KmSave => {
            if app.draft_category == KeymapCategory::FirmwareFnFlash {
                app.save_draft_to_keyboard_fn();
            } else {
                app.commit_draft_daemon_rule(false);
            }
        }
        HitTarget::KmReplaceConflict => app.commit_draft_daemon_rule(true),
        HitTarget::KmLoadConflict(cid) => {
            if let Some(rule) = app.shortcut_rules.iter().find(|r| r.id == cid).cloned() {
                app.load_daemon_rule_into_draft(&rule);
            }
        }
        HitTarget::KmCancelEdit => {
            app.editing_daemon_rule_id = None;
            app.status_msg = "Cancelled in-place rule edit.".into();
        }
        HitTarget::KmResetFwAll => app.reset_keyboard_fn_map(),
        HitTarget::KmFwLoad(idx) => {
            if let Some(&hid) = app.keyboard_fn_map.get(&idx) {
                app.load_firmware_slot_into_draft(idx, hid);
            }
        }
        HitTarget::KmFwClear(idx) => app.clear_keyboard_fn_slot(idx),
        HitTarget::KmDmToggle(id) => {
            if let Some(r) = app.shortcut_rules.iter_mut().find(|r| r.id == id) {
                r.enabled = !r.enabled;
                let _ = app.sync_shortcuts();
            }
        }
        HitTarget::KmDmEdit(id) => {
            if let Some((pos, rule)) = app
                .shortcut_rules
                .iter()
                .enumerate()
                .find(|(_, r)| r.id == id)
                .map(|(i, r)| (i, r.clone()))
            {
                app.km_dm_cursor = pos;
                app.load_daemon_rule_into_draft(&rule);
            }
        }
        HitTarget::KmDmFire(id) => app.trigger_shortcut_rule(id),
        HitTarget::KmDmDel(id) => {
            app.shortcut_rules.retain(|r| r.id != id);
            if app.editing_daemon_rule_id == Some(id) {
                app.editing_daemon_rule_id = None;
            }
            let _ = app.sync_shortcuts();
        }
        HitTarget::DiagRun => app.run_diagnostics(),
        HitTarget::DiagEcho => app.run_echo_benchmark(),
        HitTarget::DiagSweepToggle => {
            app.diag_sweep_active = !app.diag_sweep_active;
            app.diag_sweep_started_at = app.diag_sweep_active.then(Instant::now);
        }
        HitTarget::DiagFollowToggle => app.diag_follow_press = !app.diag_follow_press,
        HitTarget::DiagClearPeaks => {
            app.switch_tests.clear();
            app.clear_stuck_travel();
        }
        HitTarget::DiagPickKey(idx) => {
            app.focus = FocusPane::DiagTestTable;
            app.focus_key_index(idx);
            if let Some(pos) = keymap::WIN60HE_KEYS.iter().position(|k| k.index == idx) {
                app.diag_table_cursor = pos;
            }
        }
        HitTarget::PromptSubmit => app.submit_prompt(),
        HitTarget::ModalClose => {
            app.show_help = false;
            app.prompt_modal = None;
        }
        HitTarget::MascotDismiss => {
            app.mascots.miku_visible = false;
            app.mascots.teto_visible = false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_lookup_prefers_topmost_child_over_parent() {
        let regions = vec![
            HitRegion {
                rect: Rect::new(0, 0, 40, 10),
                action: HitTarget::Focus(FocusPane::MainActuation),
            },
            HitRegion {
                rect: Rect::new(5, 2, 12, 1),
                action: HitTarget::ActApply,
            },
        ];
        assert!(matches!(find_hit(&regions, 7, 2), Some(HitTarget::ActApply)));
        assert!(matches!(
            find_hit(&regions, 2, 2),
            Some(HitTarget::Focus(FocusPane::MainActuation))
        ));
    }
}
