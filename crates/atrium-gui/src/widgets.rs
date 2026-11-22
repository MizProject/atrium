//! Domain hardware widgets: 61-key WIN 60 HE board canvas, 2D switch cutaway schematic,
//! keymap badges, and Select option items.

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::component::select::SelectItem;
use gpui_kit::{
    AnyElement, App, ClickEvent, Context, Div, IntoElement as _, MouseButton, SharedString, div,
    prelude::*, px, rgb,
};

use atrium_core::ipc::{ShortcutAction, ShortcutRule, VirtualKey};
use atrium_core::keymap::{KEYBOARD_CANVAS_HEIGHT, KEYBOARD_CANVAS_WIDTH, WIN60HE_KEYS};
use atrium_core::protocol::TriggerMode;

use crate::AtriumGui;
use crate::kb_3d::{self, Kb3dSceneInput, MascotPose, switch::stem_color_hex_for_switch};
use crate::state::{
    ConnectedState, KeymapCategory, KeymapLayerView, SwitchInspectionSnapshot, format_rgb_hex,
    pack_rgb_u32,
};
use crate::theme::{self, HardwareColors};

pub const COLOR_SWATCHES: [(&str, [u8; 3]); 16] = [
    ("Off / Black", [0, 0, 0]),
    ("Pure White", [255, 255, 255]),
    ("Ice Blue", [186, 230, 253]),
    ("Crimson", [239, 35, 60]),
    ("Coral", [255, 107, 107]),
    ("Orange", [255, 122, 0]),
    ("Amber", [245, 158, 11]),
    ("Gold", [250, 204, 21]),
    ("Lime", [132, 204, 22]),
    ("Emerald", [16, 220, 128]),
    ("Mint", [45, 212, 191]),
    ("Aqua Blue", [0, 215, 255]),
    ("Sky Blue", [56, 189, 248]),
    ("Royal Blue", [59, 130, 246]),
    ("Violet", [168, 85, 247]),
    ("Hot Pink", [244, 63, 148]),
];

pub const SWITCH_OPTIONS: [(u8, &str); 4] = [(1, "HM1"), (2, "HH1"), (3, "CY1"), (5, "TC1")];

pub const KB_SCALE_DEFAULT: f32 = 1.24;

#[derive(Clone, Debug, PartialEq)]
pub struct SelectOption<T: Clone + PartialEq + 'static> {
    pub label: SharedString,
    pub value: T,
}

impl<T: Clone + PartialEq + 'static> SelectOption<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            value,
        }
    }
}

impl<T: Clone + PartialEq + 'static> SelectItem for SelectOption<T> {
    type Value = T;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

pub struct MetricCell {
    pub label: SharedString,
    pub value: AnyElement,
}

impl MetricCell {
    pub fn text(label: impl Into<SharedString>, value: impl Into<SharedString>, cx: &App) -> Self {
        let fg = cx.theme().foreground;
        Self {
            label: label.into(),
            value: div()
                .w_full()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(fg)
                .child(value.into())
                .into_any_element(),
        }
    }

    pub fn element(label: impl Into<SharedString>, value: AnyElement) -> Self {
        Self {
            label: label.into(),
            value,
        }
    }
}

/// Responsive key-value metric grid that wraps cleanly across breakpoints and
/// keeps labels and values within cell boundaries without mid-word wrapping.
pub fn render_metric_table(cells: Vec<MetricCell>, min_col_px: f32, cx: &App) -> Div {
    let border = cx.theme().border;
    let bg = cx.theme().background;
    let muted = cx.theme().muted_foreground;
    let radius = cx.theme().radius;

    h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .gap_2()
        .children(cells.into_iter().map(|cell| {
            v_flex()
                .flex_1()
                .min_w(px(min_col_px))
                .overflow_hidden()
                .px_2p5()
                .py_1p5()
                .gap_0p5()
                .rounded(radius)
                .border_1()
                .border_color(border)
                .bg(bg)
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.0))
                        .text_color(muted)
                        .child(cell.label),
                )
                .child(div().w_full().min_w_0().overflow_hidden().child(cell.value))
        }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyboardCanvasMode {
    ActuationAndTravel,
    Calibration,
    ColorMockup,
    HotkeyPicker,
    DiagnosticsDepth,
}

pub fn hardware_tag(
    label: impl Into<SharedString>,
    is_firmware: bool,
    emphasized: bool,
    hc: &HardwareColors,
) -> Div {
    let (bg, border, ink) = if is_firmware {
        (
            if emphasized { hc.fw_glow } else { hc.fw_bg },
            hc.fw_border,
            hc.fw_ink,
        )
    } else {
        (
            if emphasized { hc.dm_active } else { hc.dm_bg },
            hc.dm_border,
            hc.dm_ink,
        )
    };
    div()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .bg(rgb(bg))
        .border_1()
        .border_color(rgb(border))
        .text_size(px(10.0))
        .text_color(rgb(ink))
        .whitespace_nowrap()
        .child(label.into())
}

pub fn action_kind_tag(action: &ShortcutAction) -> &'static str {
    match action {
        ShortcutAction::EmitKey { .. } => "uinput Key",
        ShortcutAction::RunCommand { .. } => "Shell Cmd",
        ShortcutAction::ActuationPreset { .. } => "Actuation",
        ShortcutAction::ToggleSocd => "SOCD",
        ShortcutAction::LightingPreset { .. } | ShortcutAction::ToggleLighting => "RGB",
        ShortcutAction::SetPollRate { .. } => "Poll Rate",
    }
}

impl AtriumGui {
    pub(crate) fn render_keyboard_canvas(
        &self,
        state: &ConnectedState,
        mode: KeyboardCanvasMode,
        scale: f32,
        time_s: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hc = HardwareColors::from_app(cx);
        let board_w = KEYBOARD_CANVAS_WIDTH as f32 * scale + 28.0;
        let board_h = KEYBOARD_CANVAS_HEIGHT as f32 * scale + 28.0;

        let active_custom_slot = state.light_tab.custom_slot_index();
        let impulses = if mode == KeyboardCanvasMode::ColorMockup {
            state.active_impulses()
        } else {
            Vec::new()
        };
        let (aqua_keys, aqua_pulsing) = state.active_daemon_indicators();
        let fn_held = state.is_fn_held();
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
                daemon_aqua_keys: &aqua_keys,
                daemon_aqua_pulsing: &aqua_pulsing,
            };
            Some(kb_3d::evaluate_key_colors(&scene))
        } else {
            None
        };

        let hotkey_filter = state.keymap_layer_view;

        let mut canvas = div()
            .relative()
            .mt(px(16.0))
            .w(px(board_w))
            .h(px(board_h))
            .flex_shrink_0()
            .rounded_xl()
            .bg(rgb(hc.deck_bg))
            .border_1()
            .border_color(rgb(hc.deck_border))
            .child(
                div()
                    .absolute()
                    .left(px(14.0 + 46.0 * scale))
                    .top(px(-9.0))
                    .w(px(22.0))
                    .h(px(9.0))
                    .rounded_t_sm()
                    .bg(rgb(hc.key_selected_bg))
                    .border_1()
                    .border_color(rgb(hc.deck_border)),
            )
            .child(
                div()
                    .absolute()
                    .left(px(-8.0))
                    .top(px(15.0))
                    .w(px(8.0))
                    .h(px(62.0 * scale * 0.85))
                    .rounded_l_md()
                    .bg(rgb(hc.key_bg))
                    .border_1()
                    .border_color(rgb(hc.deck_border)),
            )
            .child(
                div()
                    .absolute()
                    .right(px(18.0))
                    .bottom(px(4.0))
                    .text_color(rgb(hc.key_dim_ink))
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

            let mut hotkey_top_tag: Option<(&'static str, u32)> = None;
            let mut hotkey_bar: Option<(u32, Option<u32>, f32)> = None;

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
                    let bg = if let Some(m) = live_mm {
                        theme::depth_fill_hsla(m, state.max_mm, hc.is_dark)
                    } else if is_selected {
                        rgb(hc.key_selected_bg).into()
                    } else {
                        rgb(hc.key_bg).into()
                    };
                    let border = if is_focused || live_mm.is_some() {
                        hc.key_border_focus
                    } else if is_selected {
                        hc.key_border_selected
                    } else {
                        hc.key_border
                    };
                    let sub_c = if live_mm.is_some() || act.is_some_and(|c| c.mode != TriggerMode::Fixed) {
                        hc.key_ink
                    } else {
                        hc.key_sub_ink
                    };
                    (bg, border, hc.key_ink, sub, sub_c)
                }
                KeyboardCanvasMode::Calibration => {
                    let bg = if is_cal_done {
                        rgb(hc.ok_bg).into()
                    } else {
                        rgb(hc.key_bg).into()
                    };
                    let border = if is_cal_done {
                        hc.ok_border
                    } else if state.cal_active {
                        hc.warn_border
                    } else {
                        hc.key_border
                    };
                    let sub = if is_cal_done { "OK".into() } else { "-".into() };
                    let sub_c = if is_cal_done {
                        hc.ok_ink
                    } else if state.cal_active {
                        hc.warn_ink
                    } else {
                        hc.key_dim_ink
                    };
                    (bg, border, hc.key_ink, sub, sub_c)
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
                        (srgb_arr[0] as u16 / 2 + 12) as u8,
                        (srgb_arr[1] as u16 / 2 + 14) as u8,
                        (srgb_arr[2] as u16 / 2 + 18) as u8,
                    ]);
                    let border = if is_focused {
                        hc.key_border_focus
                    } else if is_selected {
                        hc.key_border_selected
                    } else {
                        pack_rgb_u32([
                            srgb_arr[0].max(36),
                            srgb_arr[1].max(40),
                            srgb_arr[2].max(48),
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
                        0xF3F4F6,
                        hex_sub[..4].to_string(),
                        0xD1D5DB,
                    )
                }
                KeyboardCanvasMode::HotkeyPicker => {
                    let vis = hotkey_key_appearance(
                        state,
                        hotkey_filter,
                        k.index,
                        fn_held,
                        &aqua_keys,
                        &aqua_pulsing,
                        &hc,
                    );
                    hotkey_top_tag = vis.top_tag;
                    hotkey_bar = vis.bottom_bar;
                    (
                        rgb(vis.bg).into(),
                        vis.border,
                        hc.key_ink,
                        vis.sub_label,
                        vis.sub_color,
                    )
                }
                KeyboardCanvasMode::DiagnosticsDepth => {
                    let stat = state.switch_tests.get(&k.index);
                    if let Some(m) = live_mm {
                        (
                            theme::depth_fill_hsla(m, state.max_mm, hc.is_dark),
                            hc.key_border_focus,
                            hc.key_ink,
                            format!("{m:.2}"),
                            hc.key_ink,
                        )
                    } else if let Some(s) = stat {
                        if s.released_zero && s.peak_mm >= 0.30 {
                            (
                                rgb(hc.ok_bg).into(),
                                if is_focused {
                                    hc.key_border_focus
                                } else {
                                    hc.ok_border
                                },
                                hc.key_ink,
                                format!("OK {:.1}", s.peak_mm),
                                hc.ok_ink,
                            )
                        } else if s.peak_mm > 0.05 {
                            (
                                rgb(hc.warn_bg).into(),
                                if is_focused {
                                    hc.key_border_focus
                                } else {
                                    hc.warn_border
                                },
                                hc.key_ink,
                                format!("{:.2}?", s.peak_mm),
                                hc.warn_ink,
                            )
                        } else {
                            (
                                rgb(hc.key_bg).into(),
                                if is_focused {
                                    hc.key_border_focus
                                } else {
                                    hc.key_border
                                },
                                hc.key_ink,
                                "0.00".into(),
                                hc.key_dim_ink,
                            )
                        }
                    } else {
                        (
                            rgb(if is_focused {
                                hc.key_selected_bg
                            } else {
                                hc.key_bg
                            })
                            .into(),
                            if is_focused {
                                hc.key_border_focus
                            } else {
                                hc.key_border
                            },
                            hc.key_ink,
                            "-".into(),
                            hc.key_dim_ink,
                        )
                    }
                }
            };

            let bottom_bar_fill = if let Some((left_col, right_col, frac)) = hotkey_bar {
                Some((left_col, right_col, frac))
            } else if mode == KeyboardCanvasMode::DiagnosticsDepth {
                let measured = live_mm
                    .or_else(|| state.switch_tests.get(&k.index).map(|s| s.peak_mm))
                    .unwrap_or(0.0);
                let frac = (measured / state.max_mm.max(0.5)).clamp(0.0, 1.0) as f32;
                let bar_col = if live_mm.is_some() {
                    hc.dm_border
                } else if state
                    .switch_tests
                    .get(&k.index)
                    .is_some_and(|s| s.released_zero && s.peak_mm >= 0.30)
                {
                    hc.ok_ink
                } else if measured > 0.05 {
                    hc.warn_ink
                } else {
                    hc.key_border
                };
                Some((bar_col, None, frac))
            } else {
                None
            };

            let key_index = k.index;
            let key_font_top = 9.5 * (scale / KB_SCALE_DEFAULT).clamp(0.85, 1.05);
            let mut key_tile = div()
                .id(("kb-key", (mode as usize) * 256 + usize::from(key_index)))
                .absolute()
                .left(px(kx))
                .top(px(ky))
                .w(px(kw))
                .h(px(kh))
                .rounded(cx.theme().radius)
                .bg(key_bg)
                .border_1()
                .border_color(rgb(border_color))
                .px_1()
                .py_0p5()
                .flex()
                .flex_col()
                .justify_between()
                .cursor_pointer()
                .hover(|s| s.border_color(rgb(hc.key_border_focus)))
                .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                    this.on_canvas_key_click(mode, key_index, ev, window, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, _, window, cx| {
                        if mode == KeyboardCanvasMode::ColorMockup
                            && let Ok(st) = &mut this.conn
                            && let Some(slot_idx) = st.light_tab.custom_slot_index()
                        {
                            let brush = st.custom_slots[usize::from(slot_idx)][key_index as usize];
                            st.custom_brush = brush;
                            this.status_line = format!(
                                "Sampled {} from key {}.",
                                format_rgb_hex(brush),
                                k.name
                            );
                            this.sync_color_pickers_from_state(window, cx);
                            cx.notify();
                        }
                    }),
                )
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_0p5()
                        .child(
                            div()
                                .text_size(px(key_font_top))
                                .text_color(rgb(top_line))
                                .child(k.name),
                        )
                        .when_some(hotkey_top_tag, |row, (tag, tag_col)| {
                            row.child(div().text_size(px(7.5)).text_color(rgb(tag_col)).child(tag))
                        }),
                )
                .child(
                    div()
                        .text_size(px(8.5 * (scale / KB_SCALE_DEFAULT).clamp(0.85, 1.05)))
                        .text_color(rgb(sub_color))
                        .child(sub_label),
                );
            if let Some((left_col, right_col, frac)) = bottom_bar_fill {
                let track_w = (kw - 6.0).max(4.0);
                let mut bar = div()
                    .w_full()
                    .h(px(2.5))
                    .rounded_xs()
                    .bg(rgb(hc.track_bg))
                    .overflow_hidden()
                    .flex()
                    .flex_row();
                if let Some(rc) = right_col {
                    bar = bar
                        .child(div().flex_1().h_full().bg(rgb(left_col)))
                        .child(div().flex_1().h_full().bg(rgb(rc)));
                } else {
                    bar = bar.child(
                        div()
                            .w(px((track_w * frac).max(0.0)))
                            .h_full()
                            .bg(rgb(left_col)),
                    );
                }
                key_tile = key_tile.child(bar);
            }
            canvas = canvas.child(key_tile);
        }

        div()
            .id(("kb-scroll", mode as usize))
            .w_full()
            .min_w_0()
            .overflow_x_scrollbar()
            .child(
                div()
                    .min_w_full()
                    .flex()
                    .justify_center()
                    .py_1()
                    .pl_2()
                    .child(canvas),
            )
            .into_any_element()
    }

    fn on_canvas_key_click(
        &mut self,
        mode: KeyboardCanvasMode,
        key_index: u8,
        ev: &ClickEvent,
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) {
        let mut sync_main = false;
        let mut sync_keymaps = false;
        {
            let Ok(state) = &mut self.conn else {
                return;
            };
            match mode {
                KeyboardCanvasMode::Calibration => {}
                KeyboardCanvasMode::HotkeyPicker => {
                    if key_index == atrium_core::keymap::idx::FN {
                        self.status_line = "Fn (#122) is reserved for Firmware [FW] Flash.".into();
                    } else if ev.modifiers().shift {
                        if state.draft_category == KeymapCategory::FirmwareFnFlash {
                            self.status_line =
                                "Firmware Flash uses Fn; switch to Daemon Level for custom modifiers."
                                    .into();
                        } else {
                            state.draft_rule_modifier = Some(key_index);
                            sync_keymaps = true;
                            if let Some(k) = atrium_core::keymap::by_index(key_index) {
                                self.status_line =
                                    format!("Daemon modifier: {} (#{key_index})", k.name);
                            }
                        }
                    } else {
                        state.draft_rule_key = key_index;
                        match state.keymap_layer_view {
                            KeymapLayerView::FirmwareOnly => {
                                state.set_keymap_draft_category(KeymapCategory::FirmwareFnFlash);
                            }
                            KeymapLayerView::DaemonOnly => {
                                state.set_keymap_draft_category(KeymapCategory::DaemonMemory);
                            }
                            KeymapLayerView::Both => {}
                        }
                        sync_keymaps = true;
                        let fw = state.keyboard_fn_map.get(&key_index).map(|&hid| {
                            format!("FW: Fn+Key -> {}", atrium_core::keymap::usb_hid_name(hid))
                        });
                        let dm = state
                            .shortcut_rules
                            .iter()
                            .find(|r| r.trigger_key == key_index)
                            .map(|r| format!("DM: {} ({})", r.gesture_label(), r.action.summary()));
                        if let Some(k) = atrium_core::keymap::by_index(key_index) {
                            self.status_line = match (fw, dm) {
                                (Some(f), Some(d)) => format!("{} (#{key_index}): {f} | {d}", k.name),
                                (Some(f), None) => format!("{} (#{key_index}): {f}", k.name),
                                (None, Some(d)) => format!("{} (#{key_index}): {d}", k.name),
                                (None, None) => format!("{} (#{key_index}): unmapped", k.name),
                            };
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
                    sync_main = true;
                }
                KeyboardCanvasMode::DiagnosticsDepth => {
                    state.focused_key = key_index;
                    self.dirty_switch_3d = true;
                    if let Some(k) = atrium_core::keymap::by_index(key_index) {
                        self.status_line = format!("Inspecting {} (#{}).", k.name, key_index);
                    }
                }
            }
        }
        if sync_main {
            self.sync_main_controls_from_state(window, cx);
        }
        if sync_keymaps {
            self.sync_keymap_controls_from_state(window, cx);
        }
        cx.notify();
    }
}

pub fn render_switch_2d_cross_section(spec: &SwitchInspectionSnapshot, hc: &HardwareColors) -> Div {
    let max_mm = spec.max_mm.max(0.5);
    let travel_frac = (spec.travel_mm / max_mm).clamp(0.0, 1.0) as f32;
    let act_frac = (spec.actuation_mm / max_mm).clamp(0.02, 0.98) as f32;

    let max_stroke_px = 68.0_f32;
    let travel_px = travel_frac * max_stroke_px;
    let act_offset_px = act_frac * max_stroke_px;

    let stem_hex = stem_color_hex_for_switch(spec.switch_id);
    let act_col = if spec.triggered {
        hc.ok_ink
    } else if spec.rapid_trigger {
        0xA855F7
    } else {
        hc.dm_border
    };

    let chamber_left = 74.0_f32;
    let chamber_w = 148.0_f32;
    let magnet_rest_bot = 152.0_f32;
    let magnet_bot_y = magnet_rest_bot + travel_px;
    let pcb_top_y = 242.0_f32;
    let act_line_y = magnet_rest_bot + act_offset_px;

    let plunger_bot_y = 118.0_f32 + travel_px;
    let spring_bot_y = 228.0_f32;
    let spring_span = (spring_bot_y - plunger_bot_y).max(20.0);
    let coils = 7usize;

    let mut stage = div()
        .relative()
        .w(px(304.0))
        .max_w_full()
        .h(px(288.0))
        .rounded_lg()
        .bg(rgb(hc.deck_bg))
        .border_1()
        .border_color(rgb(hc.deck_border))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .left(px(10.0))
                .top(px(8.0))
                .text_size(px(10.0))
                .text_color(rgb(hc.key_sub_ink))
                .child(format!("2D CUTAWAY · {} ({})", spec.key_name, spec.switch_label)),
        )
        .child(
            div()
                .absolute()
                .left(px(12.0))
                .top(px(magnet_rest_bot - 6.0))
                .text_size(px(9.5))
                .text_color(rgb(hc.key_dim_ink))
                .child("0.00"),
        )
        .child(
            div()
                .absolute()
                .left(px(46.0))
                .top(px(magnet_rest_bot))
                .w(px(2.0))
                .h(px(max_stroke_px))
                .bg(rgb(hc.key_border)),
        )
        .child(
            div()
                .absolute()
                .left(px(12.0))
                .top(px(magnet_rest_bot + max_stroke_px - 6.0))
                .text_size(px(9.5))
                .text_color(rgb(hc.key_dim_ink))
                .child(format!("{max_mm:.2}")),
        )
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
        .child(
            div()
                .absolute()
                .left(px(chamber_left))
                .top(px(96.0))
                .w(px(14.0))
                .h(px(56.0))
                .rounded_tl_md()
                .bg(rgb(0x4B5563))
                .border_1()
                .border_color(rgb(0x9CA3AF)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w - 14.0))
                .top(px(96.0))
                .w(px(14.0))
                .h(px(56.0))
                .rounded_tr_md()
                .bg(rgb(0x4B5563))
                .border_1()
                .border_color(rgb(0x9CA3AF)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left))
                .top(px(152.0))
                .w(px(14.0))
                .h(px(90.0))
                .bg(rgb(0x272E3B))
                .border_1()
                .border_color(rgb(hc.key_border)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w - 14.0))
                .top(px(152.0))
                .w(px(14.0))
                .h(px(90.0))
                .bg(rgb(0x272E3B))
                .border_1()
                .border_color(rgb(hc.key_border)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + 14.0))
                .top(px(104.0))
                .w(px(chamber_w - 28.0))
                .h(px(pcb_top_y - 104.0))
                .bg(rgb(0x11151C)),
        );

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
                .bg(rgb(0xD8B468)),
        );
    }

    stage
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
                .child(format!("ACT {:.2}", spec.actuation_mm)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left - 8.0))
                .top(px(24.0 + travel_px))
                .w(px(chamber_w + 16.0))
                .h(px(26.0))
                .rounded_t_md()
                .bg(rgb(0x232936))
                .border_1()
                .border_color(rgb(0x6B7280))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.5))
                .text_color(rgb(0xF3F4F6))
                .child(spec.key_name),
        )
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
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w * 0.5 - 12.0))
                .top(px(magnet_bot_y - 24.0))
                .w(px(24.0))
                .h(px(12.0))
                .rounded_t_xs()
                .bg(rgb(0xCBD5E1)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w * 0.5 - 12.0))
                .top(px(magnet_bot_y - 12.0))
                .w(px(24.0))
                .h(px(12.0))
                .rounded_b_xs()
                .bg(rgb(0xEF4444)),
        )
        .child(
            div()
                .absolute()
                .left(px(chamber_left + chamber_w * 0.5 - 16.0))
                .top(px(pcb_top_y - 9.0))
                .w(px(32.0))
                .h(px(9.0))
                .rounded_t_xs()
                .bg(rgb(0x181E29))
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
                .bg(rgb(0x151B26))
                .border_1()
                .border_color(rgb(hc.deck_border))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(9.0))
                .text_color(rgb(act_col))
                .child(format!(
                    "HALL IC · {:.0}% · {}",
                    spec.flux_pct,
                    if spec.triggered { "TRIGGERED" } else { "READY" }
                )),
        )
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

struct HotkeyKeyVisual {
    bg: u32,
    border: u32,
    top_tag: Option<(&'static str, u32)>,
    sub_label: String,
    sub_color: u32,
    bottom_bar: Option<(u32, Option<u32>, f32)>,
}

fn hotkey_key_appearance(
    state: &ConnectedState,
    filter: KeymapLayerView,
    key_index: u8,
    fn_held: bool,
    aqua_keys: &std::collections::BTreeSet<u8>,
    aqua_pulsing: &std::collections::BTreeSet<u8>,
    hc: &HardwareColors,
) -> HotkeyKeyVisual {
    let show_fw = matches!(filter, KeymapLayerView::Both | KeymapLayerView::FirmwareOnly);
    let show_dm = matches!(filter, KeymapLayerView::Both | KeymapLayerView::DaemonOnly);

    let fw_hid = show_fw
        .then(|| state.keyboard_fn_map.get(&key_index).copied())
        .flatten();
    let dm_active = show_dm
        .then(|| {
            state
                .shortcut_rules
                .iter()
                .find(|r| r.enabled && r.trigger_key == key_index)
        })
        .flatten();
    let dm_muted = (show_dm && dm_active.is_none())
        .then(|| {
            state
                .shortcut_rules
                .iter()
                .find(|r| !r.enabled && r.trigger_key == key_index)
        })
        .flatten();
    let is_dm_mod = show_dm
        && state
            .shortcut_rules
            .iter()
            .any(|r| r.enabled && r.modifier_key == Some(key_index));

    let is_aqua = show_dm && aqua_keys.contains(&key_index);
    let is_aqua_pulse = show_dm && aqua_pulsing.contains(&key_index);
    let is_fw_glow = fn_held && fw_hid.is_some();

    let is_draft_target = state.draft_rule_key == key_index;
    let is_draft_mod = match state.draft_category {
        KeymapCategory::FirmwareFnFlash => key_index == atrium_core::keymap::idx::FN,
        KeymapCategory::DaemonMemory => state.draft_rule_modifier == Some(key_index),
    };
    let top_tag = if is_draft_target {
        Some((
            "TRG",
            if state.draft_category == KeymapCategory::FirmwareFnFlash {
                hc.fw_ink
            } else {
                hc.dm_ink
            },
        ))
    } else if is_draft_mod {
        Some(("MOD", hc.dm_mod_ink))
    } else {
        None
    };

    let (bg, base_border, sub_label, sub_color, bottom_bar) = if is_aqua_pulse {
        let bar = if fw_hid.is_some() {
            (hc.fw_border, Some(hc.dm_pulse_border), 1.0)
        } else {
            (hc.dm_pulse_border, None, 1.0)
        };
        (
            hc.dm_pulse,
            hc.dm_pulse_border,
            if fw_hid.is_some() { "FW+LIVE".into() } else { "DM:LIVE".into() },
            hc.dm_pulse_ink,
            Some(bar),
        )
    } else if is_aqua {
        let bar = if fw_hid.is_some() {
            (hc.fw_border, Some(hc.dm_border), 1.0)
        } else {
            (hc.dm_border, None, 1.0)
        };
        (
            hc.dm_active,
            hc.dm_border,
            if fw_hid.is_some() { "FW+ACTV".into() } else { "DM:ACTV".into() },
            hc.dm_ink,
            Some(bar),
        )
    } else if fw_hid.is_some() && dm_active.is_some() {
        (
            hc.overlap_bg,
            hc.overlap_border,
            "FW+DM".into(),
            hc.overlap_ink,
            Some((hc.fw_border, Some(hc.dm_border), 1.0)),
        )
    } else if let Some(hid) = fw_hid {
        (
            if is_fw_glow { hc.fw_glow } else { hc.fw_bg },
            hc.fw_border,
            short_kb_fn_badge(hid),
            hc.fw_ink,
            Some((hc.fw_border, None, 1.0)),
        )
    } else if let Some(rule) = dm_active {
        let paused = !state.shortcut_watcher_active;
        (
            hc.dm_bg,
            if paused { hc.dm_dim_border } else { hc.dm_border },
            short_daemon_rule_badge(rule),
            hc.dm_ink,
            Some((
                if paused { hc.dm_dim_border } else { hc.dm_border },
                None,
                1.0,
            )),
        )
    } else if dm_muted.is_some() {
        (
            hc.dm_muted_bg,
            hc.dm_muted_border,
            "DM:Off".into(),
            hc.dm_muted_ink,
            Some((hc.dm_muted_border, None, 1.0)),
        )
    } else if is_dm_mod {
        (
            hc.dm_mod_bg,
            hc.dm_mod_border,
            "DM:Mod".into(),
            hc.dm_mod_ink,
            Some((hc.dm_mod_border, None, 1.0)),
        )
    } else {
        (
            if is_draft_target || is_draft_mod {
                hc.key_selected_bg
            } else {
                hc.key_bg
            },
            if is_draft_mod {
                hc.key_border_selected
            } else {
                hc.key_border
            },
            format!("#{}", key_index),
            hc.key_sub_ink,
            None,
        )
    };

    let border = if is_draft_target && !is_aqua && !is_aqua_pulse {
        hc.key_border_focus
    } else {
        base_border
    };

    HotkeyKeyVisual {
        bg,
        border,
        top_tag,
        sub_label,
        sub_color,
        bottom_bar,
    }
}

fn short_daemon_rule_badge(rule: &ShortcutRule) -> String {
    match &rule.action {
        ShortcutAction::EmitKey { key } => {
            let short = match key {
                VirtualKey::Up => "Up",
                VirtualKey::Down => "Dn",
                VirtualKey::Left => "Lt",
                VirtualKey::Right => "Rt",
                VirtualKey::Home => "Hm",
                VirtualKey::End => "Ed",
                VirtualKey::PageUp => "PU",
                VirtualKey::PageDown => "PD",
                VirtualKey::Insert => "In",
                VirtualKey::Delete => "Dl",
                VirtualKey::Escape => "Es",
                VirtualKey::Grave => "`",
                VirtualKey::PrintScreen => "Pr",
                VirtualKey::Mute => "Mt",
                VirtualKey::VolumeDown => "V-",
                VirtualKey::VolumeUp => "V+",
                VirtualKey::PlayPause => "Pl",
                VirtualKey::NextSong => "Nx",
                VirtualKey::PreviousSong => "Pv",
                _ => key.label(),
            };
            format!("DM:{short}")
        }
        ShortcutAction::ActuationPreset { .. } => "DM:Act".into(),
        ShortcutAction::ToggleSocd => "DM:Scd".into(),
        ShortcutAction::LightingPreset { .. } | ShortcutAction::ToggleLighting => "DM:RGB".into(),
        ShortcutAction::SetPollRate { khz } => format!("DM:{khz}k"),
        ShortcutAction::RunCommand { .. } => "DM:Cmd".into(),
    }
}

fn short_kb_fn_badge(hid: u8) -> String {
    match hid {
        0x52 => "FW:Up".into(),
        0x51 => "FW:Dn".into(),
        0x50 => "FW:Lt".into(),
        0x4F => "FW:Rt".into(),
        0x49 => "FW:Ins".into(),
        0x4A => "FW:Hom".into(),
        0x4B => "FW:PgU".into(),
        0x4C => "FW:Del".into(),
        0x4D => "FW:End".into(),
        0x4E => "FW:PgD".into(),
        0x29 => "FW:Esc".into(),
        0x35 => "FW:`".into(),
        0x46 => "FW:Prt".into(),
        _ => {
            let label = atrium_core::keymap::usb_hid_name(hid);
            let compact: String = label
                .chars()
                .filter(|c| !c.is_whitespace())
                .take(4)
                .collect();
            format!("FW:{compact}")
        }
    }
}

fn linear_to_srgb_u8(c: f32) -> u8 {
    let clamped = c.clamp(0.0, 1.0);
    let srgb = if clamped <= 0.0031308 {
        clamped * 12.92
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    };
    (srgb * 255.0).round().clamp(0.0, 255.0) as u8
}

/// 2D anatomical schematic fallback for the generic 75% unsupported keyboard rendition.
pub fn render_unsupported_2d_canvas(
    model_label: &str,
    hc: &HardwareColors,
    scale: f32,
) -> AnyElement {
    const GENERIC_2D_W: f32 = 625.0;
    const GENERIC_2D_H: f32 = 255.0;
    let board_w = GENERIC_2D_W * scale + 28.0;
    let board_h = GENERIC_2D_H * scale + 36.0;

    let mut canvas = div()
        .relative()
        .mt(px(10.0))
        .w(px(board_w))
        .h(px(board_h))
        .flex_shrink_0()
        .rounded_xl()
        .bg(rgb(hc.deck_bg))
        .border_1()
        .border_color(rgb(hc.deck_border))
        .child(
            div()
                .absolute()
                .left(px(14.0 + 52.0 * scale))
                .top(px(-9.0))
                .w(px(26.0))
                .h(px(9.0))
                .rounded_t_sm()
                .bg(rgb(hc.key_selected_bg))
                .border_1()
                .border_color(rgb(hc.deck_border)),
        )
        .child(
            div()
                .absolute()
                .left(px(14.0))
                .top(px(4.0))
                .text_xs()
                .text_color(rgb(hc.key_sub_ink))
                .child(format!("2D PROTOTYPE · {model_label}")),
        );

    for k in kb_3d::generic::GENERIC_KEYS {
        let x = 14.0 + (k.x as f32) * scale;
        let y = 24.0 + (k.y as f32) * scale;
        let w = (k.width as f32) * scale;
        let h = (k.height as f32) * scale;
        let is_accent = matches!(k.name, "Esc" | "Enter");

        let key = div()
            .id(("gen-2d-key", usize::from(k.index)))
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(w))
            .h(px(h))
            .rounded_md()
            .overflow_hidden()
            .bg(rgb(if is_accent {
                hc.dm_active
            } else {
                hc.key_bg
            }))
            .border_1()
            .border_color(rgb(if is_accent {
                hc.dm_border
            } else {
                hc.key_border
            }))
            .flex()
            .flex_col()
            .justify_between()
            .p(px(2.0))
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_color(rgb(if is_accent { hc.dm_ink } else { hc.key_ink }))
                    .text_size(px(10.0))
                    .child(k.name),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_size(px(8.0))
                    .text_color(rgb(hc.key_dim_ink))
                    .child(format!("#{}", k.index)),
            );
        canvas = canvas.child(key);
    }

    div()
        .id("unsupported-2d-scroll")
        .w_full()
        .min_w_0()
        .overflow_x_scrollbar()
        .pb_1()
        .child(canvas)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmware_and_daemon_badges_use_distinct_prefixes_and_action_tags() {
        assert_eq!(short_kb_fn_badge(0x52), "FW:Up");
        assert_eq!(short_kb_fn_badge(0x4C), "FW:Del");

        let dm_key = ShortcutRule {
            id: 1,
            name: "Up".into(),
            enabled: true,
            modifier_key: Some(atrium_core::keymap::idx::FN),
            trigger_key: atrium_core::keymap::idx::R,
            trigger_depth_mm: 1.5,
            action: ShortcutAction::EmitKey {
                key: VirtualKey::Up,
            },
        };
        let dm_cmd = ShortcutRule {
            id: 2,
            name: "Shell".into(),
            enabled: true,
            modifier_key: None,
            trigger_key: atrium_core::keymap::idx::T,
            trigger_depth_mm: 1.5,
            action: ShortcutAction::RunCommand {
                command: "echo ok".into(),
            },
        };
        assert_eq!(short_daemon_rule_badge(&dm_key), "DM:Up");
        assert_eq!(short_daemon_rule_badge(&dm_cmd), "DM:Cmd");
        assert_eq!(action_kind_tag(&dm_key.action), "uinput Key");
        assert_eq!(action_kind_tag(&dm_cmd.action), "Shell Cmd");
    }
}
