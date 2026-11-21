//! Diagnostics workspace: 4-layer fault probe, 61-key depth check, and 3D/2D switch cutaway.

use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, Div, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, ScrollDelta,
    ScrollEvent, Styled as _, div, img, prelude::*, px,
};

use reaula_core::diagnostics::{CheckStatus, DiagnosticLayer, DiagnosticReport};
use reaula_core::keymap;

use crate::ReaulaGui;
use crate::kb_3d::{SWITCH_3D_HEIGHT, SWITCH_3D_WIDTH, SwitchCameraPreset};
use crate::state::{ConnectedState, SwitchViewMode, offline_diagnostic_report};
use crate::theme::HardwareColors;
use crate::widgets::{KeyboardCanvasMode, render_switch_2d_cross_section};

impl ReaulaGui {
    pub(crate) fn view_diagnostics(&self, cx: &mut Context<Self>) -> Div {
        let report_opt: Option<DiagnosticReport> = match &self.conn {
            Ok(st) => st.diag_report.clone(),
            Err(_) => self.offline_diag.clone(),
        };

        let alert = match (&report_opt, report_opt.as_ref().and_then(|r| r.attribution.primary_fault))
        {
            (Some(rep), None) => Alert::success("dx-summary", rep.attribution.summary.clone()),
            (Some(rep), Some(_)) => Alert::error("dx-summary", rep.attribution.summary.clone()),
            _ => Alert::info(
                "dx-summary",
                "Run 4-Layer Diagnostic to isolate Kernel/USB, Daemon IPC, Keyboard HID, and Software checks.",
            ),
        };

        let mut header_box = GroupBox::new()
            .outline()
            .title("4-Layer Fault Isolation")
            .child(alert)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("dx-run-all")
                            .primary()
                            .small()
                            .icon(Icon::new(IconName::Activity))
                            .label("Run 4-Layer Diagnostic")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.trigger_diagnostics(cx);
                            })),
                    )
                    .child(
                        Button::new("dx-echo")
                            .outline()
                            .small()
                            .icon(Icon::new(IconName::Zap))
                            .label("5x HID Echo RTT")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    match st.run_echo_benchmark() {
                                        Ok(msg) => this.status_line = msg,
                                        Err(e) => this.status_line = e,
                                    }
                                } else {
                                    this.status_line = "Daemon offline.".into();
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("dx-zero-sw")
                            .outline()
                            .small()
                            .icon(Icon::new(IconName::RotateCcw))
                            .label("Clear Test Peaks")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.reset_switch_tests();
                                    this.dirty_switch_3d = true;
                                    this.status_line = "Cleared switch response peaks.".into();
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
            header_box = header_box.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Echo samples: {lat_txt}")),
            );
        }

        let mut layers_grid = v_flex().gap_3();
        if let Some(rep) = &report_opt {
            for pair in [
                [DiagnosticLayer::Kernel, DiagnosticLayer::Daemon],
                [DiagnosticLayer::Keyboard, DiagnosticLayer::Software],
            ] {
                let mut pair_row = h_flex().flex_wrap().items_stretch().gap_3();
                for layer in pair {
                    let mut layer_box = GroupBox::new().outline().title(layer.label());
                    let checks: Vec<_> = rep.checks.iter().filter(|c| c.layer == layer).collect();
                    for c in checks {
                        let st_col = match c.status {
                            CheckStatus::Pass => cx.theme().success,
                            CheckStatus::Warn => cx.theme().warning,
                            CheckStatus::Fail => cx.theme().danger,
                        };
                        let lat = c
                            .latency_ms
                            .map_or_else(String::new, |m| format!(" ({m:.2} ms)"));
                        let mut item = v_flex()
                            .p_2()
                            .gap_0p5()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(st_col)
                                    .child(format!("[{}] {}{}", c.status.label(), c.name, lat)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(c.detail.clone()),
                            );
                        if let Some(rem) = &c.remedy {
                            item = item.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().warning)
                                    .child(format!("Fix: {rem}")),
                            );
                        }
                        layer_box = layer_box.child(item);
                    }
                    pair_row = pair_row.child(div().flex_1().min_w(px(340.0)).child(layer_box));
                }
                layers_grid = layers_grid.child(pair_row);
            }
        }

        let visual_bench = self
            .conn
            .as_ref()
            .ok()
            .map(|st| self.render_diagnostics_visual_bench(st, cx));

        let bench_row = self.render_diagnostics_bottom_row(cx);

        let mut root = v_flex().w_full().gap_3().child(header_box).child(layers_grid);
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

        let time_s = self.start_instant.elapsed().as_secs_f32();
        let depth_box = GroupBox::new()
            .outline()
            .title("Keycap Depth & 0.00 mm Return Check")
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "Verified return: {verified_zero}/{total_keys} · Reached: {touched}/{total_keys} · Held: {}",
                        state.live_depths.len()
                    ))
                    .child("Green OK · Amber check · Click to inspect"),
            )
            .child(self.render_keyboard_canvas(
                state,
                KeyboardCanvasMode::DiagnosticsDepth,
                1.06,
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

        let sw_controls = h_flex()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().text_xs().child(format!(
                        "{} (#{}) · {}",
                        snap.key_name, snap.key_index, snap.switch_label
                    )))
                    .child(
                        Switch::new("dx-follow-sw")
                            .small()
                            .checked(state.diag_follow_press)
                            .label("Follow")
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                if let Ok(st) = &mut this.conn {
                                    st.diag_follow_press = *checked;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("dx-sweep")
                            .xsmall()
                            .outline()
                            .selected(state.diag_sweep_active)
                            .label(if state.diag_sweep_active {
                                "Stop Sweep"
                            } else {
                                "Sweep"
                            })
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
                TabBar::new("dx-sw-mode")
                    .segmented()
                    .small()
                    .selected_index(usize::from(effective_mode == SwitchViewMode::ThreeD))
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Ok(st) = &mut this.conn {
                            st.diag_switch_view = if *ix == 1 && this.switch_renderer_3d.is_ok() {
                                this.dirty_switch_3d = true;
                                SwitchViewMode::ThreeD
                            } else {
                                SwitchViewMode::TwoD
                            };
                        }
                        this.maybe_render_switch_3d();
                        cx.notify();
                    }))
                    .child(Tab::new().label("2D"))
                    .child(Tab::new().label(if gpu_ready { "3D" } else { "3D N/A" })),
            );

        let hc = HardwareColors::from_app(cx);
        let stage = match effective_mode {
            SwitchViewMode::ThreeD => {
                let presets = h_flex().gap_1().children(
                    [
                        (SwitchCameraPreset::Iso, "Iso"),
                        (SwitchCameraPreset::Cutaway, "Cutaway"),
                        (SwitchCameraPreset::Side, "Side"),
                        (SwitchCameraPreset::Top, "Top"),
                    ]
                    .into_iter()
                    .map(|(preset, label)| {
                        Button::new(("dx-cam", preset as usize))
                            .xsmall()
                            .outline()
                            .selected(state.diag_switch_preset == Some(preset))
                            .label(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_switch_camera_preset(preset, cx);
                            }))
                    }),
                );

                let viewport = match (&self.switch_renderer_3d, &self.preview_switch_3d_img) {
                    (Ok(_), Some(frame)) => div()
                        .id("switch-3d-viewport")
                        .w(px(SWITCH_3D_WIDTH as f32 * 0.74))
                        .h(px(SWITCH_3D_HEIGHT as f32 * 0.74))
                        .rounded(cx.theme().radius)
                        .overflow_hidden()
                        .border_1()
                        .border_color(cx.theme().border)
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
                        .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| {
                            let dy = match ev.delta {
                                ScrollDelta::Lines(p) => p.y * 0.35,
                                ScrollDelta::Pixels(p) => f32::from(p.y) * 0.008,
                            };
                            if let Ok(st) = &mut this.conn {
                                st.diag_switch_cam.adjust_zoom(-dy);
                                st.diag_switch_preset = None;
                                this.dirty_switch_3d = true;
                            }
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .child(img(frame.clone()).w_full().h_full())
                        .into_any_element(),
                    _ => render_switch_2d_cross_section(&snap, &hc).into_any_element(),
                };

                v_flex().items_center().gap_1p5().child(presets).child(viewport).into_any_element()
            }
            SwitchViewMode::TwoD => v_flex()
                .items_center()
                .child(render_switch_2d_cross_section(&snap, &hc))
                .into_any_element(),
        };

        let metrics = DescriptionList::horizontal()
            .columns(3)
            .bordered(true)
            .small()
            .item("Depth", format!("{:.2}/{:.2} mm", snap.travel_mm, snap.max_mm), 1)
            .item(
                "Flux / State",
                format!(
                    "{:.0}% · {}",
                    snap.flux_pct,
                    if snap.triggered { "TRIG" } else { "READY" }
                ),
                1,
            )
            .item("Peak", format!("{:.2} mm", snap.peak_mm), 1)
            .item(
                "Actuation",
                if snap.rapid_trigger {
                    format!(
                        "{:.2} mm (RT {:.2}/{:.2})",
                        snap.actuation_mm, snap.rt_press_mm, snap.rt_release_mm
                    )
                } else {
                    format!("{:.2} mm Fixed", snap.actuation_mm)
                },
                1,
            )
            .item(
                "Deadband",
                format!("{:.2}/{:.2} mm", snap.db_top_mm, snap.db_bottom_mm),
                1,
            )
            .item(
                "Presses",
                format!(
                    "{} · {}",
                    snap.presses,
                    if snap.sweeping {
                        "SWEEP"
                    } else if snap.released_zero {
                        "0.00 OK"
                    } else {
                        "IDLE"
                    }
                ),
                1,
            );

        let switch_box = GroupBox::new()
            .outline()
            .title("Switch Mechanism Cutaway")
            .child(sw_controls)
            .child(stage)
            .child(metrics);

        h_flex()
            .w_full()
            .flex_wrap()
            .items_stretch()
            .gap_3()
            .child(div().flex_1().min_w(px(540.0)).child(depth_box))
            .child(div().w(px(380.0)).flex_shrink_0().child(switch_box))
    }

    fn render_diagnostics_bottom_row(&self, cx: &mut Context<Self>) -> Div {
        let mut switch_card = GroupBox::new().outline().title("Switch Response Log");
        let mut feedback_card = GroupBox::new().outline().title("RPC Telemetry");

        if let Ok(st) = &self.conn {
            if st.switch_tests.is_empty() {
                switch_card = switch_card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Press keys with Live Travel ON to log travel peak and 0.00 return."),
                );
            } else {
                let mut rows: Vec<_> = st.switch_tests.iter().collect();
                rows.sort_by_key(|b| std::cmp::Reverse(b.1.last_seen));
                for (&idx, stat) in rows.into_iter().take(8) {
                    let kname = keymap::by_index(idx).map_or("Key", |k| k.name);
                    let status_txt = if stat.released_zero {
                        "0.00 OK"
                    } else if stat.current_mm > 0.02 {
                        "HELD"
                    } else {
                        "SETTLED"
                    };
                    let col = if stat.released_zero {
                        cx.theme().success
                    } else {
                        cx.theme().warning
                    };
                    switch_card = switch_card.child(
                        h_flex()
                            .id(("dx-sw-row", usize::from(idx)))
                            .px_2()
                            .py_1()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(if st.focused_key == idx {
                                cx.theme().ring
                            } else {
                                cx.theme().border
                            })
                            .justify_between()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Ok(s) = &mut this.conn {
                                    s.focused_key = idx;
                                    this.dirty_switch_3d = true;
                                }
                                cx.notify();
                            }))
                            .child(div().text_xs().child(format!(
                                "{kname} (#{idx}): now {:.2} · peak {:.2} · x{}",
                                stat.current_mm, stat.peak_mm, stat.presses
                            )))
                            .child(div().text_xs().text_color(col).child(status_txt)),
                    );
                }
            }

            for ev in st.feedback_log.iter().take(10) {
                feedback_card = feedback_card.child(
                    div()
                        .text_xs()
                        .text_color(if ev.ok {
                            cx.theme().foreground
                        } else {
                            cx.theme().danger
                        })
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
                    .text_color(cx.theme().muted_foreground)
                    .child(offline_note.attribution.summary),
            );
            feedback_card = feedback_card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Connect to reaulad to inspect live RPC RTT."),
            );
        }

        h_flex()
            .w_full()
            .flex_wrap()
            .items_stretch()
            .gap_3()
            .child(div().flex_1().min_w(px(340.0)).child(switch_card))
            .child(div().flex_1().min_w(px(340.0)).child(feedback_card))
    }
}
