//! Diagnostics workspace: 4-Layer Fault Isolation, 5x HID Echo RTT,
//! Hall-Effect Switch Mechanism Cutaway visualization, All-61 Switch Return Table,
//! and scrollable RPC Feedback Log.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, List, ListItem, Paragraph, Wrap};

use atrium_core::diagnostics::CheckStatus;
use atrium_core::keymap::WIN60HE_KEYS;

use crate::state::{App, FocusPane, HitRegion, HitTarget};
use crate::theme;
use crate::ui::{ActionChip, render_chip_bar};

pub fn draw(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(12)])
        .split(area);

    // Top Action Bar
    let act_focused = app.focus == FocusPane::DiagActions;
    let act_blk = theme::panel_block("4-Layer Fault Isolation & Test Controls", act_focused);
    let act_inner = act_blk.inner(v[0]);
    f.render_widget(act_blk, v[0]);
    hits.push(HitRegion {
        rect: v[0],
        action: HitTarget::Focus(FocusPane::DiagActions),
    });

    if act_inner.height >= 1 {
        render_chip_bar(
            f,
            act_inner,
            &[
                ActionChip::new("Run 4-Layer Diag (D)", false, HitTarget::DiagRun)
                    .focus_mark(act_focused && app.diag_action_cursor == 0),
                ActionChip::new("5x Echo RTT (E)", false, HitTarget::DiagEcho)
                    .focus_mark(act_focused && app.diag_action_cursor == 1),
                ActionChip::new(
                    if app.diag_sweep_active { "Sweep:ON" } else { "Sweep:OFF" },
                    app.diag_sweep_active,
                    HitTarget::DiagSweepToggle,
                )
                .focus_mark(act_focused && app.diag_action_cursor == 2),
                ActionChip::new(
                    if app.diag_follow_press { "Follow:ON" } else { "Follow:OFF" },
                    app.diag_follow_press,
                    HitTarget::DiagFollowToggle,
                )
                .focus_mark(act_focused && app.diag_action_cursor == 3),
                ActionChip::new("Zero Peaks", false, HitTarget::DiagClearPeaks)
                    .focus_mark(act_focused && app.diag_action_cursor == 4),
            ],
            hits,
        );
    }

    let main_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(v[1]);

    // Left Column: Switch Mechanism Cutaway (top) + 4-Layer Diag / Feedback Log (bottom)
    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(14), Constraint::Min(6)])
        .split(main_cols[0]);

    draw_switch_cutaway(f, left_rows[0], app, hits);
    draw_diag_and_log(f, left_rows[1], app, hits);

    // Right Column: 61-Key Switch Response & Return Table
    draw_switch_table(f, main_cols[1], app, hits);
}

fn draw_switch_cutaway(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::DiagSwitchInspector;
    let snap = app.focused_switch_inspection();
    let title = format!(
        "Switch Cutaway: {} (#{}) [{}]{}",
        snap.key_name,
        snap.key_index,
        snap.switch_label,
        if snap.sweeping { " [SWEEP]" } else { "" }
    );
    let blk = theme::panel_block(title, focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::DiagSwitchInspector),
    });

    if inner.height < 6 || inner.width < 28 {
        return;
    }

    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(19), Constraint::Min(12)])
        .split(inner);

    // ASCII/Unicode animated cross-section showing plunger + spring + magnet + Hall IC
    let max_plunge = 4usize;
    let frac = (snap.travel_mm / snap.max_mm.max(0.5)).clamp(0.0, 1.0);
    let plunge = (frac * max_plunge as f64).round() as usize;

    let cap_color = if snap.triggered {
        theme::DM_AQUA_BORDER
    } else {
        theme::INK_ACCENT
    };
    let mut schematic = Vec::new();
    for _ in 0..plunge {
        schematic.push(Line::from("                 "));
    }
    schematic.push(Line::from(Span::styled(
        "  ╔═══════════╗  ",
        Style::default().fg(cap_color).add_modifier(Modifier::BOLD),
    )));
    schematic.push(Line::from(Span::styled(
        format!("  ║ {:^9} ║  ", snap.key_name),
        Style::default().fg(cap_color).add_modifier(Modifier::BOLD),
    )));
    schematic.push(Line::from(Span::styled(
        "  ╚═════╦═════╝  ",
        Style::default().fg(theme::INK_SECONDARY),
    )));
    let stem_rows = (5usize).saturating_sub(plunge).max(1);
    for r in 0..stem_rows {
        let coil = if r % 2 == 0 { "│╱╲│" } else { "│╲╱│" };
        schematic.push(Line::from(Span::styled(
            format!("      {coil}      "),
            Style::default().fg(theme::INK_MUTED),
        )));
    }
    schematic.push(Line::from(Span::styled(
        "     [NdFeB]     ",
        Style::default()
            .fg(if snap.triggered {
                theme::CYAN_HIGHLIGHT
            } else {
                theme::FW_AMBER_INK
            })
            .add_modifier(Modifier::BOLD),
    )));
    schematic.push(Line::from(Span::styled(
        "  ───[HALL IC]── ",
        Style::default().fg(theme::INK_OK),
    )));
    f.render_widget(Paragraph::new(schematic), split[0]);

    // Telemetry + Travel & Flux gauges on right side of cutaway
    let v_right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(3),
        ])
        .split(split[1]);

    let travel_gauge = Gauge::default()
        .block(theme::well_block("Travel", theme::BORDER_SUBTLE))
        .gauge_style(Style::default().fg(if snap.triggered {
            theme::DM_AQUA_BORDER
        } else {
            theme::INK_OK
        }))
        .ratio(frac)
        .label(format!("{:.2} / {:.2} mm", snap.travel_mm, snap.max_mm));
    f.render_widget(travel_gauge, v_right[0]);

    let flux_gauge = Gauge::default()
        .block(theme::well_block("Hall Flux", theme::BORDER_SUBTLE))
        .gauge_style(Style::default().fg(theme::FW_AMBER_INK))
        .ratio((snap.flux_pct / 100.0).clamp(0.0, 1.0))
        .label(format!("{:.0}%", snap.flux_pct));
    f.render_widget(flux_gauge, v_right[1]);

    let stats = vec![
        Line::from(format!(
            "State  : {}",
            if snap.triggered { "TRIGGERED" } else { "ARMS / IDLE" }
        )),
        Line::from(format!(
            "Actuate: {:.2} mm ({})",
            snap.actuation_mm,
            if snap.rapid_trigger {
                format!("RT {:.2}/{:.2}", snap.rt_press_mm, snap.rt_release_mm)
            } else {
                "Fixed".into()
            }
        )),
        Line::from(format!(
            "Peak   : {:.2} mm | Presses: {}",
            snap.peak_mm, snap.presses
        )),
        Line::from(format!(
            "DB     : {:.2} / {:.2} mm | Ret0: {}",
            snap.db_top_mm,
            snap.db_bottom_mm,
            if snap.released_zero { "OK" } else { "-" }
        )),
    ];
    f.render_widget(Paragraph::new(stats).wrap(Wrap { trim: false }), v_right[2]);
}

fn draw_diag_and_log(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::DiagFeedbackLog;
    let blk = theme::panel_block("4-Layer Report & HID/RPC Feedback Log", focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::DiagFeedbackLog),
    });

    if inner.height < 3 {
        return;
    }

    let mut lines: Vec<Line<'_>> = Vec::new();
    if let Some(rep) = &app.diag_report {
        lines.push(Line::from(Span::styled(
            format!("Attribution: {}", rep.attribution.summary),
            Style::default()
                .fg(if rep.attribution.primary_fault.is_some() {
                    theme::INK_WARN
                } else {
                    theme::INK_OK
                })
                .add_modifier(Modifier::BOLD),
        )));
        for c in &rep.checks {
            let (tag, col) = match c.status {
                CheckStatus::Pass => ("PASS", theme::INK_OK),
                CheckStatus::Warn => ("WARN", theme::INK_WARN),
                CheckStatus::Fail => ("FAIL", theme::INK_DANGER),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("[{tag}] "), Style::default().fg(col).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{:?} · {}: {}", c.layer, c.name, c.detail),
                    Style::default().fg(theme::INK_SECONDARY),
                ),
            ]));
        }
    } else {
        lines.push(Line::from(Span::styled(
            "Press 'D' or click 'Run 4-Layer Diag' for Kernel/Daemon/KB/SW audit.",
            Style::default().fg(theme::INK_MUTED),
        )));
    }

    if !app.echo_latencies_ms.is_empty() {
        let min = app.echo_latencies_ms.iter().copied().fold(f64::INFINITY, f64::min);
        let max = app.echo_latencies_ms.iter().copied().fold(0.0f64, f64::max);
        let avg = app.echo_latencies_ms.iter().sum::<f64>() / app.echo_latencies_ms.len() as f64;
        lines.push(Line::from(Span::styled(
            format!("Echo RTT (5x): min={min:.2} avg={avg:.2} max={max:.2} ms"),
            Style::default().fg(theme::CYAN_HIGHLIGHT),
        )));
    }

    for ev in &app.feedback_log {
        let col = if ev.ok { theme::INK_OK } else { theme::INK_DANGER };
        lines.push(Line::from(vec![
            Span::styled(
                format!("#{:<2} [{} {:>4.1}ms] ", ev.id, if ev.ok { "OK" } else { "ER" }, ev.rtt_ms),
                Style::default().fg(col),
            ),
            Span::styled(
                format!("{}: {}", ev.label, ev.detail),
                Style::default().fg(theme::INK_SECONDARY),
            ),
        ]));
    }

    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.diag_log_scroll, 0)),
        inner,
    );
}

fn draw_switch_table(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::DiagTestTable;
    let blk = theme::panel_block(
        format!(
            "All-61 Key Depth & Return Check (Tested: {}/{})",
            app.switch_tests.len(),
            WIN60HE_KEYS.len()
        ),
        focused,
    );
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::DiagTestTable),
    });

    if inner.height < 3 {
        return;
    }

    let visible_rows = usize::from(inner.height);
    let total = WIN60HE_KEYS.len();
    let offset = app
        .diag_table_cursor
        .saturating_sub(visible_rows / 2)
        .min(total.saturating_sub(visible_rows));

    let items: Vec<ListItem> = WIN60HE_KEYS
        .iter()
        .enumerate()
        .skip(offset)
        .take(visible_rows)
        .map(|(row_i, key)| {
            let live_now = app.live_depths.get(&key.index).map(|(d, _)| *d).unwrap_or(0.0);
            let stat = app.switch_tests.get(&key.index);
            let peak = stat.map_or(live_now, |s| s.peak_mm.max(live_now));
            let presses = stat.map_or(0, |s| s.presses);
            let (badge, col) = if live_now > 0.10 {
                ("HELD", theme::CYAN_HIGHLIGHT)
            } else if stat.is_some_and(|s| s.released_zero && s.presses > 0) {
                ("0.00 RET OK", theme::INK_OK)
            } else if stat.is_some_and(|s| s.presses > 0) {
                ("CHK RETURN", theme::INK_WARN)
            } else {
                ("UNTESTED", theme::INK_MUTED)
            };

            let is_sel = row_i == app.diag_table_cursor || key.index == app.current_key().index;
            let mark = if row_i == app.diag_table_cursor { "▶" } else { " " };
            let line = format!(
                "{mark} {:<7} #{:<3} now {:>4.2} peak {:>4.2} p:{:<2} {}",
                key.name, key.index, live_now, peak, presses, badge
            );
            let mut st = Style::default().fg(if is_sel { theme::INK_PRIMARY } else { col });
            if is_sel {
                st = st.bg(theme::SHADE_REGAL).add_modifier(Modifier::BOLD);
            }
            ListItem::new(line).style(st)
        })
        .collect();

    f.render_widget(List::new(items), inner);

    for (vis_idx, key) in WIN60HE_KEYS.iter().enumerate().skip(offset).take(visible_rows) {
        let y = inner.y + (vis_idx - offset) as u16;
        if y < inner.y + inner.height {
            hits.push(HitRegion {
                rect: Rect::new(inner.x, y, inner.width, 1),
                action: HitTarget::DiagPickKey(key.index),
            });
        }
    }
}

pub fn handle_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('d') | KeyCode::Char('D') => {
            app.run_diagnostics();
            return;
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            app.run_echo_benchmark();
            return;
        }
        _ => {}
    }

    match app.focus {
        FocusPane::DiagActions => match code {
            KeyCode::Left | KeyCode::Char('h') => {
                app.diag_action_cursor = app.diag_action_cursor.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                app.diag_action_cursor = (app.diag_action_cursor + 1).min(4);
            }
            KeyCode::Enter | KeyCode::Char(' ') => match app.diag_action_cursor {
                0 => app.run_diagnostics(),
                1 => app.run_echo_benchmark(),
                2 => {
                    app.diag_sweep_active = !app.diag_sweep_active;
                    app.diag_sweep_started_at =
                        app.diag_sweep_active.then(std::time::Instant::now);
                }
                3 => app.diag_follow_press = !app.diag_follow_press,
                _ => {
                    app.switch_tests.clear();
                    app.clear_stuck_travel();
                }
            },
            _ => {}
        },
        FocusPane::DiagSwitchInspector => match code {
            KeyCode::Left | KeyCode::Char('h') => app.move_key_cursor(0, -1),
            KeyCode::Right | KeyCode::Char('l') => app.move_key_cursor(0, 1),
            KeyCode::Up | KeyCode::Char('k') => app.move_key_cursor(-1, 0),
            KeyCode::Down | KeyCode::Char('j') => app.move_key_cursor(1, 0),
            KeyCode::Char(' ') => {
                app.diag_sweep_active = !app.diag_sweep_active;
                app.diag_sweep_started_at = app.diag_sweep_active.then(std::time::Instant::now);
            }
            _ => {}
        },
        FocusPane::DiagTestTable => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                app.diag_table_cursor = app.diag_table_cursor.saturating_sub(1);
                if let Some(k) = WIN60HE_KEYS.get(app.diag_table_cursor) {
                    app.focus_key_index(k.index);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.diag_table_cursor + 1 < WIN60HE_KEYS.len() {
                    app.diag_table_cursor += 1;
                    if let Some(k) = WIN60HE_KEYS.get(app.diag_table_cursor) {
                        app.focus_key_index(k.index);
                    }
                }
            }
            KeyCode::PageUp => {
                app.diag_table_cursor = app.diag_table_cursor.saturating_sub(10);
                if let Some(k) = WIN60HE_KEYS.get(app.diag_table_cursor) {
                    app.focus_key_index(k.index);
                }
            }
            KeyCode::PageDown => {
                app.diag_table_cursor = (app.diag_table_cursor + 10).min(WIN60HE_KEYS.len() - 1);
                if let Some(k) = WIN60HE_KEYS.get(app.diag_table_cursor) {
                    app.focus_key_index(k.index);
                }
            }
            _ => {}
        },
        FocusPane::DiagFeedbackLog => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                app.diag_log_scroll = app.diag_log_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.diag_log_scroll = app.diag_log_scroll.saturating_add(1);
            }
            _ => {}
        },
        _ => {}
    }
}
