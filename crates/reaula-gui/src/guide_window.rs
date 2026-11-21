//! Separate reference window for WIN 60 HE mechanics, keymaps, shortcuts, and systemd setup.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::group_box::{GroupBox, GroupBoxVariants as _};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, Keystroke, ParentElement as _, Render,
    Styled as _, Window, WindowBounds, WindowOptions, div, px, size,
};

pub struct GuideWindow {
    focus_handle: FocusHandle,
    active_tab: usize,
}

impl GuideWindow {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            active_tab: 0,
        }
    }
}

impl Focusable for GuideWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn open_or_focus(slot: &mut Option<AnyWindowHandle>, cx: &mut App) -> Result<(), String> {
    if let Some(handle) = *slot
        && handle
            .update(cx, |_, window, _| {
                window.activate_window();
            })
            .is_ok()
    {
        return Ok(());
    }

    let bounds = Bounds::centered(None, size(px(820.0), px(620.0)), cx);
    let (handle, view) = gpui_kit::open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        },
        cx,
        |_, cx| cx.new(GuideWindow::new),
    )
    .map_err(|e| format!("failed to open Guide window: {e}"))?;

    let _ = handle.update(cx, |_, window, cx| {
        window.set_window_title("Reaula Guide & Hardware Reference");
        view.focus_handle(cx).focus(window, cx);
        window.activate_window();
    });
    *slot = Some(handle);
    Ok(())
}

impl Render for GuideWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = h_flex()
            .w_full()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(IconName::BookOpen)
                    .child(div().text_sm().child("WIN 60 HE Reference & Guide")),
            )
            .child(
                Button::new("guide-close")
                    .small()
                    .outline()
                    .label("Close")
                    .on_click(|_, window, _| {
                        window.remove_window();
                    }),
            );

        let tabs = TabBar::new("guide-tabs")
            .segmented()
            .small()
            .selected_index(self.active_tab)
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.active_tab = *ix;
                cx.notify();
            }))
            .child(Tab::new().label("Actuation & Calib"))
            .child(Tab::new().label("Keymaps [FW]/[DM]"))
            .child(Tab::new().label("Audio Sync"))
            .child(Tab::new().label("Shortcuts"))
            .child(Tab::new().label("Daemon & Systemd"));

        let body_content = match self.active_tab {
            0 => render_actuation_section(),
            1 => render_keymap_section(),
            2 => render_audio_section(),
            3 => render_shortcuts_section(),
            _ => render_daemon_section(),
        };

        let scroll_body = v_flex()
            .id("guide-scroll")
            .flex_1()
            .min_h_0()
            .p_4()
            .gap_4()
            .overflow_y_scrollbar()
            .child(tabs)
            .child(body_content);

        let focus = self.focus_handle.clone();
        if !focus.is_focused(window) {
            focus.focus(window, cx);
        }

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.focus_handle)
            .on_key_down(|ev: &KeyDownEvent, window, _| {
                if ev.keystroke.key == "escape" || ev.keystroke.key == "f1" {
                    window.remove_window();
                }
            })
            .child(header)
            .child(scroll_body)
    }
}

fn render_actuation_section() -> gpui_kit::AnyElement {
    v_flex()
        .gap_3()
        .child(
            GroupBox::new()
                .outline()
                .title("Hall-Effect Mechanics")
                .child(
                    DescriptionList::horizontal()
                        .columns(1)
                        .label_width(px(180.))
                        .bordered(true)
                        .item(
                            "Actuation Point",
                            "0.08..=3.40 mm in 0.010 mm steps for Fixed mode.",
                            1,
                        )
                        .item(
                            "Rapid Trigger (RT / Split)",
                            "Resets and re-triggers dynamically at any travel height.",
                            1,
                        )
                        .item(
                            "Top & Bottom Deadband",
                            "Hysteresis buffer near 0.00 mm rest and bottom-out to avoid bounce.",
                            1,
                        )
                        .item(
                            "Switch Profiles",
                            "HM1, HH1, CY1, TC1 curves applied via safe Read-Modify-Write.",
                            1,
                        ),
                ),
        )
        .child(
            GroupBox::new()
                .outline()
                .title("Calibration Protocol")
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_1p5()
                        .text_xs()
                        .child(div().w_full().min_w_0().whitespace_normal().child("1. Re-calibrate after swapping switches or if rest travel is above 0.00 mm."))
                        .child(div().w_full().min_w_0().whitespace_normal().child("2. Leave all keys untouched before arming All-Key or Any-Key sampling."))
                        .child(div().w_full().min_w_0().whitespace_normal().child("3. Switching tabs or closing the app automatically disarms calibration.")),
                ),
        )
        .into_any_element()
}

fn render_keymap_section() -> gpui_kit::AnyElement {
    v_flex()
        .gap_3()
        .child(
            GroupBox::new()
                .outline()
                .title("Layer Comparison")
                .child(
                    DescriptionList::horizontal()
                        .columns(1)
                        .label_width(px(180.))
                        .bordered(true)
                        .item(
                            "[FW] Flash (Amber)",
                            "Stored on MCU via cmd 0x18. Works without Linux/daemon; chord is Fn + Key for USB HID keys.",
                            1,
                        )
                        .item(
                            "[DM] Daemon (Aqua #00D7FF)",
                            "Runs in reaulad with EVIOCGRAB + /dev/uinput. Supports single keys, non-Fn chords, analog depth, media, presets, and unprivileged shell.",
                            1,
                        )
                        .item(
                            "Fn (#122) Rule",
                            "Fn is strictly reserved for Firmware [FW] so Daemon layers never collide.",
                            1,
                        ),
                ),
        )
        .into_any_element()
}

fn render_audio_section() -> gpui_kit::AnyElement {
    v_flex()
        .gap_3()
        .child(
            GroupBox::new()
                .outline()
                .title("Daemon-Hosted Audio Sync")
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_1p5()
                        .text_xs()
                        .child(div().w_full().min_w_0().whitespace_normal().child("reaulad runs a privilege-dropped --capture-pcm worker feeding 14-band FFT + beat metrics to cmd 0x09."))
                        .child(div().w_full().min_w_0().whitespace_normal().child("Audio Sync keeps running in the background after closing the GUI until explicitly stopped."))
                        .child(div().w_full().min_w_0().whitespace_normal().child("Continuous per-key streaming is experimental and may accelerate LED wear on some units.")),
                ),
        )
        .into_any_element()
}

fn render_shortcuts_section() -> gpui_kit::AnyElement {
    let key_row = |keys: &'static str, desc: &'static str| {
        let mut row = h_flex().w_full().min_w_0().items_center().justify_between().gap_3();
        if let Ok(ks) = Keystroke::parse(keys) {
            row = row.child(Kbd::new(ks));
        } else {
            row = row.child(div().flex_shrink_0().text_xs().child(keys));
        }
        row.child(div().flex_1().min_w_0().whitespace_normal().text_xs().child(desc))
    };

    v_flex()
        .gap_3()
        .child(
            GroupBox::new()
                .outline()
                .title("Desktop App Controls")
                .child(
                    v_flex()
                        .gap_2()
                        .child(key_row("f1", "Open or focus this Guide & Help window"))
                        .child(key_row("escape", "Dismiss active modal, dropdown, or close this window"))
                        .child(key_row("shift-click", "Select multiple keys or set Daemon chord modifier"))
                        .child(key_row("scroll", "Zoom 3D keyboard or switch cutaway without scrolling parent page")),
                ),
        )
        .child(
            GroupBox::new()
                .outline()
                .title("Built-In Keyboard Fn Shortcuts")
                .child(
                    DescriptionList::horizontal()
                        .columns(1)
                        .label_width(px(180.))
                        .bordered(true)
                        .item("Fn + \\|", "Cycle onboard firmware lighting modes", 1)
                        .item("Fn + Up / Down", "Adjust hardware LED brightness", 1)
                        .item("Fn + Left / Right", "Adjust dynamic lighting speed", 1)
                        .item("Fn + Win", "Toggle hardware Windows key lock", 1),
                ),
        )
        .into_any_element()
}

fn render_daemon_section() -> gpui_kit::AnyElement {
    v_flex()
        .gap_3()
        .child(
            GroupBox::new()
                .outline()
                .title("Daemon & 4-Layer Diagnostics")
                .child(
                    DescriptionList::horizontal()
                        .columns(1)
                        .label_width(px(180.))
                        .bordered(true)
                        .item("Standalone", "reaula daemon start | status | stop", 1)
                        .item("Systemd", "reaula daemon systemd install --enable --start", 1)
                        .item("User Unit", "reaula daemon systemd install --user --enable --start", 1)
                        .item(
                            "4-Layer Fault Probe",
                            "Isolates Kernel/USB, Daemon/IPC (SO_PEERCRED), Keyboard HID, and Software Codec.",
                            1,
                        ),
                ),
        )
        .into_any_element()
}
