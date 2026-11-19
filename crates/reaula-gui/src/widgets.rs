//! Shared Regal Navy paper-matte GPUI components, dropdowns, Material icon buttons,
//! and parameterized 61-key WIN 60 HE keyboard mockup.

use std::rc::Rc;

use gpui::{
    BoxShadow, ClickEvent, Context, Corner, Div, ElementId, Hsla, MouseButton, SharedString,
    Stateful, anchored, deferred, div, point, prelude::*, px, rgb, rgba,
};

use reaula_core::keymap::{KEYBOARD_CANVAS_HEIGHT, KEYBOARD_CANVAS_WIDTH, WIN60HE_KEYS};
use reaula_core::protocol::TriggerMode;

use crate::ReaulaGui;
use crate::icons::{self, MaterialIcon};
use crate::kb_3d::{self, Kb3dSceneInput, MascotPose, switch::stem_color_hex_for_switch};
use crate::state::{
    ConnectedState, DropdownId, SwitchInspectionSnapshot, format_rgb_hex, pack_rgb_u32,
};
use crate::theme;

pub const COLOR_SWATCHES: [(&str, [u8; 3]); 8] = [
    ("Red", [255, 0, 0]),
    ("Cyan", [0, 216, 255]),
    ("Emerald", [16, 220, 128]),
    ("Violet", [168, 85, 247]),
    ("Amber", [245, 158, 11]),
    ("Rose", [244, 63, 94]),
    ("Blue", [59, 130, 246]),
    ("White", [255, 255, 255]),
];

pub const SWITCH_OPTIONS: [(u8, &str); 4] = [(1, "HM1"), (2, "HH1"), (3, "CY1"), (5, "TC1")];

pub const KB_SCALE_DEFAULT: f32 = 1.38;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyboardCanvasMode {
    ActuationAndTravel,
    Calibration,
    ColorMockup,
    HotkeyPicker,
    DiagnosticsDepth,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonTone {
    Primary,
    Secondary,
    Warn { active: bool },
    Danger,
}

/// Uniform paper-matte toggle / filter pill with optional Material Icon.
pub fn icon_pill(
    id: impl Into<ElementId>,
    ic: Option<MaterialIcon>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    let bg = if active {
        theme::SHADE_REGAL
    } else {
        theme::SHADE_DEEP
    };
    let border = if active {
        theme::BORDER_FOCUS
    } else {
        theme::BORDER_MATTE
    };
    let fg = if active {
        theme::INK_PRIMARY
    } else {
        theme::INK_SECONDARY
    };

    let mut d = div()
        .id(id.into())
        .px_2p5()
        .py_1()
        .rounded_md()
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_xs()
        .text_color(rgb(fg))
        .whitespace_nowrap()
        .flex()
        .flex_row()
        .items_center()
        .gap_1p5()
        .cursor_pointer()
        .hover(|s| {
            s.bg(rgb(theme::SHADE_REGAL))
                .border_color(rgb(if active {
                    theme::BORDER_FOCUS
                } else {
                    theme::BORDER_REGAL
                }))
                .text_color(rgb(theme::INK_PRIMARY))
        });

    if let Some(m) = ic {
        d = d.child(icons::icon(m, 14.0, fg));
    }
    d.child(label.into())
}

pub fn pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    icon_pill(id, None, label, active)
}

/// Unified paper-matte action button parameterized by tone & optional Material Icon.
pub fn icon_btn(
    id: impl Into<ElementId>,
    ic: Option<MaterialIcon>,
    label: impl Into<SharedString>,
    tone: ButtonTone,
) -> Stateful<Div> {
    let (bg, border, fg, hover_border) = match tone {
        ButtonTone::Primary => (
            theme::SHADE_REGAL,
            theme::BORDER_FOCUS,
            theme::INK_PRIMARY,
            theme::INK_PRIMARY,
        ),
        ButtonTone::Secondary => (
            theme::SHADE_DEEP,
            theme::BORDER_MATTE,
            theme::INK_SECONDARY,
            theme::BORDER_REGAL,
        ),
        ButtonTone::Warn { active } => (
            if active {
                theme::SHADE_REGAL
            } else {
                theme::SHADE_DEEP
            },
            theme::BORDER_WARN,
            theme::INK_WARN,
            theme::INK_WARN,
        ),
        ButtonTone::Danger => (
            theme::SHADE_DEEP,
            theme::BORDER_DANGER,
            theme::INK_DANGER,
            theme::INK_DANGER,
        ),
    };

    let mut d = div()
        .id(id.into())
        .px_3()
        .py_1p5()
        .rounded_md()
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_xs()
        .text_color(rgb(fg))
        .whitespace_nowrap()
        .flex()
        .flex_row()
        .items_center()
        .gap_1p5()
        .cursor_pointer()
        .hover(move |s| {
            let styled = if matches!(tone, ButtonTone::Secondary) {
                s.bg(rgb(theme::SHADE_REGAL))
                    .text_color(rgb(theme::INK_PRIMARY))
            } else {
                s.bg(rgb(if matches!(tone, ButtonTone::Danger) {
                    theme::SHADE_BASE
                } else {
                    theme::SHADE_REGAL
                }))
            };
            styled.border_color(rgb(hover_border)).opacity(0.96)
        });

    if let Some(m) = ic {
        d = d.child(icons::icon(m, 14.0, fg));
    }
    d.child(label.into())
}

pub fn primary_btn(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    icon_btn(id, None, label, ButtonTone::Primary)
}

pub fn secondary_btn(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    icon_btn(id, None, label, ButtonTone::Secondary)
}

/// Uniform archival paper-matte card (`SHADE_BASE`) with stacked full-width header rule.
pub fn card_box(title: impl Into<SharedString>, subtitle: Option<&str>) -> Div {
    let mut header = div()
        .w_full()
        .pb_2p5()
        .border_b_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .flex()
        .flex_col()
        .gap_0p5()
        .flex_shrink_0()
        .child(
            div()
                .w_full()
                .text_sm()
                .text_color(rgb(theme::INK_PRIMARY))
                .child(title.into()),
        );
    if let Some(sub) = subtitle {
        header = header.child(
            div()
                .w_full()
                .text_xs()
                .text_color(rgb(theme::INK_MUTED))
                .child(sub.to_string()),
        );
    }

    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_3p5()
        .p_4()
        .rounded_xl()
        .bg(rgb(theme::SHADE_BASE))
        .border_1()
        .border_color(rgb(theme::BORDER_MATTE))
        .child(header)
}

/// Recessed `SHADE_DEEP` container nested inside `SHADE_BASE` cards for crisp tonal layering.
pub fn recessed_well() -> Div {
    div()
        .flex_shrink_0()
        .p_3()
        .rounded_lg()
        .bg(rgb(theme::SHADE_DEEP))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
}

/// Recessed matte metric tile (`SHADE_DEEP` inside `SHADE_BASE` card).
pub fn stat_tile(label: &str, value: impl Into<SharedString>, value_color: u32) -> Div {
    div()
        .min_w(px(145.0))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap_1()
        .p_2p5()
        .px_3()
        .rounded_lg()
        .bg(rgb(theme::SHADE_DEEP))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .child(
            div()
                .text_size(px(10.0))
                .text_color(rgb(theme::INK_MUTED))
                .child(label.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(value_color))
                .child(value.into()),
        )
}

pub struct DropdownChoice {
    pub label: SharedString,
    pub swatch: Option<u32>,
    pub icon: Option<MaterialIcon>,
    pub selected: bool,
}

fn swatch_square(color_u32: u32) -> Div {
    div()
        .w(px(11.0))
        .h(px(11.0))
        .rounded_sm()
        .bg(rgb(color_u32))
        .border_1()
        .border_color(rgb(theme::BORDER_REGAL))
        .flex_shrink_0()
}

fn linear_to_srgb_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0).round() as u8
}

impl ReaulaGui {
    /// Clean Regal Navy paper-matte dropdown select with deferred anchored popup.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_dropdown<F>(
        &self,
        id: DropdownId,
        caption: Option<&str>,
        current_label: impl Into<SharedString>,
        current_swatch: Option<u32>,
        current_icon: Option<MaterialIcon>,
        width_px: f32,
        choices: Vec<DropdownChoice>,
        on_select: F,
        cx: &mut Context<Self>,
    ) -> Div
    where
        F: Fn(&mut Self, usize, &mut Context<Self>) + 'static,
    {
        let is_open = self.open_dropdown == Some(id);
        let id_num = id as usize;
        let handler = Rc::new(on_select);

        let mut trigger_left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .flex_1()
            .min_w_0();
        if let Some(sw) = current_swatch {
            trigger_left = trigger_left.child(swatch_square(sw));
        }
        if let Some(ic) = current_icon {
            trigger_left = trigger_left.child(icons::icon(ic, 14.0, theme::INK_ACCENT));
        }
        trigger_left = trigger_left.child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(rgb(theme::INK_PRIMARY))
                .child(current_label.into()),
        );

        let trigger = div()
            .id(("dd-trig", id_num))
            .w(px(width_px))
            .h(px(32.0))
            .px_2p5()
            .rounded_md()
            .bg(rgb(if is_open {
                theme::SHADE_REGAL
            } else {
                theme::SHADE_DEEP
            }))
            .border_1()
            .border_color(rgb(if is_open {
                theme::BORDER_FOCUS
            } else {
                theme::BORDER_MATTE
            }))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap_2()
            .cursor_pointer()
            .hover(|s| {
                s.bg(rgb(theme::SHADE_REGAL))
                    .border_color(rgb(theme::BORDER_REGAL))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.open_dropdown = if this.open_dropdown == Some(id) {
                    None
                } else {
                    Some(id)
                };
                cx.notify();
            }))
            .child(trigger_left)
            .child(icons::icon(
                if is_open {
                    MaterialIcon::ExpandLess
                } else {
                    MaterialIcon::ExpandMore
                },
                16.0,
                theme::INK_SECONDARY,
            ));

        let mut container = div().flex().flex_col().gap_1().flex_shrink_0();
        if let Some(cap) = caption {
            container = container.child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgb(theme::INK_MUTED))
                    .child(cap.to_string()),
            );
        }
        container = container.child(trigger);

        if is_open {
            let mut menu_list = div()
                .id(("dd-scroll", id_num))
                .w(px(width_px.max(190.0)))
                .max_h(px(248.0))
                .overflow_y_scroll()
                .rounded_lg()
                .bg(rgb(theme::SHADE_BASE))
                .border_1()
                .border_color(rgb(theme::BORDER_REGAL))
                .p_1()
                .flex()
                .flex_col()
                .gap_0p5()
                .occlude()
                .shadow(vec![BoxShadow {
                    color: rgba(0x060c16cc).into(),
                    offset: point(px(0.0), px(10.0)),
                    blur_radius: px(22.0),
                    spread_radius: px(0.0),
                }])
                .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                    if this.open_dropdown == Some(id) {
                        this.open_dropdown = None;
                        cx.notify();
                    }
                }));

            for (idx, choice) in choices.into_iter().enumerate() {
                let h = Rc::clone(&handler);
                let mut item = div()
                    .id(("dd-opt", id_num * 256 + idx))
                    .px_2p5()
                    .py_1p5()
                    .rounded_md()
                    .bg(rgb(if choice.selected {
                        theme::SHADE_REGAL
                    } else {
                        theme::SHADE_BASE
                    }))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(theme::SHADE_REGAL)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_dropdown = None;
                        h(this, idx, cx);
                        cx.notify();
                    }));

                let mut item_left = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .min_w_0();
                if let Some(sw) = choice.swatch {
                    item_left = item_left.child(swatch_square(sw));
                }
                if let Some(ic) = choice.icon {
                    item_left = item_left.child(icons::icon(ic, 14.0, theme::INK_SECONDARY));
                }
                item_left = item_left.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(rgb(if choice.selected {
                            theme::INK_PRIMARY
                        } else {
                            theme::INK_SECONDARY
                        }))
                        .child(choice.label),
                );
                item = item.child(item_left);
                if choice.selected {
                    item = item.child(icons::icon(MaterialIcon::Check, 14.0, theme::INK_ACCENT));
                }
                menu_list = menu_list.child(item);
            }

            container = container.child(
                div().relative().h(px(0.0)).child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopLeft)
                            .offset(point(px(0.0), px(4.0)))
                            .snap_to_window_with_margin(px(8.0))
                            .child(menu_list),
                    )
                    .with_priority(40),
                ),
            );
        }

        container
    }

    pub(crate) fn render_keyboard_canvas(
        &self,
        state: &ConnectedState,
        mode: KeyboardCanvasMode,
        scale: f32,
        time_s: f32,
        cx: &mut Context<Self>,
    ) -> Div {
        let board_w = KEYBOARD_CANVAS_WIDTH as f32 * scale + 28.0;
        let board_h = KEYBOARD_CANVAS_HEIGHT as f32 * scale + 28.0;

        let active_custom_slot = state.light_tab.custom_slot_index();
        let impulses = if mode == KeyboardCanvasMode::ColorMockup {
            state.active_impulses()
        } else {
            Vec::new()
        };
        let preview_colors = if mode == KeyboardCanvasMode::ColorMockup {
            let depths = state.live_depths_array();
            let custom_ref = state.effective_preview_custom_frame();
            let scene = Kb3dSceneInput {
                light: &state.light_state,
                custom_colors: custom_ref,
                use_custom_colors: custom_ref.is_some(),
                typing_only: state.typing_only_effects,
                impulses: &impulses,
                selected_keys: &state.selected_keys,
                focused_key: Some(state.focused_key),
                live_depths_mm: &depths,
                time_s,
                miku: MascotPose::default(),
                teto: MascotPose::default(),
            };
            Some(kb_3d::evaluate_key_colors(&scene))
        } else {
            None
        };

        let existing_shortcut_keys: Vec<u8> = if mode == KeyboardCanvasMode::HotkeyPicker {
            state
                .shortcut_rules
                .iter()
                .filter(|r| r.enabled)
                .map(|r| r.trigger_key)
                .collect()
        } else {
            Vec::new()
        };

        let mut canvas = div()
            .relative()
            .mt(px(18.0))
            .w(px(board_w))
            .h(px(board_h))
            .flex_shrink_0()
            .rounded_xl()
            .bg(rgb(theme::SHADE_DEEP))
            .border_1()
            .border_color(rgb(theme::BORDER_MATTE))
            // WIN 60 HE rear-left USB Type-C plug housing + braided wire stub
            .child(
                div()
                    .absolute()
                    .left(px(14.0 + 46.0 * scale))
                    .top(px(-10.0))
                    .w(px(22.0))
                    .h(px(10.0))
                    .rounded_t_sm()
                    .bg(rgb(theme::SHADE_REGAL))
                    .border_1()
                    .border_color(rgb(theme::BORDER_MATTE)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(14.0 + 46.0 * scale + 8.0))
                    .top(px(-20.0))
                    .w(px(6.0))
                    .h(px(10.0))
                    .rounded_t_sm()
                    .bg(rgb(theme::SHADE_BASE))
                    .border_1()
                    .border_color(rgb(theme::BORDER_SUBTLE)),
            )
            // WIN 60 HE left-edge strap/lanyard lug visual detail
            .child(
                div()
                    .absolute()
                    .left(px(-9.0))
                    .top(px(15.0))
                    .w(px(9.0))
                    .h(px(62.0 * scale * 0.85))
                    .rounded_l_md()
                    .bg(rgb(theme::SHADE_BASE))
                    .border_1()
                    .border_color(rgb(theme::BORDER_MATTE)),
            )
            // Bottom-right subtle embossed AULA emblem indicator
            .child(
                div()
                    .absolute()
                    .right(px(18.0))
                    .bottom(px(4.0))
                    .text_color(rgb(theme::INK_DIM))
                    .text_size(px(8.5))
                    .child("AULA"),
            );

        for (slot, k) in WIN60HE_KEYS.iter().enumerate() {
            let kx = 14.0 + (k.x as f32 - 5.0) * scale;
            let ky = 14.0 + (k.y as f32 - 5.0) * scale;
            let kw = (k.width as f32 - 3.0) * scale;
            let kh = (k.height as f32 - 3.0) * scale;

            let is_selected = state.selected_keys.contains(&k.index);
            let is_focused = state.focused_key == k.index;
            let live_mm = state.live_depths.get(&k.index).map(|(m, _)| *m);
            let is_cal_done = state.cal_calibrated.contains(&k.index);

            let (key_bg, border_color, top_line, sub_label, sub_color) = match mode {
                KeyboardCanvasMode::ActuationAndTravel => {
                    let act = state.actuations.get(&k.index).copied();
                    let cfg_mm = act
                        .map(|c| f64::from(c.travel_raw) * state.unit_mm)
                        .unwrap_or(1.70);
                    let sub = match live_mm {
                        Some(m) => format!("{m:.2}"),
                        None => match act {
                            Some(c) if c.mode != TriggerMode::Fixed => format!("{cfg_mm:.2}R"),
                            _ => format!("{cfg_mm:.2}"),
                        },
                    };
                    let bg: Hsla = if let Some(m) = live_mm {
                        theme::depth_matte_hsla(m, state.max_mm)
                    } else if is_selected {
                        rgb(theme::SHADE_REGAL).into()
                    } else {
                        rgb(theme::SHADE_BASE).into()
                    };
                    let border = if is_focused || live_mm.is_some() {
                        theme::BORDER_FOCUS
                    } else if is_selected {
                        theme::BORDER_REGAL
                    } else {
                        theme::BORDER_SUBTLE
                    };
                    let sub_c = if live_mm.is_some() {
                        theme::INK_PRIMARY
                    } else if act.is_some_and(|c| c.mode != TriggerMode::Fixed) {
                        theme::INK_ACCENT
                    } else {
                        theme::INK_MUTED
                    };
                    (bg, border, theme::INK_PRIMARY, sub, sub_c)
                }
                KeyboardCanvasMode::Calibration => {
                    let bg: Hsla = if is_cal_done {
                        rgb(theme::SHADE_REGAL).into()
                    } else {
                        rgb(theme::SHADE_BASE).into()
                    };
                    let border = if is_cal_done {
                        theme::BORDER_FOCUS
                    } else if state.cal_active {
                        theme::BORDER_WARN
                    } else {
                        theme::BORDER_SUBTLE
                    };
                    let sub = if is_cal_done {
                        "OK".into()
                    } else {
                        "—".into()
                    };
                    let sub_c = if is_cal_done {
                        theme::INK_OK
                    } else if state.cal_active {
                        theme::INK_WARN
                    } else {
                        theme::INK_DIM
                    };
                    (bg, border, theme::INK_PRIMARY, sub, sub_c)
                }
                KeyboardCanvasMode::ColorMockup => {
                    let lin = preview_colors
                        .as_ref()
                        .map(|arr| arr[slot])
                        .unwrap_or([0.0; 3]);
                    let srgb_arr = [
                        linear_to_srgb_u8(lin[0]),
                        linear_to_srgb_u8(lin[1]),
                        linear_to_srgb_u8(lin[2]),
                    ];
                    let bg_u32 = pack_rgb_u32([
                        (srgb_arr[0] as u16 / 2 + 16) as u8,
                        (srgb_arr[1] as u16 / 2 + 27) as u8,
                        (srgb_arr[2] as u16 / 2 + 44) as u8,
                    ]);
                    let border = if is_focused {
                        theme::BORDER_FOCUS
                    } else if is_selected {
                        theme::BORDER_REGAL
                    } else {
                        pack_rgb_u32([
                            srgb_arr[0].max(32),
                            srgb_arr[1].max(54),
                            srgb_arr[2].max(88),
                        ])
                    };
                    let hex_sub = if let Some(slot_idx) = active_custom_slot {
                        format_rgb_hex(state.custom_slots[usize::from(slot_idx)][k.index as usize])
                    } else {
                        format_rgb_hex(srgb_arr)
                    };
                    (
                        rgb(bg_u32).into(),
                        border,
                        theme::INK_PRIMARY,
                        hex_sub[..4].to_string(),
                        pack_rgb_u32([
                            srgb_arr[0].max(168),
                            srgb_arr[1].max(178),
                            srgb_arr[2].max(194),
                        ]),
                    )
                }
                KeyboardCanvasMode::HotkeyPicker => {
                    let is_draft_mod = state.draft_rule_modifier == Some(k.index);
                    let is_draft_target = state.draft_rule_key == k.index;
                    let kb_fn_hid = state.keyboard_fn_map.get(&k.index).copied();
                    let has_rule = existing_shortcut_keys.contains(&k.index);
                    let bg: Hsla = if is_draft_target || is_draft_mod || kb_fn_hid.is_some() {
                        rgb(theme::SHADE_REGAL).into()
                    } else {
                        rgb(theme::SHADE_BASE).into()
                    };
                    let border = if is_draft_target {
                        theme::BORDER_FOCUS
                    } else if is_draft_mod || has_rule {
                        theme::BORDER_REGAL
                    } else if kb_fn_hid.is_some() {
                        theme::BORDER_MATTE
                    } else {
                        theme::BORDER_SUBTLE
                    };
                    let sub = if is_draft_target {
                        "TRIG".into()
                    } else if is_draft_mod {
                        "MOD".into()
                    } else if let Some(hid) = kb_fn_hid {
                        short_kb_fn_badge(hid)
                    } else if has_rule {
                        "RULE".into()
                    } else {
                        format!("#{}", k.index)
                    };
                    let sub_c = if is_draft_target {
                        theme::INK_PRIMARY
                    } else if is_draft_mod {
                        theme::INK_ACCENT
                    } else if kb_fn_hid.is_some() {
                        theme::INK_OK
                    } else if has_rule {
                        theme::INK_SECONDARY
                    } else {
                        theme::INK_MUTED
                    };
                    (bg, border, theme::INK_PRIMARY, sub, sub_c)
                }
                KeyboardCanvasMode::DiagnosticsDepth => {
                    let stat = state.switch_tests.get(&k.index);
                    if let Some(m) = live_mm {
                        (
                            theme::depth_matte_hsla(m, state.max_mm),
                            theme::BORDER_FOCUS,
                            theme::INK_PRIMARY,
                            format!("{m:.2}"),
                            theme::INK_PRIMARY,
                        )
                    } else if let Some(s) = stat {
                        if s.released_zero && s.peak_mm >= 0.30 {
                            (
                                rgb(0x112b42).into(),
                                if is_focused {
                                    theme::BORDER_FOCUS
                                } else {
                                    theme::INK_OK
                                },
                                theme::INK_PRIMARY,
                                format!("OK {:.1}", s.peak_mm),
                                theme::INK_OK,
                            )
                        } else if s.peak_mm > 0.05 {
                            (
                                rgb(0x282117).into(),
                                if is_focused {
                                    theme::BORDER_FOCUS
                                } else {
                                    theme::BORDER_WARN
                                },
                                theme::INK_PRIMARY,
                                format!("{:.2}?", s.peak_mm),
                                theme::INK_WARN,
                            )
                        } else {
                            (
                                rgb(theme::SHADE_BASE).into(),
                                if is_focused {
                                    theme::BORDER_FOCUS
                                } else {
                                    theme::BORDER_SUBTLE
                                },
                                theme::INK_PRIMARY,
                                "0.00".into(),
                                theme::INK_DIM,
                            )
                        }
                    } else {
                        (
                            rgb(if is_focused {
                                theme::SHADE_REGAL
                            } else {
                                theme::SHADE_BASE
                            })
                            .into(),
                            if is_focused {
                                theme::BORDER_FOCUS
                            } else {
                                theme::BORDER_SUBTLE
                            },
                            theme::INK_PRIMARY,
                            "—".into(),
                            theme::INK_DIM,
                        )
                    }
                }
            };

            let diag_bar_fill = if mode == KeyboardCanvasMode::DiagnosticsDepth {
                let measured = live_mm
                    .or_else(|| state.switch_tests.get(&k.index).map(|s| s.peak_mm))
                    .unwrap_or(0.0);
                let frac = (measured / state.max_mm.max(0.5)).clamp(0.0, 1.0) as f32;
                let bar_col = if live_mm.is_some() {
                    theme::INK_ACCENT
                } else if state
                    .switch_tests
                    .get(&k.index)
                    .is_some_and(|s| s.released_zero && s.peak_mm >= 0.30)
                {
                    theme::INK_OK
                } else if measured > 0.05 {
                    theme::INK_WARN
                } else {
                    theme::BORDER_SUBTLE
                };
                Some((frac, bar_col))
            } else {
                None
            };

            let key_index = k.index;
            let mut key_tile = div()
                    .id(("kb-key", (mode as usize) * 256 + usize::from(key_index)))
                    .absolute()
                    .left(px(kx))
                    .top(px(ky))
                    .w(px(kw))
                    .h(px(kh))
                    .rounded_md()
                    .bg(key_bg)
                    .border_1()
                    .border_color(rgb(border_color))
                    .px_1()
                    .py_0p5()
                    .flex()
                    .flex_col()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|s| s.border_color(rgb(theme::BORDER_FOCUS)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.on_canvas_key_click(mode, key_index, ev, cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, _, cx| {
                            if mode == KeyboardCanvasMode::ColorMockup
                                && let Ok(st) = &mut this.conn
                                && let Some(slot_idx) = st.light_tab.custom_slot_index()
                            {
                                st.custom_brush =
                                    st.custom_slots[usize::from(slot_idx)][key_index as usize];
                                this.status_line = format!(
                                    "Sampled color {} from key {}.",
                                    format_rgb_hex(st.custom_brush),
                                    k.name
                                );
                                cx.notify();
                            }
                        }),
                    )
                    .child(
                        div()
                            .text_size(px(9.5 * (scale / KB_SCALE_DEFAULT).clamp(0.85, 1.05)))
                            .text_color(rgb(top_line))
                            .child(k.name),
                    )
                    .child(
                        div()
                            .text_size(px(8.5 * (scale / KB_SCALE_DEFAULT).clamp(0.85, 1.05)))
                            .text_color(rgb(sub_color))
                            .child(sub_label),
                    );
            if let Some((frac, bar_col)) = diag_bar_fill {
                let fill_w = ((kw - 6.0).max(4.0) * frac).max(0.0);
                key_tile = key_tile.child(
                    div()
                        .w_full()
                        .h(px(2.5))
                        .rounded_xs()
                        .bg(rgb(0x0b1424))
                        .overflow_hidden()
                        .child(div().w(px(fill_w)).h_full().bg(rgb(bar_col))),
                );
            }
            canvas = canvas.child(key_tile);
        }

        div()
            .w_full()
            .flex_shrink_0()
            .flex()
            .justify_center()
            .py_1()
            .pl_3()
            .child(canvas)
    }

    fn on_canvas_key_click(
        &mut self,
        mode: KeyboardCanvasMode,
        key_index: u8,
        ev: &ClickEvent,
        cx: &mut Context<Self>,
    ) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        match mode {
            KeyboardCanvasMode::Calibration => {}
            KeyboardCanvasMode::HotkeyPicker => {
                if ev.modifiers().shift {
                    state.draft_rule_modifier = Some(key_index);
                    if let Some(k) = reaula_core::keymap::by_index(key_index) {
                        self.status_line = format!(
                            "Hotkey chord modifier set to {} (slot {})",
                            k.name, key_index
                        );
                    }
                } else {
                    state.draft_rule_key = key_index;
                    if let Some(k) = reaula_core::keymap::by_index(key_index) {
                        self.status_line =
                            format!("Hotkey trigger set to {} (slot {})", k.name, key_index);
                    }
                }
            }
            KeyboardCanvasMode::ColorMockup => {
                if let Some(slot_idx) = state.light_tab.custom_slot_index()
                    && state.custom_paint_mode
                {
                    state.custom_slots[usize::from(slot_idx)][key_index as usize] =
                        state.custom_brush;
                    state.focused_key = key_index;
                    self.dirty_3d = true;
                    cx.notify();
                    return;
                }
                apply_key_selection(state, key_index, ev);
                self.dirty_3d = true;
            }
            KeyboardCanvasMode::ActuationAndTravel => {
                apply_key_selection(state, key_index, ev);
                self.dirty_3d = true;
            }
            KeyboardCanvasMode::DiagnosticsDepth => {
                state.focused_key = key_index;
                self.dirty_switch_3d = true;
                if let Some(k) = reaula_core::keymap::by_index(key_index) {
                    self.status_line =
                        format!("Inspecting Hall-effect switch {} (#{}).", k.name, key_index);
                }
            }
        }
        cx.notify();
    }
}

/// 2D cutaway cross-section schematic mimicking the Hall-effect switch mechanism.
/// Serves both as a manual `2D` inspection view and automatic fallback when `wgpu` is unavailable.
pub fn render_switch_2d_cross_section(spec: &SwitchInspectionSnapshot) -> Div {
    let max_mm = spec.max_mm.max(0.5);
    let travel_frac = (spec.travel_mm / max_mm).clamp(0.0, 1.0) as f32;
    let act_frac = (spec.actuation_mm / max_mm).clamp(0.02, 0.98) as f32;

    let max_stroke_px = 68.0_f32;
    let travel_px = travel_frac * max_stroke_px;
    let act_offset_px = act_frac * max_stroke_px;

    let stem_hex = stem_color_hex_for_switch(spec.switch_id);
    let act_col = if spec.triggered {
        theme::INK_OK
    } else if spec.rapid_trigger {
        0xa855f7
    } else {
        theme::INK_ACCENT
    };

    // Vertical stack anchors inside the 320 x 288 cutaway viewport
    let chamber_left = 86.0_f32;
    let chamber_w = 148.0_f32;
    let magnet_rest_bot = 152.0_f32;
    let magnet_bot_y = magnet_rest_bot + travel_px;
    let pcb_top_y = 242.0_f32;
    let act_line_y = magnet_rest_bot + act_offset_px;

    // Compressing coil spring stack between bottom PCB seat and plunger flange
    let plunger_bot_y = 118.0_f32 + travel_px;
    let spring_bot_y = 228.0_f32;
    let spring_span = (spring_bot_y - plunger_bot_y).max(20.0);
    let coils = 7usize;

    let mut stage = div()
        .relative()
        .w(px(330.0))
        .h(px(288.0))
        .rounded_lg()
        .bg(rgb(0x070f1d))
        .border_1()
        .border_color(rgb(theme::BORDER_SUBTLE))
        .overflow_hidden()
        // Top-left technical schematic header label
        .child(
            div()
                .absolute()
                .left(px(10.0))
                .top(px(8.0))
                .text_size(px(10.0))
                .text_color(rgb(theme::INK_MUTED))
                .child(format!(
                    "2D CROSS-SECTION · {} ({})",
                    spec.key_name, spec.switch_label
                )),
        )
        // Left-side millimeter ruler gauge (0.00 -> max_mm)
        .child(
            div()
                .absolute()
                .left(px(12.0))
                .top(px(magnet_rest_bot - 6.0))
                .text_size(px(9.5))
                .text_color(rgb(theme::INK_DIM))
                .child("0.00"),
        )
        .child(
            div()
                .absolute()
                .left(px(46.0))
                .top(px(magnet_rest_bot))
                .w(px(2.0))
                .h(px(max_stroke_px))
                .bg(rgb(theme::BORDER_MATTE)),
        )
        .child(
            div()
                .absolute()
                .left(px(12.0))
                .top(px(magnet_rest_bot + max_stroke_px - 6.0))
                .text_size(px(9.5))
                .text_color(rgb(theme::INK_DIM))
                .child(format!("{max_mm:.2}")),
        )
        // Live stem depth tick on ruler
        .child(
            div()
                .absolute()
                .left(px(42.0))
                .top(px(magnet_bot_y - 2.0))
                .w(px(10.0))
                .h(px(4.0))
                .rounded_xs()
                .bg(rgb(act_col)),
        )
        // Cutaway Housing Outer & Inner Side Walls (translucent PC top + Nylon bottom)
        .child(
            div()
                .absolute()
                .left(px(chamber_left))
                .top(px(96.0))
                .w(px(14.0))
                .h(px(56.0))
                .rounded_tl_md()
                .bg(rgb(0x475e7d))
                .border_1()
                .border_color(rgb(0x7c97ba)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w - 14.0))
                .top(px(96.0))
                .w(px(14.0))
                .h(px(56.0))
                .rounded_tr_md()
                .bg(rgb(0x475e7d))
                .border_1()
                .border_color(rgb(0x7c97ba)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left))
                .top(px(152.0))
                .w(px(14.0))
                .h(px(90.0))
                .bg(rgb(0x1b293f))
                .border_1()
                .border_color(rgb(theme::BORDER_MATTE)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w - 14.0))
                .top(px(152.0))
                .w(px(14.0))
                .h(px(90.0))
                .bg(rgb(0x1b293f))
                .border_1()
                .border_color(rgb(theme::BORDER_MATTE)),
        )
        // Chamber interior backdrop
        .child(
            div()
                .absolute()
                .left(px(chamber_left + 14.0))
                .top(px(104.0))
                .w(px(chamber_w - 28.0))
                .h(px(pcb_top_y - 104.0))
                .bg(rgb(0x0b1526)),
        );

    // Compressing metallic coil spring segments
    for c in 0..coils {
        let frac = (c as f32 + 0.5) / (coils as f32);
        let y = plunger_bot_y + frac * spring_span;
        let stagger = if c % 2 == 0 { -2.0 } else { 2.0 };
        stage = stage.child(
            div()
                .absolute()
                .left(px(chamber_left + 44.0 + stagger))
                .top(px(y))
                .w(px(chamber_w - 88.0))
                .h(px(3.5))
                .rounded_xs()
                .bg(rgb(0xd8b468)),
        );
    }

    // Actuation threshold laser line + right label
    stage = stage
        .child(
            div()
                .absolute()
                .left(px(chamber_left + 16.0))
                .top(px(act_line_y))
                .w(px(chamber_w - 32.0))
                .h(px(2.0))
                .bg(rgb(act_col)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w + 8.0))
                .top(px(act_line_y - 7.0))
                .text_size(px(9.5))
                .text_color(rgb(act_col))
                .child(format!("ACT {:.2} mm", spec.actuation_mm)),
        )
        // Moving Keycap Crown & Cutaway Skirt
        .child(
            div()
                .absolute()
                .left(px(chamber_left - 8.0))
                .top(px(24.0 + travel_px))
                .w(px(chamber_w + 16.0))
                .h(px(26.0))
                .rounded_t_md()
                .bg(rgb(0x213657))
                .border_1()
                .border_color(rgb(theme::BORDER_REGAL))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.5))
                .text_color(rgb(theme::INK_PRIMARY))
                .child(spec.key_name),
        )
        // Moving POM Cross Stem + Plunger
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w * 0.5 - 8.0))
                .top(px(50.0 + travel_px))
                .w(px(16.0))
                .h(px(46.0))
                .bg(rgb(stem_hex)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + 20.0))
                .top(px(88.0 + travel_px))
                .w(px(chamber_w - 40.0))
                .h(px(30.0))
                .rounded_sm()
                .bg(rgb(stem_hex)),
        )
        // Moving Neodymium Magnet [S (silver) | N (red)] embedded in plunger base
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w * 0.5 - 12.0))
                .top(px(magnet_bot_y - 24.0))
                .w(px(24.0))
                .h(px(12.0))
                .rounded_t_xs()
                .bg(rgb(0xcbd5e1)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w * 0.5 - 12.0))
                .top(px(magnet_bot_y - 12.0))
                .w(px(24.0))
                .h(px(12.0))
                .rounded_b_xs()
                .bg(rgb(0xef4444)),
        )
        // Magnetic flux halo + SMD Hall-Effect Sensor IC + FR-4 PCB Deck
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w * 0.5 - (18.0 + 18.0 * travel_frac)))
                .top(px(pcb_top_y - 14.0))
                .w(px((18.0 + 18.0 * travel_frac) * 2.0))
                .h(px(14.0))
                .rounded_t_md()
                .bg(rgb(if spec.triggered {
                    0x104f46
                } else if travel_frac > 0.05 {
                    0x123e68
                } else {
                    0x0c1b2e
                })),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w * 0.5 - 16.0))
                .top(px(pcb_top_y - 9.0))
                .w(px(32.0))
                .h(px(9.0))
                .rounded_t_xs()
                .bg(rgb(0x161c28))
                .border_1()
                .border_color(rgb(act_col)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left - 18.0))
                .top(px(pcb_top_y))
                .w(px(chamber_w + 36.0))
                .h(px(16.0))
                .rounded_xs()
                .bg(rgb(0x10273b))
                .border_1()
                .border_color(rgb(theme::BORDER_REGAL))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(9.0))
                .text_color(rgb(act_col))
                .child(format!(
                    "PCB HALL IC · FLUX {:.0}% · {}",
                    spec.flux_pct,
                    if spec.triggered { "TRIGGERED" } else { "READY" }
                )),
        );

    stage
}

fn apply_key_selection(state: &mut ConnectedState, key_index: u8, ev: &ClickEvent) {
    let mods = ev.modifiers();
    if mods.shift || mods.control {
        if !state.selected_keys.remove(&key_index) {
            state.selected_keys.insert(key_index);
        }
    } else {
        state.selected_keys.clear();
        state.selected_keys.insert(key_index);
    }
    state.sync_draft_from_key(key_index);
}

fn short_kb_fn_badge(hid: u8) -> String {
    match hid {
        0x52 => "KB:Up".into(),
        0x51 => "KB:Dn".into(),
        0x50 => "KB:Lt".into(),
        0x4F => "KB:Rt".into(),
        0x49 => "KB:Ins".into(),
        0x4A => "KB:Hom".into(),
        0x4B => "KB:PgU".into(),
        0x4C => "KB:Del".into(),
        0x4D => "KB:End".into(),
        0x4E => "KB:PgD".into(),
        0x29 => "KB:Esc".into(),
        0x35 => "KB:`".into(),
        0x46 => "KB:Prt".into(),
        _ => {
            let label = reaula_core::keymap::usb_hid_name(hid);
            let compact: String = label
                .chars()
                .filter(|c| !c.is_whitespace())
                .take(4)
                .collect();
            format!("KB:{compact}")
        }
    }
}
