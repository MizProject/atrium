//! Reaula WIN 60 HE desktop workbench built with GPUI Kit and wgpu.

mod guide_window;
mod kb_3d;
mod state;
mod theme;
mod views;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{SearchableVec, SelectEvent, SelectState};
use gpui_kit::component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarFooter, SidebarHeader, SidebarMenu, SidebarMenuItem,
    SidebarToggleButton,
};
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Icon, Sizable as _, ThemeMode, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Bounds, Context, Entity, FocusHandle,
    KeyDownEvent, ParentElement as _, Render, RenderImage, Styled as _, Subscription, WeakEntity,
    Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};

use reaula_core::audio_fx::{AudioEffectStyle, AudioPalette};
use reaula_core::diagnostics::DiagnosticReport;
use reaula_core::ipc::VirtualKey;
use reaula_core::keymap;
use reaula_core::protocol::{
    self, LIGHT_MAX_LEVEL, PollRate, TriggerConfig, TriggerMode, mm_to_raw,
};

use kb_3d::{
    CameraPreset, Kb3dCamera, Kb3dSceneInput, KeyboardRenderer3D, SwitchCamera, SwitchCameraPreset,
    SwitchRenderer3D, SwitchSceneInput,
};
use state::{
    ConnectedState, KeymapCategory, LightViewMode, MascotKeySource, NavSection,
    ShortcutActionPreset, SwitchViewMode, offline_diagnostic_report,
};
use views::lighting::{hsla_to_rgb_u8, rgb_u8_to_hsla};
use widgets::SelectOption;

const LIGHT_MODES: [u8; 16] = [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 100];
const LIGHT_DIRECTIONS: [(u8, &str); 4] =
    [(0, "Right ->"), (1, "Left <-"), (2, "Down v"), (3, "Up ^")];
const AUDIO_EFFECT_OPTIONS: [(AudioEffectStyle, &str); 4] = [
    (AudioEffectStyle::Spectrum, "Spectrum Bars"),
    (AudioEffectStyle::Pulse, "Radial Beat Pulse"),
    (AudioEffectStyle::Wave, "Spectral Wave"),
    (AudioEffectStyle::Meter, "VU Level Meter"),
];
const AUDIO_PALETTE_OPTIONS: [(AudioPalette, &str); 5] = [
    (AudioPalette::Aurora, "Aurora Gradient"),
    (AudioPalette::Regal, "Slate Frost"),
    (AudioPalette::Neon, "Neon Cyber"),
    (AudioPalette::Inferno, "Inferno Ember"),
    (AudioPalette::Custom, "Custom FG / BG"),
];
const HOTKEY_MODIFIER_OPTIONS: [(Option<u8>, &str); 6] = [
    (None, "None (Single Key)"),
    (Some(keymap::idx::CAPS), "Caps Lock"),
    (Some(keymap::idx::R_CTRL), "Right Ctrl"),
    (Some(keymap::idx::L_ALT), "Left Alt"),
    (Some(keymap::idx::R_ALT), "Right Alt"),
    (Some(keymap::idx::MENU), "Menu Key"),
];

#[derive(Parser)]
#[command(
    name = "reaula-gui",
    version,
    about = "GPUI Kit + wgpu driver for Aula WIN 60 HE"
)]
struct Args {
    /// Connect to a specific reaulad Unix domain socket
    #[arg(long)]
    socket: Option<PathBuf>,
}

pub(crate) struct ReaulaGui {
    pub(crate) socket: Option<PathBuf>,
    pub(crate) nav: NavSection,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) guide_window: Option<AnyWindowHandle>,
    pub(crate) conn: Result<ConnectedState, String>,
    pub(crate) offline_diag: Option<DiagnosticReport>,
    pub(crate) status_line: String,
    pub(crate) start_instant: Instant,

    pub(crate) renderer_3d: Result<KeyboardRenderer3D, String>,
    pub(crate) preview_3d_img: Option<Arc<RenderImage>>,
    pub(crate) switch_renderer_3d: Result<SwitchRenderer3D, String>,
    pub(crate) preview_switch_3d_img: Option<Arc<RenderImage>>,
    pub(crate) pending_drop_images: Vec<Arc<RenderImage>>,
    pub(crate) dirty_3d: bool,
    pub(crate) last_3d_render: Instant,
    pub(crate) dirty_switch_3d: bool,
    pub(crate) last_switch_3d_render: Instant,
    pub(crate) last_hotkey_poll: Instant,
    pub(crate) last_dialog_tick: Instant,
    pub(crate) dragging_3d: bool,
    pub(crate) last_mouse_3d: Option<(f32, f32)>,
    pub(crate) dragging_switch_3d: bool,
    pub(crate) last_mouse_switch_3d: Option<(f32, f32)>,

    pub(crate) root_focus: FocusHandle,

    // GPUI Kit component states
    pub(crate) slider_actuation: Entity<SliderState>,
    pub(crate) slider_rt_press: Entity<SliderState>,
    pub(crate) slider_rt_release: Entity<SliderState>,
    pub(crate) slider_db_top: Entity<SliderState>,
    pub(crate) slider_db_bot: Entity<SliderState>,

    pub(crate) sel_light_effect: Entity<SelectState<SearchableVec<SelectOption<u8>>>>,
    pub(crate) sel_light_direction: Entity<SelectState<Vec<SelectOption<u8>>>>,
    pub(crate) slider_light_bri: Entity<SliderState>,
    pub(crate) slider_light_spd: Entity<SliderState>,
    pub(crate) picker_light_fg: Entity<ColorPickerState>,
    pub(crate) picker_light_bg: Entity<ColorPickerState>,
    pub(crate) picker_custom_brush: Entity<ColorPickerState>,
    pub(crate) slider_custom_bri: Entity<SliderState>,

    pub(crate) sel_audio_source: Entity<SelectState<SearchableVec<SelectOption<usize>>>>,
    pub(crate) sel_audio_effect: Entity<SelectState<Vec<SelectOption<AudioEffectStyle>>>>,
    pub(crate) sel_audio_palette: Entity<SelectState<Vec<SelectOption<AudioPalette>>>>,
    pub(crate) slider_audio_fps: Entity<SliderState>,
    pub(crate) slider_audio_bri: Entity<SliderState>,
    pub(crate) slider_audio_gain: Entity<SliderState>,
    pub(crate) picker_audio_fg: Entity<ColorPickerState>,
    pub(crate) picker_audio_bg: Entity<ColorPickerState>,

    pub(crate) sel_hk_modifier: Entity<SelectState<Vec<SelectOption<Option<u8>>>>>,
    pub(crate) sel_hk_trigger: Entity<SelectState<SearchableVec<SelectOption<u8>>>>,
    pub(crate) sel_hk_action: Entity<SelectState<Vec<SelectOption<ShortcutActionPreset>>>>,
    pub(crate) sel_hk_vk: Entity<SelectState<SearchableVec<SelectOption<VirtualKey>>>>,
    pub(crate) slider_hk_depth: Entity<SliderState>,
    pub(crate) input_hk_cmd: Entity<InputState>,

    pub(crate) _subscriptions: Vec<Subscription>,
}

impl ReaulaGui {
    fn new(socket: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let conn = ConnectedState::connect(socket.clone());
        let status_line = match &conn {
            Ok(s) => format!(
                "Connected to reaulad (uid={}, pid={})",
                s.server_cred.uid, s.daemon_status.daemon_pid
            ),
            Err(e) => e.clone(),
        };
        let initial_kb_id = conn.as_ref().ok().and_then(|s| s.keyboard_id());
        let renderer_3d = KeyboardRenderer3D::new_for_keyboard(initial_kb_id);
        let switch_renderer_3d = match &renderer_3d {
            Ok(kb_r) => {
                let (dev, q) = kb_r.shared_gpu();
                SwitchRenderer3D::from_shared_device(dev, q)
            }
            Err(e) => Err(e.clone()),
        };

        let init_travel = conn.as_ref().map_or(1.70, |s| s.draft_travel_mm) as f32;
        let init_min = conn.as_ref().map_or(0.08, |s| s.min_mm) as f32;
        let init_max = conn.as_ref().map_or(3.40, |s| s.max_mm) as f32;
        let init_rt_p = conn.as_ref().map_or(0.30, |s| s.draft_rt_press_mm) as f32;
        let init_rt_r = conn.as_ref().map_or(0.30, |s| s.draft_rt_release_mm) as f32;
        let init_db_t = conn.as_ref().map_or(0.05, |s| s.draft_db_top_mm) as f32;
        let init_db_b = conn.as_ref().map_or(0.05, |s| s.draft_db_bottom_mm) as f32;

        let slider_actuation = cx.new(|_| {
            SliderState::new()
                .min(init_min)
                .max(init_max)
                .step(0.01)
                .default_value(init_travel)
        });
        let slider_rt_press = cx.new(|_| {
            SliderState::new()
                .min(0.01)
                .max(2.50)
                .step(0.01)
                .default_value(init_rt_p)
        });
        let slider_rt_release = cx.new(|_| {
            SliderState::new()
                .min(0.01)
                .max(2.50)
                .step(0.01)
                .default_value(init_rt_r)
        });
        let slider_db_top = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(1.0)
                .step(0.01)
                .default_value(init_db_t)
        });
        let slider_db_bot = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(1.0)
                .step(0.01)
                .default_value(init_db_b)
        });

        let effect_items = SearchableVec::new(
            LIGHT_MODES
                .iter()
                .map(|&m| SelectOption::new(m, protocol::light_mode_name(m)))
                .collect::<Vec<_>>(),
        );
        let init_eff_ix = conn
            .as_ref()
            .ok()
            .and_then(|s| LIGHT_MODES.iter().position(|&m| m == s.light_state.mode))
            .map(|r| gpui_kit::component::IndexPath::default().row(r));
        let sel_light_effect =
            cx.new(|cx| SelectState::new(effect_items, init_eff_ix, window, cx).searchable(true));

        let dir_items = LIGHT_DIRECTIONS
            .iter()
            .map(|&(d, l)| SelectOption::new(d, l))
            .collect::<Vec<_>>();
        let init_dir_ix = conn
            .as_ref()
            .ok()
            .and_then(|s| {
                LIGHT_DIRECTIONS
                    .iter()
                    .position(|(d, _)| *d == s.light_state.direction)
            })
            .map(|r| gpui_kit::component::IndexPath::default().row(r));
        let sel_light_direction = cx.new(|cx| SelectState::new(dir_items, init_dir_ix, window, cx));

        let init_bri = conn
            .as_ref()
            .map_or(3.0, |s| f32::from(s.light_state.brightness));
        let init_spd = conn
            .as_ref()
            .map_or(2.0, |s| f32::from(s.light_state.speed));
        let slider_light_bri = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(f32::from(LIGHT_MAX_LEVEL))
                .step(1.0)
                .default_value(init_bri)
        });
        let slider_light_spd = cx.new(|_| {
            SliderState::new()
                .min(0.0)
                .max(f32::from(LIGHT_MAX_LEVEL))
                .step(1.0)
                .default_value(init_spd)
        });

        let init_fg = conn.as_ref().map_or([0, 215, 255], |s| s.light_state.fg);
        let init_bg = conn.as_ref().map_or([0, 0, 0], |s| s.light_state.bg);
        let init_brush = conn.as_ref().map_or([0, 216, 255], |s| s.custom_brush);
        let init_cbri = conn
            .as_ref()
            .map_or(3.0, |s| f32::from(s.custom_brightness));

        let picker_light_fg =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb_u8_to_hsla(init_fg)));
        let picker_light_bg =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb_u8_to_hsla(init_bg)));
        let picker_custom_brush = cx
            .new(|cx| ColorPickerState::new(window, cx).default_value(rgb_u8_to_hsla(init_brush)));
        let slider_custom_bri = cx.new(|_| {
            SliderState::new()
                .min(1.0)
                .max(f32::from(LIGHT_MAX_LEVEL))
                .step(1.0)
                .default_value(init_cbri)
        });

        let audio_src_items = build_audio_source_vec(conn.as_ref().ok());
        let init_asrc_ix = conn
            .as_ref()
            .ok()
            .map(|s| gpui_kit::component::IndexPath::default().row(s.audio_source_idx));
        let sel_audio_source = cx
            .new(|cx| SelectState::new(audio_src_items, init_asrc_ix, window, cx).searchable(true));

        let aeff_items = AUDIO_EFFECT_OPTIONS
            .iter()
            .map(|&(style, lbl)| SelectOption::new(style, lbl))
            .collect::<Vec<_>>();
        let init_aeff_ix = conn
            .as_ref()
            .ok()
            .and_then(|s| {
                AUDIO_EFFECT_OPTIONS
                    .iter()
                    .position(|(x, _)| *x == s.audio_config.style)
            })
            .map(|r| gpui_kit::component::IndexPath::default().row(r));
        let sel_audio_effect = cx.new(|cx| SelectState::new(aeff_items, init_aeff_ix, window, cx));

        let apal_items = AUDIO_PALETTE_OPTIONS
            .iter()
            .map(|&(p, lbl)| SelectOption::new(p, lbl))
            .collect::<Vec<_>>();
        let init_apal_ix = conn
            .as_ref()
            .ok()
            .and_then(|s| {
                AUDIO_PALETTE_OPTIONS
                    .iter()
                    .position(|(p, _)| *p == s.audio_config.palette)
            })
            .map(|r| gpui_kit::component::IndexPath::default().row(r));
        let sel_audio_palette = cx.new(|cx| SelectState::new(apal_items, init_apal_ix, window, cx));

        let init_afps = conn
            .as_ref()
            .map_or(30.0, |s| f32::from(s.audio_config.fps));
        let init_abri = conn
            .as_ref()
            .map_or(3.0, |s| f32::from(s.audio_config.brightness));
        let init_again = conn.as_ref().map_or(1.0, |s| s.audio_config.gain);
        let init_afg = conn.as_ref().map_or([0, 215, 255], |s| s.audio_config.fg);
        let init_abg = conn.as_ref().map_or([0, 0, 0], |s| s.audio_config.bg);

        let slider_audio_fps = cx.new(|_| {
            SliderState::new()
                .min(20.0)
                .max(60.0)
                .step(10.0)
                .default_value(init_afps)
        });
        let slider_audio_bri = cx.new(|_| {
            SliderState::new()
                .min(1.0)
                .max(f32::from(LIGHT_MAX_LEVEL))
                .step(1.0)
                .default_value(init_abri)
        });
        let slider_audio_gain = cx.new(|_| {
            SliderState::new()
                .min(0.4)
                .max(3.0)
                .step(0.2)
                .default_value(init_again)
        });
        let picker_audio_fg =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb_u8_to_hsla(init_afg)));
        let picker_audio_bg =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(rgb_u8_to_hsla(init_abg)));

        let mod_items = HOTKEY_MODIFIER_OPTIONS
            .iter()
            .map(|&(m, lbl)| SelectOption::new(m, lbl))
            .collect::<Vec<_>>();
        let init_mod_ix = conn
            .as_ref()
            .ok()
            .and_then(|s| {
                HOTKEY_MODIFIER_OPTIONS
                    .iter()
                    .position(|(m, _)| *m == s.draft_rule_modifier)
            })
            .map(|r| gpui_kit::component::IndexPath::default().row(r));
        let sel_hk_modifier = cx.new(|cx| SelectState::new(mod_items, init_mod_ix, window, cx));

        let trig_vec: Vec<_> = keymap::WIN60HE_KEYS
            .iter()
            .filter(|k| k.index != keymap::idx::FN)
            .map(|k| SelectOption::new(k.index, format!("{} (#{})", k.name, k.index)))
            .collect();
        let init_trig_key = conn.as_ref().map_or(keymap::idx::T, |s| s.draft_rule_key);
        let init_trig_ix = trig_vec
            .iter()
            .position(|o| o.value == init_trig_key)
            .map(|r| gpui_kit::component::IndexPath::default().row(r));
        let sel_hk_trigger = cx.new(|cx| {
            SelectState::new(SearchableVec::new(trig_vec), init_trig_ix, window, cx)
                .searchable(true)
        });

        let act_items = ShortcutActionPreset::ALL
            .iter()
            .map(|&p| SelectOption::new(p, p.label()))
            .collect::<Vec<_>>();
        let init_act_ix = conn
            .as_ref()
            .ok()
            .and_then(|s| {
                ShortcutActionPreset::ALL
                    .iter()
                    .position(|&p| p == s.draft_rule_preset)
            })
            .map(|r| gpui_kit::component::IndexPath::default().row(r));
        let sel_hk_action = cx.new(|cx| SelectState::new(act_items, init_act_ix, window, cx));

        let init_cat = conn
            .as_ref()
            .map_or(KeymapCategory::FirmwareFnFlash, |s| s.draft_category);
        let vk_vec = build_vk_vec(init_cat);
        let init_vk = conn
            .as_ref()
            .map_or(VirtualKey::Up, |s| s.draft_virtual_key);
        let init_vk_ix = vk_vec
            .iter()
            .position(|o| o.value == init_vk)
            .map(|r| gpui_kit::component::IndexPath::default().row(r));
        let sel_hk_vk = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vk_vec), init_vk_ix, window, cx).searchable(true)
        });

        let init_hk_depth = conn.as_ref().map_or(1.5, |s| s.draft_rule_depth_mm as f32);
        let slider_hk_depth = cx.new(|_| {
            SliderState::new()
                .min(0.40)
                .max(3.20)
                .step(0.10)
                .default_value(init_hk_depth)
        });

        let init_cmd = conn
            .as_ref()
            .map_or_else(|_| String::new(), |s| s.draft_rule_command.clone());
        let input_hk_cmd = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Shell command, e.g. notify-send 'Hotkey triggered'")
                .default_value(init_cmd)
        });

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &slider_actuation,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let val = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => f64::from(v.start()),
                };
                if let Ok(st) = &mut this.conn {
                    st.draft_travel_mm = val.clamp(st.min_mm, st.max_mm);
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_rt_press,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let val = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => f64::from(v.start()),
                };
                if let Ok(st) = &mut this.conn {
                    st.draft_rt_press_mm = val.clamp(0.01, 2.50);
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_rt_release,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let val = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => f64::from(v.start()),
                };
                if let Ok(st) = &mut this.conn {
                    st.draft_rt_release_mm = val.clamp(0.01, 2.50);
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_db_top,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let val = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => f64::from(v.start()),
                };
                if let Ok(st) = &mut this.conn {
                    st.draft_db_top_mm = val.clamp(0.0, 1.0);
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_db_bot,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let val = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => f64::from(v.start()),
                };
                if let Ok(st) = &mut this.conn {
                    st.draft_db_bottom_mm = val.clamp(0.0, 1.0);
                }
                cx.notify();
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &sel_light_effect,
            window,
            |this, _, ev: &SelectEvent<SearchableVec<SelectOption<u8>>>, _, cx| {
                if let SelectEvent::Confirm(Some(mode)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.light_state.mode = *mode;
                    st.light_state.power_on = true;
                    this.dirty_3d = true;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &sel_light_direction,
            window,
            |this, _, ev: &SelectEvent<Vec<SelectOption<u8>>>, _, cx| {
                if let SelectEvent::Confirm(Some(dir)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.light_state.direction = *dir;
                    this.dirty_3d = true;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_light_bri,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let v = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => v.start(),
                };
                if let Ok(st) = &mut this.conn {
                    st.light_state.brightness = (v.round() as u8).clamp(0, LIGHT_MAX_LEVEL);
                    this.dirty_3d = true;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_light_spd,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let v = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => v.start(),
                };
                if let Ok(st) = &mut this.conn {
                    st.light_state.speed = (v.round() as u8).clamp(0, LIGHT_MAX_LEVEL);
                    this.dirty_3d = true;
                }
                cx.notify();
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &picker_light_fg,
            window,
            |this, _, ev: &ColorPickerEvent, _, cx| {
                if let ColorPickerEvent::Change(Some(col)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.light_state.fg = hsla_to_rgb_u8(*col);
                    st.light_state.full_color = false;
                    this.dirty_3d = true;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &picker_light_bg,
            window,
            |this, _, ev: &ColorPickerEvent, _, cx| {
                if let ColorPickerEvent::Change(Some(col)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.light_state.bg = hsla_to_rgb_u8(*col);
                    this.dirty_3d = true;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &picker_custom_brush,
            window,
            |this, _, ev: &ColorPickerEvent, _, cx| {
                if let ColorPickerEvent::Change(Some(col)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.custom_brush = hsla_to_rgb_u8(*col);
                    st.custom_paint_mode = true;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_custom_bri,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let v = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => v.start(),
                };
                if let Ok(st) = &mut this.conn {
                    st.custom_brightness = (v.round() as u8).clamp(1, LIGHT_MAX_LEVEL);
                }
                cx.notify();
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &sel_audio_source,
            window,
            |this, _, ev: &SelectEvent<SearchableVec<SelectOption<usize>>>, _, cx| {
                if let SelectEvent::Confirm(Some(idx)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.audio_source_idx = *idx;
                    st.push_audio_live_config();
                    this.dirty_3d = true;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &sel_audio_effect,
            window,
            |this, _, ev: &SelectEvent<Vec<SelectOption<AudioEffectStyle>>>, _, cx| {
                if let SelectEvent::Confirm(Some(style)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.audio_config.style = *style;
                    st.push_audio_live_config();
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &sel_audio_palette,
            window,
            |this, _, ev: &SelectEvent<Vec<SelectOption<AudioPalette>>>, _, cx| {
                if let SelectEvent::Confirm(Some(pal)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.audio_config.palette = *pal;
                    st.push_audio_live_config();
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_audio_fps,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let v = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => v.start(),
                };
                if let Ok(st) = &mut this.conn {
                    st.audio_config.fps = (v.round() as u8).clamp(20, 60);
                    st.push_audio_live_config();
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_audio_bri,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let v = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => v.start(),
                };
                if let Ok(st) = &mut this.conn {
                    st.audio_config.brightness = (v.round() as u8).clamp(1, LIGHT_MAX_LEVEL);
                    st.push_audio_live_config();
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_audio_gain,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let v = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => v.start(),
                };
                if let Ok(st) = &mut this.conn {
                    st.audio_config.gain = v.clamp(0.4, 3.0);
                    st.push_audio_live_config();
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &picker_audio_fg,
            window,
            |this, _, ev: &ColorPickerEvent, _, cx| {
                if let ColorPickerEvent::Change(Some(col)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.audio_config.fg = hsla_to_rgb_u8(*col);
                    st.push_audio_live_config();
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &picker_audio_bg,
            window,
            |this, _, ev: &ColorPickerEvent, _, cx| {
                if let ColorPickerEvent::Change(Some(col)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.audio_config.bg = hsla_to_rgb_u8(*col);
                    st.push_audio_live_config();
                }
                cx.notify();
            },
        ));

        subscriptions.push(cx.subscribe_in(
            &sel_hk_modifier,
            window,
            |this, _, ev: &SelectEvent<Vec<SelectOption<Option<u8>>>>, _, cx| {
                if let SelectEvent::Confirm(Some(m)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.draft_rule_modifier = *m;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &sel_hk_trigger,
            window,
            |this, _, ev: &SelectEvent<SearchableVec<SelectOption<u8>>>, _, cx| {
                if let SelectEvent::Confirm(Some(k)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.draft_rule_key = *k;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &sel_hk_action,
            window,
            |this, _, ev: &SelectEvent<Vec<SelectOption<ShortcutActionPreset>>>, _, cx| {
                if let SelectEvent::Confirm(Some(p)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.draft_rule_preset = *p;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &sel_hk_vk,
            window,
            |this, _, ev: &SelectEvent<SearchableVec<SelectOption<VirtualKey>>>, _, cx| {
                if let SelectEvent::Confirm(Some(vk)) = ev
                    && let Ok(st) = &mut this.conn
                {
                    st.draft_virtual_key = *vk;
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &slider_hk_depth,
            window,
            |this, _, ev: &SliderEvent, _, cx| {
                let v = match ev {
                    SliderEvent::Change(v) | SliderEvent::Release(v) => f64::from(v.start()),
                };
                if let Ok(st) = &mut this.conn {
                    st.draft_rule_depth_mm = v.clamp(0.40, 3.20);
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &input_hk_cmd,
            window,
            |this, input, ev: &InputEvent, _, cx| {
                if let InputEvent::Change = ev {
                    let text = input.read(cx).value().to_string();
                    if let Ok(st) = &mut this.conn {
                        st.draft_rule_command = sanitize_command_paste(&text);
                    }
                    cx.notify();
                }
            },
        ));

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let res = this.update(cx, |view, cx| {
                    let mut changed = false;
                    let mut dialog_tick = false;
                    let auto_focus = view.nav == NavSection::Diagnostics
                        && view.conn.as_ref().is_ok_and(|s| s.diag_follow_press);
                    if let Ok(state) = &mut view.conn {
                        if state.drain_travel(auto_focus) {
                            view.dirty_3d = true;
                            view.dirty_switch_3d = true;
                            changed = true;
                        }
                        if state.drain_calibration() {
                            changed = true;
                        }
                        let (audio_changed, audio_err) = state.drain_audio();
                        if let Some(err) = audio_err {
                            view.status_line = format!("Audio Sync stopped: {err}");
                            view.dirty_3d = true;
                            changed = true;
                        } else if audio_changed {
                            view.dirty_3d = true;
                            changed = true;
                        }
                        if state.audio_disclaimer_modal_open
                            && !state.audio_disclaimer_accepted
                            && view.last_dialog_tick.elapsed() >= Duration::from_millis(200)
                        {
                            view.last_dialog_tick = Instant::now();
                            dialog_tick = true;
                            changed = true;
                        }
                        if (view.nav == NavSection::Hotkeys || view.nav == NavSection::ColorLight)
                            && state.shortcut_watcher_active
                            && view.last_hotkey_poll.elapsed() >= Duration::from_millis(220)
                        {
                            view.last_hotkey_poll = Instant::now();
                            if state.poll_shortcut_logs_quiet() {
                                view.dirty_3d = true;
                                changed = true;
                            }
                        }
                    }
                    if view.nav == NavSection::ColorLight && view.tick_color_light_stage() {
                        changed = true;
                    }
                    if view.nav == NavSection::Diagnostics && view.tick_diagnostics_stage() {
                        changed = true;
                    }
                    if dialog_tick {
                        cx.refresh_windows();
                    } else if changed {
                        cx.notify();
                    }
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            socket,
            nav: NavSection::Main,
            sidebar_collapsed: false,
            guide_window: None,
            conn,
            offline_diag: None,
            status_line,
            start_instant: Instant::now(),
            renderer_3d,
            preview_3d_img: None,
            switch_renderer_3d,
            preview_switch_3d_img: None,
            pending_drop_images: Vec::new(),
            dirty_3d: true,
            last_3d_render: Instant::now() - Duration::from_secs(1),
            dirty_switch_3d: true,
            last_switch_3d_render: Instant::now() - Duration::from_secs(1),
            last_hotkey_poll: Instant::now(),
            last_dialog_tick: Instant::now(),
            dragging_3d: false,
            last_mouse_3d: None,
            dragging_switch_3d: false,
            last_mouse_switch_3d: None,
            root_focus: cx.focus_handle(),
            slider_actuation,
            slider_rt_press,
            slider_rt_release,
            slider_db_top,
            slider_db_bot,
            sel_light_effect,
            sel_light_direction,
            slider_light_bri,
            slider_light_spd,
            picker_light_fg,
            picker_light_bg,
            picker_custom_brush,
            slider_custom_bri,
            sel_audio_source,
            sel_audio_effect,
            sel_audio_palette,
            slider_audio_fps,
            slider_audio_bri,
            slider_audio_gain,
            picker_audio_fg,
            picker_audio_bg,
            sel_hk_modifier,
            sel_hk_trigger,
            sel_hk_action,
            sel_hk_vk,
            slider_hk_depth,
            input_hk_cmd,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn sync_main_controls_from_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(st) = &self.conn else {
            return;
        };
        let t = st.draft_travel_mm as f32;
        let p = st.draft_rt_press_mm as f32;
        let r = st.draft_rt_release_mm as f32;
        let dbt = st.draft_db_top_mm as f32;
        let dbb = st.draft_db_bottom_mm as f32;
        self.slider_actuation
            .update(cx, |s, cx| s.set_value(t, window, cx));
        self.slider_rt_press
            .update(cx, |s, cx| s.set_value(p, window, cx));
        self.slider_rt_release
            .update(cx, |s, cx| s.set_value(r, window, cx));
        self.slider_db_top
            .update(cx, |s, cx| s.set_value(dbt, window, cx));
        self.slider_db_bot
            .update(cx, |s, cx| s.set_value(dbb, window, cx));
    }

    pub(crate) fn sync_color_pickers_from_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(st) = &self.conn else {
            return;
        };
        let ls = st.light_state;
        let brush = st.custom_brush;
        let ac = &st.audio_config;
        let fg = ls.fg;
        let bg = ls.bg;
        let afg = ac.fg;
        let abg = ac.bg;
        let mode = ls.mode;
        let dir = ls.direction;
        let bri = f32::from(ls.brightness);
        let spd = f32::from(ls.speed);
        let cbri = f32::from(st.custom_brightness);

        self.sel_light_effect
            .update(cx, |s, cx| s.set_selected_value(&mode, window, cx));
        self.sel_light_direction
            .update(cx, |s, cx| s.set_selected_value(&dir, window, cx));
        self.slider_light_bri
            .update(cx, |s, cx| s.set_value(bri, window, cx));
        self.slider_light_spd
            .update(cx, |s, cx| s.set_value(spd, window, cx));
        self.picker_light_fg
            .update(cx, |p, cx| p.set_value(rgb_u8_to_hsla(fg), window, cx));
        self.picker_light_bg
            .update(cx, |p, cx| p.set_value(rgb_u8_to_hsla(bg), window, cx));
        self.picker_custom_brush
            .update(cx, |p, cx| p.set_value(rgb_u8_to_hsla(brush), window, cx));
        self.slider_custom_bri
            .update(cx, |s, cx| s.set_value(cbri, window, cx));
        self.picker_audio_fg
            .update(cx, |p, cx| p.set_value(rgb_u8_to_hsla(afg), window, cx));
        self.picker_audio_bg
            .update(cx, |p, cx| p.set_value(rgb_u8_to_hsla(abg), window, cx));
    }

    pub(crate) fn sync_audio_source_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let items = build_audio_source_vec(self.conn.as_ref().ok());
        let idx = self.conn.as_ref().map_or(0, |s| s.audio_source_idx);
        self.sel_audio_source.update(cx, |s, cx| {
            s.set_items(items, window, cx);
            s.set_selected_value(&idx, window, cx);
        });
    }

    pub(crate) fn sync_keymap_controls_from_state(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(st) = &self.conn else {
            return;
        };
        let cat = st.draft_category;
        let mod_k = st.draft_rule_modifier;
        let trig_k = st.draft_rule_key;
        let preset = st.draft_rule_preset;
        let vk = st.draft_virtual_key;
        let depth = st.draft_rule_depth_mm as f32;
        let cmd = st.draft_rule_command.clone();

        self.sel_hk_modifier
            .update(cx, |s, cx| s.set_selected_value(&mod_k, window, cx));
        self.sel_hk_trigger
            .update(cx, |s, cx| s.set_selected_value(&trig_k, window, cx));
        self.sel_hk_action
            .update(cx, |s, cx| s.set_selected_value(&preset, window, cx));
        let vk_vec = SearchableVec::new(build_vk_vec(cat));
        self.sel_hk_vk.update(cx, |s, cx| {
            s.set_items(vk_vec, window, cx);
            s.set_selected_value(&vk, window, cx);
        });
        self.slider_hk_depth
            .update(cx, |s, cx| s.set_value(depth, window, cx));
        self.input_hk_cmd
            .update(cx, |inp, cx| inp.set_value(cmd, window, cx));
    }

    pub(crate) fn tick_color_light_stage(&mut self) -> bool {
        let Ok(state) = &self.conn else {
            return false;
        };
        match state.light_view_mode {
            LightViewMode::ThreeD => self.maybe_render_3d(),
            LightViewMode::TwoD => {
                let (aqua_keys, _) = state.active_daemon_indicators();
                if state.effective_preview_custom_frame().is_some()
                    && !state.audio_active
                    && !state.typing_only_effects
                    && aqua_keys.is_empty()
                {
                    return false;
                }
                let active_anim = state.animate_3d
                    || !state.active_impulses().is_empty()
                    || !aqua_keys.is_empty();
                if active_anim && self.last_3d_render.elapsed() >= Duration::from_millis(33) {
                    self.last_3d_render = Instant::now();
                    true
                } else {
                    false
                }
            }
        }
    }

    pub(crate) fn maybe_render_3d(&mut self) -> bool {
        let Ok(renderer) = &mut self.renderer_3d else {
            return false;
        };
        let Ok(state) = &mut self.conn else {
            return false;
        };
        let model_switched = renderer.set_model(state.keyboard_id());
        let mascot_toggled = state.mascots.consume_dirty();
        let impulses = state.active_impulses();
        let (aqua_keys, aqua_pulsing) = state.active_daemon_indicators();
        let active_anim = state.animate_3d
            || !impulses.is_empty()
            || state.mascots.any_visible()
            || !aqua_keys.is_empty();
        let anim_due = active_anim && self.last_3d_render.elapsed() >= Duration::from_millis(33);
        let init_needed = self.preview_3d_img.is_none();
        if !(self.dirty_3d || model_switched || mascot_toggled || anim_due || init_needed) {
            return false;
        }

        let now = Instant::now();
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
            time_s: self.start_instant.elapsed().as_secs_f32(),
            miku: state.mascots.miku_pose(now),
            teto: state.mascots.teto_pose(now),
            daemon_aqua_keys: &aqua_keys,
            daemon_aqua_pulsing: &aqua_pulsing,
        };

        if let Ok(next_img) = renderer.render_frame(&state.camera_3d, &scene) {
            if let Some(old) = self.preview_3d_img.replace(next_img) {
                self.pending_drop_images.push(old);
            }
            self.dirty_3d = false;
            self.last_3d_render = now;
            return true;
        }
        false
    }

    pub(crate) fn tick_diagnostics_stage(&mut self) -> bool {
        let Ok(state) = &self.conn else {
            return false;
        };
        let use_3d =
            state.diag_switch_view == SwitchViewMode::ThreeD && self.switch_renderer_3d.is_ok();
        if use_3d {
            self.maybe_render_switch_3d()
        } else if (state.diag_sweep_active || !state.live_depths.is_empty())
            && self.last_switch_3d_render.elapsed() >= Duration::from_millis(33)
        {
            self.last_switch_3d_render = Instant::now();
            true
        } else {
            false
        }
    }

    pub(crate) fn maybe_render_switch_3d(&mut self) -> bool {
        let Ok(renderer) = &self.switch_renderer_3d else {
            return false;
        };
        let Ok(state) = &self.conn else {
            return false;
        };
        let anim_due = state.diag_sweep_active
            && self.last_switch_3d_render.elapsed() >= Duration::from_millis(33);
        let init_needed = self.preview_switch_3d_img.is_none();
        if !(self.dirty_switch_3d || anim_due || init_needed) {
            return false;
        }

        let snap = state.focused_switch_inspection();
        let led_rgb = state
            .effective_preview_custom_frame()
            .and_then(|arr| arr.get(usize::from(snap.key_index)).copied())
            .unwrap_or(state.light_state.fg);
        let now = Instant::now();
        let scene = SwitchSceneInput {
            switch_id: snap.switch_id,
            travel_mm: snap.travel_mm as f32,
            max_mm: snap.max_mm as f32,
            actuation_mm: snap.actuation_mm as f32,
            triggered: snap.triggered,
            rapid_trigger: snap.rapid_trigger,
            led_rgb,
            time_s: self.start_instant.elapsed().as_secs_f32(),
        };

        if let Ok(next_img) = renderer.render_frame(&state.diag_switch_cam, &scene) {
            if let Some(old) = self.preview_switch_3d_img.replace(next_img) {
                self.pending_drop_images.push(old);
            }
            self.dirty_switch_3d = false;
            self.last_switch_3d_render = now;
            return true;
        }
        false
    }

    pub(crate) fn set_switch_camera_preset(
        &mut self,
        preset: SwitchCameraPreset,
        cx: &mut Context<Self>,
    ) {
        if let Ok(state) = &mut self.conn {
            state.diag_switch_cam = SwitchCamera::from_preset(preset);
            state.diag_switch_preset = Some(preset);
            self.dirty_switch_3d = true;
        }
        cx.notify();
    }

    pub(crate) fn switch_section(&mut self, next: NavSection, cx: &mut Context<Self>) {
        if self.nav == NavSection::Calibration
            && next != NavSection::Calibration
            && let Ok(state) = &mut self.conn
            && state.cal_active
        {
            state.stop_calibration();
            let _ = state.start_travel();
        }
        self.nav = next;
        if next == NavSection::ColorLight
            && self
                .conn
                .as_ref()
                .is_ok_and(|s| s.light_view_mode == LightViewMode::ThreeD)
        {
            self.dirty_3d = true;
            self.maybe_render_3d();
        } else if next == NavSection::Diagnostics
            && self
                .conn
                .as_ref()
                .is_ok_and(|s| s.diag_switch_view == SwitchViewMode::ThreeD)
        {
            self.dirty_switch_3d = true;
            self.maybe_render_switch_3d();
        }
        cx.notify();
    }

    pub(crate) fn set_camera_preset(&mut self, preset: CameraPreset, cx: &mut Context<Self>) {
        if let Ok(state) = &mut self.conn {
            state.camera_3d = match &self.renderer_3d {
                Ok(renderer) => Kb3dCamera::from_preset_for(renderer.model(), preset),
                Err(_) => Kb3dCamera::from_preset(preset),
            };
            state.camera_preset = Some(preset);
            self.dirty_3d = true;
        }
        cx.notify();
    }

    fn reconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.conn = ConnectedState::connect(self.socket.clone());
        self.offline_diag = None;
        self.dirty_3d = true;
        self.status_line = match &self.conn {
            Ok(s) => format!(
                "Reconnected to reaulad (uid={}, pid={})",
                s.server_cred.uid, s.daemon_status.daemon_pid
            ),
            Err(e) => e.clone(),
        };
        self.sync_main_controls_from_state(window, cx);
        self.sync_color_pickers_from_state(window, cx);
        self.sync_audio_source_select(window, cx);
        self.sync_keymap_controls_from_state(window, cx);
        cx.notify();
    }

    fn toggle_travel(&mut self, cx: &mut Context<Self>) {
        if let Ok(state) = &mut self.conn {
            if state.travel_enabled {
                state.stop_travel();
                self.status_line = "Live travel paused.".into();
            } else {
                match state.start_travel() {
                    Ok(()) => self.status_line = "Live travel active.".into(),
                    Err(e) => self.status_line = e,
                }
            }
            self.dirty_3d = true;
        }
        cx.notify();
    }

    fn open_guide(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = guide_window::open_or_focus(&mut self.guide_window, cx) {
            self.status_line = e;
            cx.notify();
        }
    }

    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let next = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        theme::apply_clinical_theme(next, cx);
        cx.notify();
    }

    pub(crate) fn apply_actuation(&mut self, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        let indices = state.target_indices();
        let mode = state.draft_mode;
        let travel_mm = state.draft_travel_mm;
        let p = (mode != TriggerMode::Fixed).then_some(state.draft_rt_press_mm);
        let r = (mode == TriggerMode::RapidTriggerSplit).then_some(state.draft_rt_release_mm);
        let unit = state.unit_mm;

        match state.run_measured("Set Actuation", |c| {
            c.set_actuation(indices.clone(), mode, travel_mm, p, r)
        }) {
            Ok(()) => {
                for &idx in &indices {
                    let prev = state.actuations.get(&idx).copied();
                    state.actuations.insert(
                        idx,
                        TriggerConfig {
                            index: idx,
                            row: idx / 22,
                            col: idx % 22,
                            mode,
                            travel_raw: mm_to_raw(travel_mm, unit),
                            interval1_raw: p.map_or_else(
                                || prev.map_or(0, |c| c.interval1_raw),
                                |v| mm_to_raw(v, unit),
                            ),
                            interval2_raw: r.map_or_else(
                                || {
                                    p.map_or_else(
                                        || prev.map_or(0, |c| c.interval2_raw),
                                        |v| mm_to_raw(v, unit),
                                    )
                                },
                                |v| mm_to_raw(v, unit),
                            ),
                        },
                    );
                }
                self.status_line = format!(
                    "Applied {:?} ({:.2} mm) to {} key(s).",
                    mode,
                    travel_mm,
                    indices.len()
                );
            }
            Err(e) => self.status_line = format!("Actuation failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn apply_switch_profile(&mut self, switch_id: u8, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        let indices = state.target_indices();
        match state.run_measured("Set Switch Profile", |c| {
            c.set_switches(indices.clone(), switch_id)
        }) {
            Ok(()) => {
                state.draft_switch_id = switch_id;
                for &idx in &indices {
                    state.switches.insert(idx, switch_id);
                }
                self.status_line = format!(
                    "Switch {} -> {} key(s).",
                    protocol::switch_profile_name(switch_id),
                    indices.len()
                );
            }
            Err(e) => self.status_line = format!("Switch update failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn apply_deadband(&mut self, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        let indices = state.target_indices();
        let top_raw = (state.draft_db_top_mm * 100.0).round().clamp(0.0, 255.0) as u8;
        let bot_raw = (state.draft_db_bottom_mm * 100.0).round().clamp(0.0, 255.0) as u8;

        match state.run_measured("Set Deadband", |c| {
            c.set_deadband(indices.clone(), top_raw, bot_raw)
        }) {
            Ok(()) => {
                for &idx in &indices {
                    state.deadbands.insert(idx, (top_raw, bot_raw));
                }
                self.status_line = format!(
                    "Deadband {:.2}/{:.2} mm -> {} key(s).",
                    f64::from(top_raw) * 0.01,
                    f64::from(bot_raw) * 0.01,
                    indices.len()
                );
            }
            Err(e) => self.status_line = format!("Deadband failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn apply_poll_rate(&mut self, khz: u8, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        if PollRate::from_khz(khz).is_err() {
            return;
        }
        match state.run_measured("Set Poll Rate", |c| c.set_poll_rate(khz)) {
            Ok(()) => {
                state.poll_rate_khz = khz;
                self.status_line = format!("Polling rate {khz} kHz (board restarts).");
            }
            Err(e) => self.status_line = format!("Poll rate failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn apply_socd(&mut self, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        let enabled = state.socd_enabled;
        let pairs = state.socd_pairs.clone();
        match state.run_measured("Set SOCD", |c| c.set_socd(enabled, pairs.clone())) {
            Ok(()) => {
                self.status_line = format!(
                    "SOCD {} ({} pair(s)).",
                    if state.socd_enabled { "on" } else { "off" },
                    state.socd_pairs.len()
                );
            }
            Err(e) => self.status_line = format!("SOCD failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn apply_lighting(&mut self, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        if state.audio_active {
            state.stop_audio_sync(false);
        }
        let ls = state.light_state;
        match state.run_measured("Set Lighting", |c| c.set_light(ls)) {
            Ok(()) => {
                self.status_line = format!(
                    "Applied RGB: {} (bri {}, spd {}).",
                    protocol::light_mode_name(ls.mode),
                    ls.brightness,
                    ls.speed
                );
                self.dirty_3d = true;
            }
            Err(e) => self.status_line = format!("Lighting failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn reload_lighting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        match state.run_measured("Read Lighting", |c| c.get_light()) {
            Ok(ls) => {
                state.light_state = ls;
                self.dirty_3d = true;
                self.status_line = "Reloaded lighting from keyboard.".into();
                self.sync_color_pickers_from_state(window, cx);
            }
            Err(e) => self.status_line = format!("Lighting read failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn upload_custom_rgb(&mut self, slot: u8, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        if state.audio_active {
            state.stop_audio_sync(false);
        }
        let colors = state.custom_slots[usize::from(slot)].to_vec();
        let bri = state.custom_brightness;
        match state.run_measured("Upload Custom RGB", |c| {
            c.set_custom_rgb(slot, colors.clone(), bri)
        }) {
            Ok(()) => {
                state.active_custom_slot = slot;
                self.dirty_3d = true;
                self.status_line = format!("Uploaded Custom Slot {}.", slot + 1);
            }
            Err(e) => self.status_line = format!("Custom RGB failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn reload_custom_rgb(&mut self, slot: u8, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        match state.run_measured("Read Custom RGB", |c| c.get_custom_rgb(slot)) {
            Ok(colors) => {
                let mut arr = [[0u8; 3]; protocol::MATRIX_SLOTS];
                for (dst, src) in arr.iter_mut().zip(colors) {
                    *dst = src;
                }
                state.custom_slots[usize::from(slot)] = arr;
                self.dirty_3d = true;
                self.status_line = format!("Reloaded Custom Slot {}.", slot + 1);
            }
            Err(e) => self.status_line = format!("Custom RGB read failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn start_cal(&mut self, any_key: bool, cx: &mut Context<Self>) {
        if let Ok(state) = &mut self.conn {
            if state.audio_active {
                state.stop_audio_sync(true);
            }
            match state.start_calibration(any_key) {
                Ok(()) => self.status_line = state.cal_status_text.clone(),
                Err(e) => self.status_line = format!("Calibration failed: {e}"),
            }
        }
        cx.notify();
    }

    pub(crate) fn stop_cal(&mut self, cx: &mut Context<Self>) {
        if let Ok(state) = &mut self.conn {
            state.stop_calibration();
            let _ = state.start_travel();
            self.status_line = state.cal_status_text.clone();
        }
        cx.notify();
    }

    pub(crate) fn request_start_audio_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let accepted = self
            .conn
            .as_ref()
            .is_ok_and(|s| s.audio_disclaimer_accepted);
        if !accepted {
            self.status_line = "Review warning before starting Audio Sync.".into();
            self.open_audio_disclaimer_dialog(true, window, cx);
            return;
        }
        if let Ok(state) = &mut self.conn {
            match state.start_audio_sync() {
                Ok(()) => {
                    self.status_line = state.audio_status_text.clone();
                    self.dirty_3d = true;
                }
                Err(e) => self.status_line = e,
            }
        }
        cx.notify();
    }

    pub(crate) fn stop_audio_sync(&mut self, cx: &mut Context<Self>) {
        if let Ok(state) = &mut self.conn {
            state.stop_audio_sync(true);
            self.status_line = state.audio_status_text.clone();
            self.dirty_3d = true;
        }
        cx.notify();
    }

    pub(crate) fn confirm_audio_disclaimer(&mut self, cx: &mut Context<Self>) -> bool {
        let Ok(state) = &mut self.conn else {
            return false;
        };
        match state.confirm_audio_disclaimer() {
            Ok(should_start) => {
                if should_start {
                    match state.start_audio_sync() {
                        Ok(()) => {
                            self.status_line = state.audio_status_text.clone();
                            self.dirty_3d = true;
                        }
                        Err(e) => self.status_line = e,
                    }
                } else {
                    self.status_line = "Audio Sync warning acknowledged.".into();
                }
                cx.notify();
                true
            }
            Err(msg) => {
                self.status_line = msg;
                cx.notify();
                false
            }
        }
    }

    pub(crate) fn trigger_diagnostics(&mut self, cx: &mut Context<Self>) {
        match &mut self.conn {
            Ok(state) => match state.run_diagnostics() {
                Ok(rep) => self.status_line = rep.attribution.summary,
                Err(e) => {
                    self.offline_diag = Some(offline_diagnostic_report(&e));
                    self.status_line = format!("Diagnostics IPC error: {e}");
                }
            },
            Err(err) => {
                let rep = offline_diagnostic_report(err);
                self.status_line = rep.attribution.summary.clone();
                self.offline_diag = Some(rep);
            }
        }
        cx.notify();
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (conn_ok, daemon_info, travel_on, model_name) = match &self.conn {
            Ok(st) => (
                true,
                format!(
                    "{} kHz · pid {}",
                    st.poll_rate_khz, st.daemon_status.daemon_pid
                ),
                st.travel_enabled,
                st.model_label().to_string(),
            ),
            Err(_) => (false, "Offline".into(), false, "AULA WIN 60 HE".to_string()),
        };
        let collapsed = self.sidebar_collapsed;

        let header = SidebarHeader::new().child(if collapsed {
            h_flex()
                .w_full()
                .justify_center()
                .child(Icon::new(IconName::Keyboard))
                .into_any_element()
        } else {
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .gap_2()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    div()
                                        .w(px(6.0))
                                        .h(px(6.0))
                                        .rounded(cx.theme().radius_full())
                                        .bg(if conn_ok {
                                            cx.theme().success
                                        } else {
                                            cx.theme().danger
                                        }),
                                )
                                .child(div().text_sm().child("REAULA")),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{model_name} · {daemon_info}")),
                        ),
                )
                .child(
                    SidebarToggleButton::new()
                        .collapsed(collapsed)
                        .accessibility_label("Collapse sidebar")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.sidebar_collapsed = !this.sidebar_collapsed;
                            cx.notify();
                        })),
                )
                .into_any_element()
        });

        let menu = SidebarMenu::new().children(NavSection::ALL.into_iter().map(|sec| {
            SidebarMenuItem::new(sec.label())
                .icon(sec.icon())
                .active(self.nav == sec)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.switch_section(sec, cx);
                }))
        }));

        let footer = if collapsed {
            SidebarFooter::new().child(
                v_flex()
                    .w_full()
                    .items_center()
                    .gap_1()
                    .child(
                        SidebarToggleButton::new()
                            .collapsed(collapsed)
                            .accessibility_label("Expand sidebar")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_collapsed = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("sb-mini-guide")
                            .xsmall()
                            .ghost()
                            .icon(Icon::new(IconName::BookOpen))
                            .tooltip("Guide & Help… (F1)")
                            .accessibility_label("Open Guide & Help window")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_guide(cx);
                            })),
                    ),
            )
        } else {
            SidebarFooter::new().child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(div().text_xs().child("Live Travel"))
                            .child(
                                Switch::new("sb-travel-sw")
                                    .small()
                                    .checked(travel_on)
                                    .on_change(cx.listener(|this, _, _, cx| {
                                        this.toggle_travel(cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                Button::new("sb-zero")
                                    .xsmall()
                                    .outline()
                                    .flex_1()
                                    .icon(Icon::new(IconName::RotateCcw))
                                    .label("Zero")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Ok(st) = &mut this.conn {
                                            st.clear_stuck_travel();
                                            this.dirty_3d = true;
                                            this.status_line = "Cleared cached travel.".into();
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("sb-reconnect")
                                    .xsmall()
                                    .outline()
                                    .flex_1()
                                    .icon(Icon::new(IconName::RefreshCw))
                                    .label("Reconnect")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reconnect(window, cx);
                                    })),
                            ),
                    ),
            )
        };

        Sidebar::new("reaula-sidebar")
            .w(px(220.0))
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(collapsed)
            .header(header)
            .child(menu)
            .footer(footer)
    }
}

impl Render for ReaulaGui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for dead in self.pending_drop_images.drain(..) {
            let _ = window.drop_image(dead);
        }

        let is_dark = cx.theme().is_dark();
        let (conn_ok, hw_badge) = match &self.conn {
            Ok(s) => (
                s.daemon_status.device_connected,
                format!(
                    "{} · {} · {:.2} mm",
                    if s.daemon_status.device_connected {
                        "ONLINE"
                    } else {
                        "WAITING USB"
                    },
                    s.probe
                        .firmware_string
                        .as_deref()
                        .and_then(|f| f.rsplit_once(",SI,").map(|(_, t)| t))
                        .unwrap_or("HID"),
                    s.max_mm
                ),
            ),
            Err(_) => (false, "Daemon Offline".into()),
        };

        let top_header = h_flex()
            .w_full()
            .min_w_0()
            .px_4()
            .py_2()
            .bg(cx.theme().sidebar)
            .border_b_1()
            .border_color(cx.theme().border)
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                h_flex()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(Icon::new(self.nav.icon()))
                    .child(div().min_w_0().truncate().text_sm().child(self.nav.label())),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(
                        h_flex()
                            .max_w(px(320.0))
                            .min_w_0()
                            .overflow_hidden()
                            .px_2()
                            .py_1()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .items_center()
                            .gap_1p5()
                            .child(
                                div()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .flex_shrink_0()
                                    .rounded(cx.theme().radius_full())
                                    .bg(if conn_ok {
                                        cx.theme().success
                                    } else {
                                        cx.theme().danger
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(hw_badge),
                            ),
                    )
                    .child(
                        Button::new("top-theme-btn")
                            .xsmall()
                            .outline()
                            .icon(Icon::new(if is_dark {
                                IconName::Sun
                            } else {
                                IconName::Moon
                            }))
                            .tooltip("Toggle Light / Dark theme")
                            .accessibility_label("Toggle Light / Dark theme")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_theme(cx);
                            })),
                    )
                    .child(
                        Button::new("top-guide-btn")
                            .xsmall()
                            .outline()
                            .icon(Icon::new(IconName::BookOpen))
                            .label("Guide & Help…")
                            .tooltip("Open reference window (F1)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_guide(cx);
                            })),
                    ),
            );

        let footer_right = match &self.conn {
            Ok(s) => {
                let travel_txt = s.last_sample.map_or_else(
                    || "Travel idle".into(),
                    |(idx, mm)| {
                        let k = keymap::by_index(idx).map_or("?", |k| k.name);
                        format!("{k}: {mm:.2} mm")
                    },
                );
                let rtt_txt = s.feedback_log.front().map_or_else(String::new, |ev| {
                    format!(
                        " · {} ({:.2} ms)",
                        if ev.ok { "OK" } else { "ERR" },
                        ev.rtt_ms
                    )
                });
                format!("{travel_txt}{rtt_txt}")
            }
            Err(_) => "Offline".into(),
        };

        let footer = h_flex()
            .w_full()
            .px_4()
            .py_1p5()
            .bg(cx.theme().status_bar)
            .border_t_1()
            .border_color(cx.theme().status_bar_border)
            .items_center()
            .justify_between()
            .gap_3()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(self.status_line.clone()),
            )
            .child(div().flex_shrink_0().child(footer_right));

        let content_area = div()
            .id("page-scroll")
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scrollbar()
            .p_4()
            .child(self.render_active_section(window, cx));

        h_flex()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if window.has_focused_input(cx) {
                    return;
                }
                if ev.keystroke.key == "f1" {
                    this.open_guide(cx);
                    return;
                }
                let token = ev
                    .keystroke
                    .key_char
                    .as_deref()
                    .unwrap_or(ev.keystroke.key.as_str());
                if let Ok(state) = &mut this.conn
                    && state
                        .mascots
                        .record_code(MascotKeySource::Window, token, Instant::now())
                {
                    this.dirty_3d = true;
                    if this.nav == NavSection::ColorLight
                        && state.light_view_mode == LightViewMode::ThreeD
                    {
                        this.maybe_render_3d();
                    }
                    cx.notify();
                }
            }))
            .child(self.render_sidebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(top_header)
                    .child(content_area)
                    .child(footer),
            )
    }
}

fn build_audio_source_vec(conn: Option<&ConnectedState>) -> SearchableVec<SelectOption<usize>> {
    let items = conn
        .map(|st| {
            st.audio_sources
                .iter()
                .enumerate()
                .map(|(i, s)| SelectOption::new(i, s.label()))
                .collect()
        })
        .unwrap_or_else(|| vec![SelectOption::new(0, "Demo Synth".to_string())]);
    SearchableVec::new(items)
}

fn build_vk_vec(cat: KeymapCategory) -> Vec<SelectOption<VirtualKey>> {
    let list = if cat == KeymapCategory::FirmwareFnFlash {
        ConnectedState::firmware_compatible_virtual_keys()
    } else {
        VirtualKey::ALL.to_vec()
    };
    list.into_iter()
        .map(|vk| SelectOption::new(vk, vk.label()))
        .collect()
}

pub(crate) fn sanitize_command_paste(text: &str) -> String {
    text.chars()
        .filter(|&c| c != '\r' && c != '\n' && !c.is_control())
        .collect()
}

fn main() {
    let args = Args::parse();
    let socket = args.socket;

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::apply_clinical_theme(ThemeMode::Dark, cx);

            let bounds = Bounds::centered(None, size(px(1340.0), px(860.0)), cx);
            let (_win, entity) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                cx,
                move |window, cx| cx.new(|cx| ReaulaGui::new(socket, window, cx)),
            )
            .expect("failed to open window");

            let _ = _win.update(cx, |_, window, cx| {
                window.set_window_title("Reaula - WIN 60 HE Workbench");
                let focus = entity.read(cx).root_focus.clone();
                focus.focus(window, cx);
            });

            // Daemon retains Audio Sync across GUI exit; only terminate UI app when all windows close.
            cx.on_window_closed(move |cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_command_paste_strips_newlines_and_controls() {
        assert_eq!(
            sanitize_command_paste("notify-send 'hi'\nrm -rf\x07\r"),
            "notify-send 'hi'rm -rf"
        );
    }
}
