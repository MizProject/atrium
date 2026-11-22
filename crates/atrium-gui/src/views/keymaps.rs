//! Dual-layer Keymaps studio: `[FW]` Firmware Flash (Amber) vs `[DM]` Daemon Memory (Aqua #00D7FF).

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::select::Select;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{Context, Div, ParentElement as _, Styled as _, Window, div, prelude::*, px, rgb};

use atrium_core::keymap;

use crate::AtriumGui;
use crate::state::{
    ArrowClusterPreset, ConnectedState, KeymapCategory, KeymapLayerView, ShortcutActionPreset,
};
use crate::theme::HardwareColors;
use crate::widgets::{KeyboardCanvasMode, action_kind_tag, hardware_tag};

const ARROW_PRESETS: [(ArrowClusterPreset, &str); 3] = [
    (ArrowClusterPreset::Rdfg, "R/D/F/G"),
    (ArrowClusterPreset::Tfgh, "T/F/G/H"),
    (ArrowClusterPreset::Wasd, "W/A/S/D"),
];

impl AtriumGui {
    pub(crate) fn view_hotkeys(
        &self,
        state: &ConnectedState,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        v_flex()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .gap_3()
            .child(self.render_hotkey_board_card(state, cx))
            .child(self.render_hotkey_composer(state, cx))
            .child(self.render_hotkey_rule_columns(state, cx))
            .child(self.render_hotkey_logs(state, cx))
    }

    fn render_hotkey_board_card(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let hc = HardwareColors::from_app(cx);
        let (aqua_keys, aqua_pulsing) = state.active_daemon_indicators();
        let filter_idx = match state.keymap_layer_view {
            KeymapLayerView::Both => 0,
            KeymapLayerView::FirmwareOnly => 1,
            KeymapLayerView::DaemonOnly => 2,
        };

        let toolbar = h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                TabBar::new("hk-layer-filter")
                    .segmented()
                    .small()
                    .selected_index(filter_idx)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            st.keymap_layer_view = match *ix {
                                1 => KeymapLayerView::FirmwareOnly,
                                2 => KeymapLayerView::DaemonOnly,
                                _ => KeymapLayerView::Both,
                            };
                        }
                        cx.notify();
                    }))
                    .child(Tab::new().label("Both Layers"))
                    .child(Tab::new().label("[FW] Flash"))
                    .child(Tab::new().label("[DM] Daemon")),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Switch::new("hk-watcher-sw")
                            .small()
                            .checked(state.shortcut_watcher_active)
                            .label("Daemon Watcher")
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.shortcut_watcher_active = *checked;
                                    let _ = st.sync_shortcuts();
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("hk-record")
                            .xsmall()
                            .with_variant(if state.shortcut_recording {
                                gpui_kit::component::button::ButtonVariant::Warning
                            } else {
                                gpui_kit::component::button::ButtonVariant::Secondary
                            })
                            .label(if state.shortcut_recording {
                                "Stop Capture"
                            } else {
                                "Memorize Gesture"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.shortcut_recording = !st.shortcut_recording;
                                    st.record_first_key = None;
                                    if st.shortcut_recording {
                                        if !st.travel_enabled {
                                            let _ = st.start_travel();
                                        }
                                        st.captured_gesture_banner = Some(
                                            "Press single key or hold modifier + 2nd key on keyboard."
                                                .into(),
                                        );
                                    }
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("hk-reload")
                            .xsmall()
                            .outline()
                            .icon(Icon::new(IconName::RefreshCw))
                            .label("Reload")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    let _ = st.refresh_shortcuts();
                                    let _ = st.refresh_keyboard_fn_map();
                                    this.status_line = "Reloaded [FW] & [DM] layers.".into();
                                }
                                cx.notify();
                            })),
                    ),
            );

        let legend_row = h_flex()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap_2()
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(hardware_tag("FW: Flash (Amber)", true, false, &hc))
                    .child(hardware_tag("DM: Daemon (Aqua #00D7FF)", false, false, &hc))
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_sm()
                            .bg(rgb(hc.overlap_bg))
                            .border_1()
                            .border_color(rgb(hc.overlap_border))
                            .text_size(px(10.0))
                            .text_color(rgb(hc.overlap_ink))
                            .child("FW+DM Overlap"),
                    ),
            )
            .child(if !aqua_pulsing.is_empty() {
                hardware_tag(format!("DM ACTIVE ({})", aqua_pulsing.len()), false, true, &hc)
            } else if !aqua_keys.is_empty() {
                hardware_tag(format!("DM LAYER ({})", aqua_keys.len()), false, true, &hc)
            } else if state.is_fn_held() {
                hardware_tag("FW Fn HELD", true, true, &hc)
            } else {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Click key: trigger · Shift+click: non-Fn modifier")
            });

        let mut card = GroupBox::new()
            .outline()
            .title("Keymap Matrix")
            .child(toolbar)
            .child(legend_row);

        if !state.uinput_ready {
            let reason = state
                .uinput_error
                .as_deref()
                .unwrap_or("/dev/uinput unavailable in atriumd");
            card = card.child(Alert::error("uinput-offline", format!("[DM] uinput offline: {reason}")));
        }

        if let Some(banner) = &state.captured_gesture_banner {
            card = card.child(Alert::info("hk-capture-banner", banner.clone()));
        }

        div().w_full().child(card.child(self.render_keyboard_canvas(
            state,
            KeyboardCanvasMode::HotkeyPicker,
            1.14,
            0.0,
            cx,
        )))
    }

    fn render_hotkey_composer(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let hc = HardwareColors::from_app(cx);
        let pick_name = keymap::by_index(state.draft_rule_key).map_or("Key", |k| k.name);
        let gesture_preview = state.draft_gesture_label();
        let cat_idx = usize::from(state.draft_category == KeymapCategory::DaemonMemory);

        let header = h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                TabBar::new("hk-cat-tabs")
                    .segmented()
                    .small()
                    .selected_index(cat_idx)
                    .on_click(cx.listener(|this, ix: &usize, window, cx| {
                        if let Ok(st) = &mut this.conn {
                            let cat = if *ix == 0 {
                                KeymapCategory::FirmwareFnFlash
                            } else {
                                KeymapCategory::DaemonMemory
                            };
                            st.set_keymap_draft_category(cat);
                        }
                        this.sync_keymap_controls_from_state(window, cx);
                        cx.notify();
                    }))
                    .child(Tab::new().label("[FW] Firmware Flash (cmd 0x18)"))
                    .child(Tab::new().label("[DM] Daemon Memory (Non-Fn)")),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Gesture: {gesture_preview}")),
            );

        if state.draft_category == KeymapCategory::FirmwareFnFlash {
            let current_vk = state.draft_virtual_key;
            let fw_note = state
                .keyboard_fn_map
                .get(&state.draft_rule_key)
                .map(|&hid| {
                    format!(
                        "Existing slot: Fn + {pick_name} -> {} (overwrites cleanly).",
                        keymap::usb_hid_name(hid)
                    )
                });

            let mut card = GroupBox::new()
                .outline()
                .title("[FW] Firmware Flash Composer")
                .child(header)
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_end()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_xs().child("Chord"))
                                .child(hardware_tag("Fn + Key (Fixed)", true, true, &hc)),
                        )
                        .child(
                            v_flex()
                                .w(px(180.0))
                                .gap_1()
                                .child(div().text_xs().child("Trigger Key"))
                                .child(Select::new(&self.sel_hk_trigger).small()),
                        )
                        .child(
                            v_flex()
                                .w(px(210.0))
                                .gap_1()
                                .child(div().text_xs().child("USB HID Target"))
                                .child(Select::new(&self.sel_hk_vk).small()),
                        )
                        .child(
                            v_flex().gap_1().child(div().text_xs().child("Quick 4-Key Arrows")).child(
                                h_flex().gap_1().children(ARROW_PRESETS.into_iter().map(
                                    |(preset, lbl)| {
                                        Button::new(("fw-arr", preset as usize))
                                            .xsmall()
                                            .outline()
                                            .label(lbl)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Ok(st) = &mut this.conn {
                                                    match st.save_arrow_cluster_to_keyboard(preset)
                                                    {
                                                        Ok(msg) => this.status_line = msg,
                                                        Err(e) => this.status_line = e,
                                                    }
                                                }
                                                cx.notify();
                                            }))
                                    },
                                )),
                            ),
                        ),
                );

            if let Some(note) = fw_note {
                card = card.child(div().text_xs().text_color(rgb(hc.fw_ink)).child(note));
            }

            return div().w_full().child(
                card.child(
                    Button::new("hk-save-fw")
                        .primary()
                        .small()
                        .icon(Icon::new(IconName::Cpu))
                        .label(format!("Write Fn + {pick_name} -> {}", current_vk.label()))
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.set_keymap_draft_category(KeymapCategory::FirmwareFnFlash);
                                match st.save_draft_to_keyboard_fn() {
                                    Ok(msg) => this.status_line = msg,
                                    Err(e) => this.status_line = e,
                                }
                            }
                            cx.notify();
                        })),
                ),
            );
        }

        // Daemon Memory Composer
        let dm_arrow_mod = keymap::by_index(state.daemon_arrow_modifier()).map_or("Caps", |k| k.name);
        let editing_id = state.editing_daemon_rule_id;
        let conflict = state.conflicting_daemon_rule_for_draft().cloned();
        let empty_cmd = state.draft_rule_preset == ShortcutActionPreset::RunCommand
            && state.draft_rule_command.trim().is_empty();

        let mut card = GroupBox::new()
            .outline()
            .title(if let Some(eid) = editing_id {
                format!("[DM] Editing Rule #{eid}")
            } else {
                "[DM] Daemon Memory Composer".into()
            })
            .child(header)
            .child(
                h_flex()
                    .flex_wrap()
                    .items_end()
                    .gap_3()
                    .child(
                        v_flex()
                            .w(px(165.0))
                            .gap_1()
                            .child(div().text_xs().child("Modifier (Non-Fn)"))
                            .child(Select::new(&self.sel_hk_modifier).small()),
                    )
                    .child(
                        v_flex()
                            .w(px(170.0))
                            .gap_1()
                            .child(div().text_xs().child("Trigger Key"))
                            .child(Select::new(&self.sel_hk_trigger).small()),
                    )
                    .child(
                        v_flex()
                            .w(px(195.0))
                            .gap_1()
                            .child(div().text_xs().child("Action"))
                            .child(Select::new(&self.sel_hk_action).small()),
                    )
                    .when(state.draft_rule_preset == ShortcutActionPreset::EmitVirtualKey, |row| {
                        row.child(
                            v_flex()
                                .w(px(195.0))
                                .gap_1()
                                .child(div().text_xs().child("OS / Media Key"))
                                .child(Select::new(&self.sel_hk_vk).small()),
                        )
                    })
                    .child(
                        v_flex()
                            .w(px(160.0))
                            .gap_1()
                            .child(
                                h_flex()
                                    .justify_between()
                                    .text_xs()
                                    .child("Depth")
                                    .child(format!("{:.2} mm", state.draft_rule_depth_mm)),
                            )
                            .child(Slider::new(&self.slider_hk_depth).horizontal()),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_xs().child(format!("{dm_arrow_mod} + Arrows")))
                            .child(
                                h_flex().gap_1().children(ARROW_PRESETS.into_iter().map(
                                    |(preset, lbl)| {
                                        Button::new(("dm-arr", preset as usize))
                                            .xsmall()
                                            .outline()
                                            .label(lbl)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Ok(st) = &mut this.conn {
                                                    let name = st.add_daemon_arrow_cluster(preset);
                                                    match st.sync_shortcuts() {
                                                        Ok(()) => {
                                                            this.status_line =
                                                                format!("Added {name} to [DM].")
                                                        }
                                                        Err(e) => this.status_line = e,
                                                    }
                                                }
                                                cx.notify();
                                            }))
                                    },
                                )),
                            ),
                    ),
            );

        if state.draft_rule_preset == ShortcutActionPreset::RunCommand {
            card = card.child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.input_hk_cmd).small()))
                    .child(
                        Button::new("hk-cmd-sample")
                            .xsmall()
                            .outline()
                            .label("Sample Notify")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let sample = "notify-send 'Atrium WIN 60 HE' 'Hotkey triggered'";
                                if let Ok(st) = &mut this.conn {
                                    st.draft_rule_command = sample.into();
                                }
                                this.input_hk_cmd.update(cx, |input, cx| {
                                    input.set_value(sample, window, cx);
                                });
                                cx.notify();
                            })),
                    ),
            );
        }

        if let Some(eid) = editing_id {
            card = card.child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(hardware_tag(format!("EDITING #{eid}"), false, true, &hc))
                    .child(
                        Button::new("hk-cancel-edit")
                            .xsmall()
                            .outline()
                            .label("Cancel Edit")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.cancel_daemon_rule_edit();
                                    this.status_line = "Exited edit mode.".into();
                                }
                                cx.notify();
                            })),
                    ),
            );
        }

        if let Some(c) = &conflict {
            let cid = c.id;
            let c_clone = c.clone();
            card = card
                .child(Alert::warning(
                    "hk-conflict-alert",
                    format!(
                        "Rule #{cid} ({}) already uses {gesture_preview}. Load or replace to avoid duplicates.",
                        c.name
                    ),
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(("hk-load-conf", cid as usize))
                                .small()
                                .outline()
                                .label(format!("Load #{cid}"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.load_daemon_rule_into_draft(&c_clone);
                                        this.status_line = format!("Loaded #{cid} for edit.");
                                    }
                                    this.sync_keymap_controls_from_state(window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("hk-replace-conf")
                                .small()
                                .warning()
                                .label(if let Some(eid) = editing_id {
                                    format!("Update #{eid} & Replace #{cid}")
                                } else {
                                    format!("Replace #{cid}")
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        match st.commit_draft_daemon_rule(true) {
                                            Ok(msg) => this.status_line = msg,
                                            Err(e) => this.status_line = e,
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                );
        } else if empty_cmd {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(rgb(hc.warn_ink))
                    .child("Enter a shell command above before saving."),
            );
        } else {
            card = card.child(
                Button::new("hk-save-dm")
                    .primary()
                    .small()
                    .icon(Icon::new(IconName::Zap))
                    .label(if let Some(eid) = editing_id {
                        format!("Update Rule #{eid} ({gesture_preview})")
                    } else {
                        format!("Save {gesture_preview} to Daemon")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            match st.commit_draft_daemon_rule(false) {
                                Ok(msg) => this.status_line = msg,
                                Err(e) => this.status_line = e,
                            }
                        }
                        cx.notify();
                    })),
            );
        }

        div().w_full().child(card)
    }

    fn render_hotkey_rule_columns(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let hc = HardwareColors::from_app(cx);

        let mut fw_col = GroupBox::new()
            .outline()
            .title(format!("[FW] Flash Slots ({})", state.keyboard_fn_map.len()))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("hk-fw-refresh")
                            .xsmall()
                            .outline()
                            .icon(Icon::new(IconName::RefreshCw))
                            .label("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    match st.refresh_keyboard_fn_map() {
                                        Ok(()) => this.status_line = "Refreshed [FW] flash.".into(),
                                        Err(e) => this.status_line = e,
                                    }
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("hk-fw-reset")
                            .xsmall()
                            .danger()
                            .icon(Icon::new(IconName::Trash))
                            .label("Reset All [FW]")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    match st.reset_keyboard_fn_map() {
                                        Ok(()) => this.status_line = "Cleared [FW] flash.".into(),
                                        Err(e) => this.status_line = e,
                                    }
                                }
                                cx.notify();
                            })),
                    ),
            );

        if state.keyboard_fn_map.is_empty() {
            fw_col = fw_col.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Factory Fn defaults active (no custom [FW] slots)."),
            );
        } else {
            for (&slot_idx, &target_hid) in &state.keyboard_fn_map {
                let src = keymap::by_index(slot_idx).map_or("Key", |k| k.name);
                let dst = keymap::usb_hid_name(target_hid);
                fw_col = fw_col.child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .p_2()
                        .rounded(cx.theme().radius)
                        .bg(rgb(hc.fw_bg))
                        .border_1()
                        .border_color(rgb(hc.fw_dim_border))
                        .justify_between()
                        .items_center()
                        .gap_2()
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .items_center()
                                .gap_2()
                                .child(hardware_tag("FW", true, false, &hc))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .child(format!("Fn + {src} -> {dst}")),
                                ),
                        )
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .gap_1()
                                .child(
                                    Button::new(("hk-kb-edit", usize::from(slot_idx)))
                                        .xsmall()
                                        .outline()
                                        .label("Edit")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            if let Ok(st) = &mut this.conn {
                                                st.load_firmware_slot_into_draft(
                                                    slot_idx, target_hid,
                                                );
                                            }
                                            this.sync_keymap_controls_from_state(window, cx);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new(("hk-kb-clr", usize::from(slot_idx)))
                                        .xsmall()
                                        .danger()
                                        .label("Clear")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Ok(st) = &mut this.conn {
                                                let _ = st.clear_keyboard_fn_slot(slot_idx);
                                            }
                                            cx.notify();
                                        })),
                                ),
                        ),
                );
            }
        }

        let mut dm_col = GroupBox::new()
            .outline()
            .title(format!("[DM] Daemon Rules ({})", state.shortcut_rules.len()))
            .child(
                Button::new("hk-dm-save-disk")
                    .xsmall()
                    .outline()
                    .icon(Icon::new(IconName::Check))
                    .label("Save to Disk")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            match st.sync_shortcuts() {
                                Ok(()) => this.status_line = "Saved [DM] rules.".into(),
                                Err(e) => this.status_line = e,
                            }
                        }
                        cx.notify();
                    })),
            );

        if state.shortcut_rules.is_empty() {
            dm_col = dm_col.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No [DM] daemon rules configured yet."),
            );
        } else {
            for rule in &state.shortcut_rules {
                let r_id = rule.id;
                let rule_clone = rule.clone();
                let is_edit = state.editing_daemon_rule_id == Some(r_id);
                let invalid_fn = rule.modifier_key == Some(keymap::idx::FN)
                    || rule.trigger_key == keymap::idx::FN;

                dm_col = dm_col.child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .p_2()
                        .gap_1p5()
                        .rounded(cx.theme().radius)
                        .bg(rgb(if is_edit {
                            hc.dm_active
                        } else if rule.enabled {
                            hc.dm_bg
                        } else {
                            hc.dm_muted_bg
                        }))
                        .border_1()
                        .border_color(rgb(if is_edit || rule.enabled {
                            hc.dm_dim_border
                        } else {
                            hc.dm_muted_border
                        }))
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .justify_between()
                                .items_center()
                                .gap_2()
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .flex_wrap()
                                        .items_center()
                                        .gap_1p5()
                                        .child(hardware_tag("DM", false, rule.enabled, &hc))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(action_kind_tag(&rule.action)),
                                        )
                                        .when(invalid_fn, |r| {
                                            r.child(hardware_tag("Invalid Fn", true, false, &hc))
                                        })
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_xs()
                                                .child(format!(
                                                    "#{} {} (>= {:.2} mm)",
                                                    rule.id,
                                                    rule.gesture_label(),
                                                    rule.trigger_depth_mm
                                                )),
                                        ),
                                )
                                .child(
                                    div().flex_shrink_0().child(
                                        Switch::new(("hk-en-sw", r_id as usize))
                                            .small()
                                            .checked(rule.enabled)
                                            .on_change(cx.listener(move |this, en: &bool, _, cx| {
                                                if let Ok(st) = &mut this.conn {
                                                    if let Some(r) = st
                                                        .shortcut_rules
                                                        .iter_mut()
                                                        .find(|r| r.id == r_id)
                                                    {
                                                        r.enabled = *en;
                                                    }
                                                    let _ = st.sync_shortcuts();
                                                }
                                                cx.notify();
                                            })),
                                    ),
                                ),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .justify_between()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(rule.action.summary()),
                                )
                                .child(
                                    h_flex()
                                        .flex_shrink_0()
                                        .gap_1()
                                        .child(
                                            Button::new(("hk-edit-btn", r_id as usize))
                                                .xsmall()
                                                .outline()
                                                .label("Edit")
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    if let Ok(st) = &mut this.conn {
                                                        st.load_daemon_rule_into_draft(&rule_clone);
                                                    }
                                                    this.sync_keymap_controls_from_state(
                                                        window, cx,
                                                    );
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Button::new(("hk-fire-btn", r_id as usize))
                                                .xsmall()
                                                .outline()
                                                .icon(Icon::new(IconName::Play))
                                                .label("Test")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if let Ok(st) = &mut this.conn {
                                                        match st.trigger_shortcut_rule(r_id) {
                                                            Ok(log) => {
                                                                this.status_line = format!(
                                                                    "[DM] #{}: {}",
                                                                    log.rule_id, log.message
                                                                )
                                                            }
                                                            Err(e) => this.status_line = e,
                                                        }
                                                    }
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Button::new(("hk-del-btn", r_id as usize))
                                                .xsmall()
                                                .danger()
                                                .icon(Icon::new(IconName::Trash))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    if let Ok(st) = &mut this.conn {
                                                        st.shortcut_rules.retain(|r| r.id != r_id);
                                                        if st.editing_daemon_rule_id == Some(r_id) {
                                                            st.editing_daemon_rule_id = None;
                                                        }
                                                        let _ = st.sync_shortcuts();
                                                    }
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                        ),
                );
            }
        }

        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_stretch()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w(px(310.0))
                    .overflow_hidden()
                    .child(fw_col.overflow_hidden()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(310.0))
                    .overflow_hidden()
                    .child(dm_col.overflow_hidden()),
            )
    }

    fn render_hotkey_logs(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let mut card = GroupBox::new()
            .outline()
            .overflow_hidden()
            .title("[DM] Execution Feed");

        if state.shortcut_logs.is_empty() {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("No [DM] triggers yet."),
            );
        } else {
            for log in state.shortcut_logs.iter().take(6) {
                card = card.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .text_xs()
                        .text_color(if log.success {
                            cx.theme().success
                        } else {
                            cx.theme().danger
                        })
                        .child(format!(
                            "[{}] {} -> {}",
                            if log.success { "OK" } else { "ERR" },
                            log.rule_name,
                            log.message
                        )),
                );
            }
        }
        div().w_full().min_w_0().child(card)
    }
}
