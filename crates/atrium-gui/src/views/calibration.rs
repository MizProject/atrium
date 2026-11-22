//! Sensor Calibration workspace.

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::{Context, Div, ParentElement as _, Styled as _, div};

use atrium_core::keymap;

use crate::AtriumGui;
use crate::state::ConnectedState;
use crate::widgets::{KB_SCALE_DEFAULT, KeyboardCanvasMode};

impl AtriumGui {
    pub(crate) fn view_calibration(&self, state: &ConnectedState, cx: &mut Context<Self>) -> Div {
        let done_count = state.cal_calibrated.len();
        let total = keymap::WIN60HE_KEYS.len();

        let status_alert = if state.cal_active {
            Alert::warning("cal-state-alert", state.cal_status_text.clone())
                .title(format!("Calibration Armed: {done_count} / {total}"))
        } else {
            Alert::info("cal-state-alert", state.cal_status_text.clone())
                .title(format!("Calibration Ready: {done_count} / {total}"))
        };

        let controls = if !state.cal_active {
            h_flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("cal-all")
                        .primary()
                        .small()
                        .icon(Icon::new(IconName::SlidersHorizontal))
                        .label("All-Key Calibration")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_cal(false, cx);
                        })),
                )
                .child(
                    Button::new("cal-any")
                        .outline()
                        .small()
                        .icon(Icon::new(IconName::Radio))
                        .label("Any-Key Calibration")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.start_cal(true, cx);
                        })),
                )
        } else {
            h_flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("cal-stop")
                        .primary()
                        .small()
                        .icon(Icon::new(IconName::Check))
                        .label("Finish & Disarm")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.stop_cal(cx);
                        })),
                )
                .child(
                    Button::new("cal-abort")
                        .danger()
                        .small()
                        .icon(Icon::new(IconName::Trash))
                        .label("Abort")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.stop_cal(cx);
                        })),
                )
        };

        v_flex()
            .w_full()
            .gap_3()
            .child(
                GroupBox::new()
                    .outline()
                    .title("Sensor Calibration")
                    .child(status_alert)
                    .child(controls)
                    .footer(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Leave all keys untouched before arming. Switching tabs auto-disarms."),
                    ),
            )
            .child(
                GroupBox::new()
                    .outline()
                    .title("Calibration Progress Matrix")
                    .child(self.render_keyboard_canvas(
                        state,
                        KeyboardCanvasMode::Calibration,
                        KB_SCALE_DEFAULT,
                        0.0,
                        cx,
                    )),
            )
    }
}
