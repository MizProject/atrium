//! Main workspace: identity strip, 61-key travel matrix, and 3-column workbench.

use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::link::Link;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{Context, Div, IntoElement as _, ParentElement as _, Styled as _, div, px};

use atrium_core::hid_probe::{DEVELOPER_NOTICE, RECOVERY_NOTICE};
use atrium_core::keymap;
use atrium_core::protocol::TriggerMode;

use crate::AtriumGui;
use crate::config::{ConfiguredKeyboardKind, KeyboardProfileEntry};
use crate::state::{ConnectedState, NavSection, SetupHardwareMode};
use crate::widgets::{
    KB_SCALE_DEFAULT, KeyboardCanvasMode, MetricCell, SWITCH_OPTIONS, render_metric_table,
};

impl AtriumGui {
    pub(crate) fn view_main_hub(&self, cx: &mut Context<Self>) -> Div {
        let t = self.text();
        let active_kb = self.config.active_keyboard().cloned();

        let mut cards = h_flex().w_full().min_w_0().flex_wrap().gap_2p5();
        if self.config.keyboards.is_empty() {
            cards = cards.child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t.no_keyboards_empty),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("hub-empty-add")
                                    .xsmall()
                                    .primary()
                                    .icon(Icon::new(IconName::Plus))
                                    .label(t.add_keyboard_btn)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_setup_at_step(3, None, window, cx);
                                    })),
                            ),
                    ),
            );
        } else {
            for (ix, kb) in self.config.keyboards.iter().enumerate() {
                let is_active = active_kb.as_ref().is_some_and(|a| a.id == kb.id);
                let kb_id_activate = kb.id.clone();
                let kb_id_remove = kb.id.clone();
                let kind_label = match kb.kind {
                    ConfiguredKeyboardKind::Supported => t.supported_badge,
                    ConfiguredKeyboardKind::Unsupported => t.unsupported_badge,
                };
                let vid_pid = match (kb.vendor_id, kb.product_id) {
                    (Some(v), Some(p)) => format!("{v:04X}:{p:04X}"),
                    _ => kb.model_id.clone(),
                };

                cards = cards.child(
                    v_flex()
                        .min_w(px(250.0))
                        .flex_1()
                        .overflow_hidden()
                        .gap_2()
                        .p_2p5()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(if is_active {
                            cx.theme().primary
                        } else {
                            cx.theme().border
                        })
                        .bg(cx.theme().background)
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
                                        .items_center()
                                        .gap_1p5()
                                        .child(
                                            div()
                                                .w(px(7.0))
                                                .h(px(7.0))
                                                .flex_shrink_0()
                                                .rounded(cx.theme().radius_full())
                                                .bg(if is_active {
                                                    cx.theme().success
                                                } else {
                                                    cx.theme().muted_foreground
                                                }),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_xs()
                                                .child(kb.label.clone()),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(cx.theme().radius)
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .text_xs()
                                        .text_color(if kb.kind == ConfiguredKeyboardKind::Supported {
                                            cx.theme().info
                                        } else {
                                            cx.theme().warning
                                        })
                                        .child(kind_label),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{} · {}",
                                    vid_pid,
                                    kb.hid_summary.as_deref().unwrap_or(&kb.model_id)
                                )),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    Button::new(("hub-kb-activate", ix))
                                        .xsmall()
                                        .selected(is_active)
                                        .outline()
                                        .label(if is_active {
                                            t.active_badge
                                        } else {
                                            t.activate_btn
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.activate_saved_keyboard(&kb_id_activate, cx);
                                        })),
                                )
                                .child(
                                    Button::new(("hub-kb-remove", ix))
                                        .xsmall()
                                        .ghost()
                                        .icon(Icon::new(IconName::Trash))
                                        .label(t.remove_btn)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.remove_saved_keyboard(&kb_id_remove, cx);
                                        })),
                                ),
                        ),
                );
            }
        }

        let deck = GroupBox::new()
            .outline()
            .title(t.my_keyboards_title)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2p5()
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("hub-add-keyboard")
                                    .xsmall()
                                    .primary()
                                    .icon(Icon::new(IconName::Plus))
                                    .label(t.add_keyboard_btn)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_setup_at_step(3, None, window, cx);
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} profile(s)",
                                        self.config.keyboards.len()
                                    )),
                            ),
                    )
                    .child(cards),
            );

        let body = if let Some(kb) = active_kb.filter(|k| k.kind == ConfiguredKeyboardKind::Unsupported) {
            self.view_unsupported_board_panel(&kb, cx).into_any_element()
        } else {
            match &self.conn {
                Ok(state) => self.view_main(state, cx).into_any_element(),
                Err(err) => self.view_disconnected(err, cx),
            }
        };

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(deck.w_full().min_w_0().overflow_hidden())
            .child(body)
    }

    pub(crate) fn view_unsupported_board_panel(
        &self,
        kb: &KeyboardProfileEntry,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.text();
        let stage = self.render_unsupported_stage_box("hub-unsup", &kb.label, cx);

        let info_panel = GroupBox::new()
            .outline()
            .title(format!("Unsupported Keyboard · {}", kb.label))
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2p5()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                kb.hid_summary
                                    .as_deref()
                                    .unwrap_or("Generic 75% 3D representation active for unsupported HID hardware.")
                                    .to_string(),
                            ),
                    )
                    .child(
                        Alert::warning("hub-recovery-warn", RECOVERY_NOTICE)
                            .title(t.recovery_warning_title),
                    )
                    .child(Alert::info("hub-dev-reply-note", DEVELOPER_NOTICE))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("hub-reprobe-btn")
                                    .small()
                                    .primary()
                                    .icon(Icon::new(IconName::Cpu))
                                    .label("Probe HID / Export JSON\u{2026}")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_setup_at_step(
                                            3,
                                            Some(SetupHardwareMode::UnsupportedProbe),
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Link::new("hub-gh-issue")
                                    .href("https://github.com/MizProject/atrium/issues/new?template=unsupported-keyboard-support.yml")
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1()
                                            .text_xs()
                                            .child(Icon::new(IconName::ExternalLink).xsmall())
                                            .child(t.open_issue_btn),
                                    ),
                            ),
                    ),
            );

        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_start()
            .gap_3()
            .child(stage)
            .child(
                div()
                    .flex_1()
                    .min_w(px(320.0))
                    .overflow_hidden()
                    .child(info_panel.w_full().min_w_0().overflow_hidden()),
            )
    }

    pub(crate) fn view_main(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let fw = state.probe.firmware_string.as_deref().unwrap_or("Unknown");
        let range = state.probe.max_travel.map_or_else(
            || "0.08..=3.40 mm".into(),
            |m| format!("{:.2}..={:.2} mm", m.min_mm, m.max_mm),
        );
        let poll = format!(
            "{} @ {} kHz",
            state.probe.init_info.as_ref().map_or("USB", |i| i.connection_mode()),
            state.poll_rate_khz
        );
        let focus_name = keymap::by_index(state.focused_key).map_or("Key", |k| k.name);

        let identity_strip = render_metric_table(
            vec![
                MetricCell::text("Device", state.model_label(), cx),
                MetricCell::text("Firmware", short_fw_segment(fw).to_string(), cx),
                MetricCell::text("Travel", range, cx),
                MetricCell::text("Link / Poll", poll, cx),
                MetricCell::text(
                    "SOCD",
                    if state.socd_enabled {
                        format!("Active ({} pair)", state.socd_pairs.len())
                    } else {
                        "Off".into()
                    },
                    cx,
                ),
                MetricCell::text("FW Slots", format!("{}", state.keyboard_fn_map.len()), cx),
                MetricCell::text(
                    "Daemon Rules",
                    format!(
                        "{} ({})",
                        state.shortcut_rules.len(),
                        if state.shortcut_watcher_active {
                            "On"
                        } else {
                            "Paused"
                        }
                    ),
                    cx,
                ),
                MetricCell::element(
                    "Action",
                    Button::new("main-open-diag")
                        .xsmall()
                        .outline()
                        .label("Diagnostics")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.switch_section(NavSection::Diagnostics, cx);
                        }))
                        .into_any_element(),
                ),
            ],
            205.0,
            cx,
        );

        let board_header = h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .child(format!(
                        "{} selected · Focus: {} (#{})",
                        state.selected_keys.len(),
                        focus_name,
                        state.focused_key
                    ))
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(if state.travel_enabled { "Live" } else { "Paused" }),
                    ),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .child(
                        Button::new("sel-all")
                            .xsmall()
                            .outline()
                            .label("All (61)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.selected_keys = keymap::all_indices().into_iter().collect();
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("sel-wasd")
                            .xsmall()
                            .outline()
                            .label("WASD")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.selected_keys = [46, 68, 69, 70].into_iter().collect();
                                    st.sync_draft_from_key(46);
                                }
                                this.sync_main_controls_from_state(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("sel-single")
                            .xsmall()
                            .outline()
                            .label("Focused")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    let f = st.focused_key;
                                    st.selected_keys = [f].into_iter().collect();
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("zero-stuck")
                            .xsmall()
                            .outline()
                            .icon(Icon::new(IconName::RotateCcw))
                            .label("Zero")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.clear_stuck_travel();
                                    this.status_line = "Cleared cached travel readings.".into();
                                }
                                cx.notify();
                            })),
                    ),
            );

        let matrix_box = GroupBox::new()
            .outline()
            .overflow_hidden()
            .child(board_header)
            .child(self.render_keyboard_canvas(
                state,
                KeyboardCanvasMode::ActuationAndTravel,
                KB_SCALE_DEFAULT,
                0.0,
                cx,
            ));

        let control_deck = h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_stretch()
            .gap_3()
            .child(self.main_actuation_card(state, cx))
            .child(self.main_switch_deadband_card(state, cx))
            .child(self.main_socd_poll_card(state, cx));

        v_flex()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .gap_3()
            .child(identity_strip)
            .child(matrix_box)
            .child(control_deck)
    }

    fn main_actuation_card(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let mode = state.draft_mode;
        let show_rt = mode != TriggerMode::Fixed;
        let show_split = mode == TriggerMode::RapidTriggerSplit;
        let mode_idx = match mode {
            TriggerMode::Fixed | TriggerMode::Unknown(_) => 0,
            TriggerMode::RapidTrigger => 1,
            TriggerMode::RapidTriggerSplit => 2,
        };

        let mut card = GroupBox::new()
            .outline()
            .title("Actuation & Rapid Trigger")
            .child(
                TabBar::new("main-trig-mode")
                    .segmented()
                    .small()
                    .selected_index(mode_idx)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            st.draft_mode = match *ix {
                                1 => TriggerMode::RapidTrigger,
                                2 => TriggerMode::RapidTriggerSplit,
                                _ => TriggerMode::Fixed,
                            };
                        }
                        cx.notify();
                    }))
                    .child(Tab::new().flex_1().label("Fixed"))
                    .child(Tab::new().flex_1().label("Rapid Trigger"))
                    .child(Tab::new().flex_1().label("RT Split")),
            )
            .child(self.slider_metric_row(
                "Actuation Point",
                format!("{:.2} mm", state.draft_travel_mm),
                &self.slider_actuation,
                cx,
            ));

        if show_rt {
            card = card.child(self.slider_metric_row(
                if show_split { "RT Press" } else { "RT Sensitivity" },
                format!("{:.2} mm", state.draft_rt_press_mm),
                &self.slider_rt_press,
                cx,
            ));
        }
        if show_split {
            card = card.child(self.slider_metric_row(
                "RT Release",
                format!("{:.2} mm", state.draft_rt_release_mm),
                &self.slider_rt_release,
                cx,
            ));
        }

        div().flex_1().min_w(px(260.0)).overflow_hidden().child(
            card.overflow_hidden().child(
                Button::new("apply-actuation")
                    .primary()
                    .small()
                    .w_full()
                    .icon(Icon::new(IconName::Check))
                    .label("Apply Actuation")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.apply_actuation(cx);
                    })),
            ),
        )
    }

    fn main_switch_deadband_card(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let selected_sw = SWITCH_OPTIONS
            .iter()
            .position(|&(id, _)| id == state.draft_switch_id)
            .unwrap_or(0);

        let card = GroupBox::new()
            .outline()
            .title("Switch & Deadband")
            .child(
                TabBar::new("main-switch-tabs")
                    .segmented()
                    .small()
                    .selected_index(selected_sw)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Some(&(id, _)) = SWITCH_OPTIONS.get(*ix) {
                            this.apply_switch_profile(id, cx);
                        }
                    }))
                    .children(SWITCH_OPTIONS.iter().map(|&(_, label)| Tab::new().flex_1().label(label))),
            )
            .child(self.slider_metric_row(
                "Top Deadband",
                format!("{:.2} mm", state.draft_db_top_mm),
                &self.slider_db_top,
                cx,
            ))
            .child(self.slider_metric_row(
                "Bottom Deadband",
                format!("{:.2} mm", state.draft_db_bottom_mm),
                &self.slider_db_bot,
                cx,
            ))
            .child(
                Button::new("apply-db")
                    .primary()
                    .small()
                    .w_full()
                    .icon(Icon::new(IconName::Check))
                    .label("Apply Deadband")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.apply_deadband(cx);
                    })),
            );

        div().flex_1().min_w(px(260.0)).overflow_hidden().child(card.overflow_hidden())
    }

    fn main_socd_poll_card(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let pairs_label = if state.socd_pairs.is_empty() {
            "Pairs: none".to_string()
        } else {
            state
                .socd_pairs
                .iter()
                .map(|p| format!("0x{:02X}<->0x{:02X}", p.key1_hid, p.key2_hid))
                .collect::<Vec<_>>()
                .join(", ")
        };

        let poll_rates = [1u8, 2, 4, 8];
        let poll_idx = poll_rates
            .iter()
            .position(|&r| r == state.poll_rate_khz)
            .unwrap_or(0);

        let card = GroupBox::new()
            .outline()
            .title("PRCS / SOCD & Polling")
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_xs().child("SOCD Snap Priority"))
                    .child(
                        Switch::new("socd-enable-sw")
                            .small()
                            .checked(state.socd_enabled)
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.socd_enabled = *checked;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1p5()
                    .child(
                        Button::new("socd-clr")
                            .xsmall()
                            .outline()
                            .label("Clear")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.socd_pairs.clear();
                                    st.socd_enabled = false;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("socd-ad")
                            .xsmall()
                            .outline()
                            .label("A+D")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.socd_enabled = true;
                                    st.socd_pairs = vec![atrium_core::protocol::PrcsPair {
                                        key1_hid: 0x04,
                                        key2_hid: 0x07,
                                        model: 1,
                                    }];
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("socd-adws")
                            .xsmall()
                            .outline()
                            .label("A+D & W+S")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.socd_enabled = true;
                                    st.socd_pairs = vec![
                                        atrium_core::protocol::PrcsPair {
                                            key1_hid: 0x04,
                                            key2_hid: 0x07,
                                            model: 1,
                                        },
                                        atrium_core::protocol::PrcsPair {
                                            key1_hid: 0x1A,
                                            key2_hid: 0x16,
                                            model: 1,
                                        },
                                    ];
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("apply-socd")
                            .xsmall()
                            .primary()
                            .label("Apply SOCD")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.apply_socd(cx);
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(pairs_label),
            )
            .child(
                v_flex()
                    .pt_2()
                    .gap_1p5()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(div().text_xs().child("USB Poll Rate"))
                    .child(
                        TabBar::new("main-poll-tabs")
                            .segmented()
                            .small()
                            .selected_index(poll_idx)
                            .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                                if let Some(&khz) = [1u8, 2, 4, 8].get(*ix) {
                                    this.apply_poll_rate(khz, cx);
                                }
                            }))
                            .child(Tab::new().flex_1().label("1kHz"))
                            .child(Tab::new().flex_1().label("2kHz"))
                            .child(Tab::new().flex_1().label("4kHz"))
                            .child(Tab::new().flex_1().label("8kHz")),
                    ),
            );

        div().flex_1().min_w(px(260.0)).overflow_hidden().child(card.overflow_hidden())
    }

    pub(crate) fn slider_metric_row(
        &self,
        label: &str,
        value_txt: String,
        slider_state: &gpui_kit::Entity<gpui_kit::component::slider::SliderState>,
        cx: &mut Context<Self>,
    ) -> Div {
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .child(div().flex_1().min_w_0().truncate().child(label.to_string()))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(cx.theme().muted_foreground)
                            .child(value_txt),
                    ),
            )
            .child(
                Slider::new(slider_state)
                    .horizontal()
                    .accessibility_label(label.to_string()),
            )
    }
}

fn short_fw_segment(fw: &str) -> &str {
    fw.rsplit_once(",SI,").map_or(fw, |(_, tail)| tail)
}
