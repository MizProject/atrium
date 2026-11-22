// Copyright (C) 2026 MizProject Philippines
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `atrium-tui` (experimental): Ratatui terminal control panel for Aula WIN 60 HE.
//! Notice: the TUI is not yet properly developed and still has known bugs.

mod state;
mod theme;
mod ui;

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use atrium_core::legal;

use clap::Parser;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::state::App;

#[derive(Parser)]
#[command(
    name = "atrium-tui",
    version,
    about = "Experimental Ratatui TUI for Aula WIN 60 HE (incomplete, still has known bugs)",
    after_help = legal::CLI_LEGAL_AFTER_HELP
)]
struct Args {
    /// Connect to a specific atriumd Unix domain socket
    #[arg(long)]
    socket: Option<PathBuf>,
}

/// Panic-safe RAII guard ensuring terminal raw mode, mouse tracking, and alternate screen
/// are always restored on exit or unwind.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|e| format!("failed to enable raw terminal: {e}"))?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )
        .map_err(|e| {
            let _ = disable_raw_mode();
            e.to_string()
        })?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
    }
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--about") {
        println!(
            "{}",
            legal::format_about_banner("atrium-tui", env!("CARGO_PKG_VERSION"))
        );
        return ExitCode::SUCCESS;
    }
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("atrium-tui error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let mut app = App::connect(args.socket)?;

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        prev_hook(info);
    }));

    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;
    let _ = terminal.clear();

    let loop_res = run_event_loop(&mut terminal, &mut app);
    let _ = terminal.show_cursor();
    drop(_guard);
    loop_res
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), String> {
    loop {
        app.drain_travel_samples();
        app.drain_calibration();
        app.poll_audio_sync();
        app.poll_shortcut_logs_quiet();

        terminal
            .draw(|f| ui::draw(f, app))
            .map_err(|e| e.to_string())?;

        if event::poll(Duration::from_millis(20)).map_err(|e| e.to_string())? {
            match event::read().map_err(|e| e.to_string())? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if ui::handle_key(app, key.code, key.modifiers) {
                        return Ok(());
                    }
                }
                Event::Mouse(me) => {
                    if ui::handle_mouse(app, me) {
                        return Ok(());
                    }
                }
                Event::Paste(text) => {
                    if let Some(modal) = &mut app.prompt_modal {
                        modal.insert_str(&text);
                    }
                }
                Event::Resize(_, _) | Event::Key(_) | Event::FocusGained | Event::FocusLost => {}
            }
        }
    }
}
