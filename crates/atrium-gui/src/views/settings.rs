//! Application Settings page: language, appearance, workbench defaults,
//! Linux `/dev/hidraw*` permission helper (including unsupported keyboards),
//! and MizProject open-source support links.

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::link::Link;
use gpui_kit::component::select::Select;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{Context, Div, ParentElement as _, Styled as _, div, px};

use crate::ReaulaGui;
use crate::config;
use crate::views::setup::{
    copy_command_row, render_hid_permission_card, status_indicator, theme_preview_card,
};

const GH_SUPPORT_ISSUE_URL: &str =
    "https://github.com/MizProject/reaula/issues/new?template=support-request.yml";
const GH_UNSUPPORTED_ISSUE_URL: &str =
    "https://github.com/MizProject/reaula/issues/new?template=unsupported-keyboard-support.yml";

impl ReaulaGui {
    pub(crate) fn view_settings(&self, cx: &mut Context<Self>) -> Div {
        let t = self.text();
        let is_dark = cx.theme().is_dark();
        let env = &self.setup.env_status;
        let cfg_path = config::config_file_path().display().to_string();
        let selected_cand = self.setup.selected_candidate();

        let appearance_group = GroupBox::new()
            .outline()
            .title(format!("{} / {}", t.choose_language, t.choose_theme))
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
                            .child(t.settings_subtitle),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .gap_1()
                            .child(div().text_xs().child(t.choose_language))
                            .child(
                                div()
                                    .w_full()
                                    .max_w(px(420.0))
                                    .min_w_0()
                                    .child(Select::new(&self.sel_setup_lang).small()),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .flex_wrap()
                            .gap_3()
                            .child(theme_preview_card(
                                "settings-theme-dark",
                                true,
                                is_dark,
                                t.dark_mode_label,
                                cx,
                            ))
                            .child(theme_preview_card(
                                "settings-theme-light",
                                false,
                                !is_dark,
                                t.light_mode_label,
                                cx,
                            )),
                    ),
            );

        let prefs_group = GroupBox::new()
            .outline()
            .title(t.settings_workbench_heading)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_xs()
                                    .child(t.settings_prefer_2d_label),
                            )
                            .child(
                                Switch::new("settings-prefer-2d-sw")
                                    .small()
                                    .checked(self.config.prefer_2d_stage)
                                    .on_change(cx.listener(|this, next: &bool, _, cx| {
                                        this.set_prefer_2d_stage(*next, cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .justify_between()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_xs()
                                    .child(t.settings_auto_travel_label),
                            )
                            .child(
                                Switch::new("settings-auto-travel-sw")
                                    .small()
                                    .checked(self.config.auto_live_travel)
                                    .on_change(cx.listener(|this, next: &bool, _, cx| {
                                        this.set_auto_live_travel(*next, cx);
                                    })),
                            ),
                    )
                    .child(copy_command_row(
                        "settings-copy-cfg-path",
                        format!(
                            "Configuration File ({} saved keyboard profile(s))",
                            self.config.keyboards.len()
                        ),
                        cfg_path,
                        cx,
                    ))
                    .child(
                        h_flex()
                            .w_full()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("settings-open-onboarding")
                                    .small()
                                    .outline()
                                    .icon(Icon::new(IconName::Wrench))
                                    .label(t.settings_open_setup_btn)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_setup_at_step(0, None, window, cx);
                                    })),
                            )
                            .child(
                                Button::new("settings-reset-prefs")
                                    .small()
                                    .outline()
                                    .icon(Icon::new(IconName::RotateCcw))
                                    .label(t.settings_reset_prefs_btn)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reset_app_preferences(window, cx);
                                    })),
                            ),
                    ),
            );

        let mut hid_group_body = v_flex()
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
                    .child(t.settings_hid_perms_desc),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w(px(200.0)).child(status_indicator(
                            env.hidraw_rw_ok > 0,
                            env.hidraw_summary.clone(),
                            cx,
                        )),
                    )
                    .child(
                        div().flex_1().min_w(px(200.0)).child(status_indicator(
                            env.uinput_ok,
                            env.uinput_detail.clone(),
                            cx,
                        )),
                    )
                    .child(
                        Button::new("settings-refresh-hid")
                            .xsmall()
                            .outline()
                            .icon(Icon::new(IconName::RefreshCw))
                            .label(t.refresh_checks)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.refresh_setup_checks(window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_xs().child(t.select_hid_label))
                    .child(Select::new(&self.sel_setup_hid).small()),
            );

        if let Some(cand) = selected_cand {
            hid_group_body =
                hid_group_body.child(render_hid_permission_card("settings-hid-perm", cand, cx));
        } else {
            hid_group_body = hid_group_body.child(Alert::info(
                "settings-hid-none",
                "No /dev/hidraw* nodes detected right now. Plug in your keyboard via USB and click Refresh.",
            ));
        }

        let hid_perms_group = GroupBox::new()
            .outline()
            .title(t.settings_hid_perms_heading)
            .child(hid_group_body);

        let about_group = GroupBox::new()
            .outline()
            .title(t.settings_about_heading)
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t.disclaimer_privacy),
                    )
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t.disclaimer_liability),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_3()
                            .child(
                                Link::new("settings-gh-support")
                                    .href(GH_SUPPORT_ISSUE_URL)
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1()
                                            .text_xs()
                                            .child(Icon::new(IconName::ExternalLink).xsmall())
                                            .child("GitHub Support Issue"),
                                    ),
                            )
                            .child(
                                Link::new("settings-gh-unsupported")
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
                    ),
            );

        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(340.0))
                            .overflow_hidden()
                            .child(appearance_group.w_full().min_w_0().overflow_hidden()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(340.0))
                            .overflow_hidden()
                            .child(prefs_group.w_full().min_w_0().overflow_hidden()),
                    ),
            )
            .child(hid_perms_group.w_full().min_w_0().overflow_hidden())
            .child(about_group.w_full().min_w_0().overflow_hidden())
    }
}
