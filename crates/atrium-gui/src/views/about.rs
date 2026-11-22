// Copyright (C) 2026 MizProject Philippines
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! About dialog showing Atrium credits, contacts, and GNU AGPL-3.0 legal notices.

use atrium_core::legal;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::link::Link;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{Context, ObjectFit, ParentElement as _, Styled as _, Window, div, img, prelude::*, px};

use crate::AtriumGui;

impl AtriumGui {
    pub(crate) fn open_about_dialog(&self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().clone();
        let view_close = view.clone();
        let view_cancel = view.clone();
        let icon_img = self.app_icon_img.clone();

        window.open_dialog(cx, move |dialog, _window, cx| {
            let show_license = view.read(cx).about_show_license;
            let view_toggle = view.clone();
            let view_btn_close = view_close.clone();
            let view_on_cancel = view_cancel.clone();

            let header_row = h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_3()
                .child(if let Some(icon) = &icon_img {
                    img(icon.clone())
                        .w(px(52.0))
                        .h(px(52.0))
                        .rounded(cx.theme().radius_lg)
                        .border_1()
                        .border_color(cx.theme().border)
                        .object_fit(ObjectFit::Contain)
                        .into_any_element()
                } else {
                    div()
                        .w(px(52.0))
                        .h(px(52.0))
                        .rounded(cx.theme().radius_lg)
                        .border_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(IconName::Keyboard).large())
                        .into_any_element()
                })
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_lg().child(legal::APP_NAME))
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(cx.theme().radius)
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .bg(cx.theme().secondary)
                                        .text_xs()
                                        .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded(cx.theme().radius)
                                        .border_1()
                                        .border_color(cx.theme().border)
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(legal::LICENSE_ID),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Linux Hall-Effect & RGB Keyboard Workbench"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .child(legal::COPYRIGHT_NOTICE),
                        ),
                );

            let credits_card = GroupBox::new()
                .outline()
                .title("Credits & Contacts")
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .child(
                            div()
                                .w_full()
                                .whitespace_normal()
                                .text_xs()
                                .child("Created and maintained by MizProject Philippines (@Mizumo_prjkt)."),
                        )
                        .child(
                            div()
                                .w_full()
                                .whitespace_normal()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Subsystems: atrium-core (HID codecs & FFT audio engine), atriumd (SO_PEERCRED daemon & systemd), atrium CLI, atrium-tui, and atrium-gui (GPUI Kit + wgpu)."),
                        )
                        .child(
                            div()
                                .w_full()
                                .whitespace_normal()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("Community thanks: AULA WIN 60 HE & Hall-effect contributors and HID probe testers."),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .flex_wrap()
                                .items_center()
                                .gap_2()
                                .pt_1()
                                .child(
                                    Link::new("about-email-link")
                                        .href(format!("mailto:{}", legal::CONTACT_EMAIL))
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_1()
                                                .text_xs()
                                                .child(Icon::new(IconName::Mail).xsmall())
                                                .child(legal::CONTACT_EMAIL),
                                        ),
                                )
                                .child(
                                    Clipboard::new("about-copy-email")
                                        .xsmall()
                                        .label("Copy Email")
                                        .value(legal::CONTACT_EMAIL),
                                )
                                .child(
                                    Link::new("about-tg-link")
                                        .href(legal::CONTACT_TELEGRAM_URL)
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_1()
                                                .text_xs()
                                                .child(Icon::new(IconName::Send).xsmall())
                                                .child(format!("Telegram {}", legal::CONTACT_TELEGRAM)),
                                        ),
                                )
                                .child(
                                    Link::new("about-repo-link")
                                        .href(legal::REPO_URL)
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_1()
                                                .text_xs()
                                                .child(Icon::new(IconName::ExternalLink).xsmall())
                                                .child("GitHub Source"),
                                        ),
                                )
                                .child(
                                    Link::new("about-issues-link")
                                        .href(legal::ISSUES_URL)
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_1()
                                                .text_xs()
                                                .child(Icon::new(IconName::ExternalLink).xsmall())
                                                .child("Issues"),
                                        ),
                                ),
                        ),
                );

            let mut license_card = GroupBox::new()
                .outline()
                .title(legal::LICENSE_TITLE)
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .p_2p5()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .text_xs()
                        .whitespace_normal()
                        .child(legal::AGPL_V3_PREAMBLE_NOTICE),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .items_center()
                        .pt_1()
                        .gap_2()
                        .child(
                            Link::new("about-gnu-link")
                                .href(legal::GNU_LICENSES_URL)
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .text_xs()
                                        .child(Icon::new(IconName::ExternalLink).xsmall())
                                        .child(legal::GNU_LICENSES_URL),
                                ),
                        )
                        .child(
                            Button::new("about-toggle-license-btn")
                                .xsmall()
                                .outline()
                                .icon(Icon::new(IconName::BookOpen))
                                .label(if show_license {
                                    "Hide Full AGPLv3 License"
                                } else {
                                    "Read Full AGPLv3 License"
                                })
                                .on_click(move |_, _, cx| {
                                    view_toggle.update(cx, |this, cx| {
                                        this.about_show_license = !this.about_show_license;
                                        cx.refresh_windows();
                                    });
                                }),
                        ),
                );

            if show_license {
                license_card = license_card.child(
                    div()
                        .w_full()
                        .h(px(210.0))
                        .p_2p5()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .w_full()
                                .whitespace_normal()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(legal::LICENSE_FULL_TEXT),
                        ),
                );
            }

            let body = v_flex()
                .w_full()
                .min_w_0()
                .gap_3()
                .child(header_row)
                .child(credits_card)
                .child(license_card);

            let footer = DialogFooter::new().child(
                Button::new("about-close-btn")
                    .primary()
                    .small()
                    .label("Close")
                    .on_click(move |_, window, cx| {
                        view_btn_close.update(cx, |this, _| {
                            this.about_show_license = false;
                        });
                        window.close_dialog(cx);
                    }),
            );

            dialog
                .w(px(680.0))
                .max_w(px(720.0))
                .title("About Atrium")
                .close_button(true)
                .keyboard(true)
                .overlay_closable(true)
                .child(body)
                .footer(footer)
                .on_cancel(move |_, _, cx| {
                    view_on_cancel.update(cx, |this, _| {
                        this.about_show_license = false;
                    });
                    true
                })
        });
    }
}
