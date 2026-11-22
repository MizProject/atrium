//! Guarded Hall-effect sensor calibration view with 61-key live progress matrix.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Gauge, Paragraph, Wrap};

use atrium_core::keymap::WIN60HE_KEYS;

use crate::state::{App, FocusPane, HitRegion, HitTarget};
use crate::theme;
use crate::ui::keyboard::{MatrixMode, draw_keyboard_matrix};
use crate::ui::{ActionChip, render_chip_bar};

pub fn draw(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(11)])
        .split(area);

    let act_focused = app.focus == FocusPane::CalActions;
    let top_blk = theme::panel_block("Guarded Hall-Effect Sensor Calibration", act_focused);
    let top_inner = top_blk.inner(split[0]);
    f.render_widget(top_blk, split[0]);
    hits.push(HitRegion {
        rect: split[0],
        action: HitTarget::Focus(FocusPane::CalActions),
    });

    if top_inner.height >= 5 {
        let v = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Length(2),
                Constraint::Min(0),
            ])
            .split(top_inner);

        let info = Paragraph::new(vec![
            Line::from(Span::styled(
                "Safety rule: Keep ALL keys completely untouched before arming. Live travel pauses while armed.",
                Style::default().fg(theme::INK_WARN),
            )),
            Line::from(Span::styled(
                format!("Status: {}", app.cal_status_text),
                Style::default()
                    .fg(if app.cal_active { theme::CYAN_HIGHLIGHT } else { theme::INK_PRIMARY })
                    .add_modifier(Modifier::BOLD),
            )),
        ])
        .wrap(Wrap { trim: false });
        f.render_widget(info, v[0]);

        render_chip_bar(
            f,
            v[1],
            &[
                ActionChip::new("Arm All-Key (61)", app.cal_active && !app.cal_any_key, HitTarget::CalArmAll)
                    .focus_mark(act_focused && app.cal_action_cursor == 0),
                ActionChip::new("Arm Any-Key", app.cal_active && app.cal_any_key, HitTarget::CalArmAny)
                    .focus_mark(act_focused && app.cal_action_cursor == 1),
                ActionChip::new("Disarm / Finish", false, HitTarget::CalStop)
                    .focus_mark(act_focused && app.cal_action_cursor == 2),
            ],
            hits,
        );

        let total = WIN60HE_KEYS.len().max(1);
        let ratio = (app.cal_calibrated.len() as f64 / total as f64).clamp(0.0, 1.0);
        let gauge = Gauge::default()
            .block(theme::well_block("Calibration Progress", theme::BORDER_SUBTLE))
            .gauge_style(Style::default().fg(theme::INK_OK).bg(theme::SHADE_DEEP))
            .ratio(ratio)
            .label(format!("{}/{total} keys calibrated", app.cal_calibrated.len()));
        f.render_widget(gauge, v[2]);
    }

    draw_keyboard_matrix(
        f,
        split[1],
        app,
        MatrixMode::Calibration,
        app.focus == FocusPane::CalMap,
        "Calibration 61-Key Completion Map (CAL = Done)",
        hits,
    );
}

pub fn handle_key(app: &mut App, code: KeyCode) {
    match app.focus {
        FocusPane::CalActions => match code {
            KeyCode::Left | KeyCode::Char('h') => {
                app.cal_action_cursor = app.cal_action_cursor.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                app.cal_action_cursor = (app.cal_action_cursor + 1).min(2);
            }
            KeyCode::Enter | KeyCode::Char(' ') => match app.cal_action_cursor {
                0 => app.start_calibration(false),
                1 => app.start_calibration(true),
                _ => app.stop_calibration(),
            },
            KeyCode::Char('a') => app.start_calibration(false),
            KeyCode::Char('k') => app.start_calibration(true),
            KeyCode::Char('s') => app.stop_calibration(),
            _ => {}
        },
        FocusPane::CalMap => match code {
            KeyCode::Left | KeyCode::Char('h') => app.move_key_cursor(0, -1),
            KeyCode::Right | KeyCode::Char('l') => app.move_key_cursor(0, 1),
            KeyCode::Up | KeyCode::Char('k') => app.move_key_cursor(-1, 0),
            KeyCode::Down | KeyCode::Char('j') => app.move_key_cursor(1, 0),
            _ => {}
        },
        _ => {}
    }
}
