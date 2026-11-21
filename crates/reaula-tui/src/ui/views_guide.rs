//! WIN 60 HE Interactive Guide & Hardware Reference workspace.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::state::{App, FocusPane, HitRegion, HitTarget};
use crate::theme;
use crate::ui::{ActionChip, render_chip_bar};

pub fn draw(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::GuideBody;
    let blk = theme::panel_block("WIN 60 HE Hardware Reference & Dual-Layer Guide", focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    if inner.height < 4 {
        return;
    }

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(inner);

    let sections: [(&str, &[&str]); 5] = [
        (
            "1. Hall-Effect Actuation, Rapid Trigger & Deadband",
            &[
                "• Actuation Point (0.08..=3.40 mm, 0.010 mm steps): Controls exact magnet travel depth where Fixed triggers.",
                "• Rapid Trigger (RT / RT Split): Dynamically resets & re-triggers anywhere along travel instead of fixed depth.",
                "• Top & Bottom Deadband: Adds safe travel hysteresis near rest (0.00 mm) and bottom-out to avoid double taps.",
                "• Switch Profiles (HM1/HH1/CY1/TC1): Selects factory magnetic flux curves per key via safe Read-Modify-Write.",
            ],
        ),
        (
            "2. Firmware [FW] vs Daemon [DM] Keymap Architecture",
            &[
                "• [FW] Firmware Flash (cmd 0x18, Amber #F59E0B): Stored inside keyboard MCU flash; persists with zero host software.",
                "• [DM] Daemon Memory (reaulad, Aqua #00D7FF): Triggered at exact Hall-effect mm depth via EVIOCGRAB + /dev/uinput.",
                "• Fn (#122) is strictly reserved for Firmware [FW] so Daemon chords never collide (hold Caps/Ctrl/Alt/Menu instead).",
                "• Commands configured on [DM] RunCommand drop privileges cleanly to your non-root session via systemd-run --user.",
            ],
        ),
        (
            "3. Daemon Audio Equalizer & Safety Notice",
            &[
                "• reaulad hosts a privilege-dropped --capture-pcm child helper feeding 14-band FFT/beat metrics to cmd 0x09.",
                "• Closing reaula-tui or reaula-gui keeps the Audio EQ active in the background until stopped.",
                "• Continuous per-key RGB streaming is experimental; confirm the 10-second liability modal before starting.",
            ],
        ),
        (
            "4. Built-In Hardware Shortcuts",
            &[
                "• Fn + \\|        Cycle onboard RGB lighting modes",
                "• Fn + Space     Hold ~3s to factory-reset hardware (use Diagnostics instead of raw resets)",
                "• Fn + Win       Toggle Win-Key lock on hardware",
                "• Ctrl+C / q     Cleanly restore terminal and exit reaula-tui (keyboard stays untouched)",
            ],
        ),
        (
            "5. Daemon & Systemd Management",
            &[
                "• Standalone : `reaula daemon start` / `reaula daemon status` / `reaula daemon stop`",
                "• Systemd    : `reaula daemon systemd install --enable --start` -> `reaulad@<user>.service` (Type=notify + WatchdogSec=30s)",
                "• User Unit  : `reaula daemon systemd install --user --enable --start` (when udev rule grants hidraw)",
            ],
        ),
    ];

    let mut lines = Vec::new();
    for (title, items) in sections {
        lines.push(Line::from(Span::styled(
            title,
            Style::default()
                .fg(theme::CYAN_HIGHLIGHT)
                .add_modifier(Modifier::BOLD),
        )));
        for item in items {
            lines.push(Line::from(Span::styled(
                *item,
                Style::default().fg(theme::INK_SECONDARY),
            )));
        }
        lines.push(Line::from(""));
    }

    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((app.guide_scroll, 0)),
        split[0],
    );

    render_chip_bar(
        f,
        split[1],
        &[
            ActionChip::new("Scroll Up (k)", false, HitTarget::GuideScroll(-3)),
            ActionChip::new("Scroll Down (j)", false, HitTarget::GuideScroll(3)),
            ActionChip::new("Top (g)", false, HitTarget::GuideScroll(-1000)),
        ],
        hits,
    );
}

pub fn handle_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Up | KeyCode::Char('k') => app.guide_scroll = app.guide_scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => app.guide_scroll = app.guide_scroll.saturating_add(1),
        KeyCode::PageUp => app.guide_scroll = app.guide_scroll.saturating_sub(8),
        KeyCode::PageDown => app.guide_scroll = app.guide_scroll.saturating_add(8),
        KeyCode::Char('g') | KeyCode::Home => app.guide_scroll = 0,
        _ => {}
    }
}
