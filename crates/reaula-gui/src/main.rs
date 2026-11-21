//! Reaula WIN 60 HE desktop control panel built with GPUI + wgpu.

mod icons;
mod kb_3d;
mod state;
mod theme;
mod views;
mod widgets;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use gpui::{
    App, Application, AsyncApp, Bounds, Context, Div, EntityInputHandler, FocusHandle, KeyDownEvent,
    Pixels, RenderImage, UTF16Selection, WeakEntity, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};

use reaula_core::diagnostics::DiagnosticReport;
use reaula_core::keymap;
use reaula_core::protocol::{MATRIX_SLOTS, PollRate, TriggerConfig, TriggerMode, mm_to_raw};

use icons::MaterialIcon;
use kb_3d::{
    CameraPreset, Kb3dCamera, Kb3dSceneInput, KeyboardRenderer3D, SwitchCamera,
    SwitchCameraPreset, SwitchRenderer3D, SwitchSceneInput,
};
use state::{
    ConnectedState, DropdownId, LightViewMode, MascotKeySource, NavSection, SwitchViewMode,
    offline_diagnostic_report,
};
use widgets::{icon_btn, icon_pill};

#[derive(Parser)]
#[command(name = "reaula-gui", version, about = "GPUI + wgpu desktop driver for Aula WIN 60 HE")]
struct Args {
    /// Connect to a specific reaulad Unix domain socket
    #[arg(long)]
    socket: Option<PathBuf>,
}

pub(crate) type PickerBoundsMap = Rc<RefCell<BTreeMap<(&'static str, u8), Bounds<Pixels>>>>;

pub(crate) struct ReaulaGui {
    pub(crate) socket: Option<PathBuf>,
    pub(crate) nav: NavSection,
    pub(crate) open_dropdown: Option<DropdownId>,
    pub(crate) conn: Result<ConnectedState, String>,
    pub(crate) offline_diag: Option<DiagnosticReport>,
    pub(crate) status_line: String,
    pub(crate) start_instant: Instant,

    pub(crate) renderer_3d: Result<KeyboardRenderer3D, String>,
    pub(crate) preview_3d_img: Option<Arc<RenderImage>>,
    pub(crate) switch_renderer_3d: Result<SwitchRenderer3D, String>,
    pub(crate) preview_switch_3d_img: Option<Arc<RenderImage>>,
    pub(crate) pending_drop_images: Vec<Arc<RenderImage>>,
    pub(crate) paper_grain: Arc<RenderImage>,
    pub(crate) dirty_3d: bool,
    pub(crate) last_3d_render: Instant,
    pub(crate) dirty_switch_3d: bool,
    pub(crate) last_switch_3d_render: Instant,
    pub(crate) last_hotkey_poll: Instant,
    pub(crate) dragging_3d: bool,
    pub(crate) last_mouse_3d: Option<(f32, f32)>,
    pub(crate) dragging_switch_3d: bool,
    pub(crate) last_mouse_switch_3d: Option<(f32, f32)>,
    pub(crate) picker_bounds: PickerBoundsMap,
    pub(crate) picker_hues: BTreeMap<&'static str, f32>,
    pub(crate) dragging_picker: Option<(&'static str, u8)>,

    pub(crate) root_focus: FocusHandle,
    pub(crate) cmd_focus: FocusHandle,
    pub(crate) focus_cmd_next_frame: bool,
}

impl ReaulaGui {
    fn new(socket: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let conn = ConnectedState::connect(socket.clone());
        let status_line = match &conn {
            Ok(s) => format!(
                "Connected to reaulad (uid={}, pid={}) via {}",
                s.server_cred.uid, s.daemon_status.daemon_pid, s.daemon_status.socket_path
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

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let res = this.update(cx, |view, cx| {
                    let mut changed = false;
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
                        if state.audio_disclaimer_modal_open && !state.audio_disclaimer_accepted {
                            changed = true;
                        }
                        if (view.nav == NavSection::Hotkeys
                            || view.nav == NavSection::ColorLight)
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
                    if view.nav == NavSection::ColorLight
                        && view.tick_color_light_stage()
                    {
                        changed = true;
                    }
                    if view.nav == NavSection::Diagnostics
                        && view.tick_diagnostics_stage()
                    {
                        changed = true;
                    }
                    if changed {
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
            open_dropdown: None,
            conn,
            offline_diag: None,
            status_line,
            start_instant: Instant::now(),
            renderer_3d,
            preview_3d_img: None,
            switch_renderer_3d,
            preview_switch_3d_img: None,
            pending_drop_images: Vec::new(),
            paper_grain: theme::create_paper_grain_image(),
            dirty_3d: true,
            last_3d_render: Instant::now() - Duration::from_secs(1),
            dirty_switch_3d: true,
            last_switch_3d_render: Instant::now() - Duration::from_secs(1),
            last_hotkey_poll: Instant::now(),
            dragging_3d: false,
            last_mouse_3d: None,
            dragging_switch_3d: false,
            last_mouse_switch_3d: None,
            picker_bounds: Rc::new(RefCell::new(BTreeMap::new())),
            picker_hues: BTreeMap::new(),
            dragging_picker: None,
            root_focus: cx.focus_handle(),
            cmd_focus: cx.focus_handle(),
            focus_cmd_next_frame: false,
        }
    }

    /// Advance live 2D or 3D Lighting Studio previews at ~30 fps when animations or impulses are active.
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

    /// Advance the Diagnostics switch mechanism stage (`wgpu` 3D cutaway or 2D fallback) at ~30 fps.
    pub(crate) fn tick_diagnostics_stage(&mut self) -> bool {
        let Ok(state) = &self.conn else {
            return false;
        };
        let use_3d = state.diag_switch_view == SwitchViewMode::ThreeD
            && self.switch_renderer_3d.is_ok();
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
        self.open_dropdown = None;
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

    fn reconnect(&mut self, cx: &mut Context<Self>) {
        self.conn = ConnectedState::connect(self.socket.clone());
        self.open_dropdown = None;
        self.offline_diag = None;
        self.dirty_3d = true;
        self.status_line = match &self.conn {
            Ok(s) => format!(
                "Reconnected to reaulad (uid={}, pid={})",
                s.server_cred.uid, s.daemon_status.daemon_pid
            ),
            Err(e) => e.clone(),
        };
        cx.notify();
    }

    fn toggle_travel(&mut self, cx: &mut Context<Self>) {
        if let Ok(state) = &mut self.conn {
            if state.travel_enabled {
                state.stop_travel();
                self.status_line = "Live travel monitor paused.".into();
            } else {
                match state.start_travel() {
                    Ok(()) => self.status_line = "Live travel monitor active.".into(),
                    Err(e) => self.status_line = e,
                }
            }
            self.dirty_3d = true;
        }
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
                                || p.map_or_else(|| prev.map_or(0, |c| c.interval2_raw), |v| mm_to_raw(v, unit)),
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
            Err(e) => self.status_line = format!("Actuation write failed: {e}"),
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
                    "Updated switch profile -> {} for {} key(s).",
                    reaula_core::protocol::switch_profile_name(switch_id),
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
                    "Updated deadband ({:.2}/{:.2} mm) for {} key(s).",
                    f64::from(top_raw) * 0.01,
                    f64::from(bot_raw) * 0.01,
                    indices.len()
                );
            }
            Err(e) => self.status_line = format!("Deadband write failed: {e}"),
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
                self.status_line = format!("Set USB polling rate to {khz} kHz (keyboard restarts briefly).");
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
        match state.run_measured("Set SOCD / PRCS", |c| c.set_socd(enabled, pairs.clone())) {
            Ok(()) => {
                self.status_line = format!(
                    "SOCD {} ({} pair(s)).",
                    if state.socd_enabled { "enabled" } else { "disabled" },
                    state.socd_pairs.len()
                );
            }
            Err(e) => self.status_line = format!("SOCD write failed: {e}"),
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
                    "Applied RGB: {} (bri={}, spd={}).",
                    reaula_core::protocol::light_mode_name(ls.mode),
                    ls.brightness,
                    ls.speed
                );
                self.dirty_3d = true;
            }
            Err(e) => self.status_line = format!("Lighting write failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn reload_lighting(&mut self, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        match state.run_measured("Read Lighting", |c| c.get_light()) {
            Ok(ls) => {
                state.light_state = ls;
                self.dirty_3d = true;
                self.status_line = "Reloaded lighting state from keyboard.".into();
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
                self.status_line = format!("Uploaded Custom RGB Slot {} to keyboard.", slot + 1);
            }
            Err(e) => self.status_line = format!("Custom RGB upload failed: {e}"),
        }
        cx.notify();
    }

    pub(crate) fn reload_custom_rgb(&mut self, slot: u8, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        match state.run_measured("Read Custom RGB", |c| c.get_custom_rgb(slot)) {
            Ok(colors) => {
                let mut arr = [[0u8; 3]; MATRIX_SLOTS];
                for (dst, src) in arr.iter_mut().zip(colors) {
                    *dst = src;
                }
                state.custom_slots[usize::from(slot)] = arr;
                self.dirty_3d = true;
                self.status_line = format!("Reloaded Custom RGB Slot {} from keyboard.", slot + 1);
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
                Err(e) => self.status_line = format!("Calibration start failed: {e}"),
            }
        }
        cx.notify();
    }

    pub(crate) fn request_start_audio_sync(&mut self, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
        };
        if !state.audio_disclaimer_accepted {
            state.open_audio_disclaimer(true);
            self.status_line = "Read and accept the hardware warning before enabling Audio Sync.".into();
            cx.notify();
            return;
        }
        match state.start_audio_sync() {
            Ok(()) => {
                self.status_line = state.audio_status_text.clone();
                self.dirty_3d = true;
            }
            Err(e) => self.status_line = e,
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

    pub(crate) fn confirm_audio_disclaimer(&mut self, cx: &mut Context<Self>) {
        let Ok(state) = &mut self.conn else {
            return;
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
                    self.status_line = "Audio Sync hardware disclaimer acknowledged.".into();
                }
            }
            Err(msg) => {
                self.status_line = msg;
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

    pub(crate) fn trigger_diagnostics(&mut self, cx: &mut Context<Self>) {
        match &mut self.conn {
            Ok(state) => match state.run_diagnostics() {
                Ok(rep) => {
                    self.status_line = rep.attribution.summary;
                }
                Err(e) => {
                    self.offline_diag = Some(offline_diagnostic_report(&e));
                    self.status_line = format!("Diagnostics failed over IPC: {e}");
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

    fn render_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let (conn_ok, daemon_info, travel_on) = match &self.conn {
            Ok(st) => (
                true,
                format!("uid={} pid={} | {}kHz", st.server_cred.uid, st.daemon_status.daemon_pid, st.poll_rate_khz),
                st.travel_enabled,
            ),
            Err(_) => (false, "reaulad offline".into(), false),
        };

        div()
            .w(px(238.0))
            .flex_shrink_0()
            .h_full()
            .bg(rgb(theme::SHADE_BASE))
            .border_r_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .p_4()
            .flex()
            .flex_col()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .pb_3()
                            .border_b_1()
                            .border_color(rgb(theme::BORDER_SUBTLE))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2p5()
                                    .child(
                                        div()
                                            .w(px(24.0))
                                            .h(px(24.0))
                                            .rounded_md()
                                            .bg(rgb(theme::SHADE_REGAL))
                                            .border_1()
                                            .border_color(rgb(theme::BORDER_FOCUS))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(px(10.0))
                                            .text_color(rgb(theme::INK_PRIMARY))
                                            .child("R"),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(rgb(theme::INK_PRIMARY))
                                                    .child("REAULA"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(9.5))
                                                    .text_color(rgb(theme::INK_MUTED))
                                                    .child(
                                                        self.conn
                                                            .as_ref()
                                                            .map_or_else(
                                                                |_| "AULA WIN 60 HE".to_string(),
                                                                |s| s.model_label().to_ascii_uppercase(),
                                                            ),
                                                    ),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .bg(rgb(theme::SHADE_DEEP))
                                    .border_1()
                                    .border_color(rgb(theme::BORDER_SUBTLE))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(6.0))
                                            .h(px(6.0))
                                            .rounded_full()
                                            .bg(rgb(if conn_ok {
                                                theme::INK_OK
                                            } else {
                                                theme::INK_DANGER
                                            })),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .text_color(rgb(if conn_ok {
                                                theme::INK_SECONDARY
                                            } else {
                                                theme::INK_DANGER
                                            }))
                                            .child(daemon_info),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .children(NavSection::ALL.iter().enumerate().map(|(idx, &sec)| {
                                let active = self.nav == sec;
                                div()
                                    .id(("nav-item", idx))
                                    .relative()
                                    .p_2()
                                    .px_2p5()
                                    .rounded_lg()
                                    .bg(rgb(if active {
                                        theme::SHADE_REGAL
                                    } else {
                                        theme::SHADE_BASE
                                    }))
                                    .border_1()
                                    .border_color(rgb(if active {
                                        theme::BORDER_REGAL
                                    } else {
                                        theme::SHADE_BASE
                                    }))
                                    .cursor_pointer()
                                    .hover(|s| {
                                        s.bg(rgb(theme::SHADE_REGAL))
                                            .border_color(rgb(if active {
                                                theme::BORDER_FOCUS
                                            } else {
                                                theme::BORDER_MATTE
                                            }))
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.switch_section(sec, cx);
                                    }))
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2p5()
                                    .child(
                                        div()
                                            .w(px(2.5))
                                            .h(px(18.0))
                                            .rounded_full()
                                            .bg(rgb(if active {
                                                theme::BORDER_FOCUS
                                            } else {
                                                theme::SHADE_BASE
                                            })),
                                    )
                                    .child(
                                        div()
                                            .w(px(28.0))
                                            .h(px(28.0))
                                            .rounded_md()
                                            .bg(rgb(theme::SHADE_DEEP))
                                            .border_1()
                                            .border_color(rgb(if active {
                                                theme::BORDER_REGAL
                                            } else {
                                                theme::BORDER_SUBTLE
                                            }))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(icons::icon(
                                                sec.icon(),
                                                15.0,
                                                if active {
                                                    theme::INK_PRIMARY
                                                } else {
                                                    theme::INK_MUTED
                                                },
                                            )),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_xs()
                                                    .text_color(rgb(if active {
                                                        theme::INK_PRIMARY
                                                    } else {
                                                        theme::INK_SECONDARY
                                                    }))
                                                    .child(sec.label()),
                                            )
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(9.5))
                                                    .text_color(rgb(theme::INK_MUTED))
                                                    .child(sec.subtitle()),
                                            ),
                                    )
                            })),
                    ),
            )
            .child(
                div()
                    .pt_3()
                    .border_t_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        icon_pill(
                            "sb-travel",
                            Some(MaterialIcon::Sensors),
                            if travel_on { "Live Travel: ON" } else { "Live Travel: OFF" },
                            travel_on,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_travel(cx);
                        })),
                    )
                    .child(
                        icon_btn(
                            "sb-zero",
                            Some(MaterialIcon::RestartAlt),
                            "Zero Stuck Keys",
                            widgets::ButtonTone::Secondary,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Ok(st) = &mut this.conn {
                                st.clear_stuck_travel();
                                this.dirty_3d = true;
                                this.status_line = "Cleared cached travel readings.".into();
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_btn(
                            "sb-reconnect",
                            Some(MaterialIcon::Refresh),
                            "Reconnect / Refresh",
                            widgets::ButtonTone::Secondary,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.reconnect(cx);
                        })),
                    ),
            )
    }
}

fn utf16_offset_to_byte(s: &str, utf16_offset: usize) -> usize {
    let mut acc = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        if acc >= utf16_offset {
            return byte_idx;
        }
        acc += ch.len_utf16();
    }
    s.len()
}

fn utf16_range_to_bytes(s: &str, range: Range<usize>) -> Range<usize> {
    let total_utf16 = s.encode_utf16().count();
    let start_u16 = range.start.min(total_utf16);
    let end_u16 = range.end.clamp(start_u16, total_utf16);
    utf16_offset_to_byte(s, start_u16)..utf16_offset_to_byte(s, end_u16)
}

pub(crate) fn sanitize_command_paste(text: &str) -> String {
    text.chars()
        .filter(|&c| c != '\r' && c != '\n' && !c.is_control())
        .collect()
}

impl EntityInputHandler for ReaulaGui {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let Ok(st) = &self.conn else {
            return None;
        };
        let total = st.draft_rule_command.encode_utf16().count();
        let clamped = range.start.min(total)..range.end.clamp(range.start.min(total), total);
        *adjusted_range = Some(clamped.clone());
        let byte_range = utf16_range_to_bytes(&st.draft_rule_command, clamped);
        Some(st.draft_rule_command[byte_range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let len16 = self
            .conn
            .as_ref()
            .map_or(0, |st| st.draft_rule_command.encode_utf16().count());
        Some(UTF16Selection {
            range: len16..len16,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(st) = &mut self.conn else {
            return;
        };
        let clean = sanitize_command_paste(text);
        if let Some(r16) = range {
            let br = utf16_range_to_bytes(&st.draft_rule_command, r16);
            st.draft_rule_command.replace_range(br, &clean);
        } else {
            st.draft_rule_command.push_str(&clean);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_text_in_range(range, new_text, window, cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(element_bounds)
    }

    fn character_index_for_point(
        &mut self,
        _point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for ReaulaGui {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        for dead in self.pending_drop_images.drain(..) {
            let _ = window.drop_image(dead);
        }
        if self.focus_cmd_next_frame {
            self.focus_cmd_next_frame = false;
            window.focus(&self.cmd_focus);
        } else if self.nav != NavSection::Hotkeys && self.cmd_focus.is_focused(window) {
            window.focus(&self.root_focus);
        }

        let top_header = div()
            .h(px(50.0))
            .px_5()
            .gap_4()
            .bg(rgb(theme::SHADE_BASE))
            .border_b_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2p5()
                    .child(icons::icon(self.nav.icon(), 16.0, theme::INK_ACCENT))
                    .child(
                        div()
                            .whitespace_nowrap()
                            .text_sm()
                            .text_color(rgb(theme::INK_PRIMARY))
                            .child(self.nav.label()),
                    )
                    .child(div().text_xs().text_color(rgb(theme::INK_DIM)).child("·"))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(theme::INK_MUTED))
                            .child(self.nav.subtitle()),
                    ),
            )
            .child(
                div()
                    .max_w(px(390.0))
                    .min_w_0()
                    .px_2p5()
                    .py_1()
                    .rounded_md()
                    .bg(rgb(theme::SHADE_DEEP))
                    .border_1()
                    .border_color(rgb(theme::BORDER_SUBTLE))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded_full()
                            .flex_shrink_0()
                            .bg(rgb(if self.conn.as_ref().is_ok_and(|s| s.daemon_status.device_connected) {
                                theme::INK_OK
                            } else {
                                theme::INK_DANGER
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(rgb(theme::INK_SECONDARY))
                            .child(match &self.conn {
                                Ok(s) => format!(
                                    "{} · {} · {:.2} mm",
                                    if s.daemon_status.device_connected {
                                        "ONLINE"
                                    } else {
                                        "WAITING USB"
                                    },
                                    short_fw_label(s.probe.firmware_string.as_deref().unwrap_or("?")),
                                    s.max_mm
                                ),
                                Err(_) => "Daemon Offline".into(),
                            }),
                    ),
            );

        let footer_right = match &self.conn {
            Ok(s) => {
                let travel_txt = s.last_sample.map_or_else(
                    || "Travel idle".into(),
                    |(idx, mm)| {
                        let k = keymap::by_index(idx).map_or("?", |k| k.name);
                        format!("Last travel: {k} = {mm:.2} mm")
                    },
                );
                let rtt_txt = s.feedback_log.front().map_or_else(
                    String::new,
                    |ev| format!("  ·  {} ({:.2} ms)", if ev.ok { "OK" } else { "ERR" }, ev.rtt_ms),
                );
                format!("{travel_txt}{rtt_txt}")
            }
            Err(_) => "Offline".into(),
        };

        let footer = div()
            .h(px(30.0))
            .px_5()
            .gap_4()
            .bg(rgb(theme::SHADE_DEEP))
            .border_t_1()
            .border_color(rgb(theme::BORDER_SUBTLE))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .text_xs()
            .text_color(rgb(theme::INK_MUTED))
            .child(div().flex_1().min_w_0().truncate().child(self.status_line.clone()))
            .child(div().flex_shrink_0().whitespace_nowrap().child(footer_right));

        let content_area = div()
            .id("page-scroll")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .p_5()
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .child(self.render_active_section(window, cx)),
            );

        let disclaimer_modal = self
            .conn
            .as_ref()
            .ok()
            .filter(|s| s.audio_disclaimer_modal_open)
            .map(|s| self.render_audio_disclaimer_modal(s, cx));

        let mut root = div()
            .relative()
            .w_full()
            .h_full()
            .overflow_hidden()
            .flex()
            .flex_row()
            .bg(rgb(theme::SHADE_DEEP))
            .text_color(rgb(theme::INK_PRIMARY))
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if this.cmd_focus.is_focused(window) {
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
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    .child(top_header)
                    .child(content_area)
                    .child(footer),
            )
            .child(theme::paper_grain_overlay(self.paper_grain.clone()));
        if let Some(modal) = disclaimer_modal {
            root = root.child(modal);
        }
        root
    }
}

fn short_fw_label(fw: &str) -> &str {
    // e.g. "W669,34,KB,SI,SI2825KR-AHEARGB,V3.18.00" -> "SI2825KR-AHEARGB V3.18.00" (or trimmed segment)
    if let Some((_, tail)) = fw.rsplit_once(",SI,") {
        tail
    } else {
        fw
    }
}

fn main() {
    let args = Args::parse();
    let socket = args.socket;

    Application::new()
        .with_assets(icons::IconAssets)
        .run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1280.0), px(840.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let sock = socket.clone();
                let entity = cx.new(|cx| ReaulaGui::new(sock, cx));
                let focus = entity.read(cx).root_focus.clone();
                window.focus(&focus);
                entity
            },
        )
        .expect("failed to open GPUI window");

        cx.on_window_closed(move |cx| {
            // Audio Sync runs inside `reaulad` so closing the GUI keeps keyboard effects active.
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
        assert_eq!(utf16_range_to_bytes("abc", 1..2), 1..2);
    }
}
