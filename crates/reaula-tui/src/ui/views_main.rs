//! Main workspace view: interactive 61-key matrix, Key Inspector, Actuation/RT editor,
//! Switch Profile & Deadband configurator, Polling Rate, PRCS/SOCD & Win-Lock controls.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use reaula_core::keymap;
use reaula_core::protocol::{self, PrcsPair, TriggerMode};

use crate::state::{App, FocusPane, HitRegion, HitTarget, PromptPurpose, TextPromptModal};
use crate::theme;
use crate::ui::keyboard::{MatrixMode, draw_keyboard_matrix};
use crate::ui::{ActionChip, render_chip_bar};

pub fn draw(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let narrow = area.width < 96 || area.height < 26;
    let kb_height = if narrow { 13 } else { 17 };

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(kb_height),
            Constraint::Length(2),
            Constraint::Min(10),
        ])
        .split(area);

    let title = format!(
        "WIN 60 HE Layout ({} selected | Space=Toggle, a=All, w=WASD, x=Single)",
        app.target_indices().len()
    );
    draw_keyboard_matrix(
        f,
        outer[0],
        app,
        MatrixMode::Main,
        app.focus == FocusPane::MainKeyboard,
        &title,
        hits,
    );

    // Selection + Inspector summary strip
    let cur = app.current_key();
    let cfg = app.actuations.get(&cur.index);
    let sw = app.switches.get(&cur.index).copied().unwrap_or(1);
    let db = app.deadbands.get(&cur.index).copied().unwrap_or((0, 0));
    let (live_idx, live_mm) = app.last_sample.unwrap_or((cur.index, 0.0));
    let live_name = keymap::by_index(live_idx).map_or("?", |k| k.name);

    let bar_split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(38), Constraint::Length(34)])
        .split(outer[1]);

    let inspect = Line::from(vec![
        Span::styled(
            format!(" {} (#{}) ", cur.name, cur.index),
            Style::default()
                .bg(theme::SHADE_REGAL)
                .fg(theme::INK_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            match cfg {
                Some(c) => format!(
                    "{} @ {:.2}mm | Sw:{} | DB:{:.2}/{:.2} | Live:{}:{:.2}mm",
                    c.mode.label(),
                    f64::from(c.travel_raw) * app.unit_mm,
                    protocol::switch_profile_name(sw),
                    f64::from(db.0) * 0.01,
                    f64::from(db.1) * 0.01,
                    live_name,
                    live_mm
                ),
                None => format!("Sw:{} | Live:{}:{:.2}mm", protocol::switch_profile_name(sw), live_name, live_mm),
            },
            Style::default().fg(theme::INK_SECONDARY),
        ),
    ]);
    f.render_widget(Paragraph::new(inspect).style(Style::default().bg(theme::SHADE_BASE)), bar_split[0]);

    render_chip_bar(
        f,
        bar_split[1],
        &[
            ActionChip::new("All (61)", false, HitTarget::SelectAll),
            ActionChip::new("WASD", false, HitTarget::SelectWasd),
            ActionChip::new("Focus", false, HitTarget::SelectFocusOnly),
        ],
        hits,
    );

    if narrow {
        let stack = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(outer[2]);
        draw_actuation_card(f, stack[0], app, hits);
        draw_switch_deadband_card(f, stack[1], app, hits);
        draw_socd_poll_card(f, stack[2], app, hits);
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(34),
                Constraint::Percentage(33),
                Constraint::Percentage(33),
            ])
            .split(outer[2]);
        draw_actuation_card(f, cols[0], app, hits);
        draw_switch_deadband_card(f, cols[1], app, hits);
        draw_socd_poll_card(f, cols[2], app, hits);
    }
}

fn draw_actuation_card(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::MainActuation;
    let blk = theme::panel_block("Actuation & Rapid Trigger", focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::MainActuation),
    });

    if inner.height < 6 || inner.width < 18 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    // Row 0: Trigger Mode
    render_chip_bar(
        f,
        rows[0],
        &[
            ActionChip::new(
                "Fixed",
                app.draft_mode == TriggerMode::Fixed,
                HitTarget::ActMode(TriggerMode::Fixed),
            )
            .focus_mark(focused && app.main_act_row == 0),
            ActionChip::new(
                "RT",
                app.draft_mode == TriggerMode::RapidTrigger,
                HitTarget::ActMode(TriggerMode::RapidTrigger),
            ),
            ActionChip::new(
                "RT Split",
                app.draft_mode == TriggerMode::RapidTriggerSplit,
                HitTarget::ActMode(TriggerMode::RapidTriggerSplit),
            ),
        ],
        hits,
    );

    // Row 1: Travel point
    render_stepper_row(
        f,
        rows[1],
        StepperRow {
            label: "Travel",
            value_text: &format!("{:.2} mm", app.draft_travel_mm),
            row_focused: focused && app.main_act_row == 1,
            row_target: HitTarget::ActField(1),
            dec_target: HitTarget::ActTravelStep(-0.05),
            inc_target: HitTarget::ActTravelStep(0.05),
        },
        hits,
    );

    // Row 2: RT Press
    render_stepper_row(
        f,
        rows[2],
        StepperRow {
            label: "RT Press",
            value_text: &format!("{:.2} mm", app.draft_rt_press_mm),
            row_focused: focused && app.main_act_row == 2,
            row_target: HitTarget::ActField(2),
            dec_target: HitTarget::ActRtPressStep(-0.05),
            inc_target: HitTarget::ActRtPressStep(0.05),
        },
        hits,
    );

    // Row 3: RT Release
    render_stepper_row(
        f,
        rows[3],
        StepperRow {
            label: "RT Rel",
            value_text: &format!("{:.2} mm", app.draft_rt_release_mm),
            row_focused: focused && app.main_act_row == 3,
            row_target: HitTarget::ActField(3),
            dec_target: HitTarget::ActRtRelStep(-0.05),
            inc_target: HitTarget::ActRtRelStep(0.05),
        },
        hits,
    );

    // Row 4: Apply
    render_chip_bar(
        f,
        rows[4],
        &[
            ActionChip::new(
                format!("Apply Actuation ({} key)", app.target_indices().len()),
                focused && app.main_act_row == 4,
                HitTarget::ActApply,
            )
            .focus_mark(focused && app.main_act_row == 4),
        ],
        hits,
    );
}

fn draw_switch_deadband_card(
    f: &mut Frame<'_>,
    area: Rect,
    app: &App,
    hits: &mut Vec<HitRegion>,
) {
    let focused = app.focus == FocusPane::MainSwitchDeadband;
    let blk = theme::panel_block("Switch Profile & Deadband", focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::MainSwitchDeadband),
    });

    if inner.height < 6 || inner.width < 18 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    // Row 0: Switch choices
    render_chip_bar(
        f,
        rows[0],
        &[
            ActionChip::new("HM1", app.draft_switch_id == 1, HitTarget::SwPick(1))
                .focus_mark(focused && app.main_sw_row == 0),
            ActionChip::new("HH1", app.draft_switch_id == 2, HitTarget::SwPick(2)),
            ActionChip::new("CY1", app.draft_switch_id == 3, HitTarget::SwPick(3)),
            ActionChip::new("TC1", app.draft_switch_id == 5, HitTarget::SwPick(5)),
        ],
        hits,
    );

    // Row 1: Apply switch
    render_chip_bar(
        f,
        rows[1],
        &[ActionChip::new(
            format!("Apply Switch ({})", protocol::switch_profile_name(app.draft_switch_id)),
            focused && app.main_sw_row == 1,
            HitTarget::SwApply,
        )
        .focus_mark(focused && app.main_sw_row == 1)],
        hits,
    );

    // Row 2: Top deadband
    render_stepper_row(
        f,
        rows[2],
        StepperRow {
            label: "Top DB",
            value_text: &format!("{:.2} mm", app.draft_db_top_mm),
            row_focused: focused && app.main_sw_row == 2,
            row_target: HitTarget::DbField(2),
            dec_target: HitTarget::DbTopStep(-0.02),
            inc_target: HitTarget::DbTopStep(0.02),
        },
        hits,
    );

    // Row 3: Bottom deadband
    render_stepper_row(
        f,
        rows[3],
        StepperRow {
            label: "Bot DB",
            value_text: &format!("{:.2} mm", app.draft_db_bottom_mm),
            row_focused: focused && app.main_sw_row == 3,
            row_target: HitTarget::DbField(3),
            dec_target: HitTarget::DbBottomStep(-0.02),
            inc_target: HitTarget::DbBottomStep(0.02),
        },
        hits,
    );

    // Row 4: Apply deadband
    render_chip_bar(
        f,
        rows[4],
        &[ActionChip::new(
            "Apply Deadband",
            focused && app.main_sw_row == 4,
            HitTarget::DbApply,
        )
        .focus_mark(focused && app.main_sw_row == 4)],
        hits,
    );
}

fn draw_socd_poll_card(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::MainSocdPoll;
    let blk = theme::panel_block("Poll, SOCD & Win-Lock", focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::MainSocdPoll),
    });

    if inner.height < 6 || inner.width < 18 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    // Row 0: USB Polling rate
    render_chip_bar(
        f,
        rows[0],
        &[
            ActionChip::new("1k", app.poll_rate_khz == 1, HitTarget::PollPick(1))
                .focus_mark(focused && app.main_socd_row == 0),
            ActionChip::new("2k", app.poll_rate_khz == 2, HitTarget::PollPick(2)),
            ActionChip::new("4k", app.poll_rate_khz == 4, HitTarget::PollPick(4)),
            ActionChip::new("8k", app.poll_rate_khz == 8, HitTarget::PollPick(8)),
        ],
        hits,
    );

    // Row 1: SOCD toggle + quick presets
    render_chip_bar(
        f,
        rows[1],
        &[
            ActionChip::new(
                if app.socd_enabled { "SOCD:ON" } else { "SOCD:OFF" },
                app.socd_enabled,
                HitTarget::SocdToggle,
            )
            .focus_mark(focused && app.main_socd_row == 1),
            ActionChip::new("Clr", app.socd_pairs.is_empty(), HitTarget::SocdPresetClear),
            ActionChip::new("A+D", false, HitTarget::SocdPresetAd),
            ActionChip::new("AD+WS", false, HitTarget::SocdPresetAdWs),
            ActionChip::new("Pair..", false, HitTarget::SocdPromptPair),
        ],
        hits,
    );

    // Row 2: Active pairs summary + Apply SOCD
    let pair_desc = if app.socd_pairs.is_empty() {
        "SOCD: none".into()
    } else {
        app.socd_pairs
            .iter()
            .take(2)
            .map(|p| {
                let n1 = reaula_core::keymap::WIN60HE_KEYS
                    .iter()
                    .find(|k| k.hid_code == p.key1_hid)
                    .map_or("?", |k| k.name);
                let n2 = reaula_core::keymap::WIN60HE_KEYS
                    .iter()
                    .find(|k| k.hid_code == p.key2_hid)
                    .map_or("?", |k| k.name);
                format!("{n1}<->{n2}")
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    render_chip_bar(
        f,
        rows[2],
        &[ActionChip::new(
            format!("Apply SOCD ({pair_desc})"),
            focused && app.main_socd_row == 2,
            HitTarget::SocdApply,
        )
        .focus_mark(focused && app.main_socd_row == 2)],
        hits,
    );

    // Row 3: Win-Lock flags
    render_chip_bar(
        f,
        rows[3],
        &[
            ActionChip::new("Win", app.win_lock.disable_win, HitTarget::WinLockFlag(0))
                .focus_mark(focused && app.main_socd_row == 3),
            ActionChip::new("S+Tab", app.win_lock.disable_shift_tab, HitTarget::WinLockFlag(1)),
            ActionChip::new("A+Tab", app.win_lock.disable_alt_tab, HitTarget::WinLockFlag(2)),
            ActionChip::new("A+F4", app.win_lock.disable_alt_f4, HitTarget::WinLockFlag(3)),
            ActionChip::new("Lock", app.win_lock.win_lock_on, HitTarget::WinLockFlag(4)),
        ],
        hits,
    );

    // Row 4: Apply Win-Lock
    render_chip_bar(
        f,
        rows[4],
        &[ActionChip::new(
            "Apply Win-Lock Flags",
            focused && app.main_socd_row == 4,
            HitTarget::WinLockApply,
        )
        .focus_mark(focused && app.main_socd_row == 4)],
        hits,
    );
}

pub struct StepperRow<'a> {
    pub label: &'a str,
    pub value_text: &'a str,
    pub row_focused: bool,
    pub row_target: HitTarget,
    pub dec_target: HitTarget,
    pub inc_target: HitTarget,
}

pub fn render_stepper_row(
    f: &mut Frame<'_>,
    area: Rect,
    spec: StepperRow<'_>,
    hits: &mut Vec<HitRegion>,
) {
    if area.width < 12 {
        return;
    }
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(10), Constraint::Length(19)])
        .split(area);

    let lbl_style = if spec.row_focused {
        Style::default()
            .fg(theme::CYAN_HIGHLIGHT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::INK_SECONDARY)
    };
    let prefix = if spec.row_focused { "▶ " } else { "  " };
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("{prefix}{}: {}", spec.label, spec.value_text),
            lbl_style,
        )])),
        split[0],
    );
    hits.push(HitRegion {
        rect: split[0],
        action: spec.row_target,
    });

    render_chip_bar(
        f,
        split[1],
        &[
            ActionChip::new(" - ", false, spec.dec_target),
            ActionChip::new(" + ", false, spec.inc_target),
        ],
        hits,
    );
}

pub fn handle_key(app: &mut App, code: KeyCode) {
    match app.focus {
        FocusPane::MainKeyboard => match code {
            KeyCode::Left | KeyCode::Char('h') => app.move_key_cursor(0, -1),
            KeyCode::Right | KeyCode::Char('l') => app.move_key_cursor(0, 1),
            KeyCode::Up | KeyCode::Char('k') => app.move_key_cursor(-1, 0),
            KeyCode::Down | KeyCode::Char('j') => app.move_key_cursor(1, 0),
            KeyCode::Char(' ') => {
                let idx = app.current_key().index;
                if !app.selected_indices.remove(&idx) {
                    app.selected_indices.insert(idx);
                }
            }
            KeyCode::Char('a') => {
                app.selected_indices = keymap::all_indices().into_iter().collect();
                app.status_msg = "Selected all 61 keys.".into();
            }
            KeyCode::Char('w') => {
                app.selected_indices = [46, 68, 69, 70].into_iter().collect();
                app.status_msg = "Selected WASD.".into();
            }
            KeyCode::Char('x') => {
                app.selected_indices.clear();
                app.selected_indices.insert(app.current_key().index);
            }
            KeyCode::Enter => app.focus = FocusPane::MainActuation,
            _ => {}
        },
        FocusPane::MainActuation => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                app.main_act_row = app.main_act_row.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.main_act_row = (app.main_act_row + 1).min(4);
            }
            KeyCode::Left | KeyCode::Char('h') => adjust_act_row(app, -1),
            KeyCode::Right | KeyCode::Char('l') => adjust_act_row(app, 1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if app.main_act_row == 0 {
                    adjust_act_row(app, 1);
                } else {
                    app.apply_actuation();
                }
            }
            KeyCode::Char('m') => adjust_act_mode(app, 1),
            _ => {}
        },
        FocusPane::MainSwitchDeadband => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                app.main_sw_row = app.main_sw_row.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.main_sw_row = (app.main_sw_row + 1).min(4);
            }
            KeyCode::Left | KeyCode::Char('h') => adjust_sw_row(app, -1),
            KeyCode::Right | KeyCode::Char('l') => adjust_sw_row(app, 1),
            KeyCode::Enter | KeyCode::Char(' ') => match app.main_sw_row {
                0 => adjust_sw_row(app, 1),
                1 => app.apply_switch(),
                2 | 3 => adjust_sw_row(app, 1),
                _ => app.apply_deadband(),
            },
            _ => {}
        },
        FocusPane::MainSocdPoll => match code {
            KeyCode::Up | KeyCode::Char('k') => {
                app.main_socd_row = app.main_socd_row.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.main_socd_row = (app.main_socd_row + 1).min(4);
            }
            KeyCode::Left | KeyCode::Char('h') => adjust_socd_row(app, -1),
            KeyCode::Right | KeyCode::Char('l') => adjust_socd_row(app, 1),
            KeyCode::Enter | KeyCode::Char(' ') => match app.main_socd_row {
                0 => adjust_socd_row(app, 1),
                1 => {
                    app.socd_enabled = !app.socd_enabled;
                }
                2 => app.apply_socd(),
                3 => app.win_lock.disable_win = !app.win_lock.disable_win,
                _ => app.apply_win_lock(),
            },
            KeyCode::Char('e') => {
                app.prompt_modal = Some(TextPromptModal::new(
                    PromptPurpose::SocdCustomPair,
                    "Custom PRCS / SOCD Pairs",
                    "Format: KEY1:KEY2:MODEL (e.g. A:D:2, W:S:1 · blank clears)",
                    "A:D:2",
                ));
            }
            _ => {}
        },
        _ => {}
    }
}

fn adjust_act_mode(app: &mut App, dir: isize) {
    let modes = [
        TriggerMode::Fixed,
        TriggerMode::RapidTrigger,
        TriggerMode::RapidTriggerSplit,
    ];
    let cur = modes.iter().position(|&m| m == app.draft_mode).unwrap_or(0) as isize;
    let next = (cur + dir).rem_euclid(modes.len() as isize) as usize;
    app.draft_mode = modes[next];
}

fn adjust_act_row(app: &mut App, dir: isize) {
    let step = if dir >= 0 { 0.05 } else { -0.05 };
    match app.main_act_row {
        0 => adjust_act_mode(app, dir),
        1 => app.draft_travel_mm = (app.draft_travel_mm + step).clamp(app.min_mm, app.max_mm),
        2 => app.draft_rt_press_mm = (app.draft_rt_press_mm + step).clamp(0.05, 2.00),
        3 => app.draft_rt_release_mm = (app.draft_rt_release_mm + step).clamp(0.05, 2.00),
        _ => {}
    }
}

fn adjust_sw_row(app: &mut App, dir: isize) {
    let step = if dir >= 0 { 0.02 } else { -0.02 };
    match app.main_sw_row {
        0 => {
            const SW: [u8; 4] = [1, 2, 3, 5];
            let pos = SW.iter().position(|&s| s == app.draft_switch_id).unwrap_or(0) as isize;
            let next = (pos + dir).rem_euclid(SW.len() as isize) as usize;
            app.draft_switch_id = SW[next];
        }
        2 => app.draft_db_top_mm = (app.draft_db_top_mm + step).clamp(0.0, 1.0),
        3 => app.draft_db_bottom_mm = (app.draft_db_bottom_mm + step).clamp(0.0, 1.0),
        _ => {}
    }
}

fn adjust_socd_row(app: &mut App, dir: isize) {
    match app.main_socd_row {
        0 => {
            const RATES: [u8; 4] = [1, 2, 4, 8];
            let pos = RATES.iter().position(|&r| r == app.poll_rate_khz).unwrap_or(3) as isize;
            let next = (pos + dir).rem_euclid(RATES.len() as isize) as usize;
            app.apply_poll_rate(RATES[next]);
        }
        1 => {
            if dir >= 0 {
                app.socd_enabled = true;
                if app.socd_pairs.is_empty() {
                    app.socd_pairs = vec![PrcsPair {
                        key1_hid: 0x04,
                        key2_hid: 0x07,
                        model: 1,
                    }];
                }
            } else {
                app.socd_enabled = false;
            }
        }
        3 => app.win_lock.win_lock_on = dir >= 0,
        _ => {}
    }
}
