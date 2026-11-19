//! Sidebar page renderers for `reaula-gui`.

use gpui::{
    Context, Div, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, ScrollDelta,
    ScrollWheelEvent, Stateful, Window, div, img, prelude::*, px, rgb, rgba,
};

use reaula_core::audio_fx::{AudioEffectStyle, AudioPalette, NUM_BANDS};
use reaula_core::diagnostics::{CheckStatus, DiagnosticLayer, DiagnosticReport};
use reaula_core::ipc::VirtualKey;
use reaula_core::keymap;
use reaula_core::protocol::{self, LIGHT_MAX_LEVEL, PrcsPair, TriggerMode};

use crate::ReaulaGui;
use crate::icons::{self, MaterialIcon};
use crate::kb_3d::{
    CameraPreset, KB3D_HEIGHT, KB3D_WIDTH, SWITCH_3D_HEIGHT, SWITCH_3D_WIDTH, SwitchCameraPreset,
};
use crate::state::{
    AUDIO_DISCLAIMER_COOLDOWN_SECS, ArrowClusterPreset, AudioSourceOption, ConnectedState,
    DropdownId, LightEditorTab, LightViewMode, NavSection, ShortcutActionPreset, SwitchViewMode,
    format_cap_pills, format_rgb_hex, offline_diagnostic_report, pack_rgb_u32,
    protocol_light_summary,
};
use crate::theme;
use crate::widgets::{
    ButtonTone, COLOR_SWATCHES, DropdownChoice, KB_SCALE_DEFAULT, KeyboardCanvasMode,
    SWITCH_OPTIONS, card_box, icon_btn, icon_pill, pill, primary_btn, recessed_well,
    render_switch_2d_cross_section, secondary_btn, stat_tile,
};

const AUDIO_EFFECT_OPTIONS: [(AudioEffectStyle, &str); 4] = [
    (AudioEffectStyle::Spectrum, "Spectrum Bars"),
    (AudioEffectStyle::Pulse, "Radial Beat Pulse"),
    (AudioEffectStyle::Wave, "Spectral Wave"),
    (AudioEffectStyle::Meter, "VU Level Meter"),
];
const AUDIO_PALETTE_OPTIONS: [(AudioPalette, &str); 5] = [
    (AudioPalette::Aurora, "Aurora Gradient"),
    (AudioPalette::Regal, "Regal Navy Frost"),
    (AudioPalette::Neon, "Neon Cyber"),
    (AudioPalette::Inferno, "Inferno Ember"),
    (AudioPalette::Custom, "Custom FG / BG"),
];
const AUDIO_FPS_OPTIONS: [u8; 5] = [20, 30, 40, 50, 60];

const LIGHT_MODES: [u8; 16] = [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 100];
const LIGHT_DIRECTIONS: [(u8, &str); 4] =
    [(0, "Right ->"), (1, "Left <-"), (2, "Down v"), (3, "Up ^")];
const ARROW_CLUSTER_OPTIONS: [(ArrowClusterPreset, &str); 3] = [
    (ArrowClusterPreset::Rdfg, "Fn + R/D/F/G Arrows"),
    (ArrowClusterPreset::Tfgh, "Fn + T/F/G/H Arrows"),
    (ArrowClusterPreset::Wasd, "Fn + W/A/S/D Arrows"),
];
const HOTKEY_MODIFIER_OPTIONS: [(Option<u8>, &str); 6] = [
    (None, "None (Single Key)"),
    (Some(keymap::idx::FN), "Fn (Default)"),
    (Some(keymap::idx::CAPS), "Caps Lock"),
    (Some(keymap::idx::R_CTRL), "Right Ctrl"),
    (Some(keymap::idx::L_ALT), "Left Alt"),
    (Some(keymap::idx::MENU), "Menu Key"),
];
const QUICK_TRIGGER_KEYS: [u8; 11] = [
    keymap::idx::R,
    keymap::idx::T,
    keymap::idx::G,
    keymap::idx::F,
    keymap::idx::H,
    keymap::idx::W,
    keymap::idx::A,
    keymap::idx::S,
    keymap::idx::D,
    keymap::idx::ESC,
    keymap::idx::R_CTRL,
];

impl ReaulaGui {
    pub(crate) fn render_active_section(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        match self.nav {
            NavSection::Diagnostics => self.view_diagnostics(cx),
            NavSection::Guide => self.view_guide(),
            _ => match &self.conn {
                Err(err) => self.view_disconnected(err),
                Ok(state) => match self.nav {
                    NavSection::Main => self.view_main(state, cx),
                    NavSection::Calibration => self.view_calibration(state, cx),
                    NavSection::ColorLight => self.view_color_light(state, cx),
                    NavSection::Hotkeys => self.view_hotkeys(state, cx),
                    NavSection::Guide | NavSection::Diagnostics => unreachable!(),
                },
            },
        }
    }

    fn view_disconnected(&self, err: &str) -> Div {
        card_box(
            "Daemon Not Connected",
            Some("Run Diagnostics or start `reaulad` to communicate with your WIN 60 HE"),
        )
        .child(
            recessed_well()
                .border_color(rgb(theme::BORDER_DANGER))
                .text_xs()
                .text_color(rgb(theme::INK_DANGER))
                .child(err.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(theme::INK_SECONDARY))
                .child("Start daemon: cargo run -p reaula-cli -- daemon start (then click Reconnect in the sidebar)."),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(theme::INK_MUTED))
                .child("Tip: Open the Diagnostics tab in the sidebar to isolate whether Kernel/USB or Daemon IPC failed."),
        )
    }

    // -----------------------------------------------------------------------
    // 1. MAIN: Identity, Status & Full Key Configuration Deck
    // -----------------------------------------------------------------------
    fn view_main(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let caps = format_cap_pills(&state.probe);
        let fw = state.probe.firmware_string.as_deref().unwrap_or("Unknown");
        let build = state.probe.build_date.as_deref().unwrap_or("Unknown");
        let travel_txt = state.probe.max_travel.map_or_else(
            || "0.08..=3.40 mm".into(),
            |m| {
                format!(
                    "{:.2}..={:.2} mm ({:.3} step)",
                    m.min_mm, m.max_mm, m.unit_mm
                )
            },
        );
        let focus_name = keymap::by_index(state.focused_key).map_or("Key", |k| k.name);
        let last_event = state.feedback_log.front().map_or_else(
            || "No commands dispatched yet".into(),
            |ev| {
                format!(
                    "[{} {:.2} ms] {} -> {}",
                    if ev.ok { "OK" } else { "ERR" },
                    ev.rtt_ms,
                    ev.label,
                    ev.detail
                )
            },
        );

        let identity_card = card_box(
            "Aula WIN 60 HE — Hardware Identity & Live Status",
            Some(&format!(
                "SO_PEERCRED uid={} pid={} | {}",
                state.server_cred.uid,
                state.daemon_status.daemon_pid,
                state.daemon_status.socket_path
            )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(stat_tile(
                    "Keyboard Model",
                    "AULA WIN 60 HE (2E3C:C365)",
                    theme::INK_PRIMARY,
                ))
                .child(stat_tile(
                    "Firmware String",
                    fw.to_string(),
                    theme::INK_SECONDARY,
                ))
                .child(stat_tile(
                    "Firmware Build",
                    build.to_string(),
                    theme::INK_SECONDARY,
                ))
                .child(stat_tile(
                    "Connection & Poll",
                    format!(
                        "{} @ {} kHz ({} Hz)",
                        state
                            .probe
                            .init_info
                            .as_ref()
                            .map_or("USB", |i| i.connection_mode()),
                        state.poll_rate_khz,
                        u16::from(state.poll_rate_khz) * 1000
                    ),
                    theme::INK_OK,
                ))
                .child(stat_tile(
                    "Analog Travel Range",
                    travel_txt,
                    theme::INK_ACCENT,
                ))
                .child(stat_tile(
                    "Active RGB Lighting",
                    protocol_light_summary(&state.light_state),
                    theme::INK_PRIMARY,
                ))
                .child(stat_tile(
                    "PRCS / SOCD",
                    if state.socd_enabled {
                        format!("Enabled ({} pair(s))", state.socd_pairs.len())
                    } else {
                        format!("Disabled ({} stored pair(s))", state.socd_pairs.len())
                    },
                    if state.socd_enabled {
                        theme::INK_OK
                    } else {
                        theme::INK_MUTED
                    },
                ))
                .child(stat_tile(
                    "Daemon Hotkeys",
                    format!(
                        "{} rule(s) ({})",
                        state.shortcut_rules.len(),
                        if state.shortcut_watcher_active {
                            "WATCHING"
                        } else {
                            "PAUSED"
                        }
                    ),
                    if state.shortcut_watcher_active {
                        theme::INK_ACCENT
                    } else {
                        theme::INK_MUTED
                    },
                )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_1p5()
                .children(caps.into_iter().map(|(label, supported)| {
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .bg(rgb(if supported {
                            theme::SHADE_REGAL
                        } else {
                            theme::SHADE_DEEP
                        }))
                        .border_1()
                        .border_color(rgb(if supported {
                            theme::BORDER_REGAL
                        } else {
                            theme::BORDER_SUBTLE
                        }))
                        .text_size(px(10.0))
                        .text_color(rgb(if supported {
                            theme::INK_PRIMARY
                        } else {
                            theme::INK_DIM
                        }))
                        .child(format!(
                            "{}: {}",
                            label,
                            if supported { "YES" } else { "NO" }
                        ))
                })),
        )
        .child(
            recessed_well()
                .p_2p5()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::INK_SECONDARY))
                        .child(format!("Last Response Feedback: {last_event}")),
                )
                .child(
                    secondary_btn("main-go-diag", "Open Diagnostics Bench").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.switch_section(NavSection::Diagnostics, cx);
                        },
                    )),
                ),
        );

        let selection_bar = div()
            .w_full()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w(px(260.0))
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child(format!(
                        "Selected: {} key(s) | Focused: {} (slot {}) — Shift/Ctrl+click multi-select",
                        state.selected_keys.len(),
                        focus_name,
                        state.focused_key
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill("sel-all", "All (61)", false).on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            st.selected_keys = keymap::all_indices().into_iter().collect();
                        }
                        cx.notify();
                    })))
                    .child(pill("sel-wasd", "WASD", false).on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            st.selected_keys = [46, 68, 69, 70].into_iter().collect();
                            st.sync_draft_from_key(46);
                        }
                        cx.notify();
                    })))
                    .child(pill("sel-single", "Focused Only", false).on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            let f = st.focused_key;
                            st.selected_keys = [f].into_iter().collect();
                        }
                        cx.notify();
                    })))
                    .child(
                        icon_pill("zero-stuck", Some(MaterialIcon::RestartAlt), "Zero Travel", false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.clear_stuck_travel();
                                    this.status_line = "Cleared cached live travel readings.".into();
                                }
                                cx.notify();
                            })),
                    ),
            );

        let keymap_card = card_box(
            "60% Key Matrix & Live Travel Monitor",
            Some(if state.travel_enabled {
                "Live Travel STREAMING (0.01 mm)"
            } else {
                "Live Travel PAUSED"
            }),
        )
        .child(selection_bar)
        .child(self.render_keyboard_canvas(
            state,
            KeyboardCanvasMode::ActuationAndTravel,
            KB_SCALE_DEFAULT,
            0.0,
            cx,
        ));

        let control_deck = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_4()
            .child(self.main_actuation_card(state, cx))
            .child(self.main_switch_deadband_card(state, cx))
            .child(self.main_socd_poll_card(state, cx));

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(identity_card)
            .child(keymap_card)
            .child(control_deck)
    }

    fn main_actuation_card(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let mode = state.draft_mode;
        let show_rt = mode != TriggerMode::Fixed;
        let show_split = mode == TriggerMode::RapidTriggerSplit;

        let mut card =
            card_box("Actuation & Rapid Trigger", None)
                .flex_1()
                .min_w(px(295.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(pill("m-fix", "Fixed", mode == TriggerMode::Fixed).on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.draft_mode = TriggerMode::Fixed;
                                }
                                cx.notify();
                            }),
                        ))
                        .child(
                            pill("m-rt", "Rapid Trigger", mode == TriggerMode::RapidTrigger)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.draft_mode = TriggerMode::RapidTrigger;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            pill("m-rts", "RT Split", mode == TriggerMode::RapidTriggerSplit)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.draft_mode = TriggerMode::RapidTriggerSplit;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(self.mm_stepper_row(
                    "Actuation Point",
                    state.draft_travel_mm,
                    [-0.20, -0.05, 0.05, 0.20],
                    cx,
                    |st, d| {
                        st.draft_travel_mm = (st.draft_travel_mm + d).clamp(st.min_mm, st.max_mm);
                    },
                ));

        if show_rt {
            card = card.child(self.mm_stepper_row(
                if show_split {
                    "RT Press"
                } else {
                    "RT Sensitivity"
                },
                state.draft_rt_press_mm,
                [-0.10, -0.02, 0.02, 0.10],
                cx,
                |st, d| {
                    st.draft_rt_press_mm = (st.draft_rt_press_mm + d).clamp(0.01, 2.50);
                },
            ));
        }
        if show_split {
            card = card.child(self.mm_stepper_row(
                "RT Release",
                state.draft_rt_release_mm,
                [-0.10, -0.02, 0.02, 0.10],
                cx,
                |st, d| {
                    st.draft_rt_release_mm = (st.draft_rt_release_mm + d).clamp(0.01, 2.50);
                },
            ));
        }

        card.child(
            icon_btn(
                "apply-actuation",
                Some(MaterialIcon::Check),
                "Apply Actuation to Selected",
                ButtonTone::Primary,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.apply_actuation(cx);
            })),
        )
    }

    fn main_switch_deadband_card(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        card_box("Switch Profile & Deadband", Some("Safe RMW"))
            .flex_1()
            .min_w(px(295.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .children(SWITCH_OPTIONS.iter().map(|&(id, label)| {
                        pill(
                            ("sw-opt", usize::from(id)),
                            label,
                            state.draft_switch_id == id,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.apply_switch_profile(id, cx);
                        }))
                    })),
            )
            .child(self.mm_stepper_row(
                "Top Deadband",
                state.draft_db_top_mm,
                [-0.05, -0.01, 0.01, 0.05],
                cx,
                |st, d| {
                    st.draft_db_top_mm = (st.draft_db_top_mm + d).clamp(0.0, 1.0);
                },
            ))
            .child(self.mm_stepper_row(
                "Bot Deadband",
                state.draft_db_bottom_mm,
                [-0.05, -0.01, 0.01, 0.05],
                cx,
                |st, d| {
                    st.draft_db_bottom_mm = (st.draft_db_bottom_mm + d).clamp(0.0, 1.0);
                },
            ))
            .child(
                icon_btn(
                    "apply-db",
                    Some(MaterialIcon::Check),
                    "Apply Deadband to Selected",
                    ButtonTone::Primary,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.apply_deadband(cx);
                })),
            )
    }

    fn main_socd_poll_card(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let pairs_label = if state.socd_pairs.is_empty() {
            "Pairs: (none configured)".to_string()
        } else {
            state
                .socd_pairs
                .iter()
                .map(|p| {
                    format!(
                        "HID 0x{:02X}<->0x{:02X} (m={})",
                        p.key1_hid, p.key2_hid, p.model
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };

        card_box("PRCS / SOCD & USB Polling", None)
            .flex_1()
            .min_w(px(295.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::INK_SECONDARY))
                            .child("SOCD Mode"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(pill("socd-on", "Enabled", state.socd_enabled).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.socd_enabled = true;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(pill("socd-off", "Disabled", !state.socd_enabled).on_click(
                                cx.listener(|this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.socd_enabled = false;
                                    }
                                    cx.notify();
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill("socd-clr", "Clear Pairs", false).on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.socd_pairs.clear();
                                st.socd_enabled = false;
                            }
                            cx.notify();
                        },
                    )))
                    .child(pill("socd-ad", "A+D Last-Win", false).on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.socd_enabled = true;
                                st.socd_pairs = vec![PrcsPair {
                                    key1_hid: 0x04,
                                    key2_hid: 0x07,
                                    model: 1,
                                }];
                            }
                            cx.notify();
                        },
                    )))
                    .child(pill("socd-adws", "A+D & W+S", false).on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.socd_enabled = true;
                                st.socd_pairs = vec![
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
                            cx.notify();
                        },
                    )))
                    .child(
                        primary_btn("apply-socd", "Apply SOCD").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.apply_socd(cx);
                            },
                        )),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child(pairs_label),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::INK_SECONDARY))
                            .child("USB Polling"),
                    )
                    .child(div().flex().flex_row().gap_1().children(
                        [1u8, 2, 4, 8].into_iter().map(|khz| {
                            pill(
                                ("poll-rate", usize::from(khz)),
                                format!("{khz}kHz"),
                                state.poll_rate_khz == khz,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.apply_poll_rate(khz, cx);
                                },
                            ))
                        }),
                    )),
            )
    }

    fn mm_stepper_row<F>(
        &self,
        label: &'static str,
        value_mm: f64,
        steps: [f64; 4],
        cx: &mut Context<Self>,
        update: F,
    ) -> Div
    where
        F: Fn(&mut ConnectedState, f64) + Copy + 'static,
    {
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_SECONDARY))
                    .child(label),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        pill((label, 0usize), format!("{:+.2}", steps[0]), false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    update(st, steps[0]);
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        pill((label, 1usize), format!("{:+.2}", steps[1]), false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    update(st, steps[1]);
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        div()
                            .w(px(62.0))
                            .flex()
                            .justify_center()
                            .text_xs()
                            .text_color(rgb(theme::INK_PRIMARY))
                            .child(format!("{value_mm:.2} mm")),
                    )
                    .child(
                        pill((label, 2usize), format!("{:+.2}", steps[2]), false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    update(st, steps[2]);
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        pill((label, 3usize), format!("{:+.2}", steps[3]), false).on_click(
                            cx.listener(move |this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    update(st, steps[3]);
                                }
                                cx.notify();
                            }),
                        ),
                    ),
            )
    }

    // -----------------------------------------------------------------------
    // 2. GUIDE: User Manual & Hardware Mechanics Reference
    // -----------------------------------------------------------------------
    fn view_guide(&self) -> Div {
        let sections: [(&str, &[&str]); 5] = [
            (
                "1. Hall-Effect Actuation, Rapid Trigger & Deadband",
                &[
                    "- Actuation Point (0.08..=3.40 mm, 0.010 mm steps): Controls exact magnet travel depth where a Fixed key registers.",
                    "- Rapid Trigger (RT / RT Split): Dynamically re-arms reset & press anywhere in keystroke travel rather than a fixed point.",
                    "- Top & Bottom Deadband: Adds safe travel hysteresis near rest (0.00 mm) and bottom-out (3.40+ mm) to prevent double taps.",
                    "- Switch Profiles (HM1/HH1/CY1/TC1): Selects factory magnetic flux curves per key via safe Read-Modify-Write.",
                ],
            ),
            (
                "2. Hardware Fn Key Shortcuts (Built Into WIN 60 HE Firmware)",
                &[
                    "- Fn + \\|: Cycle onboard RGB lighting effects directly on the keyboard.",
                    "- Fn + Up / Down (or Right-Ctrl / Fn+[{ / ]}): Adjust hardware LED brightness.",
                    "- Fn + Left / Right: Adjust dynamic RGB animation speed.",
                    "- Fn + Win: Toggle Windows/Super key lock during games.",
                    "- Note: Hardware factory reset is locked out by Reaula's protocol safety guard to protect your calibration.",
                ],
            ),
            (
                "3. Sensor Calibration Safety Protocol",
                &[
                    "- Run Calibration after swapping switches (HM1/HH1/CY1/TC1) or if rest travel reads non-zero or stops short of 3.40 mm.",
                    "- All-Key Calibration wipes current calibration offsets in firmware RAM until every key is pressed smoothly to full bottom-out.",
                    "- Any-Key Calibration lets you re-zero specific keys only. Reaula automatically sends Disarm/Stop on tab switch or window close.",
                ],
            ),
            (
                "4. Daemon Hotkeys & SO_PEERCRED Security Architecture",
                &[
                    "- Hardware USB HID (`/dev/hidraw3`) stays owned solely by `reaulad` inside `/run/reaula` (`0700` dir, `0600` socket).",
                    "- Kernel `SO_PEERCRED` verifies every client UID/PID before reading bytes — zero raw sockets or root GUI privileges needed.",
                    "- Daemon-memory Hotkeys watch hardware live travel edges inside `reaulad`. External commands always drop UID/GID back to your non-root user.",
                ],
            ),
            (
                "5. Using 4-Layer Diagnostics to Isolate Faults",
                &[
                    "- Kernel / USB: Checks `/sys/class/hidraw/*`, USB VID:PID `2E3C:C365`, driver binding, and HID report descriptor (UsagePage 0xFF1B).",
                    "- Daemon / IPC: Verifies Unix socket reachability, `SO_PEERCRED` peer auth, and `/dev/hidraw` open status.",
                    "- Keyboard Hardware: Measures live heartbeat (`0x01`), FW info (`0x0D`), lighting (`0x07`), and multi-chunk table transfer integrity.",
                    "- Software / Codec: Runs deterministic internal 61-key bitmask, trigger packet, and safety guard checks.",
                ],
            ),
        ];

        let mut root = div().flex().flex_col().gap_3();
        root = root.child(
            card_box(
                "Aula WIN 60 HE & Reaula Driver Guide",
                Some("Everything you need to tune, calibrate, light, and diagnose your board"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child("Quick start: Use Main to configure actuation/SOCD, Color & Light for 3D + 2D RGB previews, Hotkeys for onboard Fn flash & daemon backup, and Diagnostics if keys misbehave."),
            ),
        );

        for (title, bullets) in sections {
            let mut card = card_box(title, None);
            for line in bullets {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::INK_SECONDARY))
                        .child((*line).to_string()),
                );
            }
            root = root.child(card);
        }
        root
    }

    // -----------------------------------------------------------------------
    // 3. CALIBRATION: Guarded Sensor Calibration View
    // -----------------------------------------------------------------------
    fn view_calibration(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let done_count = state.cal_calibrated.len();
        let warn_banner = div()
            .p_3()
            .rounded_lg()
            .bg(rgb(if state.cal_active { theme::SHADE_REGAL } else { theme::SHADE_DEEP }))
            .border_1()
            .border_color(rgb(if state.cal_active { theme::BORDER_WARN } else { theme::BORDER_SUBTLE }))
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if state.cal_active { theme::INK_WARN } else { theme::INK_PRIMARY }))
                    .child(format!("Calibration State: {}", state.cal_status_text)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child("Leave all keys untouched before starting. Leaving this tab or closing the window automatically disarms firmware calibration."),
            );

        let mut controls = div().flex().flex_row().flex_wrap().gap_2();
        if !state.cal_active {
            controls = controls
                .child(
                    icon_btn(
                        "cal-all",
                        Some(MaterialIcon::Tune),
                        "Start All-Key Calibration",
                        ButtonTone::Primary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.start_cal(false, cx);
                    })),
                )
                .child(
                    icon_btn(
                        "cal-any",
                        Some(MaterialIcon::Sensors),
                        "Start Any-Key Calibration",
                        ButtonTone::Secondary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.start_cal(true, cx);
                    })),
                );
        } else {
            controls = controls
                .child(
                    icon_btn(
                        "cal-stop",
                        Some(MaterialIcon::Check),
                        "Finish & Disarm Calibration",
                        ButtonTone::Primary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.stop_cal(cx);
                    })),
                )
                .child(
                    icon_btn(
                        "cal-abort",
                        Some(MaterialIcon::DeleteOutline),
                        "Abort Calibration",
                        ButtonTone::Danger,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.stop_cal(cx);
                    })),
                );
        }

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                card_box(
                    "Hall-Effect Switch Travel Calibration",
                    Some(&format!(
                        "{done_count} / {} keys recorded",
                        keymap::WIN60HE_KEYS.len()
                    )),
                )
                .child(warn_banner)
                .child(controls),
            )
            .child(
                card_box(
                    "Live Calibration Matrix Progress",
                    Some("Highlighted Regal Navy keys have completed rest + bottom-out sampling"),
                )
                .child(self.render_keyboard_canvas(
                    state,
                    KeyboardCanvasMode::Calibration,
                    KB_SCALE_DEFAULT,
                    0.0,
                    cx,
                )),
            )
    }

    // -----------------------------------------------------------------------
    // 4. COLOR & LIGHT: Unified Stage ([2D | 3D] Switch) + Dropdown Editors
    // -----------------------------------------------------------------------
    fn view_color_light(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let time_s = self.start_instant.elapsed().as_secs_f32();
        let ls = &state.light_state;
        let view_mode = state.light_view_mode;

        let active_layer_label = if state.audio_active {
            format!(
                "Audio Sync · {}",
                state.audio_config.style.label()
            )
        } else {
            match state.light_tab {
                LightEditorTab::FirmwareEffects => {
                    format!("Effect · {}", protocol::light_mode_name(ls.mode))
                }
                LightEditorTab::CustomSlot1 => "Custom Matrix · Slot 1".into(),
                LightEditorTab::CustomSlot2 => "Custom Matrix · Slot 2".into(),
                LightEditorTab::AudioSync => "Audio Sync (Experimental)".into(),
            }
        };

        let view_switch = div()
            .p_0p5()
            .rounded_lg()
            .bg(rgb(theme::SHADE_DEEP))
            .border_1()
            .border_color(rgb(theme::BORDER_MATTE))
            .flex()
            .flex_row()
            .gap_1()
            .child(
                icon_pill(
                    "view-mode-2d",
                    Some(MaterialIcon::GridView),
                    "2D",
                    view_mode == LightViewMode::TwoD,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Ok(st) = &mut this.conn {
                        st.light_view_mode = LightViewMode::TwoD;
                    }
                    cx.notify();
                })),
            )
            .child(
                icon_pill(
                    "view-mode-3d",
                    Some(MaterialIcon::ViewInAr),
                    "3D",
                    view_mode == LightViewMode::ThreeD,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Ok(st) = &mut this.conn {
                        st.light_view_mode = LightViewMode::ThreeD;
                        this.dirty_3d = true;
                    }
                    this.maybe_render_3d();
                    cx.notify();
                })),
            );

        let stage_controls = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        pill(
                            "ltab-fw",
                            "Firmware Effect",
                            state.light_tab == LightEditorTab::FirmwareEffects,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.light_tab = LightEditorTab::FirmwareEffects;
                                this.dirty_3d = true;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        pill(
                            "ltab-c1",
                            "Custom Slot 1",
                            state.light_tab == LightEditorTab::CustomSlot1,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.light_tab = LightEditorTab::CustomSlot1;
                                this.dirty_3d = true;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        pill(
                            "ltab-c2",
                            "Custom Slot 2",
                            state.light_tab == LightEditorTab::CustomSlot2,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.light_tab = LightEditorTab::CustomSlot2;
                                this.dirty_3d = true;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_pill(
                            "ltab-audio",
                            Some(MaterialIcon::GraphicEq),
                            if state.audio_active {
                                "Audio Sync · LIVE"
                            } else {
                                "Audio Sync"
                            },
                            state.light_tab == LightEditorTab::AudioSync,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.light_tab = LightEditorTab::AudioSync;
                                this.dirty_3d = true;
                            }
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .when(view_mode == LightViewMode::ThreeD, |d| {
                        d.children(
                            [
                                ("cam-iso", CameraPreset::Perspective, "Iso"),
                                ("cam-top", CameraPreset::Top, "Top"),
                                ("cam-side", CameraPreset::Side, "Side"),
                                ("cam-front", CameraPreset::Front, "Front"),
                            ]
                            .into_iter()
                            .map(|(id, preset, lbl)| {
                                icon_pill(
                                    id,
                                    (preset == CameraPreset::Perspective)
                                        .then_some(MaterialIcon::Videocam),
                                    lbl,
                                    state.camera_preset == Some(preset),
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.set_camera_preset(preset, cx);
                                    },
                                ))
                            }),
                        )
                    })
                    .child(
                        icon_pill(
                            "anim-stage",
                            Some(if state.animate_3d {
                                MaterialIcon::Pause
                            } else {
                                MaterialIcon::PlayArrow
                            }),
                            if state.animate_3d {
                                "Animate"
                            } else {
                                "Paused"
                            },
                            state.animate_3d,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.animate_3d = !st.animate_3d;
                                this.dirty_3d = true;
                            }
                            cx.notify();
                        })),
                    )
                    .when(
                        view_mode == LightViewMode::TwoD
                            && state.light_tab.custom_slot_index().is_some(),
                        |d| {
                            d.child(
                                icon_pill(
                                    "stage-paint-toggle",
                                    Some(MaterialIcon::Brush),
                                    if state.custom_paint_mode {
                                        "Paint Clicks: ON"
                                    } else {
                                        "Paint Clicks: OFF"
                                    },
                                    state.custom_paint_mode,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Ok(st) = &mut this.conn {
                                            st.custom_paint_mode = !st.custom_paint_mode;
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                        },
                    )
                    .child(
                        icon_pill(
                            "typing-only-3d",
                            Some(MaterialIcon::Sensors),
                            if state.typing_only_effects {
                                "Calm Idle: ON"
                            } else {
                                "Calm Idle: OFF"
                            },
                            state.typing_only_effects,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.typing_only_effects = !st.typing_only_effects;
                                this.dirty_3d = true;
                            }
                            cx.notify();
                        })),
                    )
                    .child(view_switch),
            );

        let stage_body = match view_mode {
            LightViewMode::ThreeD => match (&self.renderer_3d, &self.preview_3d_img) {
                (Ok(_), Some(frame)) => div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("kb-3d-viewport")
                            .w(px(KB3D_WIDTH as f32 * 0.90))
                            .h(px(KB3D_HEIGHT as f32 * 0.90))
                            .rounded_lg()
                            .overflow_hidden()
                            .border_1()
                            .border_color(rgb(theme::BORDER_SUBTLE))
                            .cursor_grab()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    this.dragging_3d = true;
                                    this.last_mouse_3d = Some((f32::from(ev.position.x), f32::from(ev.position.y)));
                                    cx.notify();
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| {
                                    this.dragging_3d = false;
                                    this.last_mouse_3d = None;
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| {
                                    this.dragging_3d = false;
                                    this.last_mouse_3d = None;
                                }),
                            )
                            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                                if this.dragging_3d && ev.dragging() {
                                    let nx = f32::from(ev.position.x);
                                    let ny = f32::from(ev.position.y);
                                    if let Some((lx, ly)) = this.last_mouse_3d
                                        && let Ok(st) = &mut this.conn
                                    {
                                        st.camera_3d.orbit(nx - lx, ny - ly);
                                        st.camera_preset = None;
                                        this.dirty_3d = true;
                                    }
                                    this.last_mouse_3d = Some((nx, ny));
                                    cx.notify();
                                }
                            }))
                            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                                let dy = match ev.delta {
                                    ScrollDelta::Lines(p) => p.y * 0.45,
                                    ScrollDelta::Pixels(p) => f32::from(p.y) * 0.01,
                                };
                                if let Ok(st) = &mut this.conn {
                                    st.camera_3d.adjust_zoom(-dy);
                                    st.camera_preset = None;
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            }))
                            .child(img(frame.clone()).w_full().h_full()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(theme::INK_MUTED))
                            .child(format!(
                                "Drag to orbit · Scroll wheel to zoom · yaw={:.2} pitch={:.2} zoom={:.1}",
                                state.camera_3d.yaw, state.camera_3d.pitch, state.camera_3d.zoom
                            )),
                    )
                    .into_any_element(),
                (Ok(_), None) => recessed_well()
                    .w(px(748.0))
                    .h(px(320.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child("Initializing wgpu 3D WIN 60 HE stage...")
                    .into_any_element(),
                (Err(err), _) => recessed_well()
                    .border_color(rgb(theme::BORDER_DANGER))
                    .text_xs()
                    .text_color(rgb(theme::INK_DANGER))
                    .child(format!("3D wgpu unavailable ({err}). Switch to 2D view above."))
                    .into_any_element(),
            },
            LightViewMode::TwoD => div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .child(self.render_keyboard_canvas(
                    state,
                    KeyboardCanvasMode::ColorMockup,
                    KB_SCALE_DEFAULT,
                    time_s,
                    cx,
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::INK_MUTED))
                        .child(if state.light_tab.custom_slot_index().is_some() {
                            if state.custom_paint_mode {
                                format!(
                                    "Click keys to paint {} · Right-click any key to eyedrop color",
                                    format_rgb_hex(state.custom_brush)
                                )
                            } else {
                                "Click selects keys · Shift+Click multi-select · Right-click samples key color".into()
                            }
                        } else {
                            "Live 2D top-down LED projection · Toggle Custom Slot above to paint per-key colors".into()
                        }),
                )
                .into_any_element(),
        };

        let stage_telemetry = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_3()
            .child(stat_tile("Active Layer", active_layer_label, theme::INK_PRIMARY).flex_1())
            .child(
                stat_tile(
                    "Brightness",
                    format!("{} / {}", ls.brightness, LIGHT_MAX_LEVEL),
                    theme::INK_ACCENT,
                )
                .flex_1(),
            )
            .child(
                stat_tile(
                    "Animation Speed",
                    format!("{} / {}", ls.speed, LIGHT_MAX_LEVEL),
                    theme::INK_ACCENT,
                )
                .flex_1(),
            )
            .child(
                stat_tile(
                    "Color / Power",
                    format!(
                        "{} · {}",
                        if ls.full_color {
                            "Rainbow".into()
                        } else {
                            format_rgb_hex(ls.fg)
                        },
                        if ls.power_on { "ON" } else { "OFF" }
                    ),
                    if ls.power_on {
                        theme::INK_OK
                    } else {
                        theme::INK_MUTED
                    },
                )
                .flex_1(),
            );

        let stage_card = card_box(
            "WIN 60 HE Lighting Studio",
            Some(if view_mode == LightViewMode::ThreeD {
                "3D Interactive Preview"
            } else {
                "2D Top-Down Matrix"
            }),
        )
        .child(stage_controls)
        .child(recessed_well().flex().justify_center().child(stage_body))
        .child(stage_telemetry);

        let editor_card = match state.light_tab {
            LightEditorTab::FirmwareEffects => self.render_firmware_light_editor(state, cx),
            LightEditorTab::CustomSlot1 => self.render_custom_slot_editor(state, 0, cx),
            LightEditorTab::CustomSlot2 => self.render_custom_slot_editor(state, 1, cx),
            LightEditorTab::AudioSync => self.render_audio_sync_editor(state, cx),
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(stage_card)
            .child(editor_card)
    }

    fn render_firmware_light_editor(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let ls = &state.light_state;

        let effect_choices: Vec<DropdownChoice> = LIGHT_MODES
            .iter()
            .map(|&m| DropdownChoice {
                label: protocol::light_mode_name(m).into(),
                swatch: None,
                icon: Some(MaterialIcon::Palette),
                selected: ls.mode == m,
            })
            .collect();

        let dir_label = LIGHT_DIRECTIONS
            .iter()
            .find(|(d, _)| *d == ls.direction)
            .map_or("Right ->", |(_, l)| *l);
        let dir_choices: Vec<DropdownChoice> = LIGHT_DIRECTIONS
            .iter()
            .map(|&(d, l)| DropdownChoice {
                label: l.into(),
                swatch: None,
                icon: None,
                selected: ls.direction == d,
            })
            .collect();

        let top_controls =
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap_4()
                .child(self.render_dropdown(
                    DropdownId::LightEffect,
                    Some("Lighting Effect"),
                    protocol::light_mode_name(ls.mode),
                    None,
                    Some(MaterialIcon::Palette),
                    200.0,
                    effect_choices,
                    |this, idx, _| {
                        if let Ok(st) = &mut this.conn {
                            st.light_state.mode = LIGHT_MODES[idx];
                            st.light_state.power_on = true;
                            this.dirty_3d = true;
                        }
                    },
                    cx,
                ))
                .child(self.render_dropdown(
                    DropdownId::LightDirection,
                    Some("Direction"),
                    dir_label,
                    None,
                    None,
                    135.0,
                    dir_choices,
                    |this, idx, _| {
                        if let Ok(st) = &mut this.conn {
                            st.light_state.direction = LIGHT_DIRECTIONS[idx].0;
                            this.dirty_3d = true;
                        }
                    },
                    cx,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(rgb(theme::INK_MUTED))
                                .child("Brightness"),
                        )
                        .child(div().flex().flex_row().gap_1().children(
                            (0..=LIGHT_MAX_LEVEL).map(|b| {
                                pill(("lbri", usize::from(b)), format!("{b}"), ls.brightness == b)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Ok(st) = &mut this.conn {
                                            st.light_state.brightness = b;
                                            this.dirty_3d = true;
                                        }
                                        cx.notify();
                                    }))
                            }),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(rgb(theme::INK_MUTED))
                                .child("Speed"),
                        )
                        .child(div().flex().flex_row().gap_1().children(
                            (0..=LIGHT_MAX_LEVEL).map(|s| {
                                pill(("lspd", usize::from(s)), format!("{s}"), ls.speed == s)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Ok(st) = &mut this.conn {
                                            st.light_state.speed = s;
                                            this.dirty_3d = true;
                                        }
                                        cx.notify();
                                    }))
                            }),
                        )),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .pb_0p5()
                        .child(
                            pill(
                                "l-fullcol",
                                if ls.full_color {
                                    "Rainbow: ON"
                                } else {
                                    "Rainbow: OFF"
                                },
                                ls.full_color,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.light_state.full_color = !st.light_state.full_color;
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                        )
                        .child(
                            icon_pill(
                                "l-power",
                                Some(MaterialIcon::PowerSettings),
                                if ls.power_on { "LED: ON" } else { "LED: OFF" },
                                ls.power_on,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.light_state.power_on = !st.light_state.power_on;
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                        ),
                );

        let color_wells = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_4()
            .child(self.render_color_channel_well(
                DropdownId::LightFgPreset,
                "fg",
                "Foreground Color",
                ls.fg,
                |this, rgb_val| {
                    if let Ok(st) = &mut this.conn {
                        st.light_state.fg = rgb_val;
                        st.light_state.full_color = false;
                        this.dirty_3d = true;
                    }
                },
                |this, ch, delta| {
                    if let Ok(st) = &mut this.conn {
                        let cur = st.light_state.fg[ch] as i16;
                        st.light_state.fg[ch] = (cur + delta).clamp(0, 255) as u8;
                        st.light_state.full_color = false;
                        this.dirty_3d = true;
                    }
                },
                cx,
            ))
            .child(self.render_color_channel_well(
                DropdownId::LightBgPreset,
                "bg",
                "Background Color",
                ls.bg,
                |this, rgb_val| {
                    if let Ok(st) = &mut this.conn {
                        st.light_state.bg = rgb_val;
                        this.dirty_3d = true;
                    }
                },
                |this, ch, delta| {
                    if let Ok(st) = &mut this.conn {
                        let cur = st.light_state.bg[ch] as i16;
                        st.light_state.bg[ch] = (cur + delta).clamp(0, 255) as u8;
                        this.dirty_3d = true;
                    }
                },
                cx,
            ));

        card_box(
            "Firmware Lighting Configurator",
            Some("Changes preview immediately; apply to persist on keyboard"),
        )
        .child(top_controls)
        .child(color_wells)
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    icon_btn(
                        "apply-fw-light",
                        Some(MaterialIcon::Save),
                        "Apply Lighting to Keyboard",
                        ButtonTone::Primary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.apply_lighting(cx);
                    })),
                )
                .child(
                    icon_btn(
                        "reload-fw-light",
                        Some(MaterialIcon::Refresh),
                        "Read Current Lighting",
                        ButtonTone::Secondary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.reload_lighting(cx);
                    })),
                ),
        )
    }

    fn render_custom_slot_editor(
        &self,
        state: &ConnectedState,
        slot: u8,
        cx: &mut Context<Self>,
    ) -> Div {
        let matrix_actions = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                icon_pill(
                    "paint-toggle",
                    Some(MaterialIcon::Brush),
                    if state.custom_paint_mode {
                        "Brush Paint: ACTIVE"
                    } else {
                        "Brush Paint: OFF"
                    },
                    state.custom_paint_mode,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Ok(st) = &mut this.conn {
                        st.custom_paint_mode = !st.custom_paint_mode;
                    }
                    cx.notify();
                })),
            )
            .child(
                secondary_btn("paint-fill-sel", "Fill Selected").on_click(cx.listener(
                    move |this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            let col = st.custom_brush;
                            for idx in st.target_indices() {
                                st.custom_slots[usize::from(slot)][idx as usize] = col;
                            }
                            this.dirty_3d = true;
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                secondary_btn("paint-fill-all", "Fill All 61").on_click(cx.listener(
                    move |this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            let col = st.custom_brush;
                            for k in keymap::WIN60HE_KEYS {
                                st.custom_slots[usize::from(slot)][k.index as usize] = col;
                            }
                            this.dirty_3d = true;
                        }
                        cx.notify();
                    },
                )),
            )
            .child(
                icon_btn(
                    "paint-clear",
                    Some(MaterialIcon::DeleteOutline),
                    "Clear Black",
                    ButtonTone::Danger,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Ok(st) = &mut this.conn {
                        st.custom_slots[usize::from(slot)] = [[0, 0, 0]; protocol::MATRIX_SLOTS];
                        this.dirty_3d = true;
                    }
                    cx.notify();
                })),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_1()
                    .children((1..=LIGHT_MAX_LEVEL).map(|b| {
                        pill(
                            ("cbri", usize::from(b)),
                            format!("Bri {b}"),
                            state.custom_brightness == b,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.custom_brightness = b;
                            }
                            cx.notify();
                        }))
                    })),
            );

        card_box(
            format!("Per-Key Custom RGB Matrix — Slot {}", slot + 1),
            Some("Paint keys in 2D view (or select & Fill Selected), then upload to keyboard"),
        )
        .child(matrix_actions)
        .child(self.render_color_channel_well(
            DropdownId::CustomBrushPreset,
            "brush",
            "Paint Brush Color",
            state.custom_brush,
            |this, rgb_val| {
                if let Ok(st) = &mut this.conn {
                    st.custom_brush = rgb_val;
                    st.custom_paint_mode = true;
                }
            },
            |this, ch, delta| {
                if let Ok(st) = &mut this.conn {
                    let cur = st.custom_brush[ch] as i16;
                    st.custom_brush[ch] = (cur + delta).clamp(0, 255) as u8;
                }
            },
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    icon_btn(
                        "upload-custom-rgb",
                        Some(MaterialIcon::Save),
                        format!("Upload Slot {} to Keyboard", slot + 1),
                        ButtonTone::Primary,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.upload_custom_rgb(slot, cx);
                    })),
                )
                .child(
                    icon_btn(
                        "reload-custom-rgb",
                        Some(MaterialIcon::Refresh),
                        "Reload from Keyboard",
                        ButtonTone::Secondary,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reload_custom_rgb(slot, cx);
                    })),
                ),
        )
    }

    fn render_audio_sync_editor(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let cfg = &state.audio_config;
        let selected_source = state.selected_audio_source();

        let source_choices: Vec<DropdownChoice> = state
            .audio_sources
            .iter()
            .enumerate()
            .map(|(idx, opt)| DropdownChoice {
                label: opt.label().into(),
                swatch: None,
                icon: Some(match opt {
                    AudioSourceOption::Demo => MaterialIcon::GraphicEq,
                    AudioSourceOption::Monitor => MaterialIcon::VolumeUp,
                    _ => MaterialIcon::Sensors,
                }),
                selected: idx == state.audio_source_idx,
            })
            .collect();

        let effect_choices: Vec<DropdownChoice> = AUDIO_EFFECT_OPTIONS
            .iter()
            .map(|&(style, label)| DropdownChoice {
                label: label.into(),
                swatch: None,
                icon: Some(MaterialIcon::GraphicEq),
                selected: cfg.style == style,
            })
            .collect();

        let palette_label = AUDIO_PALETTE_OPTIONS
            .iter()
            .find(|(p, _)| *p == cfg.palette)
            .map_or("Aurora Gradient", |(_, l)| *l);
        let palette_choices: Vec<DropdownChoice> = AUDIO_PALETTE_OPTIONS
            .iter()
            .map(|&(pal, label)| DropdownChoice {
                label: label.into(),
                swatch: (pal == AudioPalette::Custom).then_some(pack_rgb_u32(cfg.fg)),
                icon: (pal != AudioPalette::Custom).then_some(MaterialIcon::Palette),
                selected: cfg.palette == pal,
            })
            .collect();

        let notice_well = recessed_well()
            .border_color(rgb(if state.audio_disclaimer_accepted {
                theme::BORDER_SUBTLE
            } else {
                theme::BORDER_WARN
            }))
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w(px(260.0))
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap_2()
                    .child(icons::icon(
                        MaterialIcon::Warning,
                        16.0,
                        if state.audio_disclaimer_accepted {
                            theme::INK_MUTED
                        } else {
                            theme::INK_WARN
                        },
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.0))
                            .text_xs()
                            .text_color(rgb(if state.audio_disclaimer_accepted {
                                theme::INK_SECONDARY
                            } else {
                                theme::INK_WARN
                            }))
                            .child(
                                "Experimental Hack Notice: AULA states WIN 60 HE does not natively support this feature. Continuous per-key RGB streaming may diminish LED lifespan on some units; use at your own risk.",
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .rounded_md()
                            .bg(rgb(theme::SHADE_BASE))
                            .border_1()
                            .border_color(rgb(theme::BORDER_MATTE))
                            .text_size(px(10.5))
                            .text_color(rgb(if state.audio_disclaimer_accepted {
                                theme::INK_OK
                            } else {
                                theme::INK_WARN
                            }))
                            .child(if state.audio_disclaimer_accepted {
                                "Risk Accepted"
                            } else {
                                "Warning Pending"
                            }),
                    )
                    .child(
                        secondary_btn("open-audio-disclaimer", "View Disclaimer").on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.open_audio_disclaimer(false);
                                }
                                cx.notify();
                            }),
                        ),
                    ),
            );

        let metrics_bands = state
            .audio_metrics
            .as_ref()
            .map_or([0.0f32; NUM_BANDS], |m| m.bands);
        let beat_hit = state
            .audio_metrics
            .as_ref()
            .is_some_and(|m| m.beat || m.beat_env > 0.35);
        let rms_text = state
            .audio_metrics
            .as_ref()
            .map_or("-90.0 dBFS".to_string(), |m| {
                format!("{:.1} dBFS", m.rms_dbfs.max(-90.0))
            });

        let spectrum_well = recessed_well()
            .flex()
            .flex_col()
            .gap_2p5()
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(7.0))
                                    .h(px(7.0))
                                    .rounded_full()
                                    .flex_shrink_0()
                                    .bg(rgb(if state.audio_active {
                                        theme::INK_OK
                                    } else {
                                        theme::INK_DIM
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme::INK_SECONDARY))
                                    .child(state.audio_status_text.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .bg(rgb(if beat_hit {
                                        theme::SHADE_REGAL
                                    } else {
                                        theme::SHADE_BASE
                                    }))
                                    .border_1()
                                    .border_color(rgb(if beat_hit {
                                        theme::BORDER_FOCUS
                                    } else {
                                        theme::BORDER_SUBTLE
                                    }))
                                    .text_size(px(10.5))
                                    .text_color(rgb(if beat_hit {
                                        theme::INK_ACCENT
                                    } else {
                                        theme::INK_DIM
                                    }))
                                    .child("BEAT"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme::INK_MUTED))
                                    .child(rms_text),
                            ),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(52.0))
                    .flex()
                    .flex_row()
                    .items_end()
                    .gap_1p5()
                    .children((0..NUM_BANDS).map(|b| {
                        let lvl = metrics_bands[b].clamp(0.0, 1.0);
                        let fill_h = (lvl * 46.0).max(3.0);
                        div()
                            .flex_1()
                            .h_full()
                            .rounded_sm()
                            .bg(rgb(theme::SHADE_BASE))
                            .border_1()
                            .border_color(rgb(theme::BORDER_SUBTLE))
                            .flex()
                            .flex_col()
                            .justify_end()
                            .overflow_hidden()
                            .child(
                                div()
                                    .w_full()
                                    .h(px(fill_h))
                                    .bg(rgb(if lvl > 0.72 {
                                        theme::INK_ACCENT
                                    } else if lvl > 0.25 {
                                        theme::SHADE_REGAL
                                    } else {
                                        theme::BORDER_REGAL
                                    })),
                            )
                    })),
            );

        let dropdowns_row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_end()
            .gap_4()
            .child(self.render_dropdown(
                DropdownId::AudioSource,
                Some("ALSA Audio Source"),
                selected_source.label(),
                None,
                Some(MaterialIcon::VolumeUp),
                275.0,
                source_choices,
                |this, idx, cx| {
                    if let Ok(st) = &mut this.conn {
                        st.audio_source_idx = idx;
                        st.push_audio_live_config();
                        this.dirty_3d = true;
                    }
                    cx.notify();
                },
                cx,
            ))
            .child(self.render_dropdown(
                DropdownId::AudioEffect,
                Some("Visualizer Effect"),
                cfg.style.label(),
                None,
                Some(MaterialIcon::GraphicEq),
                185.0,
                effect_choices,
                |this, idx, _| {
                    if let Ok(st) = &mut this.conn {
                        st.audio_config.style = AUDIO_EFFECT_OPTIONS[idx].0;
                        st.push_audio_live_config();
                    }
                },
                cx,
            ))
            .child(self.render_dropdown(
                DropdownId::AudioPalette,
                Some("Color Palette"),
                palette_label,
                (cfg.palette == AudioPalette::Custom).then_some(pack_rgb_u32(cfg.fg)),
                (cfg.palette != AudioPalette::Custom).then_some(MaterialIcon::Palette),
                180.0,
                palette_choices,
                |this, idx, _| {
                    if let Ok(st) = &mut this.conn {
                        st.audio_config.palette = AUDIO_PALETTE_OPTIONS[idx].0;
                        st.push_audio_live_config();
                    }
                },
                cx,
            ));

        let tuning_row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_end()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgb(theme::INK_MUTED))
                            .child("Stream FPS"),
                    )
                    .child(div().flex().flex_row().gap_1().children(
                        AUDIO_FPS_OPTIONS.iter().map(|&f| {
                            pill(("afps", usize::from(f)), format!("{f}"), cfg.fps == f).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.audio_config.fps = f;
                                        st.push_audio_live_config();
                                    }
                                    cx.notify();
                                }),
                            )
                        }),
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgb(theme::INK_MUTED))
                            .child("Brightness"),
                    )
                    .child(div().flex().flex_row().gap_1().children(
                        (1..=LIGHT_MAX_LEVEL).map(|b| {
                            pill(("abri", usize::from(b)), format!("{b}"), cfg.brightness == b)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.audio_config.brightness = b;
                                        st.push_audio_live_config();
                                    }
                                    cx.notify();
                                }))
                        }),
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgb(theme::INK_MUTED))
                            .child("Input Gain"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .child(secondary_btn("again-down", "-0.2").on_click(cx.listener(
                                |this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.audio_config.gain =
                                            (st.audio_config.gain - 0.2).clamp(0.4, 3.0);
                                        st.push_audio_live_config();
                                    }
                                    cx.notify();
                                },
                            )))
                            .child(
                                div()
                                    .min_w(px(46.0))
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(theme::SHADE_DEEP))
                                    .border_1()
                                    .border_color(rgb(theme::BORDER_SUBTLE))
                                    .text_xs()
                                    .text_color(rgb(theme::INK_ACCENT))
                                    .flex()
                                    .justify_center()
                                    .child(format!("{:.1}x", cfg.gain)),
                            )
                            .child(secondary_btn("again-up", "+0.2").on_click(cx.listener(
                                |this, _, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        st.audio_config.gain =
                                            (st.audio_config.gain + 0.2).clamp(0.4, 3.0);
                                        st.push_audio_live_config();
                                    }
                                    cx.notify();
                                },
                            ))),
                    ),
            );

        let mut card = card_box(
            "Audio-Reactive RGB Studio (Experimental)",
            Some("Runs 14-band FFT + beat flux inside reaulad (keeps running after GUI closes) and reports live telemetry"),
        )
        .child(notice_well)
        .child(spectrum_well)
        .child(dropdowns_row)
        .child(tuning_row);

        if cfg.palette == AudioPalette::Custom {
            card = card.child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_4()
                    .child(self.render_color_channel_well(
                        DropdownId::AudioFgPreset,
                        "afg",
                        "Audio Foreground",
                        cfg.fg,
                        |this, rgb_val| {
                            if let Ok(st) = &mut this.conn {
                                st.audio_config.fg = rgb_val;
                                st.push_audio_live_config();
                            }
                        },
                        |this, ch, delta| {
                            if let Ok(st) = &mut this.conn {
                                let cur = st.audio_config.fg[ch] as i16;
                                st.audio_config.fg[ch] = (cur + delta).clamp(0, 255) as u8;
                                st.push_audio_live_config();
                            }
                        },
                        cx,
                    ))
                    .child(self.render_color_channel_well(
                        DropdownId::AudioBgPreset,
                        "abg",
                        "Audio Background",
                        cfg.bg,
                        |this, rgb_val| {
                            if let Ok(st) = &mut this.conn {
                                st.audio_config.bg = rgb_val;
                                st.push_audio_live_config();
                            }
                        },
                        |this, ch, delta| {
                            if let Ok(st) = &mut this.conn {
                                let cur = st.audio_config.bg[ch] as i16;
                                st.audio_config.bg[ch] = (cur + delta).clamp(0, 255) as u8;
                                st.push_audio_live_config();
                            }
                        },
                        cx,
                    )),
            );
        }

        let action_btn = if state.audio_active {
            icon_btn(
                "stop-audio-sync",
                Some(MaterialIcon::Pause),
                "Stop Audio Sync & Restore Lighting",
                ButtonTone::Danger,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.stop_audio_sync(cx);
            }))
        } else {
            icon_btn(
                "start-audio-sync",
                Some(MaterialIcon::PlayArrow),
                if state.audio_disclaimer_accepted {
                    "Start Audio Sync"
                } else {
                    "Start Audio Sync (Read Warning)"
                },
                ButtonTone::Primary,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.request_start_audio_sync(cx);
            }))
        };

        card.child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(action_btn)
                .child(
                    icon_btn(
                        "refresh-audio-sources",
                        Some(MaterialIcon::Refresh),
                        "Refresh ALSA Devices",
                        ButtonTone::Secondary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            st.refresh_audio_sources();
                            this.status_line = format!(
                                "Detected {} ALSA capture source option(s).",
                                st.audio_sources.len()
                            );
                        }
                        cx.notify();
                    })),
                ),
        )
    }

    pub(crate) fn render_audio_disclaimer_modal(
        &self,
        state: &ConnectedState,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let rem = state.disclaimer_remaining_secs();
        let cooldown_done = rem <= 0.0;
        let can_confirm = state.can_confirm_audio_disclaimer();

        let checkbox_row = div()
            .id("audio-disclaimer-agree")
            .w_full()
            .p_3()
            .rounded_lg()
            .bg(rgb(theme::SHADE_DEEP))
            .border_1()
            .border_color(rgb(if state.audio_disclaimer_checked {
                theme::BORDER_FOCUS
            } else {
                theme::BORDER_MATTE
            }))
            .flex()
            .flex_row()
            .items_start()
            .gap_3()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, _, cx| {
                if let Ok(st) = &mut this.conn {
                    st.audio_disclaimer_checked = !st.audio_disclaimer_checked;
                }
                cx.notify();
            }))
            .child(
                div()
                    .w(px(18.0))
                    .h(px(18.0))
                    .mt_0p5()
                    .flex_shrink_0()
                    .rounded_sm()
                    .bg(rgb(if state.audio_disclaimer_checked {
                        theme::SHADE_REGAL
                    } else {
                        theme::SHADE_BASE
                    }))
                    .border_1()
                    .border_color(rgb(if state.audio_disclaimer_checked {
                        theme::BORDER_FOCUS
                    } else {
                        theme::BORDER_REGAL
                    }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(state.audio_disclaimer_checked, |d| {
                        d.child(icons::icon(MaterialIcon::Check, 13.0, theme::INK_ACCENT))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(220.0))
                    .text_xs()
                    .text_color(rgb(theme::INK_PRIMARY))
                    .child(
                        "I Agree — I have read and am fully aware that this feature is an unsupported workaround, accept all hardware/LED lifespan risks, and will not blame or hold this software or its author(s) liable for any keyboard defects.",
                    ),
            );

        let confirm_label = if !cooldown_done {
            format!("Wait {:.0}s...", rem.ceil())
        } else if !state.audio_disclaimer_checked {
            "Check 'I Agree' First".into()
        } else if state.audio_disclaimer_pending_start {
            "I Agree — Enable Audio Sync".into()
        } else {
            "I Agree — Acknowledge Risk".into()
        };

        let confirm_btn = if can_confirm {
            icon_btn(
                "confirm-audio-disclaimer",
                Some(MaterialIcon::Check),
                confirm_label,
                ButtonTone::Warn { active: true },
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.confirm_audio_disclaimer(cx);
            }))
        } else {
            div()
                .id("confirm-audio-disclaimer-locked")
                .px_3()
                .py_1p5()
                .rounded_md()
                .bg(rgb(theme::SHADE_DEEP))
                .border_1()
                .border_color(rgb(theme::BORDER_SUBTLE))
                .text_xs()
                .text_color(rgb(theme::INK_DIM))
                .whitespace_nowrap()
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .opacity(0.75)
                .child(icons::icon(MaterialIcon::Warning, 14.0, theme::INK_DIM))
                .child(confirm_label)
        };

        let modal_card = div()
            .id("audio-disclaimer-card")
            .occlude()
            .w(px(640.0))
            .max_w_full()
            .rounded_xl()
            .bg(rgb(theme::SHADE_BASE))
            .border_1()
            .border_color(rgb(theme::BORDER_WARN))
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .pb_3()
                    .border_b_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2p5()
                    .child(icons::icon(MaterialIcon::Warning, 20.0, theme::INK_WARN))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(220.0))
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(theme::INK_PRIMARY))
                                    .child("Experimental Audio-Reactive RGB — Hardware & Liability Warning"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme::INK_WARN))
                                    .child(format!(
                                        "Mandatory {:.0}-second review lock & explicit acknowledgement required",
                                        AUDIO_DISCLAIMER_COOLDOWN_SECS
                                    )),
                            ),
                    ),
            )
            .child(
                recessed_well()
                    .border_color(rgb(theme::BORDER_WARN))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .text_xs()
                    .text_color(rgb(theme::INK_SECONDARY))
                    .child(
                        "1. Unofficial Workaround: AULA officially states the WIN 60 HE does not support driver-less music/audio sync in firmware. This mode works as a host-side hack by streaming live Custom matrix packets (cmd 0x09) computed from ALSA FFT audio analysis.",
                    )
                    .child(
                        "2. Diminished LED / Component Lifespan: Continuously pushing rapid high-frequency per-key color updates and sustained peak LED transitions may cause uneven LED aging, diminished LED lifespan, or controller stress on some hardware units or variants.",
                    )
                    .child(
                        div()
                            .text_color(rgb(theme::INK_WARN))
                            .child(
                                "3. No Warranty & Zero Liability: This feature is provided AS IS purely at your own risk. Do not blame this software or its author(s) for any defects, LED degradation, or hardware issues on your keyboard — you have been warned.",
                            ),
                    ),
            )
            .child(checkbox_row)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if cooldown_done {
                                theme::INK_OK
                            } else {
                                theme::INK_WARN
                            }))
                            .child(if cooldown_done {
                                "10s review cooldown complete".to_string()
                            } else {
                                format!("Cooldown active: wait {:.0}s before confirming", rem.ceil())
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                secondary_btn("cancel-audio-disclaimer", "Cancel").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        if let Ok(st) = &mut this.conn {
                                            st.close_audio_disclaimer();
                                        }
                                        cx.notify();
                                    }),
                                ),
                            )
                            .child(confirm_btn),
                    ),
            );

        div()
            .id("audio-disclaimer-backdrop")
            .occlude()
            .absolute()
            .inset_0()
            .bg(rgba(0x0911_1EE6))
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            .child(modal_card)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_color_channel_well<S, N>(
        &self,
        dropdown_id: DropdownId,
        id_prefix: &'static str,
        title: &'static str,
        col: [u8; 3],
        on_preset: S,
        on_nudge: N,
        cx: &mut Context<Self>,
    ) -> Div
    where
        S: Fn(&mut Self, [u8; 3]) + Copy + 'static,
        N: Fn(&mut Self, usize, i16) + Copy + 'static,
    {
        let matched_name = COLOR_SWATCHES.iter().find(|(_, v)| *v == col).map_or_else(
            || format!("Custom ({})", format_rgb_hex(col)),
            |(n, _)| (*n).to_string(),
        );
        let choices: Vec<DropdownChoice> = COLOR_SWATCHES
            .iter()
            .map(|&(name, rgb_val)| DropdownChoice {
                label: name.into(),
                swatch: Some(pack_rgb_u32(rgb_val)),
                icon: None,
                selected: col == rgb_val,
            })
            .collect();

        recessed_well()
            .flex_1()
            .min_w(px(330.0))
            .flex()
            .flex_col()
            .gap_2p5()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(18.0))
                                    .h(px(18.0))
                                    .rounded_sm()
                                    .bg(rgb(pack_rgb_u32(col)))
                                    .border_1()
                                    .border_color(rgb(theme::BORDER_REGAL)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(theme::INK_PRIMARY))
                                    .child(format!("{title} · {}", format_rgb_hex(col))),
                            ),
                    )
                    .child(self.render_dropdown(
                        dropdown_id,
                        None,
                        matched_name,
                        Some(pack_rgb_u32(col)),
                        None,
                        160.0,
                        choices,
                        move |this, idx, _| {
                            on_preset(this, COLOR_SWATCHES[idx].1);
                        },
                        cx,
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .children([(0usize, "R"), (1, "G"), (2, "B")].into_iter().map(
                        |(ch, ch_lbl)| {
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .child(
                                    pill((id_prefix, ch * 2), format!("{ch_lbl}-15"), false)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            on_nudge(this, ch, -15);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .w(px(26.0))
                                        .flex()
                                        .justify_center()
                                        .text_xs()
                                        .text_color(rgb(theme::INK_PRIMARY))
                                        .child(format!("{}", col[ch])),
                                )
                                .child(
                                    pill((id_prefix, ch * 2 + 1), format!("{ch_lbl}+15"), false)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            on_nudge(this, ch, 15);
                                            cx.notify();
                                        })),
                                )
                        },
                    )),
            )
    }

    // -----------------------------------------------------------------------
    // 5. HOTKEYS: Daemon In-Memory Shortcuts & Triggers
    // -----------------------------------------------------------------------
    fn view_hotkeys(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let pick_name = keymap::by_index(state.draft_rule_key).map_or("Key", |k| k.name);
        let gesture_preview = match state.draft_rule_modifier {
            Some(m) => {
                let mname = keymap::by_index(m).map_or("Mod", |k| k.name);
                format!("{mname} + {pick_name}")
            }
            None => pick_name.to_string(),
        };
        let persist_label = state
            .shortcut_persist_path
            .as_deref()
            .unwrap_or("daemon memory");

        let guard_label = if state.evdev_grab_active {
            "ACTIVE (EVIOCGRAB)"
        } else if state.uinput_ready {
            "STANDBY"
        } else {
            "OFFLINE"
        };

        let arrow_choices: Vec<DropdownChoice> = ARROW_CLUSTER_OPTIONS
            .iter()
            .map(|&(_, label)| DropdownChoice {
                label: label.into(),
                swatch: None,
                icon: Some(MaterialIcon::Bolt),
                selected: false,
            })
            .collect();

        let mut top_bar = card_box(
            "Keyboard Onboard Fn Flash + Daemon Backup Hotkeys",
            Some(&format!(
                "KB Fn Flash: {} slot(s) | /dev/uinput: {} | Key-Leak Guard: {} | Daemon: {persist_label}",
                state.keyboard_fn_map.len(),
                if state.uinput_ready { "READY" } else { "OFFLINE" },
                guard_label,
            )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            icon_pill(
                                "hk-watcher-toggle",
                                Some(MaterialIcon::Sensors),
                                if state.shortcut_watcher_active {
                                    "Watcher: ACTIVE"
                                } else {
                                    "Watcher: PAUSED"
                                },
                                state.shortcut_watcher_active,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.shortcut_watcher_active = !st.shortcut_watcher_active;
                                    let _ = st.sync_shortcuts();
                                }
                                cx.notify();
                            })),
                        )
                        .child(
                            icon_btn(
                                "hk-sync",
                                Some(MaterialIcon::Save),
                                "Save / Memorize Now",
                                ButtonTone::Secondary,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    match st.sync_shortcuts() {
                                        Ok(()) => {
                                            this.status_line = "Memorized hotkey rules to daemon & disk.".into();
                                        }
                                        Err(e) => this.status_line = e,
                                    }
                                }
                                cx.notify();
                            })),
                        )
                        .child(
                            icon_btn(
                                "hk-reload",
                                Some(MaterialIcon::Refresh),
                                "Reload Both",
                                ButtonTone::Secondary,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    let _ = st.refresh_shortcuts();
                                    let _ = st.refresh_keyboard_fn_map();
                                }
                                cx.notify();
                            })),
                        )
                        .child(
                            icon_btn(
                                "hk-record",
                                Some(MaterialIcon::FiberManualRecord),
                                if state.shortcut_recording {
                                    "Recording... (Cancel)"
                                } else {
                                    "Memorize Gesture"
                                },
                                ButtonTone::Warn {
                                    active: state.shortcut_recording,
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.shortcut_recording = !st.shortcut_recording;
                                    st.record_first_key = None;
                                    if st.shortcut_recording {
                                        if !st.travel_enabled {
                                            let _ = st.start_travel();
                                        }
                                        st.captured_gesture_banner = Some(
                                            "Press a single key, OR hold Fn (or any modifier) and tap a 2nd key (e.g. Fn + T).".into(),
                                        );
                                    }
                                }
                                cx.notify();
                            })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(self.render_dropdown(
                            DropdownId::HotkeyArrowKb,
                            None,
                            "Flash Arrows -> KB",
                            None,
                            Some(MaterialIcon::Memory),
                            195.0,
                            arrow_choices
                                .iter()
                                .map(|c| DropdownChoice {
                                    label: c.label.clone(),
                                    swatch: None,
                                    icon: Some(MaterialIcon::Memory),
                                    selected: false,
                                })
                                .collect(),
                            |this, idx, _| {
                                let preset = ARROW_CLUSTER_OPTIONS[idx].0;
                                if let Ok(st) = &mut this.conn {
                                    match st.save_arrow_cluster_to_keyboard(preset) {
                                        Ok(msg) => this.status_line = msg,
                                        Err(e) => this.status_line = e,
                                    }
                                }
                            },
                            cx,
                        ))
                        .child(self.render_dropdown(
                            DropdownId::HotkeyArrowDaemon,
                            None,
                            "Add Arrows -> Daemon",
                            None,
                            Some(MaterialIcon::Bolt),
                            195.0,
                            arrow_choices,
                            |this, idx, _| {
                                let (preset, label) = ARROW_CLUSTER_OPTIONS[idx];
                                if let Ok(st) = &mut this.conn {
                                    st.add_fn_arrow_cluster(preset);
                                    match st.sync_shortcuts() {
                                        Ok(()) => {
                                            this.status_line = format!(
                                                "Memorized {label} (Key-Leak Guard suppresses raw letters)!"
                                            );
                                        }
                                        Err(e) => this.status_line = e,
                                    }
                                }
                            },
                            cx,
                        )),
                ),
        );

        if !state.uinput_ready {
            let reason = state
                .uinput_error
                .as_deref()
                .unwrap_or("/dev/uinput unavailable in reaulad");
            top_bar = top_bar.child(
                recessed_well()
                    .p_2p5()
                    .border_color(rgb(theme::BORDER_DANGER))
                    .text_xs()
                    .text_color(rgb(theme::INK_DANGER))
                    .child(format!(
                        "Virtual Key Injection (/dev/uinput) OFFLINE: {reason}"
                    )),
            );
        }

        if let Some(last_log) = state.shortcut_logs.first() {
            let col = if last_log.success {
                theme::INK_OK
            } else {
                theme::INK_WARN
            };
            top_bar = top_bar.child(div().text_xs().text_color(rgb(col)).child(format!(
                "Latest Trigger: [{}] {} -> {}",
                if last_log.success { "OK" } else { "FAIL" },
                last_log.rule_name,
                last_log.message
            )));
        }

        if let Some(banner) = &state.captured_gesture_banner {
            top_bar = top_bar.child(
                div()
                    .p_2p5()
                    .rounded_md()
                    .bg(rgb(theme::SHADE_REGAL))
                    .border_1()
                    .border_color(rgb(if state.shortcut_recording {
                        theme::BORDER_WARN
                    } else {
                        theme::BORDER_FOCUS
                    }))
                    .text_xs()
                    .text_color(rgb(if state.shortcut_recording {
                        theme::INK_WARN
                    } else {
                        theme::INK_OK
                    }))
                    .child(banner.clone()),
            );
        }

        top_bar = top_bar
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child("Tip: Click = Trigger (focused) · Shift+Click = Modifier · KB:.. = onboard WIN 60 HE flash · RULE = daemon backup with EVIOCGRAB key-leak guard."),
            )
            .child(self.render_keyboard_canvas(
                state,
                KeyboardCanvasMode::HotkeyPicker,
                1.15,
                0.0,
                cx,
            ));

        let current_mod_label = HOTKEY_MODIFIER_OPTIONS
            .iter()
            .find(|(m, _)| *m == state.draft_rule_modifier)
            .map_or_else(
                || {
                    state
                        .draft_rule_modifier
                        .and_then(keymap::by_index)
                        .map_or("Custom".into(), |k| k.name.to_string())
                },
                |(_, lbl)| (*lbl).to_string(),
            );
        let mod_choices: Vec<DropdownChoice> = HOTKEY_MODIFIER_OPTIONS
            .iter()
            .map(|&(m, lbl)| DropdownChoice {
                label: lbl.into(),
                swatch: None,
                icon: None,
                selected: state.draft_rule_modifier == m,
            })
            .collect();

        let mut trigger_indices: Vec<u8> = QUICK_TRIGGER_KEYS.to_vec();
        if !trigger_indices.contains(&state.draft_rule_key) {
            trigger_indices.insert(0, state.draft_rule_key);
        }
        let trigger_label = keymap::by_index(state.draft_rule_key).map_or_else(
            || format!("Slot #{}", state.draft_rule_key),
            |k| format!("{} (#{})", k.name, k.index),
        );
        let trigger_choices: Vec<DropdownChoice> = trigger_indices
            .iter()
            .map(|&idx| {
                let lbl = keymap::by_index(idx).map_or_else(
                    || format!("Slot #{idx}"),
                    |k| format!("{} (#{idx})", k.name),
                );
                DropdownChoice {
                    label: lbl.into(),
                    swatch: None,
                    icon: None,
                    selected: state.draft_rule_key == idx,
                }
            })
            .collect();

        let action_choices: Vec<DropdownChoice> = ShortcutActionPreset::ALL
            .iter()
            .map(|&p| DropdownChoice {
                label: p.label().into(),
                swatch: None,
                icon: Some(MaterialIcon::Bolt),
                selected: state.draft_rule_preset == p,
            })
            .collect();

        let vk_choices: Vec<DropdownChoice> = VirtualKey::ALL
            .iter()
            .map(|&vk| DropdownChoice {
                label: vk.label().into(),
                swatch: None,
                icon: None,
                selected: state.draft_virtual_key == vk,
            })
            .collect();

        let trigger_indices_for_cb = trigger_indices.clone();
        let mut composer = card_box(
            format!(
                "Compose Keybind / Fn Function — Gesture: {gesture_preview} (>= {:.2} mm)",
                state.draft_rule_depth_mm
            ),
            Some(&format!("{} rule(s) active", state.shortcut_rules.len())),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_end()
                .gap_3()
                .child(self.render_dropdown(
                    DropdownId::HotkeyModifier,
                    Some("Modifier Key"),
                    current_mod_label,
                    None,
                    None,
                    160.0,
                    mod_choices,
                    |this, idx, _| {
                        if let Ok(st) = &mut this.conn {
                            st.draft_rule_modifier = HOTKEY_MODIFIER_OPTIONS[idx].0;
                        }
                    },
                    cx,
                ))
                .child(self.render_dropdown(
                    DropdownId::HotkeyTrigger,
                    Some("Trigger Key (or click matrix)"),
                    trigger_label,
                    None,
                    None,
                    165.0,
                    trigger_choices,
                    move |this, idx, _| {
                        if let Ok(st) = &mut this.conn {
                            st.draft_rule_key = trigger_indices_for_cb[idx];
                        }
                    },
                    cx,
                ))
                .child(self.render_dropdown(
                    DropdownId::HotkeyAction,
                    Some("Action Preset"),
                    state.draft_rule_preset.label(),
                    None,
                    Some(MaterialIcon::Bolt),
                    200.0,
                    action_choices,
                    |this, idx, _| {
                        if let Ok(st) = &mut this.conn {
                            st.draft_rule_preset = ShortcutActionPreset::ALL[idx];
                        }
                    },
                    cx,
                ))
                .when(
                    state.draft_rule_preset == ShortcutActionPreset::EmitVirtualKey,
                    |d| {
                        d.child(self.render_dropdown(
                            DropdownId::HotkeyVirtualKey,
                            Some("Target OS Key"),
                            state.draft_virtual_key.label(),
                            None,
                            None,
                            185.0,
                            vk_choices,
                            |this, idx, _| {
                                if let Ok(st) = &mut this.conn {
                                    st.draft_virtual_key = VirtualKey::ALL[idx];
                                }
                            },
                            cx,
                        ))
                    },
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(rgb(theme::INK_MUTED))
                                .child("Trigger Depth"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .child(pill("hk-d-minus", "-0.20", false).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Ok(st) = &mut this.conn {
                                            st.draft_rule_depth_mm =
                                                (st.draft_rule_depth_mm - 0.20).clamp(0.40, 3.20);
                                        }
                                        cx.notify();
                                    },
                                )))
                                .child(
                                    div()
                                        .w(px(58.0))
                                        .flex()
                                        .justify_center()
                                        .text_xs()
                                        .text_color(rgb(theme::INK_PRIMARY))
                                        .child(format!("{:.2} mm", state.draft_rule_depth_mm)),
                                )
                                .child(pill("hk-d-plus", "+0.20", false).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Ok(st) = &mut this.conn {
                                            st.draft_rule_depth_mm =
                                                (st.draft_rule_depth_mm + 0.20).clamp(0.40, 3.20);
                                        }
                                        cx.notify();
                                    },
                                ))),
                        ),
                ),
        );

        if state.draft_rule_preset == ShortcutActionPreset::RunCommand {
            composer =
                composer.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .id("hk-cmd-box")
                                .flex_1()
                                .p_2p5()
                                .rounded_md()
                                .bg(rgb(theme::SHADE_DEEP))
                                .border_1()
                                .border_color(rgb(theme::BORDER_FOCUS))
                                .track_focus(&self.cmd_focus)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    window.focus(&this.cmd_focus);
                                    cx.notify();
                                }))
                                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                    if let Ok(st) = &mut this.conn {
                                        match ev.keystroke.key.as_str() {
                                            "backspace" => {
                                                st.draft_rule_command.pop();
                                            }
                                            "space" => {
                                                st.draft_rule_command.push(' ');
                                            }
                                            _ => {
                                                if let Some(ch) = &ev.keystroke.key_char {
                                                    st.draft_rule_command.push_str(ch);
                                                }
                                            }
                                        }
                                        cx.notify();
                                    }
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(theme::INK_PRIMARY))
                                        .child(format!("$ {}", state.draft_rule_command)),
                                ),
                        )
                        .child(secondary_btn("hk-cmd-sample1", "Notify Sample").on_click(
                            cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.draft_rule_command =
                                        "notify-send 'Reaula WIN 60 HE' 'Hotkey triggered'".into();
                                }
                                cx.notify();
                            }),
                        )),
                );
        }

        composer = composer.child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    icon_btn(
                        "hk-save-kb-btn",
                        Some(MaterialIcon::Memory),
                        "Save Fn Rule to Keyboard Flash (cmd 0x18)",
                        ButtonTone::Primary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            match st.save_draft_to_keyboard_fn() {
                                Ok(msg) => this.status_line = msg,
                                Err(e) => this.status_line = e,
                            }
                        }
                        cx.notify();
                    })),
                )
                .child(
                    icon_btn(
                        "hk-add-btn",
                        Some(MaterialIcon::Save),
                        "+ Add Rule to Daemon Backup (uinput + Key-Leak Guard)",
                        ButtonTone::Secondary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            let rule = st.build_draft_shortcut_rule();
                            st.shortcut_rules.push(rule);
                            st.shortcut_watcher_active = true;
                            match st.sync_shortcuts() {
                                Ok(()) => {
                                    this.status_line =
                                        "Added hotkey rule to reaulad backup (key-leak guard enabled).".into();
                                }
                                Err(e) => this.status_line = e,
                            }
                        }
                        cx.notify();
                    })),
                ),
        );

        let mut fn_flash_card = card_box(
            format!(
                "Onboard Hardware Fn Keymap (Stored in WIN 60 HE Flash — {} slot(s))",
                state.keyboard_fn_map.len()
            ),
            Some("Persists inside keyboard flash without daemon; hardware natively blocks base letters"),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    icon_btn(
                        "hk-kb-refresh",
                        Some(MaterialIcon::Refresh),
                        "Refresh from KB",
                        ButtonTone::Secondary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            match st.refresh_keyboard_fn_map() {
                                Ok(()) => {
                                    this.status_line = "Refreshed onboard Fn keymap from keyboard.".into();
                                }
                                Err(e) => this.status_line = e,
                            }
                        }
                        cx.notify();
                    })),
                )
                .child(
                    icon_btn(
                        "hk-kb-reset",
                        Some(MaterialIcon::DeleteOutline),
                        "Reset All KB Fn Flash",
                        ButtonTone::Danger,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            match st.reset_keyboard_fn_map() {
                                Ok(()) => {
                                    this.status_line = "Cleared all custom Fn bindings in keyboard flash.".into();
                                }
                                Err(e) => this.status_line = e,
                            }
                        }
                        cx.notify();
                    })),
                ),
        );

        if state.keyboard_fn_map.is_empty() {
            fn_flash_card = fn_flash_card.child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child("Factory default Fn layer on keyboard flash (no custom overrides). Use 'Flash Arrows -> KB' above to save directly to WIN 60 HE."),
            );
        } else {
            for (&slot_idx, &target_hid) in &state.keyboard_fn_map {
                let src_name = keymap::by_index(slot_idx).map_or("Key", |k| k.name);
                let dst_name = keymap::usb_hid_name(target_hid);
                fn_flash_card = fn_flash_card.child(
                    div()
                        .p_2p5()
                        .px_3()
                        .rounded_lg()
                        .bg(rgb(theme::SHADE_REGAL))
                        .border_1()
                        .border_color(rgb(theme::BORDER_REGAL))
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(220.0))
                                .text_xs()
                                .text_color(rgb(theme::INK_PRIMARY))
                                .child(format!(
                                    "Fn + {src_name} (#{slot_idx}) -> {dst_name} (HID 0x{target_hid:02X})"
                                )),
                        )
                        .child(
                            icon_btn(
                                ("hk-kb-clr", usize::from(slot_idx)),
                                Some(MaterialIcon::DeleteOutline),
                                "Clear",
                                ButtonTone::Danger,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    match st.clear_keyboard_fn_slot(slot_idx) {
                                        Ok(()) => {
                                            this.status_line = format!("Cleared Fn + {src_name} from KB flash.");
                                        }
                                        Err(e) => this.status_line = e,
                                    }
                                }
                                cx.notify();
                            })),
                        ),
                );
            }
        }

        let mut rules_card = card_box(
            "Daemon Backup Shortcut Rules (EVIOCGRAB Key-Leak Protected)",
            None,
        );
        if state.shortcut_rules.is_empty() {
            rules_card = rules_card.child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child("No hotkey rules loaded in reaulad yet. Add one above."),
            );
        } else {
            for rule in &state.shortcut_rules {
                let r_id = rule.id;
                let gesture = rule.gesture_label();
                rules_card = rules_card.child(
                    recessed_well()
                        .p_2p5()
                        .px_3()
                        .border_color(rgb(if rule.enabled {
                            theme::BORDER_REGAL
                        } else {
                            theme::BORDER_SUBTLE
                        }))
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(240.0))
                                .flex()
                                .flex_col()
                                .gap_0p5()
                                .child(
                                    div()
                                        .w_full()
                                        .text_xs()
                                        .text_color(rgb(theme::INK_PRIMARY))
                                        .child(format!(
                                            "#{} {} | Gesture: {} (>= {:.2} mm)",
                                            rule.id, rule.name, gesture, rule.trigger_depth_mm
                                        )),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .w_full()
                                        .text_xs()
                                        .text_color(rgb(theme::INK_MUTED))
                                        .child(rule.action.summary()),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    pill(
                                        ("hk-en", r_id as usize),
                                        if rule.enabled { "Enabled" } else { "Muted" },
                                        rule.enabled,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if let Ok(st) = &mut this.conn {
                                                if let Some(r) = st
                                                    .shortcut_rules
                                                    .iter_mut()
                                                    .find(|r| r.id == r_id)
                                                {
                                                    r.enabled = !r.enabled;
                                                }
                                                let _ = st.sync_shortcuts();
                                            }
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    icon_btn(
                                        ("hk-fire", r_id as usize),
                                        Some(MaterialIcon::PlayArrow),
                                        "Test Fire",
                                        ButtonTone::Secondary,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if let Ok(st) = &mut this.conn {
                                                match st.trigger_shortcut_rule(r_id) {
                                                    Ok(log) => {
                                                        this.status_line = format!(
                                                            "Triggered #{}: {}",
                                                            log.rule_id, log.message
                                                        )
                                                    }
                                                    Err(e) => this.status_line = e,
                                                }
                                            }
                                            cx.notify();
                                        },
                                    )),
                                )
                                .child(
                                    icon_btn(
                                        ("hk-del", r_id as usize),
                                        Some(MaterialIcon::DeleteOutline),
                                        "Delete",
                                        ButtonTone::Danger,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if let Ok(st) = &mut this.conn {
                                                st.shortcut_rules.retain(|r| r.id != r_id);
                                                let _ = st.sync_shortcuts();
                                            }
                                            cx.notify();
                                        },
                                    )),
                                ),
                        ),
                );
            }
        }

        let mut logs_card = card_box("Daemon Trigger History", None);
        if state.shortcut_logs.is_empty() {
            logs_card = logs_card.child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child("No triggers recorded yet. Click Test Fire or press your trigger key."),
            );
        } else {
            for log in state.shortcut_logs.iter().take(8) {
                logs_card = logs_card.child(
                    div()
                        .text_xs()
                        .text_color(rgb(if log.success {
                            theme::INK_OK
                        } else {
                            theme::INK_DANGER
                        }))
                        .child(format!(
                            "[{}] {} -> {}",
                            if log.success { "OK" } else { "FAIL" },
                            log.rule_name,
                            log.message
                        )),
                );
            }
        }

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(top_bar)
            .child(composer)
            .child(fn_flash_card)
            .child(rules_card)
            .child(logs_card)
    }

    // -----------------------------------------------------------------------
    // 6. DIAGNOSTICS: 4-Layer Fault Isolation + Test & Response Feedback UI
    // -----------------------------------------------------------------------
    fn view_diagnostics(&self, cx: &mut Context<Self>) -> Div {
        let report_opt: Option<DiagnosticReport> = match &self.conn {
            Ok(st) => st.diag_report.clone(),
            Err(_) => self.offline_diag.clone(),
        };

        let banner_text = report_opt.as_ref().map_or_else(
            || "Click 'Run 4-Layer Diagnostic' to test Kernel/USB, Daemon/IPC, Keyboard Hardware, and Software Codec.".to_string(),
            |r| r.attribution.summary.clone(),
        );
        let (banner_bg, banner_border, banner_fg) = match report_opt
            .as_ref()
            .and_then(|r| r.attribution.primary_fault)
        {
            None if report_opt.is_some() => {
                (theme::SHADE_REGAL, theme::BORDER_FOCUS, theme::INK_OK)
            }
            Some(_) => (theme::SHADE_DEEP, theme::BORDER_DANGER, theme::INK_DANGER),
            None => (
                theme::SHADE_DEEP,
                theme::BORDER_SUBTLE,
                theme::INK_SECONDARY,
            ),
        };

        let mut header_card = card_box(
            "4-Layer Fault Isolation & Test Feedback",
            Some("Kernel / USB · Daemon / IPC · Keyboard · Software"),
        )
        .child(
            div()
                .p_3()
                .rounded_lg()
                .bg(rgb(banner_bg))
                .border_1()
                .border_color(rgb(banner_border))
                .text_xs()
                .text_color(rgb(banner_fg))
                .child(banner_text),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(
                    icon_btn(
                        "dx-run-all",
                        Some(MaterialIcon::MonitorHeart),
                        "Run 4-Layer Diagnostic",
                        ButtonTone::Primary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.trigger_diagnostics(cx);
                    })),
                )
                .child(
                    icon_btn(
                        "dx-echo",
                        Some(MaterialIcon::Bolt),
                        "Run 5x HID Echo Burst",
                        ButtonTone::Secondary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            match st.run_echo_benchmark() {
                                Ok(msg) => this.status_line = msg,
                                Err(e) => this.status_line = e,
                            }
                        } else {
                            this.status_line =
                                "Daemon offline: connect reaulad to benchmark HID echo.".into();
                        }
                        cx.notify();
                    })),
                )
                .child(
                    icon_btn(
                        "dx-zero-sw",
                        Some(MaterialIcon::RestartAlt),
                        "Clear Test Peaks / Stuck",
                        ButtonTone::Secondary,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            st.reset_switch_tests();
                            this.dirty_switch_3d = true;
                            this.status_line = "Cleared switch response test peaks.".into();
                        }
                        cx.notify();
                    })),
                ),
        );

        if let Ok(st) = &self.conn
            && !st.echo_latencies_ms.is_empty()
        {
            let lat_txt = st
                .echo_latencies_ms
                .iter()
                .map(|m| format!("{m:.2} ms"))
                .collect::<Vec<_>>()
                .join("  ·  ");
            header_card = header_card.child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_ACCENT))
                    .child(format!("HID Echo Burst Samples: {lat_txt}")),
            );
        }

        let mut layers_grid = div().flex().flex_col().gap_4();
        if let Some(rep) = &report_opt {
            for pair in [
                [DiagnosticLayer::Kernel, DiagnosticLayer::Daemon],
                [DiagnosticLayer::Keyboard, DiagnosticLayer::Software],
            ] {
                let mut pair_row = div().flex().flex_row().flex_wrap().gap_4();
                for layer in pair {
                    let mut layer_card = card_box(layer.label(), None).flex_1().min_w(px(360.0));
                    let checks: Vec<_> = rep.checks.iter().filter(|c| c.layer == layer).collect();
                    if checks.is_empty() {
                        layer_card = layer_card.child(
                            div()
                                .text_xs()
                                .text_color(rgb(theme::INK_MUTED))
                                .child("(no checks)"),
                        );
                    } else {
                        for c in checks {
                            let (st_col, st_border) = match c.status {
                                CheckStatus::Pass => (theme::INK_OK, theme::BORDER_SUBTLE),
                                CheckStatus::Warn => (theme::INK_WARN, theme::BORDER_WARN),
                                CheckStatus::Fail => (theme::INK_DANGER, theme::BORDER_DANGER),
                            };
                            let lat = c
                                .latency_ms
                                .map_or_else(String::new, |m| format!(" ({m:.2} ms)"));
                            let mut item = recessed_well()
                                .p_2p5()
                                .border_color(rgb(st_border))
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .min_w_0()
                                        .w_full()
                                        .text_xs()
                                        .text_color(rgb(st_col))
                                        .child(format!("[{}] {}{}", c.status.label(), c.name, lat)),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .w_full()
                                        .text_xs()
                                        .text_color(rgb(theme::INK_SECONDARY))
                                        .child(c.detail.clone()),
                                );
                            if let Some(rem) = &c.remedy {
                                item = item.child(
                                    div()
                                        .min_w_0()
                                        .w_full()
                                        .text_xs()
                                        .text_color(rgb(theme::INK_WARN))
                                        .child(format!("Fix: {rem}")),
                                );
                            }
                            layer_card = layer_card.child(item);
                        }
                    }
                    pair_row = pair_row.child(layer_card);
                }
                layers_grid = layers_grid.child(pair_row);
            }
        }

        // Visual 61-keycap depth check + wgpu 3D / 2D Hall-effect switch cutaway
        let visual_bench = self
            .conn
            .as_ref()
            .ok()
            .map(|st| self.render_diagnostics_visual_bench(st, cx));

        // Switch Response Verification + Command Feedback Log
        let mut bench_row = div().flex().flex_row().flex_wrap().gap_4();
        let mut switch_card = card_box(
            "Switch Response Check (Press Keys Now)",
            Some("Verifies analog reach & 0.00 mm return — click a row to inspect switch"),
        )
        .flex_1()
        .min_w(px(360.0));
        let mut feedback_card = card_box(
            "HID / RPC Response Feedback Log",
            Some("Latest daemon calls and RTT ms"),
        )
        .flex_1()
        .min_w(px(360.0));

        if let Ok(st) = &self.conn {
            if st.switch_tests.is_empty() {
                switch_card = switch_card.child(
                    div()
                        .text_xs()
                        .text_color(rgb(theme::INK_MUTED))
                        .child("Press any keys with Live Travel ON to record press depth, peak mm, and 0.00 mm release check."),
                );
            } else {
                let mut rows: Vec<_> = st.switch_tests.iter().collect();
                rows.sort_by_key(|b| std::cmp::Reverse(b.1.last_seen));
                for (&idx, stat) in rows.into_iter().take(10) {
                    let kname = keymap::by_index(idx).map_or("Key", |k| k.name);
                    let status_txt = if stat.released_zero {
                        "0.00 RETURN OK"
                    } else if stat.current_mm > 0.02 {
                        "PRESSED / HELD"
                    } else {
                        "SETTLED"
                    };
                    let col = if stat.released_zero {
                        theme::INK_OK
                    } else {
                        theme::INK_WARN
                    };
                    let focused = st.focused_key == idx;
                    switch_card = switch_card.child(
                        recessed_well()
                            .id(("dx-sw-row", usize::from(idx)))
                            .p_1p5()
                            .px_2p5()
                            .border_color(rgb(if focused {
                                theme::BORDER_FOCUS
                            } else {
                                theme::BORDER_SUBTLE
                            }))
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .justify_between()
                            .gap_2()
                            .cursor_pointer()
                            .hover(|s| s.border_color(rgb(theme::BORDER_FOCUS)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Ok(s) = &mut this.conn {
                                    s.focused_key = idx;
                                    this.dirty_switch_3d = true;
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(220.0))
                                    .text_xs()
                                    .text_color(rgb(theme::INK_PRIMARY))
                                    .child(format!(
                                        "{kname} (#{idx}): now {:.2} mm | peak {:.2} mm | presses {}",
                                        stat.current_mm, stat.peak_mm, stat.presses
                                    )),
                            )
                            .child(div().text_xs().text_color(rgb(col)).child(status_txt)),
                    );
                }
            }

            for ev in st.feedback_log.iter().take(12) {
                feedback_card = feedback_card.child(
                    div()
                        .min_w_0()
                        .w_full()
                        .text_xs()
                        .text_color(rgb(if ev.ok {
                            theme::INK_ACCENT
                        } else {
                            theme::INK_DANGER
                        }))
                        .child(format!(
                            "#{} [{} {:.2} ms] {}: {}",
                            ev.id,
                            if ev.ok { "OK" } else { "ERR" },
                            ev.rtt_ms,
                            ev.label,
                            ev.detail
                        )),
                );
            }
        } else {
            let offline_note = offline_diagnostic_report("Daemon offline");
            switch_card = switch_card.child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child(offline_note.attribution.summary),
            );
            feedback_card = feedback_card.child(
                div()
                    .text_xs()
                    .text_color(rgb(theme::INK_MUTED))
                    .child("Connect to reaulad to stream live RPC RTT telemetry."),
            );
        }

        bench_row = bench_row.child(switch_card).child(feedback_card);

        let mut root = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(header_card)
            .child(layers_grid);
        if let Some(vb) = visual_bench {
            root = root.child(vb);
        }
        root.child(bench_row)
    }

    fn render_diagnostics_visual_bench(
        &self,
        state: &ConnectedState,
        cx: &mut Context<Self>,
    ) -> Div {
        let total_keys = keymap::WIN60HE_KEYS.len();
        let verified_zero = keymap::WIN60HE_KEYS
            .iter()
            .filter(|k| {
                state
                    .switch_tests
                    .get(&k.index)
                    .is_some_and(|s| s.released_zero && s.peak_mm >= 0.30)
            })
            .count();
        let touched = keymap::WIN60HE_KEYS
            .iter()
            .filter(|k| {
                state
                    .switch_tests
                    .get(&k.index)
                    .is_some_and(|s| s.peak_mm > 0.05)
            })
            .count();
        let held_now = state.live_depths.len();

        let summary_strip = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(pill(
                        "dx-verified-zero",
                        format!("Verified 0.00 Return: {verified_zero}/{total_keys}"),
                        verified_zero == total_keys,
                    ))
                    .child(pill(
                        "dx-reached-travel",
                        format!("Reached Travel: {touched}/{total_keys}"),
                        touched > 0,
                    ))
                    .child(pill(
                        "dx-pressed-now",
                        format!("Pressed Now: {held_now}"),
                        held_now > 0,
                    )),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgb(theme::INK_MUTED))
                    .child("Cyan = live held · Green = OK 0.00 return · Amber = held/no return · Click key to inspect"),
            );

        let time_s = self.start_instant.elapsed().as_secs_f32();
        let depth_map_card = card_box(
            "All 61 Keycaps — Live & Peak Depth Check",
            Some("Press every key to full bottom-out to verify full analog reach and clean 0.00 mm release"),
        )
        .flex_1()
        .min_w(px(640.0))
        .child(summary_strip)
        .child(self.render_keyboard_canvas(
            state,
            KeyboardCanvasMode::DiagnosticsDepth,
            1.14,
            time_s,
            cx,
        ));

        let snap = state.focused_switch_inspection();
        let gpu_ready = self.switch_renderer_3d.is_ok();
        let effective_mode = if gpu_ready {
            state.diag_switch_view
        } else {
            SwitchViewMode::TwoD
        };

        let controls_row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .child(pill(
                        "dx-focused-sw",
                        format!("{} (#{}) · {}", snap.key_name, snap.key_index, snap.switch_label),
                        true,
                    ))
                    .child(
                        icon_pill(
                            "dx-follow-press",
                            Some(MaterialIcon::Sensors),
                            if state.diag_follow_press {
                                "Follow Press: ON"
                            } else {
                                "Follow Press: OFF"
                            },
                            state.diag_follow_press,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.diag_follow_press = !st.diag_follow_press;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_pill(
                            "dx-test-sweep",
                            Some(MaterialIcon::GraphicEq),
                            if state.diag_sweep_active {
                                "Stop Sweep"
                            } else {
                                "Test Sweep"
                            },
                            state.diag_sweep_active,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.diag_sweep_active = !st.diag_sweep_active;
                                st.diag_sweep_started_at =
                                    st.diag_sweep_active.then(std::time::Instant::now);
                                this.dirty_switch_3d = true;
                            }
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(
                        icon_pill(
                            "dx-sw-3d",
                            Some(MaterialIcon::ViewInAr),
                            if gpu_ready { "3D wgpu" } else { "3D N/A" },
                            effective_mode == SwitchViewMode::ThreeD,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.switch_renderer_3d.is_ok()
                                && let Ok(st) = &mut this.conn
                            {
                                st.diag_switch_view = SwitchViewMode::ThreeD;
                                this.dirty_switch_3d = true;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_pill(
                            "dx-sw-2d",
                            Some(MaterialIcon::Tune),
                            "2D Fallback",
                            effective_mode == SwitchViewMode::TwoD,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.diag_switch_view = SwitchViewMode::TwoD;
                            }
                            cx.notify();
                        })),
                    ),
            );

        let stage_view = match effective_mode {
            SwitchViewMode::ThreeD => {
                let presets_row = div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .children(
                        [
                            (SwitchCameraPreset::Iso, "Iso"),
                            (SwitchCameraPreset::Cutaway, "Cutaway"),
                            (SwitchCameraPreset::Side, "Side"),
                            (SwitchCameraPreset::Top, "Top"),
                        ]
                        .into_iter()
                        .map(|(preset, label)| {
                            let active = state.diag_switch_preset == Some(preset);
                            icon_pill(("dx-cam", preset as usize), None, label, active).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.set_switch_camera_preset(preset, cx);
                                }),
                            )
                        }),
                    );

                let viewport = match (&self.switch_renderer_3d, &self.preview_switch_3d_img) {
                    (Ok(_), Some(frame)) => div()
                        .id("switch-3d-viewport")
                        .w(px(SWITCH_3D_WIDTH as f32 * 0.78))
                        .h(px(SWITCH_3D_HEIGHT as f32 * 0.78))
                        .rounded_lg()
                        .overflow_hidden()
                        .border_1()
                        .border_color(rgb(if snap.triggered {
                            theme::BORDER_FOCUS
                        } else {
                            theme::BORDER_SUBTLE
                        }))
                        .cursor_grab()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                this.dragging_switch_3d = true;
                                this.last_mouse_switch_3d =
                                    Some((f32::from(ev.position.x), f32::from(ev.position.y)));
                                cx.notify();
                            }),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, _| {
                                this.dragging_switch_3d = false;
                                this.last_mouse_switch_3d = None;
                            }),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(|this, _, _, _| {
                                this.dragging_switch_3d = false;
                                this.last_mouse_switch_3d = None;
                            }),
                        )
                        .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                            if this.dragging_switch_3d && ev.dragging() {
                                let nx = f32::from(ev.position.x);
                                let ny = f32::from(ev.position.y);
                                if let Some((lx, ly)) = this.last_mouse_switch_3d
                                    && let Ok(st) = &mut this.conn
                                {
                                    st.diag_switch_cam.orbit(nx - lx, ny - ly);
                                    st.diag_switch_preset = None;
                                    this.dirty_switch_3d = true;
                                }
                                this.last_mouse_switch_3d = Some((nx, ny));
                                cx.notify();
                            }
                        }))
                        .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, _, cx| {
                            let dy = match ev.delta {
                                ScrollDelta::Lines(p) => p.y * 0.35,
                                ScrollDelta::Pixels(p) => f32::from(p.y) * 0.008,
                            };
                            if let Ok(st) = &mut this.conn {
                                st.diag_switch_cam.adjust_zoom(-dy);
                                st.diag_switch_preset = None;
                                this.dirty_switch_3d = true;
                            }
                            cx.notify();
                        }))
                        .child(img(frame.clone()).w_full().h_full())
                        .into_any_element(),
                    (Ok(_), None) => recessed_well()
                        .w(px(349.0))
                        .h(px(280.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(rgb(theme::INK_MUTED))
                        .child("Rendering wgpu 3D switch cutaway...")
                        .into_any_element(),
                    (Err(_), _) => render_switch_2d_cross_section(&snap).into_any_element(),
                };

                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .child(presets_row)
                    .child(viewport)
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgb(theme::INK_MUTED))
                            .child("Drag to orbit cutaway · Scroll to zoom · Live stem, magnet & coil spring"),
                    )
                    .into_any_element()
            }
            SwitchViewMode::TwoD => div()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .child(render_switch_2d_cross_section(&snap))
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(rgb(theme::INK_MUTED))
                        .child(if gpu_ready {
                            "2D Hall-effect cross-section schematic (manual mode)"
                        } else {
                            "2D Hall-effect cross-section fallback (wgpu 3D unavailable)"
                        }),
                )
                .into_any_element(),
        };

        let metrics_row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_2()
            .child(
                stat_tile(
                    if snap.sweeping { "Sweep Depth" } else { "Live Depth" },
                    format!("{:.2} / {:.2} mm", snap.travel_mm, snap.max_mm),
                    if snap.triggered {
                        theme::INK_OK
                    } else {
                        theme::INK_ACCENT
                    },
                )
                .flex_1(),
            )
            .child(
                stat_tile(
                    "Hall Flux / State",
                    format!(
                        "{:.0}% · {}",
                        snap.flux_pct,
                        if snap.triggered { "TRIGGERED" } else { "READY" }
                    ),
                    if snap.triggered {
                        theme::INK_OK
                    } else {
                        theme::INK_PRIMARY
                    },
                )
                .flex_1(),
            )
            .child(
                stat_tile(
                    "Actuation / Peak",
                    format!(
                        "{:.2} mm · Pk {:.2} mm",
                        snap.actuation_mm, snap.peak_mm
                    ),
                    theme::INK_SECONDARY,
                )
                .flex_1(),
            )
            .child(
                stat_tile(
                    "RT / Deadband",
                    if snap.rapid_trigger {
                        format!(
                            "RT +{:.2}/-{:.2} · DB {:.2}/{:.2}",
                            snap.rt_press_mm,
                            snap.rt_release_mm,
                            snap.db_top_mm,
                            snap.db_bottom_mm
                        )
                    } else {
                        format!("Fixed · DB {:.2}/{:.2}", snap.db_top_mm, snap.db_bottom_mm)
                    },
                    theme::INK_MUTED,
                )
                .flex_1(),
            )
            .child(
                stat_tile(
                    "Return Check",
                    if snap.released_zero {
                        "0.00 OK"
                    } else if snap.presses > 0 {
                        "Check 0.00"
                    } else {
                        "Untested"
                    },
                    if snap.released_zero {
                        theme::INK_OK
                    } else {
                        theme::INK_WARN
                    },
                )
                .flex_1(),
            );

        let switch_mech_card = card_box(
            "Hall-Effect Switch Mechanism",
            Some("Cutaway stem, coil spring, magnet & PCB Hall sensor (wgpu 3D + 2D fallback)"),
        )
        .flex_1()
        .min_w(px(380.0))
        .child(controls_row)
        .child(stage_view)
        .child(metrics_row);

        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap_4()
            .child(depth_map_card)
            .child(switch_mech_card)
    }
}
