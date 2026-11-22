//! Color & Light workbench: 3D/2D keyboard studio paired with side inspector.

use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::ColorPicker;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::select::Select;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, Div, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, ParentElement as _,
    ScrollDelta, ScrollEvent, Styled as _, div, img, prelude::*, px, rgb,
};

use atrium_core::audio_fx::{AudioPalette, NUM_BANDS};
use atrium_core::keymap;
use atrium_core::protocol::{self, LIGHT_MAX_LEVEL};

use crate::AtriumGui;
use crate::kb_3d::{CameraPreset, KB3D_HEIGHT, KB3D_WIDTH};
use crate::state::{
    ConnectedState, LightEditorTab, LightViewMode, describe_rgb_human, format_rgb_hex,
    pack_rgb_u32,
};
use crate::theme::HardwareColors;
use crate::widgets::{
    COLOR_SWATCHES, KeyboardCanvasMode, MetricCell, render_metric_table,
};

impl AtriumGui {
    pub(crate) fn view_color_light(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let time_s = self.start_instant.elapsed().as_secs_f32();
        let ls = &state.light_state;
        let gpu_ready = self.renderer_3d.is_ok();
        let view_mode = if gpu_ready {
            state.light_view_mode
        } else {
            LightViewMode::TwoD
        };

        let tab_idx = match state.light_tab {
            LightEditorTab::FirmwareEffects => 0,
            LightEditorTab::CustomSlot1 => 1,
            LightEditorTab::CustomSlot2 => 2,
            LightEditorTab::AudioSync => 3,
        };

        let mode_tabs = TabBar::new("light-editor-tabs")
            .segmented()
            .small()
            .selected_index(tab_idx)
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                if let Ok(st) = &mut this.conn {
                    st.light_tab = match *ix {
                        1 => LightEditorTab::CustomSlot1,
                        2 => LightEditorTab::CustomSlot2,
                        3 => LightEditorTab::AudioSync,
                        _ => LightEditorTab::FirmwareEffects,
                    };
                    this.dirty_3d = true;
                }
                cx.notify();
            }))
            .child(Tab::new().label("Effect"))
            .child(Tab::new().label("Slot 1"))
            .child(Tab::new().label("Slot 2"))
            .child(
                Tab::new().label(if state.audio_active {
                    "Audio (Live)"
                } else {
                    "Audio Sync"
                }),
            );

        let view_tabs = TabBar::new("light-stage-view")
            .segmented()
            .small()
            .selected_index(usize::from(view_mode == LightViewMode::ThreeD))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                if let Ok(st) = &mut this.conn {
                    st.light_view_mode = if *ix == 1 && this.renderer_3d.is_ok() {
                        this.dirty_3d = true;
                        LightViewMode::ThreeD
                    } else {
                        LightViewMode::TwoD
                    };
                }
                this.maybe_render_3d();
                cx.notify();
            }))
            .child(Tab::new().label("2D"))
            .child(Tab::new().label(if gpu_ready { "3D" } else { "3D N/A" }));

        let mut toolbar = h_flex()
            .w_full()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2()
            .child(mode_tabs)
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .when(view_mode == LightViewMode::ThreeD, |row| {
                        row.children(
                            [
                                ("cam-iso", CameraPreset::Perspective, "Iso"),
                                ("cam-top", CameraPreset::Top, "Top"),
                                ("cam-side", CameraPreset::Side, "Side"),
                                ("cam-front", CameraPreset::Front, "Front"),
                            ]
                            .into_iter()
                            .map(|(id, preset, lbl)| {
                                Button::new(id)
                                    .xsmall()
                                    .outline()
                                    .selected(state.camera_preset == Some(preset))
                                    .label(lbl)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.set_camera_preset(preset, cx);
                                    }))
                            }),
                        )
                    })
                    .child(
                        Button::new("anim-stage")
                            .xsmall()
                            .outline()
                            .selected(state.animate_3d)
                            .icon(Icon::new(if state.animate_3d {
                                IconName::Pause
                            } else {
                                IconName::Play
                            }))
                            .label(if state.animate_3d { "Pause" } else { "Animate" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.animate_3d = !st.animate_3d;
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("calm-idle")
                            .xsmall()
                            .outline()
                            .selected(state.typing_only_effects)
                            .label("Calm Idle")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.typing_only_effects = !st.typing_only_effects;
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                    )
                    .child(view_tabs),
            );

        if view_mode == LightViewMode::TwoD && state.light_tab.custom_slot_index().is_some() {
            toolbar = toolbar.child(
                Switch::new("stage-paint-sw")
                    .small()
                    .checked(state.custom_paint_mode)
                    .label("Paint Clicks")
                    .on_change(cx.listener(|this, checked: &bool, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            st.custom_paint_mode = *checked;
                        }
                        cx.notify();
                    })),
            );
        }

        let stage_body = match view_mode {
            LightViewMode::ThreeD => match (&self.renderer_3d, &self.preview_3d_img) {
                (Ok(_), Some(frame)) => v_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .child(
                        div()
                            .id("kb-3d-viewport")
                            .w_full()
                            .max_w(px((KB3D_WIDTH as f32 * 0.78).min(648.0)))
                            .h(px((KB3D_HEIGHT as f32 * 0.78).min(365.0)))
                            .rounded(cx.theme().radius_lg)
                            .overflow_hidden()
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_grab()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    this.dragging_3d = true;
                                    this.last_mouse_3d =
                                        Some((f32::from(ev.position.x), f32::from(ev.position.y)));
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
                            .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| {
                                let dy = match ev.delta {
                                    ScrollDelta::Lines(p) => p.y * 0.45,
                                    ScrollDelta::Pixels(p) => f32::from(p.y) * 0.01,
                                };
                                if let Ok(st) = &mut this.conn {
                                    st.camera_3d.adjust_zoom(-dy);
                                    st.camera_preset = None;
                                    this.dirty_3d = true;
                                }
                                cx.stop_propagation();
                                cx.notify();
                            }))
                            .child(
                                img(frame.clone())
                                    .w_full()
                                    .h_full()
                                    .object_fit(ObjectFit::Contain),
                            ),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "Drag orbit · Wheel zoom · yaw {:.2} pitch {:.2}",
                                state.camera_3d.yaw, state.camera_3d.pitch
                            )),
                    )
                    .into_any_element(),
                _ => v_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(self.render_keyboard_canvas(
                        state,
                        KeyboardCanvasMode::ColorMockup,
                        1.0,
                        time_s,
                        cx,
                    ))
                    .into_any_element(),
            },
            LightViewMode::TwoD => v_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .child(self.render_keyboard_canvas(
                    state,
                    KeyboardCanvasMode::ColorMockup,
                    1.0,
                    time_s,
                    cx,
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if state.light_tab.custom_slot_index().is_some() {
                            "Click key to paint · Right-click to eyedrop"
                        } else {
                            "2D live LED projection"
                        }),
                )
                .into_any_element(),
        };

        let active_layer_label = if state.audio_active {
            format!("Audio · {}", state.audio_config.style.label())
        } else {
            match state.light_tab {
                LightEditorTab::FirmwareEffects => protocol::light_mode_name(ls.mode).to_string(),
                LightEditorTab::CustomSlot1 => "Custom Slot 1".into(),
                LightEditorTab::CustomSlot2 => "Custom Slot 2".into(),
                LightEditorTab::AudioSync => "Audio Sync".into(),
            }
        };

        let color_power = format!(
            "{} · {}",
            if ls.full_color {
                "Rainbow".into()
            } else {
                format_rgb_hex(ls.fg)
            },
            if ls.power_on { "ON" } else { "OFF" }
        );

        let telemetry_bar = render_metric_table(
            vec![
                MetricCell::text("Layer", active_layer_label, cx),
                MetricCell::text(
                    "Brightness",
                    format!("{} / {}", ls.brightness, LIGHT_MAX_LEVEL),
                    cx,
                ),
                MetricCell::text("Speed", format!("{} / {}", ls.speed, LIGHT_MAX_LEVEL), cx),
                MetricCell::text("Color / Power", color_power, cx),
            ],
            125.0,
            cx,
        );

        let stage_column = v_flex()
            .flex_1()
            .min_w(px(450.0))
            .overflow_hidden()
            .gap_3()
            .child(
                GroupBox::new()
                    .outline()
                    .overflow_hidden()
                    .child(toolbar)
                    .child(stage_body)
                    .child(telemetry_bar),
            );

        let inspector_column = div()
            .w(px(360.0))
            .max_w_full()
            .flex_shrink_0()
            .overflow_hidden()
            .child(match state.light_tab {
                LightEditorTab::FirmwareEffects => self.render_firmware_light_editor(state, cx),
                LightEditorTab::CustomSlot1 => self.render_custom_slot_editor(state, 0, cx),
                LightEditorTab::CustomSlot2 => self.render_custom_slot_editor(state, 1, cx),
                LightEditorTab::AudioSync => self.render_audio_sync_editor(state, cx),
            });

        div().w_full().min_w_0().child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_start()
                .gap_3()
                .child(stage_column)
                .child(inspector_column),
        )
    }

    fn render_firmware_light_editor(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let ls = &state.light_state;

        let card = GroupBox::new()
            .outline()
            .title("Firmware Lighting")
            .child(
                v_flex()
                    .gap_1p5()
                    .child(div().text_xs().child("Effect"))
                    .child(Select::new(&self.sel_light_effect).small()),
            )
            .child(
                v_flex()
                    .gap_1p5()
                    .child(div().text_xs().child("Direction"))
                    .child(Select::new(&self.sel_light_direction).small()),
            )
            .child(self.slider_metric_row(
                "Brightness",
                format!("{} / {}", ls.brightness, LIGHT_MAX_LEVEL),
                &self.slider_light_bri,
                cx,
            ))
            .child(self.slider_metric_row(
                "Animation Speed",
                format!("{} / {}", ls.speed, LIGHT_MAX_LEVEL),
                &self.slider_light_spd,
                cx,
            ))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        Switch::new("fw-power-sw")
                            .small()
                            .checked(ls.power_on)
                            .label("LED Power")
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.light_state.power_on = *checked;
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Switch::new("fw-rainbow-sw")
                            .small()
                            .checked(ls.full_color)
                            .label("Rainbow")
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.light_state.full_color = *checked;
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(self.render_color_channel_row(
                "fg",
                "Foreground Color",
                ls.fg,
                &self.picker_light_fg,
                |st, sw| {
                    st.light_state.fg = sw;
                    st.light_state.full_color = false;
                },
                cx,
            ))
            .child(self.render_color_channel_row(
                "bg",
                "Background Color",
                ls.bg,
                &self.picker_light_bg,
                |st, sw| {
                    st.light_state.bg = sw;
                },
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("apply-fw-light")
                            .primary()
                            .small()
                            .flex_1()
                            .icon(Icon::new(IconName::Check))
                            .label("Apply Lighting")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.apply_lighting(cx);
                            })),
                    )
                    .child(
                        Button::new("reload-fw-light")
                            .outline()
                            .small()
                            .icon(Icon::new(IconName::RefreshCw))
                            .label("Reload")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reload_lighting(window, cx);
                            })),
                    ),
            );

        div().w_full().child(card)
    }

    fn render_custom_slot_editor(
        &self,
        state: &ConnectedState,
        slot: u8,
        cx: &mut Context<Self>,
    ) -> Div {
        let card = GroupBox::new()
            .outline()
            .title(format!("Custom Matrix Slot {}", slot + 1))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        Switch::new("paint-mode-sw")
                            .small()
                            .checked(state.custom_paint_mode)
                            .label("Click to Paint")
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.custom_paint_mode = *checked;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("paint-clear")
                            .xsmall()
                            .danger()
                            .icon(Icon::new(IconName::Trash))
                            .label("Clear")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.custom_slots[usize::from(slot)] =
                                        [[0, 0, 0]; protocol::MATRIX_SLOTS];
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("paint-fill-sel")
                            .xsmall()
                            .outline()
                            .flex_1()
                            .label("Fill Selected")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    let col = st.custom_brush;
                                    for idx in st.target_indices() {
                                        st.custom_slots[usize::from(slot)][idx as usize] = col;
                                    }
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("paint-fill-all")
                            .xsmall()
                            .outline()
                            .flex_1()
                            .label("Fill All 61")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    let col = st.custom_brush;
                                    for k in keymap::WIN60HE_KEYS {
                                        st.custom_slots[usize::from(slot)][k.index as usize] = col;
                                    }
                                    this.dirty_3d = true;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(self.slider_metric_row(
                "Slot Brightness",
                format!("{} / {}", state.custom_brightness, LIGHT_MAX_LEVEL),
                &self.slider_custom_bri,
                cx,
            ))
            .child(self.render_color_channel_row(
                "brush",
                "Brush Color",
                state.custom_brush,
                &self.picker_custom_brush,
                |st, sw| {
                    st.custom_brush = sw;
                    st.custom_paint_mode = true;
                },
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("upload-custom-rgb")
                            .primary()
                            .small()
                            .flex_1()
                            .icon(Icon::new(IconName::Check))
                            .label(format!("Upload Slot {}", slot + 1))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.upload_custom_rgb(slot, cx);
                            })),
                    )
                    .child(
                        Button::new("reload-custom-rgb")
                            .outline()
                            .small()
                            .icon(Icon::new(IconName::RefreshCw))
                            .label("Reload")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.reload_custom_rgb(slot, cx);
                            })),
                    ),
            );

        div().w_full().child(card)
    }

    fn render_audio_sync_editor(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let cfg = &state.audio_config;
        let hc = HardwareColors::from_app(cx);
        let bands = state
            .audio_metrics
            .as_ref()
            .map_or([0.0f32; NUM_BANDS], |m| m.bands);
        let beat_hit = state
            .audio_metrics
            .as_ref()
            .is_some_and(|m| m.beat || m.beat_env > 0.35);
        let rms = state
            .audio_metrics
            .as_ref()
            .map_or("-90.0 dBFS".into(), |m| format!("{:.1} dBFS", m.rms_dbfs.max(-90.0)));

        let spectrum = v_flex()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .p_2p5()
            .gap_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(state.audio_status_text.clone()),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_2()
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_xs()
                                    .text_color(rgb(if beat_hit { hc.ok_ink } else { hc.key_dim_ink }))
                                    .child("BEAT"),
                            )
                            .child(div().text_color(cx.theme().muted_foreground).child(rms)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .h(px(40.0))
                    .items_end()
                    .gap_1()
                    .children((0..NUM_BANDS).map(|b| {
                        let lvl = bands[b].clamp(0.0, 1.0);
                        let fill_h = (lvl * 36.0).max(2.0);
                        div()
                            .flex_1()
                            .h_full()
                            .rounded_xs()
                            .bg(cx.theme().muted)
                            .flex()
                            .items_end()
                            .overflow_hidden()
                            .child(
                                div()
                                    .w_full()
                                    .h(px(fill_h))
                                    .bg(if lvl > 0.7 {
                                        cx.theme().info
                                    } else {
                                        cx.theme().foreground
                                    }),
                            )
                    })),
            );

        let mut card = GroupBox::new()
            .outline()
            .overflow_hidden()
            .title("Audio-Reactive RGB (Experimental)")
            .child(
                Alert::warning(
                    "audio-risk-banner",
                    if state.audio_disclaimer_accepted {
                        "Experimental mode active-ready. Continues in atriumd after closing GUI."
                    } else {
                        "Hardware risk acknowledgment required before starting audio streaming."
                    },
                ),
            )
            .child(spectrum)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1p5()
                    .child(div().text_xs().child("Audio Capture Source"))
                    .child(Select::new(&self.sel_audio_source).small()),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().text_xs().child("Effect"))
                            .child(Select::new(&self.sel_audio_effect).small()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(div().text_xs().child("Palette"))
                            .child(Select::new(&self.sel_audio_palette).small()),
                    ),
            )
            .child(self.slider_metric_row(
                "Stream FPS",
                format!("{} fps", cfg.fps),
                &self.slider_audio_fps,
                cx,
            ))
            .child(self.slider_metric_row(
                "Brightness",
                format!("{} / {}", cfg.brightness, LIGHT_MAX_LEVEL),
                &self.slider_audio_bri,
                cx,
            ))
            .child(self.slider_metric_row(
                "Input Gain",
                format!("{:.1}x", cfg.gain),
                &self.slider_audio_gain,
                cx,
            ));

        if cfg.palette == AudioPalette::Custom {
            card = card
                .child(self.render_color_channel_row(
                    "afg",
                    "Audio Foreground",
                    cfg.fg,
                    &self.picker_audio_fg,
                    |st, sw| {
                        st.audio_config.fg = sw;
                        st.push_audio_live_config();
                    },
                    cx,
                ))
                .child(self.render_color_channel_row(
                    "abg",
                    "Audio Background",
                    cfg.bg,
                    &self.picker_audio_bg,
                    |st, sw| {
                        st.audio_config.bg = sw;
                        st.push_audio_live_config();
                    },
                    cx,
                ));
        }

        let main_btn = if state.audio_active {
            Button::new("stop-audio-sync")
                .danger()
                .small()
                .flex_1()
                .icon(Icon::new(IconName::Pause))
                .label("Stop Audio Sync")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.stop_audio_sync(cx);
                }))
        } else {
            Button::new("start-audio-sync")
                .primary()
                .small()
                .flex_1()
                .icon(Icon::new(IconName::Play))
                .label("Start Audio Sync…")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.request_start_audio_sync(window, cx);
                }))
        };

        div().w_full().child(
            card.child(
                h_flex()
                    .gap_2()
                    .child(main_btn)
                    .child(
                        Button::new("refresh-audio-src")
                            .outline()
                            .small()
                            .icon(Icon::new(IconName::RefreshCw))
                            .label("Sources")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.refresh_audio_sources();
                                    this.status_line = format!(
                                        "Detected {} ALSA capture source(s).",
                                        st.audio_sources.len()
                                    );
                                }
                                this.sync_audio_source_select(window, cx);
                                cx.notify();
                            })),
                    ),
            ),
        )
    }

    fn render_color_channel_row<F>(
        &self,
        id_prefix: &'static str,
        label: &'static str,
        col: [u8; 3],
        picker_state: &gpui_kit::Entity<gpui_kit::component::color_picker::ColorPickerState>,
        on_swatch: F,
        cx: &mut Context<Self>,
    ) -> Div
    where
        F: Fn(&mut ConnectedState, [u8; 3]) + Copy + 'static,
    {
        let human = describe_rgb_human(col);
        let hex = format_rgb_hex(col);
        let swatches = COLOR_SWATCHES
            .iter()
            .map(|&(_, sw)| rgb_u8_to_hsla(sw))
            .collect::<Vec<_>>();

        v_flex()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .gap_1p5()
            .p_2()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().w_full().min_w_0().truncate().text_xs().child(label))
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{human} · {hex}")),
                            ),
                    )
                    .child(
                        ColorPicker::new(picker_state)
                            .small()
                            .featured_colors(swatches)
                            .accessibility_label(label),
                    ),
            )
            .child(
                h_flex().flex_wrap().gap_1().children(COLOR_SWATCHES.iter().enumerate().map(
                    |(idx, &(name, sw))| {
                        let active = col == sw;
                        let picker = picker_state.clone();
                        div()
                            .id((id_prefix, idx))
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded_xs()
                            .bg(rgb(pack_rgb_u32(sw)))
                            .border_1()
                            .border_color(if active {
                                cx.theme().ring
                            } else {
                                cx.theme().border
                            })
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Ok(st) = &mut this.conn {
                                    on_swatch(st, sw);
                                    this.dirty_3d = true;
                                    this.status_line = format!("{label}: {name} ({})", format_rgb_hex(sw));
                                }
                                picker.update(cx, |p, cx| {
                                    p.set_value(rgb_u8_to_hsla(sw), window, cx);
                                });
                                cx.notify();
                            }))
                    },
                )),
            )
    }
}

pub(crate) fn rgb_u8_to_hsla(c: [u8; 3]) -> gpui_kit::Hsla {
    gpui_kit::Rgba {
        r: f32::from(c[0]) / 255.0,
        g: f32::from(c[1]) / 255.0,
        b: f32::from(c[2]) / 255.0,
        a: 1.0,
    }
    .into()
}

pub(crate) fn hsla_to_rgb_u8(h: gpui_kit::Hsla) -> [u8; 3] {
    let r: gpui_kit::Rgba = h.into();
    [
        (r.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (r.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (r.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}
