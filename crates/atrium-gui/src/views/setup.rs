//! Full-screen onboarding wizard: language/theme, MizProject disclaimer,
//! system permission & daemon checks, and hardware / unsupported HID prober.

use gpui_kit::assets::IconName;
use gpui_kit::base::Selectable as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::link::Link;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::Select;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    Context, Div, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit, ParentElement as _,
    ScrollDelta, ScrollEvent, SharedString, Styled as _, div, img, prelude::*, px, rgb,
};

use atrium_core::hid_probe::{self, HidCandidateSummary, ProbeCheckStatus};
use atrium_core::keyboards;

use crate::AtriumGui;
use crate::i18n::UiText;
use crate::kb_3d::{CameraPreset, KB3D_HEIGHT, KB3D_WIDTH};
use crate::state::{NavSection, SetupHardwareMode};
use crate::theme::{self, HardwareColors};
use crate::widgets::{MetricCell, render_metric_table, render_unsupported_2d_canvas};

const GH_UNSUPPORTED_ISSUE_URL: &str =
    "https://github.com/MizProject/atrium/issues/new?template=unsupported-keyboard-support.yml";
const GH_SUPPORT_ISSUE_URL: &str =
    "https://github.com/MizProject/atrium/issues/new?template=support-request.yml";

impl AtriumGui {
    pub(crate) fn view_setup(&self, cx: &mut Context<Self>) -> Div {
        let t = self.text();
        let step = self.setup.step.min(4);
        let can_finish = self.setup.disclaimer_accepted;
        let wide_probe = step == 3 && self.setup.hw_mode == SetupHardwareMode::UnsupportedProbe;
        let container_max_w = if wide_probe { px(1100.0) } else { px(860.0) };

        let stepper = Stepper::new("setup-wizard-stepper")
            .small()
            .selected_index(step)
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.setup.step = (*ix).min(4);
                if this.setup.step == 2 || this.setup.step == 3 {
                    this.refresh_setup_env_only();
                }
                if this.setup.step == 3 && this.setup.hw_mode == SetupHardwareMode::UnsupportedProbe
                {
                    this.maybe_render_setup_3d();
                }
                cx.notify();
            }))
            .item(
                StepperItem::new()
                    .icon(IconName::Globe)
                    .child(div().text_xs().child(t.step_welcome)),
            )
            .item(
                StepperItem::new()
                    .icon(IconName::ShieldCheck)
                    .child(div().text_xs().child(t.step_disclaimer)),
            )
            .item(
                StepperItem::new()
                    .icon(IconName::Cpu)
                    .child(div().text_xs().child(t.step_system)),
            )
            .item(
                StepperItem::new()
                    .icon(IconName::Keyboard)
                    .child(div().text_xs().child(t.step_hardware)),
            )
            .item(
                StepperItem::new()
                    .icon(IconName::Check)
                    .child(div().text_xs().child(t.step_finish)),
            );

        let mut title_row = h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .justify_between()
            .items_start()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(260.0))
                    .gap_1()
                    .child(div().text_xl().child(t.setup_title))
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t.setup_subtitle),
                    ),
            );

        // Returning users who already completed onboarding can exit Setup cleanly;
        // first-time users complete Setup before entering the main workbench.
        if self.config.setup_completed {
            title_row = title_row.child(
                Button::new("setup-return-main")
                    .xsmall()
                    .outline()
                    .label(t.nav_main)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.switch_section(NavSection::Main, cx);
                    })),
            );
        }

        let wizard_header = v_flex()
            .w_full()
            .min_w_0()
            .gap_4()
            .pb_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(title_row)
            .child(div().w_full().min_w_0().overflow_hidden().child(stepper));

        let body = match step {
            0 => self.view_setup_step_welcome(t, cx),
            1 => self.view_setup_step_disclaimer(t, cx),
            2 => self.view_setup_step_system(t, cx),
            3 => self.view_setup_step_hardware(t, cx),
            _ => self.view_setup_step_finish(t, cx),
        };

        let next_group = if step < 4 {
            let block_disclaimer = step == 1 && !self.setup.disclaimer_accepted;
            h_flex()
                .flex_wrap()
                .gap_3()
                .items_center()
                .when(block_disclaimer, |el| {
                    el.child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child(Icon::new(IconName::TriangleAlert).xsmall())
                            .child(t.disclaimer_must_accept),
                    )
                })
                .child(
                    Button::new("setup-btn-next")
                        .small()
                        .primary()
                        .disabled(block_disclaimer)
                        .icon(Icon::new(IconName::ArrowRight))
                        .label(t.next_btn)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.setup.step == 1 && !this.setup.disclaimer_accepted {
                                return;
                            }
                            this.setup.step = (this.setup.step + 1).min(4);
                            if this.setup.step == 2 || this.setup.step == 3 {
                                this.refresh_setup_env_only();
                            }
                            if this.setup.step == 3
                                && this.setup.hw_mode == SetupHardwareMode::UnsupportedProbe
                            {
                                this.maybe_render_setup_3d();
                            }
                            cx.notify();
                        })),
                )
        } else {
            h_flex()
                .flex_wrap()
                .gap_3()
                .items_center()
                .when(!can_finish, |el| {
                    el.child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child(Icon::new(IconName::TriangleAlert).xsmall())
                            .child(t.disclaimer_must_accept),
                    )
                })
                .child(
                    Button::new("setup-btn-finish")
                        .small()
                        .primary()
                        .disabled(!can_finish)
                        .icon(Icon::new(IconName::Check))
                        .label(t.finish_setup_btn)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.commit_setup_and_open_main(cx);
                        })),
                )
        };

        let footer = h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap_3()
            .pt_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new("setup-btn-back")
                            .small()
                            .outline()
                            .disabled(step == 0)
                            .icon(Icon::new(IconName::ArrowLeft))
                            .label(t.back_btn)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.setup.step = this.setup.step.saturating_sub(1);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Step {} / 5", step + 1)),
                    ),
            )
            .child(next_group);

        v_flex().w_full().min_w_0().items_center().py_6().px_2().child(
            v_flex()
                .w_full()
                .max_w(container_max_w)
                .min_w_0()
                .gap_5()
                .child(wizard_header)
                .child(body)
                .child(footer),
        )
    }

    fn view_setup_step_welcome(&self, t: &'static UiText, cx: &mut Context<Self>) -> Div {
        let is_dark = cx.theme().is_dark();

        let lang_row = v_flex()
            .w_full()
            .min_w_0()
            .p_4()
            .gap_2p5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(IconName::Globe).small())
                    .child(div().text_sm().child(t.choose_language)),
            )
            .child(
                div()
                    .w_full()
                    .max_w(px(420.0))
                    .min_w_0()
                    .child(Select::new(&self.sel_setup_lang).small()),
            );

        let theme_section = v_flex()
            .w_full()
            .min_w_0()
            .p_4()
            .gap_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(if is_dark {
                        IconName::Moon
                    } else {
                        IconName::Sun
                    })
                    .small())
                    .child(div().text_sm().child(t.choose_theme)),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .gap_3()
                    .child(theme_preview_card(
                        "setup-theme-dark",
                        true,
                        is_dark,
                        t.dark_mode_label,
                        cx,
                    ))
                    .child(theme_preview_card(
                        "setup-theme-light",
                        false,
                        !is_dark,
                        t.light_mode_label,
                        cx,
                    )),
            );

        v_flex()
            .w_full()
            .min_w_0()
            .gap_4()
            .child(lang_row)
            .child(theme_section)
    }

    fn view_setup_step_disclaimer(&self, t: &'static UiText, cx: &mut Context<Self>) -> Div {
        let items = [
            (IconName::ShieldCheck, t.disclaimer_privacy),
            (IconName::TriangleAlert, t.disclaimer_liability),
            (IconName::Globe, t.disclaimer_opensource),
            (IconName::ExternalLink, t.disclaimer_issues),
        ];

        let mut list = v_flex().w_full().min_w_0().gap_2();
        for (icon, text) in items {
            list = list.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap_2p5()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(Icon::new(icon).small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .whitespace_normal()
                            .text_xs()
                            .child(text),
                    ),
            );
        }

        v_flex().w_full().min_w_0().child(
            GroupBox::new()
                .outline()
                .title(t.disclaimer_heading)
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .child(list)
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .flex_wrap()
                                .justify_between()
                                .items_center()
                                .gap_2()
                                .p_3()
                                .rounded(cx.theme().radius)
                                .border_1()
                                .border_color(if self.setup.disclaimer_accepted {
                                    cx.theme().success
                                } else {
                                    cx.theme().warning
                                })
                                .child(
                                    Checkbox::new("setup-disclaimer-ack")
                                        .checked(self.setup.disclaimer_accepted)
                                        .label(t.disclaimer_accept_chk)
                                        .on_click(cx.listener(|this, next: &bool, _, cx| {
                                            this.setup.disclaimer_accepted = *next;
                                            this.config.disclaimer_accepted = *next;
                                            this.persist_config();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Link::new("setup-issues-link")
                                        .href(GH_SUPPORT_ISSUE_URL)
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_1()
                                                .text_xs()
                                                .child(Icon::new(IconName::ExternalLink).xsmall())
                                                .child("GitHub Issues"),
                                        ),
                                ),
                        ),
                ),
        )
    }

    fn view_setup_step_system(&self, t: &'static UiText, cx: &mut Context<Self>) -> Div {
        let env = &self.setup.env_status;
        let hid_ok = env.hidraw_rw_ok > 0;
        let udev_cmd = "sudo install -m 644 udev/99-aula-win60he.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules && sudo udevadm trigger";
        let systemd_cmd = "systemctl --user enable --now atriumd.service";

        let metrics = render_metric_table(
            vec![
                MetricCell::element(
                    "HID Devices",
                    status_indicator(hid_ok, &env.hidraw_summary, cx),
                ),
                MetricCell::element(
                    "udev Rules",
                    status_indicator(
                        env.udev_rule_present,
                        if env.udev_rule_present {
                            env.udev_rule_path.clone()
                        } else {
                            "Missing 99-aula-win60he.rules".into()
                        },
                        cx,
                    ),
                ),
                MetricCell::element(
                    "/dev/uinput",
                    status_indicator(env.uinput_ok, &env.uinput_detail, cx),
                ),
                MetricCell::element(
                    "atriumd IPC",
                    status_indicator(env.daemon_connected, &env.daemon_detail, cx),
                ),
            ],
            220.0,
            cx,
        );

        let mut remedies = v_flex().w_full().min_w_0().gap_2();
        if !env.udev_rule_present || !hid_ok {
            remedies = remedies.child(copy_command_row(
                "setup-copy-udev",
                "Install WIN 60 HE udev rules (unsupported keyboards generate per-device rules in Hardware -> Probe or Settings)",
                udev_cmd,
                cx,
            ));
        }
        if let Some(uinput_fix) = &env.uinput_remedy {
            remedies = remedies.child(
                Alert::warning("setup-uinput-warn", uinput_fix.clone())
                    .title("/dev/uinput Permission / Module Check"),
            );
        }
        if !env.daemon_connected {
            remedies = remedies.child(copy_command_row(
                "setup-copy-systemd",
                "Start atriumd via systemd user unit (or CLI daemon start)",
                systemd_cmd,
                cx,
            ));
        }

        v_flex().w_full().min_w_0().child(
            GroupBox::new()
                .outline()
                .title(t.system_heading)
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .whitespace_normal()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t.system_subtitle),
                        )
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    Button::new("setup-refresh-sys")
                                        .small()
                                        .outline()
                                        .icon(Icon::new(IconName::RefreshCw))
                                        .label(t.refresh_checks)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.refresh_setup_checks(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("setup-reconnect-daemon")
                                        .small()
                                        .primary()
                                        .icon(Icon::new(IconName::RefreshCw))
                                        .label(t.reconnect_daemon)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.reconnect(window, cx);
                                            this.refresh_setup_checks(window, cx);
                                        })),
                                ),
                        )
                        .child(metrics)
                        .child(remedies),
                ),
        )
    }

    fn view_setup_step_hardware(&self, t: &'static UiText, cx: &mut Context<Self>) -> Div {
        let mode_idx = match self.setup.hw_mode {
            SetupHardwareMode::Supported => 0,
            SetupHardwareMode::UnsupportedProbe => 1,
        };

        let mode_tabs = TabBar::new("setup-hw-mode-tabs")
            .segmented()
            .small()
            .selected_index(mode_idx)
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.setup.hw_mode = if *ix == 1 {
                    SetupHardwareMode::UnsupportedProbe
                } else {
                    SetupHardwareMode::Supported
                };
                if this.setup.hw_mode == SetupHardwareMode::UnsupportedProbe {
                    this.refresh_setup_env_only();
                    this.maybe_render_setup_3d();
                }
                cx.notify();
            }))
            .child(Tab::new().label(t.supported_hw_tab))
            .child(Tab::new().label(t.unsupported_hw_tab));

        let inner = match self.setup.hw_mode {
            SetupHardwareMode::Supported => self.view_setup_supported_picker(t, cx),
            SetupHardwareMode::UnsupportedProbe => self.view_setup_unsupported_studio(t, cx),
        };

        v_flex().w_full().min_w_0().child(
            GroupBox::new()
                .outline()
                .title(t.hardware_heading)
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .child(
                            h_flex()
                                .w_full()
                                .flex_wrap()
                                .justify_between()
                                .gap_2()
                                .child(mode_tabs),
                        )
                        .child(inner),
                ),
        )
    }

    fn view_setup_supported_picker(&self, t: &'static UiText, cx: &mut Context<Self>) -> Div {
        let mut cards = v_flex().w_full().min_w_0().gap_2();
        for (ix, spec) in keyboards::all().iter().enumerate() {
            let usb = spec.primary_usb_id();
            let detected = self
                .setup
                .env_status
                .discovered_supported
                .iter()
                .find(|(id, _, _)| id == spec.id);
            let selected = self.setup.selected_supported_id == spec.id;
            let spec_id = spec.id;

            let detect_badge = match detected {
                Some((_, _, true)) => status_indicator(true, "Detected on USB (R/W)", cx),
                Some((_, _, false)) => {
                    status_indicator(false, "Detected (needs udev R/W)", cx)
                }
                None => status_indicator(false, "Not attached or wireless asleep", cx),
            };

            cards = cards.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(if selected {
                        cx.theme().primary
                    } else {
                        cx.theme().border
                    })
                    .bg(cx.theme().background)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(220.0))
                            .gap_1()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Icon::new(IconName::Keyboard).small())
                                    .child(div().text_sm().child(spec.display_name)),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} · {} keys · USB {:04X}:{:04X} · Full analog calibration, keymap & RGB",
                                        spec.form_factor,
                                        spec.keys.len(),
                                        usb.vid,
                                        usb.pid
                                    )),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(detect_badge)
                            .child(
                                Button::new(("setup-pick-supported", ix))
                                    .xsmall()
                                    .selected(selected)
                                    .outline()
                                    .label(if selected { "Selected" } else { "Select" })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.setup.selected_supported_id = spec_id.to_string();
                                        this.setup.hw_mode = SetupHardwareMode::Supported;
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t.select_supported_desc),
            )
            .child(cards)
    }

    fn view_setup_unsupported_studio(&self, t: &'static UiText, cx: &mut Context<Self>) -> Div {
        let model_label = if self.setup.model_name_hint.trim().is_empty() {
            crate::kb_3d::generic::MODEL.label
        } else {
            self.setup.model_name_hint.trim()
        };
        let has_hid = !self.setup.hid_candidates.is_empty();

        let left_3d_panel = self.render_unsupported_stage_box("setup-stage", model_label, cx);

        let mut right_probe_panel = v_flex()
            .flex_1()
            .min_w(px(320.0))
            .overflow_hidden()
            .gap_2p5()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .whitespace_normal()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t.unsupported_desc),
            )
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .gap_2()
                            .child(div().text_xs().child(t.select_hid_label))
                            .child(
                                Button::new("setup-hid-rescan")
                                    .xsmall()
                                    .outline()
                                    .icon(Icon::new(IconName::RefreshCw))
                                    .label(t.refresh_checks)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.refresh_setup_checks(window, cx);
                                    })),
                            ),
                    )
                    .child(Select::new(&self.sel_setup_hid).small()),
            )
            .when_some(self.setup.selected_candidate(), |el, cand| {
                el.child(render_hid_permission_card("setup-hid-perm", cand, cx))
            })
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_xs().child(t.model_hint_label))
                    .child(Input::new(&self.input_setup_model_hint).small()),
            )
            .child(
                Checkbox::new("setup-vendor-probe-chk")
                    .checked(self.setup.probe_allow_vendor_queries)
                    .label(t.vendor_probe_chk)
                    .on_click(cx.listener(|this, next: &bool, _, cx| {
                        this.setup.probe_allow_vendor_queries = *next;
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("setup-run-probe")
                            .small()
                            .primary()
                            .disabled(!has_hid)
                            .icon(Icon::new(IconName::Cpu))
                            .label(t.run_probe_btn)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_setup_probe(cx);
                            })),
                    )
                    .when(!has_hid, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .child("No /dev/hidraw* nodes detected. Connect USB and click Refresh."),
                        )
                    }),
            );

        if let Some(err) = &self.setup.probe_error {
            right_probe_panel =
                right_probe_panel.child(Alert::error("setup-probe-err", err.clone()));
        }

        if let Some(report) = &self.setup.probe_report {
            let caps = &report.inferred_capabilities;
            let checks_pass = report
                .checks
                .iter()
                .filter(|c| matches!(c.status, ProbeCheckStatus::Pass))
                .count();
            right_probe_panel = right_probe_panel
                .child(render_metric_table(
                    vec![
                        MetricCell::text(
                            "Interface",
                            format!(
                                "{} ({}:{})",
                                report.selected_interface.hidraw_name,
                                report.selected_interface.vid_hex,
                                report.selected_interface.pid_hex
                            ),
                            cx,
                        ),
                        MetricCell::text(
                            "Vendor Page",
                            if caps.has_vendor_usage_page {
                                "Yes"
                            } else {
                                "No"
                            },
                            cx,
                        ),
                        MetricCell::text(
                            "64B Vendor Frame",
                            if caps.has_64b_bidirectional_reports {
                                "Yes"
                            } else {
                                "No"
                            },
                            cx,
                        ),
                        MetricCell::text(
                            "Checks",
                            format!("{checks_pass}/{} pass", report.checks.len()),
                            cx,
                        ),
                    ],
                    160.0,
                    cx,
                ))
                .child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child({
                            let copied_view = cx.entity();
                            Clipboard::new("setup-copy-probe-json")
                                .small()
                                .label(t.copy_json_btn)
                                .value(self.setup.probe_json.clone())
                                .on_copied(move |_, _, cx| {
                                    copied_view.update(cx, |this, cx| {
                                        this.status_line =
                                            "Copied sanitized probe JSON to clipboard.".into();
                                        cx.notify();
                                    });
                                })
                        })
                        .child(
                            Button::new("setup-save-probe-json")
                                .small()
                                .outline()
                                .icon(Icon::new(IconName::FolderDown))
                                .label(t.save_json_btn)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_setup_probe_file(cx);
                                })),
                        )
                        .child(
                            Link::new("setup-open-unsupported-issue")
                                .href(GH_UNSUPPORTED_ISSUE_URL)
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .text_xs()
                                        .child(Icon::new(IconName::ExternalLink).xsmall())
                                        .child(t.open_issue_btn),
                                ),
                        ),
                )
                .child(Alert::info("setup-dev-followup", t.dev_followup_notice));

            if let Some(saved) = &self.setup.saved_probe_path {
                right_probe_panel = right_probe_panel.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .text_xs()
                        .text_color(cx.theme().success)
                        .child(format!("Saved probe JSON: {saved}")),
                );
            }

            let md_json: SharedString = format!("```json\n{}\n```", self.setup.probe_json).into();
            right_probe_panel = right_probe_panel.child(
                div()
                    .id("setup-probe-json-scroll")
                    .w_full()
                    .min_w_0()
                    .max_h(px(230.0))
                    .overflow_y_scrollbar()
                    .p_2()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .child(TextView::markdown("setup-probe-json-md", md_json).selectable(true)),
            );
        }

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                Alert::warning("setup-fn-esc-recovery", t.recovery_warning_body)
                    .title(t.recovery_warning_title),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .items_start()
                    .gap_3()
                    .child(left_3d_panel)
                    .child(right_probe_panel),
            )
    }

    pub(crate) fn render_unsupported_stage_box(
        &self,
        id_prefix: &'static str,
        model_label: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let gpu_ready = self.renderer_3d.is_ok();
        let use_2d = self.setup.unsupported_2d_view || !gpu_ready;

        let toolbar = h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap_1p5()
            .child(
                h_flex()
                    .gap_1()
                    .child(setup_preset_btn(
                        (id_prefix, 0),
                        "Iso",
                        CameraPreset::Perspective,
                        self.setup.camera_preset,
                        cx,
                    ))
                    .child(setup_preset_btn(
                        (id_prefix, 1),
                        "Top",
                        CameraPreset::Top,
                        self.setup.camera_preset,
                        cx,
                    ))
                    .child(setup_preset_btn(
                        (id_prefix, 2),
                        "Side",
                        CameraPreset::Side,
                        self.setup.camera_preset,
                        cx,
                    ))
                    .child(setup_preset_btn(
                        (id_prefix, 3),
                        "Front",
                        CameraPreset::Front,
                        self.setup.camera_preset,
                        cx,
                    )),
            )
            .child(
                TabBar::new((id_prefix, 9usize))
                    .segmented()
                    .xsmall()
                    .selected_index(usize::from(!use_2d))
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        this.setup.unsupported_2d_view = *ix == 0 || this.renderer_3d.is_err();
                        if !this.setup.unsupported_2d_view {
                            this.maybe_render_setup_3d();
                        }
                        cx.notify();
                    }))
                    .child(Tab::new().label("2D"))
                    .child(Tab::new().label("3D")),
            );

        let stage = if !use_2d {
            match (&self.renderer_3d, &self.preview_setup_3d_img) {
                (Ok(_), Some(frame)) => v_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id((id_prefix, 20usize))
                            .w_full()
                            .max_w(px((KB3D_WIDTH as f32 * 0.68).min(566.0)))
                            .h(px((KB3D_HEIGHT as f32 * 0.68).min(318.0)))
                            .rounded(cx.theme().radius_lg)
                            .overflow_hidden()
                            .border_1()
                            .border_color(cx.theme().border)
                            .cursor_grab()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    this.dragging_setup_3d = true;
                                    this.last_mouse_setup_3d =
                                        Some((f32::from(ev.position.x), f32::from(ev.position.y)));
                                    cx.notify();
                                }),
                            )
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| {
                                    this.dragging_setup_3d = false;
                                    this.last_mouse_setup_3d = None;
                                }),
                            )
                            .on_mouse_up_out(
                                MouseButton::Left,
                                cx.listener(|this, _, _, _| {
                                    this.dragging_setup_3d = false;
                                    this.last_mouse_setup_3d = None;
                                }),
                            )
                            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                                if this.dragging_setup_3d && ev.dragging() {
                                    let nx = f32::from(ev.position.x);
                                    let ny = f32::from(ev.position.y);
                                    if let Some((lx, ly)) = this.last_mouse_setup_3d {
                                        this.setup.camera_3d.orbit(nx - lx, ny - ly);
                                        this.setup.camera_preset = None;
                                        this.maybe_render_setup_3d();
                                    }
                                    this.last_mouse_setup_3d = Some((nx, ny));
                                    cx.notify();
                                }
                            }))
                            .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| {
                                let dy = match ev.delta {
                                    ScrollDelta::Lines(p) => p.y * 0.45,
                                    ScrollDelta::Pixels(p) => f32::from(p.y) * 0.01,
                                };
                                this.setup.camera_3d.adjust_zoom(-dy);
                                this.setup.camera_preset = None;
                                this.maybe_render_setup_3d();
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
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!(
                                "Drag orbit · Wheel zoom · Generic 75% 3D model ({})",
                                model_label
                            )),
                    ),
                _ => v_flex().w_full().min_w_0().items_center().py_6().child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Rendering generic 3D prototype..."),
                ),
            }
        } else {
            let hc = HardwareColors::from_app(cx);
            v_flex()
                .w_full()
                .min_w_0()
                .child(render_unsupported_2d_canvas(model_label, &hc, 0.85))
        };

        v_flex()
            .flex_1()
            .min_w(px(320.0))
            .overflow_hidden()
            .gap_2()
            .p_2p5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .child(toolbar)
            .child(stage)
    }

    fn view_setup_step_finish(&self, t: &'static UiText, cx: &mut Context<Self>) -> Div {
        let hint = self.input_setup_model_hint.read(cx).value().to_string();
        let draft_entry = self.setup.build_profile_entry(&hint);
        let theme_name = if cx.theme().is_dark() {
            t.dark_mode_label
        } else {
            t.light_mode_label
        };

        let summary_table = render_metric_table(
            vec![
                MetricCell::text("Language", self.config.language.display_name(), cx),
                MetricCell::text("Theme", theme_name, cx),
                MetricCell::text(
                    "Disclaimer",
                    if self.setup.disclaimer_accepted {
                        "Accepted"
                    } else {
                        "Pending"
                    },
                    cx,
                ),
                MetricCell::text(
                    "Keyboard Profile",
                    format!("{} ({})", draft_entry.label, draft_entry.model_id),
                    cx,
                ),
                MetricCell::text(
                    "Daemon IPC",
                    if self.setup.env_status.daemon_connected {
                        "Connected"
                    } else {
                        "Offline (Main Hub Still Available)"
                    },
                    cx,
                ),
            ],
            220.0,
            cx,
        );

        let is_unsupported = self.setup.hw_mode == SetupHardwareMode::UnsupportedProbe;
        v_flex().w_full().min_w_0().child(
            GroupBox::new()
                .outline()
                .title(t.summary_heading)
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .child(summary_table)
                        .when(is_unsupported, |el| {
                            el.child(Alert::info("setup-finish-dev", t.dev_followup_notice))
                                .child(
                                    h_flex().flex_wrap().gap_2().child(
                                        Link::new("setup-finish-issue-link")
                                            .href(GH_UNSUPPORTED_ISSUE_URL)
                                            .child(
                                                h_flex()
                                                    .items_center()
                                                    .gap_1()
                                                    .text_xs()
                                                    .child(Icon::new(IconName::ExternalLink).xsmall())
                                                    .child(t.open_issue_btn),
                                            ),
                                    ),
                                )
                        }),
                ),
        )
    }
}

pub(crate) fn theme_preview_card(
    id: &'static str,
    target_dark: bool,
    selected: bool,
    label: &'static str,
    cx: &mut Context<AtriumGui>,
) -> Div {
    let (canvas_hex, card_hex, border_hex, fg_hex) = theme::setup_theme_swatch(target_dark);

    let swatch = v_flex()
        .w_full()
        .h(px(46.0))
        .p_1p5()
        .gap_1()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(rgb(border_hex))
        .bg(rgb(canvas_hex))
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(div().w(px(34.0)).h(px(6.0)).rounded_sm().bg(rgb(fg_hex)))
                .child(div().w(px(14.0)).h(px(6.0)).rounded_sm().bg(rgb(border_hex))),
        )
        .child(
            div()
                .flex_1()
                .w_full()
                .rounded_sm()
                .border_1()
                .border_color(rgb(border_hex))
                .bg(rgb(card_hex)),
        );

    v_flex()
        .flex_1()
        .min_w(px(210.0))
        .gap_2()
        .p_2p5()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(if selected {
            cx.theme().primary
        } else {
            cx.theme().border
        })
        .bg(cx.theme().background)
        .child(swatch)
        .child(
            Button::new(id)
                .small()
                .w_full()
                .selected(selected)
                .outline()
                .icon(Icon::new(if selected {
                    IconName::Check
                } else if target_dark {
                    IconName::Moon
                } else {
                    IconName::Sun
                }))
                .label(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_theme_dark(target_dark, cx);
                })),
        )
}

fn setup_preset_btn(
    id: (&'static str, usize),
    label: &'static str,
    preset: CameraPreset,
    current: Option<CameraPreset>,
    cx: &mut Context<AtriumGui>,
) -> Button {
    Button::new(id)
        .xsmall()
        .selected(current == Some(preset))
        .outline()
        .label(label)
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_setup_camera_preset(preset, cx);
        }))
}

pub(crate) fn status_indicator(
    ok: bool,
    text: impl Into<SharedString>,
    cx: &gpui_kit::App,
) -> gpui_kit::AnyElement {
    h_flex()
        .min_w_0()
        .items_center()
        .gap_1p5()
        .child(
            div()
                .flex_shrink_0()
                .text_color(if ok {
                    cx.theme().success
                } else {
                    cx.theme().warning
                })
                .child(Icon::new(if ok {
                    IconName::CircleCheck
                } else {
                    IconName::TriangleAlert
                }).xsmall()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .text_xs()
                .child(text.into()),
        )
        .into_any_element()
}

pub(crate) fn copy_command_row(
    id: impl Into<gpui_kit::ElementId>,
    title: impl Into<SharedString>,
    cmd: impl Into<SharedString>,
    cx: &gpui_kit::App,
) -> Div {
    let cmd_str: SharedString = cmd.into();
    h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .justify_between()
        .items_center()
        .gap_2()
        .p_2()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            v_flex()
                .flex_1()
                .min_w(px(220.0))
                .gap_0p5()
                .child(div().text_xs().child(title.into()))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .whitespace_normal()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(cmd_str.clone()),
                ),
        )
        .child(Clipboard::new(id).small().label("Copy").value(cmd_str))
}

/// Permission guidance box tailored to the currently selected `/dev/hidraw*` node,
/// explaining why unsupported VID:PID devices are `0600` and giving exact safe commands.
pub(crate) fn render_hid_permission_card(
    id_prefix: &'static str,
    cand: &HidCandidateSummary,
    cx: &gpui_kit::App,
) -> Div {
    let cmds = hid_probe::permission_commands_for(cand);
    if cand.readable_writable {
        return v_flex().w_full().min_w_0().child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .p_2p5()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().success)
                .bg(cx.theme().background)
                .child(status_indicator(
                    true,
                    format!(
                        "/dev/{} ({}:{}) is R/W accessible for both passive descriptor reads and active vendor queries.",
                        cmds.hidraw_name, cand.vid_hex, cand.pid_hex
                    ),
                    cx,
                )),
        );
    }

    let detail = cand
        .permission_error
        .as_deref()
        .unwrap_or("Permission denied");
    v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .p_2p5()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().warning)
        .bg(cx.theme().background)
        .child(
            Alert::warning(
                (id_prefix, 0usize),
                format!(
                    "/dev/{} ({}:{}) is currently root-only ({detail}). The shipped 99-aula-win60he.rules only matches WIN 60 HE (2E3C:C365). Passive HID report-descriptor probing still works now; grant R/W access below if you want active vendor readback, replug if triggered busy, then click Refresh.",
                    cmds.hidraw_name, cand.vid_hex, cand.pid_hex
                ),
            )
            .title("Unsupported Keyboard HIDraw Permissions"),
        )
        .child(copy_command_row(
            (id_prefix, 1usize),
            format!("Persistent per-device udev rule ({})", cmds.udev_rule_filename),
            cmds.install_udev_cmd,
            cx,
        ))
        .child(copy_command_row(
            (id_prefix, 2usize),
            "Temporary single-session ACL (resets automatically on unplug or reboot)",
            cmds.temp_setfacl_cmd,
            cx,
        ))
}
