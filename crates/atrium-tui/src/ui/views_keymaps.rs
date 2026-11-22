//! Dual-layer Keymaps studio (`[FW] Firmware Flash` + `[DM] Daemon Host Memory`),
//! featuring Fn separation, in-place rule editing, conflict/empty checks, test firing,
//! gesture capture, and quick arrow presets.

use crossterm::event::KeyCode;
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph};

use reaula_core::ipc::VirtualKey;
use reaula_core::keymap::{self, WIN60HE_KEYS};

use crate::state::{
    App, ArrowClusterPreset, DAEMON_MODIFIERS, FocusPane, HitRegion, HitTarget, KeymapCategory,
    KeymapLayerView, PromptPurpose, ShortcutActionPreset, TextPromptModal,
};
use crate::theme;
use crate::ui::keyboard::{MatrixMode, draw_keyboard_matrix};
use crate::ui::{ActionChip, render_chip_bar};

pub fn draw(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let top_h = if area.height < 28 { 12 } else { 15 };
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(top_h),
            Constraint::Length(9),
            Constraint::Min(7),
        ])
        .split(area);

    // Header filter & quick arrow presets bar
    render_chip_bar(
        f,
        v[0],
        &[
            ActionChip::new(
                "Compare Both",
                app.keymap_view == KeymapLayerView::Both,
                HitTarget::KmLayerView(KeymapLayerView::Both),
            ),
            ActionChip::new(
                "FW Only [FW]",
                app.keymap_view == KeymapLayerView::FirmwareOnly,
                HitTarget::KmLayerView(KeymapLayerView::FirmwareOnly),
            ),
            ActionChip::new(
                "Daemon Only [DM]",
                app.keymap_view == KeymapLayerView::DaemonOnly,
                HitTarget::KmLayerView(KeymapLayerView::DaemonOnly),
            ),
            ActionChip::new(
                if app.shortcut_watcher_active {
                    "Watcher:ACTIVE"
                } else {
                    "Watcher:PAUSED"
                },
                app.shortcut_watcher_active,
                HitTarget::KmWatcherToggle,
            ),
            ActionChip::new("FW:RDFG", false, HitTarget::KmPresetFw(ArrowClusterPreset::Rdfg)),
            ActionChip::new("FW:TFGH", false, HitTarget::KmPresetFw(ArrowClusterPreset::Tfgh)),
            ActionChip::new("DM:RDFG", false, HitTarget::KmPresetDm(ArrowClusterPreset::Rdfg)),
            ActionChip::new("DM:TFGH", false, HitTarget::KmPresetDm(ArrowClusterPreset::Tfgh)),
            ActionChip::new("DM:WASD", false, HitTarget::KmPresetDm(ArrowClusterPreset::Wasd)),
        ],
        hits,
    );

    let matrix_title = format!(
        "Dual-Layer Map ([FW]=Amber Flash ({}) · [DM]=Aqua Daemon ({}) · Click key sets Trigger)",
        app.keyboard_fn_map.len(),
        app.shortcut_rules.len()
    );
    draw_keyboard_matrix(
        f,
        v[1],
        app,
        MatrixMode::Keymaps,
        app.focus == FocusPane::KeymapsLayerMatrix,
        &matrix_title,
        hits,
    );

    draw_composer(f, v[2], app, hits);
    draw_lists_and_logs(f, v[3], app, hits);
}

fn draw_composer(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let focused = app.focus == FocusPane::KeymapsComposer;
    let title = match (app.draft_category, app.editing_daemon_rule_id) {
        (KeymapCategory::FirmwareFnFlash, _) => {
            "Keymap Composer — Firmware Flash [FW] (cmd 0x18 · No Daemon Needed)"
        }
        (KeymapCategory::DaemonMemory, Some(eid)) => {
            // Leak-free lifetime via owned String inside panel_block
            return draw_composer_with_title(
                f,
                area,
                app,
                focused,
                hits,
                &format!("Keymap Composer — EDITING [DM] Rule #{eid} (Updates In Place)"),
            );
        }
        (KeymapCategory::DaemonMemory, None) => {
            "Keymap Composer — Daemon Memory [DM] (No Fn Allowed · EVIOCGRAB Protected)"
        }
    };
    draw_composer_with_title(f, area, app, focused, hits, title);
}

fn draw_composer_with_title(
    f: &mut Frame<'_>,
    area: Rect,
    app: &App,
    focused: bool,
    hits: &mut Vec<HitRegion>,
    title: &str,
) {
    let blk = theme::panel_block(title, focused);
    let inner = blk.inner(area);
    f.render_widget(blk, area);
    hits.push(HitRegion {
        rect: area,
        action: HitTarget::Focus(FocusPane::KeymapsComposer),
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
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    // Row 0: Target layer switcher + physical capture
    render_chip_bar(
        f,
        rows[0],
        &[
            ActionChip::new(
                "[1] Firmware [FW]",
                app.draft_category == KeymapCategory::FirmwareFnFlash,
                HitTarget::KmCategory(KeymapCategory::FirmwareFnFlash),
            )
            .focus_mark(focused && app.km_composer_row == 0),
            ActionChip::new(
                "[2] Daemon [DM]",
                app.draft_category == KeymapCategory::DaemonMemory,
                HitTarget::KmCategory(KeymapCategory::DaemonMemory),
            ),
            ActionChip::new(
                if app.shortcut_recording {
                    "Listening HW Keys... (Esc stop)"
                } else {
                    "Capture HW Keys"
                },
                app.shortcut_recording,
                HitTarget::KmRecordToggle,
            ),
        ],
        hits,
    );

    if app.draft_category == KeymapCategory::FirmwareFnFlash {
        let trig_name = keymap::by_index(app.draft_rule_key).map_or("Key", |k| k.name);
        render_chip_bar(
            f,
            rows[1],
            &[
                ActionChip::new("Modifier: Fn (#122 Hardware Locked)", true, HitTarget::KmField(1))
                    .focus_mark(focused && app.km_composer_row == 1),
            ],
            hits,
        );
        render_chip_bar(
            f,
            rows[2],
            &[
                ActionChip::new("<", false, HitTarget::KmTriggerCycle(-1)),
                ActionChip::new(format!("Trigger: {trig_name} (#{})", app.draft_rule_key), true, HitTarget::KmField(2))
                    .focus_mark(focused && app.km_composer_row == 2),
                ActionChip::new(">", false, HitTarget::KmTriggerCycle(1)),
            ],
            hits,
        );
        render_chip_bar(
            f,
            rows[3],
            &[
                ActionChip::new("<", false, HitTarget::KmVirtualCycle(-1)),
                ActionChip::new(
                    format!("OS Key: {}", app.draft_virtual_key.label()),
                    true,
                    HitTarget::KmField(3),
                )
                .focus_mark(focused && app.km_composer_row == 3),
                ActionChip::new(">", false, HitTarget::KmVirtualCycle(1)),
            ],
            hits,
        );
        render_chip_bar(
            f,
            rows[4],
            &[
                ActionChip::new("Flash Fn+Key to WIN 60 HE", focused && app.km_composer_row == 4, HitTarget::KmSave)
                    .focus_mark(focused && app.km_composer_row == 4),
                ActionChip::new("Reset ALL FW Fn", false, HitTarget::KmResetFwAll),
            ],
            hits,
        );
        if let Some(banner) = &app.captured_gesture_banner {
            f.render_widget(
                Paragraph::new(Span::styled(banner, Style::default().fg(theme::FW_AMBER_INK))),
                rows[5],
            );
        }
    } else {
        let mod_name = match app.draft_rule_modifier {
            Some(m) => keymap::by_index(m).map_or("Mod", |k| k.name),
            None => "None (Single)",
        };
        let trig_name = keymap::by_index(app.draft_rule_key).map_or("Key", |k| k.name);
        render_chip_bar(
            f,
            rows[1],
            &[
                ActionChip::new("<", false, HitTarget::KmModCycle(-1)),
                ActionChip::new(format!("Mod: {mod_name}"), true, HitTarget::KmField(1))
                    .focus_mark(focused && app.km_composer_row == 1),
                ActionChip::new(">", false, HitTarget::KmModCycle(1)),
                ActionChip::new("<", false, HitTarget::KmTriggerCycle(-1)),
                ActionChip::new(format!("Trigger: {trig_name}"), true, HitTarget::KmField(2))
                    .focus_mark(focused && app.km_composer_row == 2),
                ActionChip::new(">", false, HitTarget::KmTriggerCycle(1)),
                ActionChip::new("-", false, HitTarget::KmDepthStep(-0.10)),
                ActionChip::new(format!("{:.2}mm", app.draft_rule_depth_mm), false, HitTarget::KmField(2)),
                ActionChip::new("+", false, HitTarget::KmDepthStep(0.10)),
            ],
            hits,
        );

        let mut row2 = vec![
            ActionChip::new("<", false, HitTarget::KmPresetCycle(-1)),
            ActionChip::new(
                format!("Action: {}", app.draft_rule_preset.label()),
                true,
                HitTarget::KmField(3),
            )
            .focus_mark(focused && app.km_composer_row == 3),
            ActionChip::new(">", false, HitTarget::KmPresetCycle(1)),
        ];
        if app.draft_rule_preset == ShortcutActionPreset::EmitVirtualKey {
            row2.push(ActionChip::new("<", false, HitTarget::KmVirtualCycle(-1)));
            row2.push(ActionChip::new(
                app.draft_virtual_key.label(),
                false,
                HitTarget::KmVirtualCycle(1),
            ));
            row2.push(ActionChip::new(">", false, HitTarget::KmVirtualCycle(1)));
        } else if app.draft_rule_preset == ShortcutActionPreset::RunCommand {
            row2.push(ActionChip::new(
                format!("Cmd: \"{}\"", trim_cmd(&app.draft_rule_command, 28)),
                false,
                HitTarget::KmShellPrompt,
            ));
            row2.push(ActionChip::new("Edit Cmd..", false, HitTarget::KmShellPrompt));
        }
        render_chip_bar(f, rows[2], &row2, hits);

        // Row 3: Save / Conflict / Edit controls
        let conflict_id = app.conflicting_daemon_rule_for_draft().map(|r| r.id);
        let empty_cmd = app.draft_rule_preset == ShortcutActionPreset::RunCommand
            && app.draft_rule_command.trim().is_empty();
        let mut row3 = Vec::new();
        if let Some(eid) = app.editing_daemon_rule_id {
            row3.push(
                ActionChip::new(format!("Update [DM] Rule #{eid}"), true, HitTarget::KmSave)
                    .focus_mark(focused && app.km_composer_row == 4),
            );
            row3.push(ActionChip::new("Cancel Edit", false, HitTarget::KmCancelEdit));
        } else {
            row3.push(
                ActionChip::new("Add [DM] Rule", conflict_id.is_none() && !empty_cmd, HitTarget::KmSave)
                    .focus_mark(focused && app.km_composer_row == 4),
            );
        }
        if let Some(cid) = conflict_id {
            row3.push(ActionChip::new(
                format!("Load #{cid} to Edit"),
                false,
                HitTarget::KmLoadConflict(cid),
            ));
            row3.push(ActionChip::new(
                format!("Replace Existing #{cid}"),
                false,
                HitTarget::KmReplaceConflict,
            ));
        }
        render_chip_bar(f, rows[3], &row3, hits);

        // Row 4: Validation / Duplicate / Capture notice
        let msg = if empty_cmd {
            ("Empty shell command - click 'Edit Cmd..' before saving.", theme::INK_DANGER)
        } else if let Some(cid) = conflict_id {
            // Static formatting into Paragraph via owned String below
            let s = format!(
                "Duplicate gesture '{}' already used by rule #{cid}. Load #{cid} or Replace.",
                app.draft_gesture_label()
            );
            f.render_widget(
                Paragraph::new(Span::styled(s, Style::default().fg(theme::INK_WARN))),
                rows[4],
            );
            return;
        } else if let Some(banner) = &app.captured_gesture_banner {
            (banner.as_str(), theme::DM_AQUA_INK)
        } else {
            (
                "Runs as your non-root desktop user via systemd-run --user.",
                theme::INK_MUTED,
            )
        };
        f.render_widget(
            Paragraph::new(Span::styled(msg.0, Style::default().fg(msg.1))),
            rows[4],
        );
    }
}

fn draw_lists_and_logs(f: &mut Frame<'_>, area: Rect, app: &App, hits: &mut Vec<HitRegion>) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(area);

    // Left: Firmware [FW] slots
    let fw_focused = app.focus == FocusPane::KeymapsFirmwareList;
    let fw_blk = theme::panel_block(
        format!("[FW] Flash Slots ({})", app.keyboard_fn_map.len()),
        fw_focused,
    );
    let fw_inner = fw_blk.inner(cols[0]);
    let fw_entries: Vec<(u8, u8)> = app.keyboard_fn_map.iter().map(|(&k, &v)| (k, v)).collect();
    let fw_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(1)])
        .split(fw_inner);
    let fw_items: Vec<ListItem> = if fw_entries.is_empty() {
        vec![ListItem::new("No custom Fn flash slots. Use FW:RDFG or Flash above.")]
    } else {
        fw_entries
            .iter()
            .enumerate()
            .map(|(i, &(idx, hid))| {
                let mark = if i == app.km_fw_cursor && fw_focused { "▶ " } else { "  " };
                let kname = keymap::by_index(idx).map_or("?", |k| k.name);
                let target = keymap::usb_hid_name(hid);
                ListItem::new(format!("{mark}[FW] Fn+{kname} -> {target}")).style(Style::default().fg(
                    if i == app.km_fw_cursor && fw_focused {
                        theme::FW_AMBER_INK
                    } else {
                        theme::INK_SECONDARY
                    },
                ))
            })
            .collect()
    };
    f.render_widget(fw_blk, cols[0]);
    hits.push(HitRegion {
        rect: cols[0],
        action: HitTarget::Focus(FocusPane::KeymapsFirmwareList),
    });
    f.render_widget(List::new(fw_items), fw_split[0]);
    for (i, &(idx, _)) in fw_entries.iter().enumerate() {
        let y = fw_split[0].y + i as u16;
        if y < fw_split[0].y + fw_split[0].height {
            hits.push(HitRegion {
                rect: Rect::new(fw_split[0].x, y, fw_split[0].width, 1),
                action: HitTarget::KmFwLoad(idx),
            });
        }
    }
    if let Some(&(idx, _)) = fw_entries.get(app.km_fw_cursor) {
        render_chip_bar(
            f,
            fw_split[1],
            &[
                ActionChip::new("Edit Slot (Enter)", false, HitTarget::KmFwLoad(idx)),
                ActionChip::new("Clear Slot (d)", false, HitTarget::KmFwClear(idx)),
            ],
            hits,
        );
    }

    // Right: Daemon [DM] Rules + quick buttons + Trigger Log
    let dm_focused = app.focus == FocusPane::KeymapsDaemonList;
    let dm_blk = theme::panel_block(
        format!(
            "[DM] Daemon Rules ({}) · uinput:{} grab:{}",
            app.shortcut_rules.len(),
            if app.uinput_ready { "OK" } else { "NO" },
            if app.evdev_grab_active { "ON" } else { "OFF" }
        ),
        dm_focused,
    );
    let dm_inner = dm_blk.inner(cols[1]);
    f.render_widget(dm_blk, cols[1]);
    hits.push(HitRegion {
        rect: cols[1],
        action: HitTarget::Focus(FocusPane::KeymapsDaemonList),
    });

    if dm_inner.height < 3 {
        return;
    }

    let dm_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1), Constraint::Length(2)])
        .split(dm_inner);

    if app.shortcut_rules.is_empty() {
        f.render_widget(
            Paragraph::new("No Daemon rules configured. Add one in the Composer above."),
            dm_split[0],
        );
    } else {
        let items: Vec<ListItem> = app
            .shortcut_rules
            .iter()
            .enumerate()
            .map(|(i, rule)| {
                let selected = i == app.km_dm_cursor;
                let editing = app.editing_daemon_rule_id == Some(rule.id);
                let mark = if selected { "▶ " } else { "  " };
                let on_off = if rule.enabled { "ON " } else { "OFF" };
                let ed_tag = if editing { " [EDITING]" } else { "" };
                let st = if editing {
                    Style::default()
                        .bg(theme::DM_AQUA_ACTIVE)
                        .fg(theme::DM_AQUA_INK)
                        .add_modifier(Modifier::BOLD)
                } else if selected && dm_focused {
                    Style::default()
                        .fg(theme::CYAN_HIGHLIGHT)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::INK_SECONDARY)
                };
                ListItem::new(format!(
                    "{mark}#{:<2} [{on_off}] {} @ {:.2}mm -> {}{ed_tag}",
                    rule.id,
                    rule.gesture_label(),
                    rule.trigger_depth_mm,
                    rule.action.summary()
                ))
                .style(st)
            })
            .collect();
        f.render_widget(List::new(items), dm_split[0]);
        for (i, rule) in app.shortcut_rules.iter().enumerate() {
            let y = dm_split[0].y + i as u16;
            if y < dm_split[0].y + dm_split[0].height {
                hits.push(HitRegion {
                    rect: Rect::new(dm_split[0].x, y, dm_split[0].width, 1),
                    action: HitTarget::KmDmEdit(rule.id),
                });
            }
        }
    }

    // Selected rule action bar
    if let Some(sel_rule) = app.shortcut_rules.get(app.km_dm_cursor) {
        let id = sel_rule.id;
        render_chip_bar(
            f,
            dm_split[1],
            &[
                ActionChip::new(
                    if sel_rule.enabled { "Disable (Space)" } else { "Enable (Space)" },
                    sel_rule.enabled,
                    HitTarget::KmDmToggle(id),
                ),
                ActionChip::new("Edit in Composer (Enter)", false, HitTarget::KmDmEdit(id)),
                ActionChip::new("Test Fire (f)", false, HitTarget::KmDmFire(id)),
                ActionChip::new("Delete (d)", false, HitTarget::KmDmDel(id)),
            ],
            hits,
        );
    }

    let log_line = app.shortcut_logs.first().map_or_else(
        || "Trigger log: (waiting for rule trigger or Test Fire)".to_string(),
        |l| format!("Latest [{}]: {} -> {}", if l.success { "OK" } else { "ERR" }, l.rule_name, l.message),
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(log_line, Style::default().fg(theme::DM_AQUA_INK))),
            Line::from(Span::styled(
                format!(
                    "Persist: {}",
                    app.shortcut_persist_path.as_deref().unwrap_or("<memory-only>")
                ),
                Style::default().fg(theme::INK_MUTED),
            )),
        ]),
        dm_split[2],
    );
}

pub fn handle_key(app: &mut App, code: KeyCode) {
    match app.focus {
        FocusPane::KeymapsLayerMatrix => match code {
            KeyCode::Left | KeyCode::Char('h') => app.move_key_cursor(0, -1),
            KeyCode::Right | KeyCode::Char('l') => app.move_key_cursor(0, 1),
            KeyCode::Up | KeyCode::Char('k') => app.move_key_cursor(-1, 0),
            KeyCode::Down | KeyCode::Char('j') => app.move_key_cursor(1, 0),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let idx = app.current_key().index;
                if idx != keymap::idx::FN {
                    app.draft_rule_key = idx;
                    app.status_msg = format!(
                        "Set keymap trigger key -> {} (#{idx}).",
                        app.current_key().name
                    );
                }
            }
            KeyCode::Char('v') => {
                let pos = KeymapLayerView::ALL.iter().position(|&v| v == app.keymap_view).unwrap_or(0);
                app.keymap_view = KeymapLayerView::ALL[(pos + 1) % KeymapLayerView::ALL.len()];
            }
            _ => {}
        },
        FocusPane::KeymapsComposer => match code {
            KeyCode::Up | KeyCode::Char('k') => app.km_composer_row = app.km_composer_row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => app.km_composer_row = (app.km_composer_row + 1).min(4),
            KeyCode::Left | KeyCode::Char('h') => adjust_composer_row(app, -1),
            KeyCode::Right | KeyCode::Char('l') => adjust_composer_row(app, 1),
            KeyCode::Char('r') => {
                app.shortcut_recording = !app.shortcut_recording;
                app.record_first_key = None;
            }
            KeyCode::Char('e') if app.draft_category == KeymapCategory::DaemonMemory => {
                open_shell_prompt(app);
            }
            KeyCode::Enter => {
                if app.draft_category == KeymapCategory::FirmwareFnFlash {
                    app.save_draft_to_keyboard_fn();
                } else {
                    app.commit_draft_daemon_rule(false);
                }
            }
            _ => {}
        },
        FocusPane::KeymapsFirmwareList => {
            let keys: Vec<(u8, u8)> = app.keyboard_fn_map.iter().map(|(&k, &v)| (k, v)).collect();
            match code {
                KeyCode::Up | KeyCode::Char('k') => app.km_fw_cursor = app.km_fw_cursor.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.km_fw_cursor + 1 < keys.len() {
                        app.km_fw_cursor += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some(&(idx, hid)) = keys.get(app.km_fw_cursor) {
                        app.load_firmware_slot_into_draft(idx, hid);
                    }
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    if let Some(&(idx, _)) = keys.get(app.km_fw_cursor) {
                        app.clear_keyboard_fn_slot(idx);
                    }
                }
                _ => {}
            }
        }
        FocusPane::KeymapsDaemonList => {
            match code {
                KeyCode::Up | KeyCode::Char('k') => app.km_dm_cursor = app.km_dm_cursor.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    if app.km_dm_cursor + 1 < app.shortcut_rules.len() {
                        app.km_dm_cursor += 1;
                    }
                }
                KeyCode::Char(' ') => {
                    if let Some(rule) = app.shortcut_rules.get_mut(app.km_dm_cursor) {
                        rule.enabled = !rule.enabled;
                        let _ = app.sync_shortcuts();
                    }
                }
                KeyCode::Enter => {
                    if let Some(rule) = app.shortcut_rules.get(app.km_dm_cursor).cloned() {
                        app.load_daemon_rule_into_draft(&rule);
                        app.focus = FocusPane::KeymapsComposer;
                    }
                }
                KeyCode::Char('f') => {
                    if let Some(id) = app.shortcut_rules.get(app.km_dm_cursor).map(|r| r.id) {
                        app.trigger_shortcut_rule(id);
                    }
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    if let Some(id) = app.shortcut_rules.get(app.km_dm_cursor).map(|r| r.id) {
                        app.shortcut_rules.retain(|r| r.id != id);
                        if app.editing_daemon_rule_id == Some(id) {
                            app.editing_daemon_rule_id = None;
                        }
                        let _ = app.sync_shortcuts();
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

pub fn open_shell_prompt(app: &mut App) {
    let initial = app.draft_rule_command.clone();
    app.prompt_modal = Some(TextPromptModal::new(
        PromptPurpose::ShellCommand,
        "Daemon Shell Command (Runs as Non-Root User)",
        "Example: konsole  OR  notify-send 'Hello' 'World'",
        &initial,
    ));
}

pub fn cycle_modifier(app: &mut App, dir: isize) {
    let pos = DAEMON_MODIFIERS
        .iter()
        .position(|&m| m == app.draft_rule_modifier)
        .unwrap_or(0) as isize;
    let next = (pos + dir).rem_euclid(DAEMON_MODIFIERS.len() as isize) as usize;
    app.draft_rule_modifier = DAEMON_MODIFIERS[next];
}

pub fn cycle_trigger_key(app: &mut App, dir: isize) {
    let candidates: Vec<u8> = WIN60HE_KEYS
        .iter()
        .map(|k| k.index)
        .filter(|&i| i != keymap::idx::FN)
        .collect();
    let pos = candidates
        .iter()
        .position(|&i| i == app.draft_rule_key)
        .unwrap_or(0) as isize;
    let next = (pos + dir).rem_euclid(candidates.len() as isize) as usize;
    app.draft_rule_key = candidates[next];
}

pub fn cycle_preset(app: &mut App, dir: isize) {
    let pos = ShortcutActionPreset::ALL
        .iter()
        .position(|&p| p == app.draft_rule_preset)
        .unwrap_or(0) as isize;
    let next = (pos + dir).rem_euclid(ShortcutActionPreset::ALL.len() as isize) as usize;
    app.draft_rule_preset = ShortcutActionPreset::ALL[next];
}

pub fn cycle_virtual_key(app: &mut App, dir: isize) {
    let list = if app.draft_category == KeymapCategory::FirmwareFnFlash {
        App::firmware_compatible_virtual_keys()
    } else {
        VirtualKey::ALL.to_vec()
    };
    let pos = list
        .iter()
        .position(|&v| v == app.draft_virtual_key)
        .unwrap_or(0) as isize;
    let next = (pos + dir).rem_euclid(list.len() as isize) as usize;
    app.draft_virtual_key = list[next];
}

fn adjust_composer_row(app: &mut App, dir: isize) {
    match app.km_composer_row {
        0 => {
            let next = if app.draft_category == KeymapCategory::FirmwareFnFlash {
                KeymapCategory::DaemonMemory
            } else {
                KeymapCategory::FirmwareFnFlash
            };
            app.set_keymap_draft_category(next);
        }
        1 if app.draft_category == KeymapCategory::DaemonMemory => cycle_modifier(app, dir),
        2 => cycle_trigger_key(app, dir),
        3 => {
            if app.draft_category == KeymapCategory::FirmwareFnFlash {
                cycle_virtual_key(app, dir);
            } else {
                cycle_preset(app, dir);
            }
        }
        4 if app.draft_category == KeymapCategory::DaemonMemory => {
            app.draft_rule_depth_mm =
                (app.draft_rule_depth_mm + if dir >= 0 { 0.10 } else { -0.10 }).clamp(0.10, 3.30);
        }
        _ => {}
    }
}

fn trim_cmd(s: &str, max_c: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_c {
        s.to_string()
    } else {
        let head: String = chars.into_iter().take(max_c.saturating_sub(2)).collect();
        format!("{head}..")
    }
}
