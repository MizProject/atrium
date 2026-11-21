//! Main workspace: identity strip, 61-key travel matrix, and 3-column workbench.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{Context, Div, IntoElement as _, ParentElement as _, Styled as _, div, px};

use reaula_core::keymap;
use reaula_core::protocol::TriggerMode;

use crate::ReaulaGui;
use crate::state::{ConnectedState, NavSection};
use crate::widgets::{KB_SCALE_DEFAULT, KeyboardCanvasMode, SWITCH_OPTIONS};

impl ReaulaGui {
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

        let identity_strip = DescriptionList::horizontal()
            .columns(4)
            .bordered(true)
            .small()
            .item("Device", state.model_label(), 1)
            .item("Firmware", short_fw_segment(fw).to_string(), 1)
            .item("Travel", range, 1)
            .item("Link / Poll", poll, 1)
            .item(
                "SOCD",
                if state.socd_enabled {
                    format!("Active ({} pair)", state.socd_pairs.len())
                } else {
                    "Off".into()
                },
                1,
            )
            .item("FW Slots", format!("{}", state.keyboard_fn_map.len()), 1)
            .item(
                "Daemon Rules",
                format!(
                    "{} ({})",
                    state.shortcut_rules.len(),
                    if state.shortcut_watcher_active { "On" } else { "Paused" }
                ),
                1,
            )
            .item(
                "Action",
                Button::new("main-open-diag")
                    .xsmall()
                    .outline()
                    .label("Diagnostics")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.switch_section(NavSection::Diagnostics, cx);
                    }))
                    .into_any_element(),
                1,
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
            .flex_wrap()
            .items_stretch()
            .gap_3()
            .child(self.main_actuation_card(state, cx))
            .child(self.main_switch_deadband_card(state, cx))
            .child(self.main_socd_poll_card(state, cx));

        v_flex()
            .w_full()
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

        div().flex_1().min_w(px(280.0)).child(
            card.child(
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

        div().flex_1().min_w(px(280.0)).child(card)
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
                                    st.socd_pairs = vec![reaula_core::protocol::PrcsPair {
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
                                        reaula_core::protocol::PrcsPair {
                                            key1_hid: 0x04,
                                            key2_hid: 0x07,
                                            model: 1,
                                        },
                                        reaula_core::protocol::PrcsPair {
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

        div().flex_1().min_w(px(280.0)).child(card)
    }

    pub(crate) fn slider_metric_row(
        &self,
        label: &str,
        value_txt: String,
        slider_state: &gpui_kit::Entity<gpui_kit::component::slider::SliderState>,
        cx: &mut Context<Self>,
    ) -> Div {
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .child(label.to_string())
                    .child(div().text_color(cx.theme().muted_foreground).child(value_txt)),
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
