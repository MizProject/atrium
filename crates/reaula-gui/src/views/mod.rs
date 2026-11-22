//! Workspace views for `reaula-gui`.

mod calibration;
mod diagnostics;
mod keymaps;
pub(crate) mod lighting;
mod main_view;
mod settings;
mod setup;

use std::cell::Cell;

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{AnyElement, Context, IntoElement as _, ParentElement as _, Styled as _, Window, div, px};

use crate::ReaulaGui;
use crate::config::ConfiguredKeyboardKind;
use crate::state::{AUDIO_DISCLAIMER_COOLDOWN_SECS, NavSection};

impl ReaulaGui {
    pub(crate) fn render_active_section(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_unsupported = self
            .config
            .active_keyboard()
            .filter(|k| k.kind == ConfiguredKeyboardKind::Unsupported);

        match self.nav {
            NavSection::Setup => self.view_setup(cx).into_any_element(),
            NavSection::Main => self.view_main_hub(cx).into_any_element(),
            NavSection::Diagnostics => self.view_diagnostics(cx).into_any_element(),
            NavSection::Settings => self.view_settings(cx).into_any_element(),
            NavSection::ColorLight => {
                if let Some(kb) = active_unsupported {
                    self.view_unsupported_board_panel(kb, cx).into_any_element()
                } else {
                    match &self.conn {
                        Err(err) => self.view_disconnected(err, cx),
                        Ok(state) => self.view_color_light(state, cx).into_any_element(),
                    }
                }
            }
            _ => {
                if let Some(kb) = active_unsupported {
                    v_flex()
                        .gap_3()
                        .child(
                            Alert::info(
                                "unsupported-section-note",
                                format!(
                                    "{} is configured as an Unsupported / Probe keyboard. Per-key magnetic calibration and firmware flash keymaps apply to natively supported drivers (such as AULA WIN 60 HE).",
                                    kb.label
                                ),
                            )
                            .title("Hardware Feature Requires Supported Driver"),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("unsup-open-main")
                                        .small()
                                        .primary()
                                        .label(self.text().nav_main)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.switch_section(NavSection::Main, cx);
                                        })),
                                )
                                .child(
                                    Button::new("unsup-open-probe")
                                        .small()
                                        .outline()
                                        .label("Probe HID / Submit Issue\u{2026}")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.open_setup_at_step(
                                                3,
                                                Some(crate::state::SetupHardwareMode::UnsupportedProbe),
                                                window,
                                                cx,
                                            );
                                        })),
                                ),
                        )
                        .into_any_element()
                } else {
                    match &self.conn {
                        Err(err) => self.view_disconnected(err, cx),
                        Ok(state) => match self.nav {
                            NavSection::Calibration => {
                                self.view_calibration(state, cx).into_any_element()
                            }
                            NavSection::Hotkeys => {
                                self.view_hotkeys(state, window, cx).into_any_element()
                            }
                            _ => unreachable!(),
                        },
                    }
                }
            }
        }
    }

    fn view_disconnected(&self, err: &str, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .child(
                Alert::error("daemon-offline", err.to_string())
                    .title("Daemon Not Connected"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Start daemon: cargo run -p reaula-cli -- daemon start"),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("offline-diag-btn")
                            .primary()
                            .small()
                            .icon(Icon::new(IconName::Activity))
                            .label("Run Diagnostics")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.switch_section(NavSection::Diagnostics, cx);
                                this.trigger_diagnostics(cx);
                            })),
                    )
                    .child(
                        Button::new("offline-reconnect")
                            .outline()
                            .small()
                            .icon(Icon::new(IconName::RefreshCw))
                            .label("Reconnect")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reconnect(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn open_audio_disclaimer_dialog(
        &mut self,
        pending_start: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(state) = &mut self.conn {
            state.open_audio_disclaimer(pending_start);
        }
        let view = cx.entity().clone();
        let view_cancel = view.clone();
        let initial_opened = Cell::new(true);

        window.open_dialog(cx, move |dialog, _window, cx| {
            let _ = initial_opened.get();
            let (rem, checked, pending, can_confirm) = view.read(cx).conn.as_ref().map_or(
                (AUDIO_DISCLAIMER_COOLDOWN_SECS, false, false, false),
                |st| {
                    (
                        st.disclaimer_remaining_secs(),
                        st.audio_disclaimer_checked,
                        st.audio_disclaimer_pending_start,
                        st.can_confirm_audio_disclaimer(),
                    )
                },
            );
            let cooldown_done = rem <= 0.0;
            let elapsed_pct = (((AUDIO_DISCLAIMER_COOLDOWN_SECS - rem) / AUDIO_DISCLAIMER_COOLDOWN_SECS)
                .clamp(0.0, 1.0)
                * 100.0) as f32;

            let view_chk = view.clone();
            let view_confirm = view.clone();
            let view_btn_cancel = view.clone();

            let confirm_label = if !cooldown_done {
                format!("Wait {:.0}s", rem.ceil())
            } else if !checked {
                "Agree to Continue".into()
            } else if pending {
                "Enable Audio Sync".into()
            } else {
                "Acknowledge Risk".into()
            };

            let body = v_flex()
                .gap_3()
                .child(
                    Alert::warning(
                        "audio-warn-box",
                        "AULA states WIN 60 HE does not officially support host audio sync lighting. Continuous per-key cmd 0x09 streaming is experimental and may shorten LED lifespan.",
                    )
                    .title("Experimental Hardware Notice"),
                )
                .child(
                    v_flex()
                        .gap_1p5()
                        .child(
                            h_flex()
                                .justify_between()
                                .text_xs()
                                .child("Review cooldown (10s)")
                                .child(div().text_color(cx.theme().muted_foreground).child(
                                    if cooldown_done {
                                        "Ready".to_string()
                                    } else {
                                        format!("{:.0}s remaining", rem.ceil())
                                    },
                                )),
                        )
                        .child(Progress::new("audio-disclaimer-progress").value(elapsed_pct)),
                )
                .child(
                    Checkbox::new("audio-disclaimer-check")
                        .checked(checked)
                        .label("I understand this is an unsupported workaround and accept full hardware risk.")
                        .on_click(move |next: &bool, _, cx| {
                            view_chk.update(cx, |this, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.audio_disclaimer_checked = *next;
                                }
                                cx.notify();
                            });
                        }),
                );

            let footer = DialogFooter::new()
                .child(
                    Button::new("audio-disc-cancel")
                        .outline()
                        .small()
                        .label("Cancel")
                        .on_click(move |_, window, cx| {
                            view_btn_cancel.update(cx, |this, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.close_audio_disclaimer();
                                }
                                cx.notify();
                            });
                            window.close_dialog(cx);
                        }),
                )
                .child(
                    Button::new("audio-disc-ok")
                        .small()
                        .with_variant(if can_confirm {
                            gpui_kit::component::button::ButtonVariant::Warning
                        } else {
                            gpui_kit::component::button::ButtonVariant::Secondary
                        })
                        .disabled(!can_confirm)
                        .icon(Icon::new(if can_confirm {
                            IconName::Check
                        } else {
                            IconName::TriangleAlert
                        }))
                        .label(confirm_label)
                        .on_click(move |_, window, cx| {
                            let accepted = view_confirm.update(cx, |this, cx| {
                                this.confirm_audio_disclaimer(cx)
                            });
                            if accepted {
                                window.close_dialog(cx);
                            }
                        }),
                );

            dialog
                .w(px(560.0))
                .title("Experimental Audio Sync Warning")
                .close_button(false)
                .child(body)
                .footer(footer)
                .on_cancel({
                    let vc = view_cancel.clone();
                    move |_, _, cx| {
                        vc.update(cx, |this, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.close_audio_disclaimer();
                            }
                            cx.notify();
                        });
                        true
                    }
                })
        });
    }
}
