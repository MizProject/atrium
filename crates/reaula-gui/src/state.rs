//! Application state, daemon IPC wrappers, background streams, and diagnostic telemetry for `reaula-gui`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use reaula_core::alsa;
use reaula_core::audio_fx::{AudioEffectStyle, AudioFrameMetrics, AudioPalette};
use reaula_core::diagnostics::{self, CheckStatus, DiagnosticCheck, DiagnosticLayer, DiagnosticReport};
use reaula_core::driver::DeviceProbe;
use reaula_core::ipc::{
    AudioSyncConfig, AudioSyncSource, AudioSyncStatus, DaemonClient, DaemonStatus, FnOverride,
    IpcError, PeerCredentials, ShortcutAction, ShortcutRule, ShortcutTriggerLog,
    ShortcutsSnapshot, VirtualKey,
};
use reaula_core::keymap;
use reaula_core::protocol::{
    self, CalibrationProgress, CalibrationStatus, LIGHT_MAX_LEVEL, LightState, LiveTravelSample,
    MATRIX_SLOTS, PrcsPair, TriggerConfig, TriggerMode,
};

use crate::kb_3d::{
    CameraPreset, Kb3dCamera, KeyImpulse, MascotPose, SwitchCamera, SwitchCameraPreset,
};

pub const MAX_FEEDBACK_EVENTS: usize = 24;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavSection {
    Main,
    Guide,
    Calibration,
    ColorLight,
    Hotkeys,
    Diagnostics,
}

impl NavSection {
    pub const ALL: [Self; 6] = [
        Self::Main,
        Self::Guide,
        Self::Calibration,
        Self::ColorLight,
        Self::Hotkeys,
        Self::Diagnostics,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "Main",
            Self::Guide => "Guide",
            Self::Calibration => "Calibration",
            Self::ColorLight => "Color & Light",
            Self::Hotkeys => "Hotkeys",
            Self::Diagnostics => "Diagnostics",
        }
    }

    pub fn icon(self) -> crate::icons::MaterialIcon {
        match self {
            Self::Main => crate::icons::MaterialIcon::Keyboard,
            Self::Guide => crate::icons::MaterialIcon::MenuBook,
            Self::Calibration => crate::icons::MaterialIcon::Tune,
            Self::ColorLight => crate::icons::MaterialIcon::Palette,
            Self::Hotkeys => crate::icons::MaterialIcon::Bolt,
            Self::Diagnostics => crate::icons::MaterialIcon::MonitorHeart,
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Main => "Status, Actuation, Switches & SOCD",
            Self::Guide => "WIN 60 HE Reference & Shortcuts",
            Self::Calibration => "Guarded Sensor Calibration",
            Self::ColorLight => "3D Model, 2D Mockup & Custom RGB",
            Self::Hotkeys => "Daemon In-Memory Shortcuts",
            Self::Diagnostics => "4-Layer Fault Isolation & Test",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResponseEvent {
    pub id: u64,
    pub label: String,
    pub rtt_ms: f64,
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct SwitchTestStats {
    pub current_mm: f64,
    pub peak_mm: f64,
    pub released_zero: bool,
    pub presses: u32,
    pub was_pressed: bool,
    pub last_seen: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LightEditorTab {
    FirmwareEffects,
    CustomSlot1,
    CustomSlot2,
    AudioSync,
}

impl LightEditorTab {
    pub fn custom_slot_index(self) -> Option<u8> {
        match self {
            Self::FirmwareEffects | Self::AudioSync => None,
            Self::CustomSlot1 => Some(0),
            Self::CustomSlot2 => Some(1),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightViewMode {
    ThreeD,
    TwoD,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SwitchViewMode {
    ThreeD,
    TwoD,
}

#[derive(Clone, Debug)]
pub struct SwitchInspectionSnapshot {
    pub key_index: u8,
    pub key_name: &'static str,
    pub switch_id: u8,
    pub switch_label: &'static str,
    pub travel_mm: f64,
    pub max_mm: f64,
    pub peak_mm: f64,
    pub presses: u32,
    pub released_zero: bool,
    pub actuation_mm: f64,
    pub rapid_trigger: bool,
    pub rt_press_mm: f64,
    pub rt_release_mm: f64,
    pub db_top_mm: f64,
    pub db_bottom_mm: f64,
    pub triggered: bool,
    pub sweeping: bool,
    pub flux_pct: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DropdownId {
    LightEffect,
    LightDirection,
    LightFgPreset,
    LightBgPreset,
    CustomBrushPreset,
    AudioSource,
    AudioEffect,
    AudioPalette,
    AudioFgPreset,
    AudioBgPreset,
    HotkeyArrowKb,
    HotkeyArrowDaemon,
    HotkeyModifier,
    HotkeyTrigger,
    HotkeyAction,
    HotkeyVirtualKey,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ArrowClusterPreset {
    Rdfg,
    Tfgh,
    Wasd,
}

fn arrow_cluster_bindings(preset: ArrowClusterPreset) -> [(u8, VirtualKey, &'static str); 4] {
    match preset {
        ArrowClusterPreset::Rdfg => [
            (keymap::idx::R, VirtualKey::Up, "Fn+R -> Up"),
            (keymap::idx::F, VirtualKey::Down, "Fn+F -> Down"),
            (keymap::idx::D, VirtualKey::Left, "Fn+D -> Left"),
            (keymap::idx::G, VirtualKey::Right, "Fn+G -> Right"),
        ],
        ArrowClusterPreset::Tfgh => [
            (keymap::idx::T, VirtualKey::Up, "Fn+T -> Up"),
            (keymap::idx::G, VirtualKey::Down, "Fn+G -> Down"),
            (keymap::idx::F, VirtualKey::Left, "Fn+F -> Left"),
            (keymap::idx::H, VirtualKey::Right, "Fn+H -> Right"),
        ],
        ArrowClusterPreset::Wasd => [
            (keymap::idx::W, VirtualKey::Up, "Fn+W -> Up"),
            (keymap::idx::S, VirtualKey::Down, "Fn+S -> Down"),
            (keymap::idx::A, VirtualKey::Left, "Fn+A -> Left"),
            (keymap::idx::D, VirtualKey::Right, "Fn+D -> Right"),
        ],
    }
}

#[derive(Clone, Debug)]
pub struct TrackedImpulse {
    pub index: u8,
    pub nx: f32,
    pub ny: f32,
    pub trigger_at: Instant,
    pub strength: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShortcutActionPreset {
    EmitVirtualKey,
    FpsRapidTrigger,
    TypingComfort,
    ToggleSocd,
    ToggleLighting,
    NeonWaveLighting,
    PollRate8k,
    RunCommand,
}

impl ShortcutActionPreset {
    pub const ALL: [Self; 8] = [
        Self::EmitVirtualKey,
        Self::FpsRapidTrigger,
        Self::TypingComfort,
        Self::ToggleSocd,
        Self::ToggleLighting,
        Self::NeonWaveLighting,
        Self::PollRate8k,
        Self::RunCommand,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::EmitVirtualKey => "Emit OS Key (uinput)",
            Self::FpsRapidTrigger => "FPS RT WASD",
            Self::TypingComfort => "Typing 1.8mm",
            Self::ToggleSocd => "Toggle SOCD",
            Self::ToggleLighting => "Toggle LED",
            Self::NeonWaveLighting => "Wave Cyan RGB",
            Self::PollRate8k => "8 kHz Poll",
            Self::RunCommand => "Shell Command",
        }
    }
}

pub struct TravelThread {
    stop: Arc<AtomicBool>,
    rx: Receiver<(LiveTravelSample, f64)>,
}

impl Drop for TravelThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub const AUDIO_DISCLAIMER_COOLDOWN_SECS: f64 = 10.0;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AudioSourceOption {
    Monitor,
    DefaultMic,
    Demo,
    AlsaDevice { pcm: String, label: String },
}

impl AudioSourceOption {
    pub fn label(&self) -> String {
        match self {
            Self::Monitor => "Desktop Audio Monitor (ALSA)".into(),
            Self::DefaultMic => "Default Mic / Line-In (ALSA)".into(),
            Self::Demo => "120 BPM Built-In Demo Synth".into(),
            Self::AlsaDevice { pcm, label } => format!("{pcm} — {label}"),
        }
    }

    pub fn to_ipc(&self) -> AudioSyncSource {
        match self {
            Self::Monitor => AudioSyncSource::Monitor,
            Self::DefaultMic => AudioSyncSource::DefaultMic,
            Self::Demo => AudioSyncSource::Demo,
            Self::AlsaDevice { pcm, label } => AudioSyncSource::Device {
                pcm: pcm.clone(),
                label: label.clone(),
            },
        }
    }

    pub fn from_ipc(src: &AudioSyncSource) -> Self {
        match src {
            AudioSyncSource::Monitor => Self::Monitor,
            AudioSyncSource::DefaultMic => Self::DefaultMic,
            AudioSyncSource::Demo => Self::Demo,
            AudioSyncSource::Device { pcm, label } => Self::AlsaDevice {
                pcm: pcm.clone(),
                label: label.clone(),
            },
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AudioLiveConfig {
    pub style: AudioEffectStyle,
    pub palette: AudioPalette,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub brightness: u8,
    pub fps: u8,
    pub gain: f32,
}

impl AudioLiveConfig {
    pub fn to_ipc(self, source: &AudioSourceOption) -> AudioSyncConfig {
        AudioSyncConfig {
            source: source.to_ipc(),
            style: self.style,
            palette: self.palette,
            fg: self.fg,
            bg: self.bg,
            brightness: self.brightness,
            fps: self.fps,
            gain: self.gain,
        }
    }

    pub fn apply_from_ipc(&mut self, cfg: &AudioSyncConfig) {
        self.style = cfg.style;
        self.palette = cfg.palette;
        self.fg = cfg.fg;
        self.bg = cfg.bg;
        self.brightness = cfg.brightness;
        self.fps = cfg.fps;
        self.gain = cfg.gain;
    }
}

pub struct CalibThread {
    stop: Arc<AtomicBool>,
    rx: Receiver<CalibrationProgress>,
}

impl Drop for CalibThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MascotKeySource {
    Hardware,
    Window,
}

#[derive(Debug, Default, Clone)]
pub struct MascotEasterEggState {
    pub miku_visible: bool,
    pub teto_visible: bool,
    pub miku_since: Option<Instant>,
    pub teto_since: Option<Instant>,
    last_digit1_at: Option<Instant>,
    last_digit3_at: Option<Instant>,
    last_digit4_at: Option<Instant>,
    last_digit9_at: Option<Instant>,
    last_miku_toggle_at: Option<Instant>,
    last_teto_toggle_at: Option<Instant>,
    last_hw_echo: Option<(&'static str, Instant)>,
    dirty: bool,
}

impl MascotEasterEggState {
    const MIKU_FAST_WINDOW: Duration = Duration::from_millis(420);
    const TETO_PRESS_WINDOW: Duration = Duration::from_millis(750);
    const TOGGLE_COOLDOWN: Duration = Duration::from_millis(2000);
    const HW_ECHO_WINDOW: Duration = Duration::from_millis(90);

    pub fn any_visible(&self) -> bool {
        self.miku_visible || self.teto_visible
    }

    pub fn consume_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    pub fn record_code(&mut self, source: MascotKeySource, raw: &str, now: Instant) -> bool {
        let canon = match raw {
            "3" | "Digit3" => Some("Digit3"),
            "9" | "Digit9" => Some("Digit9"),
            "4" | "Digit4" => Some("Digit4"),
            "1" | "Digit1" => Some("Digit1"),
            _ => None,
        };

        let Some(digit) = canon else {
            // Reset pending fast sequence if an unrelated key lands in between.
            if source == MascotKeySource::Hardware
                || self
                    .last_hw_echo
                    .is_none_or(|(_, ts)| now.saturating_duration_since(ts) > Self::HW_ECHO_WINDOW)
            {
                self.last_digit1_at = None;
                self.last_digit3_at = None;
                self.last_digit4_at = None;
                self.last_digit9_at = None;
            }
            return false;
        };

        match source {
            MascotKeySource::Hardware => {
                self.last_hw_echo = Some((digit, now));
            }
            MascotKeySource::Window => {
                if let Some((last_d, ts)) = self.last_hw_echo
                    && last_d == digit
                    && now.saturating_duration_since(ts) <= Self::HW_ECHO_WINDOW
                {
                    return false;
                }
            }
        }

        let mut toggled = false;
        match digit {
            "Digit3" => {
                self.last_digit3_at = Some(now);
                if self
                    .last_digit9_at
                    .is_some_and(|t| now.saturating_duration_since(t) <= Duration::from_millis(220))
                {
                    self.last_digit9_at = None;
                    self.last_digit3_at = None;
                    toggled = self.toggle_miku(now);
                }
            }
            "Digit9" => {
                self.last_digit9_at = Some(now);
                if self
                    .last_digit3_at
                    .is_some_and(|t| now.saturating_duration_since(t) <= Self::MIKU_FAST_WINDOW)
                {
                    self.last_digit3_at = None;
                    self.last_digit9_at = None;
                    toggled = self.toggle_miku(now);
                }
            }
            "Digit4" => {
                self.last_digit4_at = Some(now);
                if self
                    .last_digit1_at
                    .is_some_and(|t| now.saturating_duration_since(t) <= Duration::from_millis(260))
                {
                    self.last_digit1_at = None;
                    self.last_digit4_at = None;
                    toggled = self.toggle_teto(now);
                }
            }
            "Digit1" => {
                self.last_digit1_at = Some(now);
                if self
                    .last_digit4_at
                    .is_some_and(|t| now.saturating_duration_since(t) <= Self::TETO_PRESS_WINDOW)
                {
                    self.last_digit4_at = None;
                    self.last_digit1_at = None;
                    toggled = self.toggle_teto(now);
                }
            }
            _ => {}
        }
        toggled
    }

    fn toggle_miku(&mut self, now: Instant) -> bool {
        if self.miku_visible
            && self
                .last_miku_toggle_at
                .is_some_and(|t| now.saturating_duration_since(t) < Self::TOGGLE_COOLDOWN)
        {
            return false;
        }
        self.miku_visible = !self.miku_visible;
        self.miku_since = self.miku_visible.then_some(now);
        self.last_miku_toggle_at = Some(now);
        self.dirty = true;
        true
    }

    fn toggle_teto(&mut self, now: Instant) -> bool {
        if self.teto_visible
            && self
                .last_teto_toggle_at
                .is_some_and(|t| now.saturating_duration_since(t) < Self::TOGGLE_COOLDOWN)
        {
            return false;
        }
        self.teto_visible = !self.teto_visible;
        self.teto_since = self.teto_visible.then_some(now);
        self.last_teto_toggle_at = Some(now);
        self.dirty = true;
        true
    }

    pub fn miku_pose(&self, now: Instant) -> MascotPose {
        Self::pose_for(self.miku_visible, self.miku_since, now)
    }

    pub fn teto_pose(&self, now: Instant) -> MascotPose {
        Self::pose_for(self.teto_visible, self.teto_since, now)
    }

    fn pose_for(visible: bool, since: Option<Instant>, now: Instant) -> MascotPose {
        if !visible {
            return MascotPose::default();
        }
        let age_s = since.map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        let p = (age_s / 0.28).clamp(0.0, 1.0);
        let smooth = p * p * (3.0 - 2.0 * p);
        let bounce = 1.0 + 0.14 * (1.0 - p) * (p * std::f32::consts::PI).sin();
        MascotPose {
            scale: (smooth * bounce).max(0.05),
            age_s,
        }
    }
}

pub struct ConnectedState {
    pub socket_arg: Option<PathBuf>,
    pub client: DaemonClient,
    pub server_cred: PeerCredentials,
    pub daemon_status: DaemonStatus,
    pub probe: DeviceProbe,
    pub unit_mm: f64,
    pub min_mm: f64,
    pub max_mm: f64,

    pub selected_keys: BTreeSet<u8>,
    pub focused_key: u8,
    pub actuations: BTreeMap<u8, TriggerConfig>,
    pub switches: BTreeMap<u8, u8>,
    pub deadbands: BTreeMap<u8, (u8, u8)>,

    pub draft_mode: TriggerMode,
    pub draft_travel_mm: f64,
    pub draft_rt_press_mm: f64,
    pub draft_rt_release_mm: f64,
    pub draft_switch_id: u8,
    pub draft_db_top_mm: f64,
    pub draft_db_bottom_mm: f64,

    pub light_state: LightState,
    pub light_tab: LightEditorTab,
    pub active_custom_slot: u8,
    pub custom_slots: [[[u8; 3]; MATRIX_SLOTS]; 2],
    pub custom_paint_mode: bool,
    pub custom_brush: [u8; 3],
    pub custom_brightness: u8,

    pub poll_rate_khz: u8,
    pub socd_enabled: bool,
    pub socd_pairs: Vec<PrcsPair>,

    pub travel_enabled: bool,
    pub travel_thread: Option<TravelThread>,
    pub live_depths: BTreeMap<u8, (f64, Instant)>,
    pub last_sample: Option<(u8, f64)>,
    pub recent_impulses: Vec<TrackedImpulse>,
    pub switch_tests: BTreeMap<u8, SwitchTestStats>,

    pub cal_active: bool,
    pub cal_any_key: bool,
    pub cal_calibrated: BTreeSet<u8>,
    pub cal_status_text: String,
    pub cal_thread: Option<CalibThread>,

    pub shortcut_rules: Vec<ShortcutRule>,
    pub shortcut_watcher_active: bool,
    pub shortcut_logs: Vec<ShortcutTriggerLog>,
    pub uinput_ready: bool,
    pub uinput_error: Option<String>,
    pub evdev_grab_active: bool,
    pub shortcut_persist_path: Option<String>,
    pub keyboard_fn_map: BTreeMap<u8, u8>,
    pub next_shortcut_id: u32,
    pub draft_rule_modifier: Option<u8>,
    pub draft_rule_key: u8,
    pub draft_rule_depth_mm: f64,
    pub draft_rule_preset: ShortcutActionPreset,
    pub draft_virtual_key: VirtualKey,
    pub draft_rule_command: String,
    pub shortcut_recording: bool,
    pub record_first_key: Option<u8>,
    pub captured_gesture_banner: Option<String>,
    pub daemon_pulses: BTreeMap<u8, Instant>,
    pub daemon_active_snapshot: BTreeSet<u8>,
    pub daemon_pulsing_snapshot: BTreeSet<u8>,

    pub diag_report: Option<DiagnosticReport>,
    pub echo_latencies_ms: Vec<f64>,
    pub feedback_log: VecDeque<ResponseEvent>,
    pub feedback_seq: u64,
    pub diag_switch_view: SwitchViewMode,
    pub diag_switch_cam: SwitchCamera,
    pub diag_switch_preset: Option<SwitchCameraPreset>,
    pub diag_sweep_active: bool,
    pub diag_sweep_started_at: Option<Instant>,
    pub diag_follow_press: bool,

    pub camera_3d: Kb3dCamera,
    pub camera_preset: Option<CameraPreset>,
    pub light_view_mode: LightViewMode,
    pub animate_3d: bool,
    pub typing_only_effects: bool,
    pub mascots: MascotEasterEggState,

    pub audio_sources: Vec<AudioSourceOption>,
    pub audio_source_idx: usize,
    pub audio_config: AudioLiveConfig,
    pub audio_active: bool,
    pub last_audio_poll: Instant,
    pub audio_status_text: String,
    pub audio_source_label: String,
    pub audio_metrics: Option<AudioFrameMetrics>,
    pub audio_preview_frame: Option<[[u8; 3]; MATRIX_SLOTS]>,
    pub audio_frames_rendered: u64,
    pub audio_packets_total: u64,
    pub audio_packets_last: usize,
    pub audio_actual_fps: f32,

    pub audio_disclaimer_accepted: bool,
    pub audio_disclaimer_modal_open: bool,
    pub audio_disclaimer_opened_at: Option<Instant>,
    pub audio_disclaimer_checked: bool,
    pub audio_disclaimer_pending_start: bool,
}

impl ConnectedState {
    pub fn connect(socket: Option<PathBuf>) -> Result<Self, String> {
        let mut client = DaemonClient::connect(socket.as_deref()).map_err(|e| e.to_string())?;
        let server_cred = client.server_credentials();
        let daemon_status = client.ping().map_err(|e| e.to_string())?;
        let probe = client.probe().map_err(|e| e.to_string())?;

        let all = keymap::all_indices();
        let (unit_mm, actuations_vec) = client
            .get_actuation(all.clone())
            .map_err(|e| e.to_string())?;
        let switches_vec = client
            .get_switches(all.clone())
            .map_err(|e| e.to_string())?;
        let deadbands_vec = client.get_deadband(all).map_err(|e| e.to_string())?;
        let light_state = client.get_light().map_err(|e| e.to_string())?;
        let active_custom_slot = client.get_active_custom_slot().unwrap_or(0);
        let slot0 = load_custom_slot(&mut client, 0);
        let slot1 = load_custom_slot(&mut client, 1);

        let poll_rate_khz = client
            .get_poll_rate()
            .unwrap_or_else(|_| probe.poll_rate_khz.unwrap_or(8));
        let (socd_enabled, socd_pairs) = client
            .get_socd()
            .unwrap_or((probe.prcs_enabled.unwrap_or(false), Vec::new()));
        let (
            shortcut_rules,
            shortcut_watcher_active,
            shortcut_logs,
            uinput_ready,
            uinput_error,
            evdev_grab_active,
            shortcut_persist_path,
            daemon_active_snapshot,
            daemon_pulsing_snapshot,
        ) = match client.get_shortcuts() {
            Ok(snap) => (
                snap.rules,
                snap.watcher_active,
                snap.recent_triggers,
                snap.uinput_ready,
                snap.uinput_error,
                snap.evdev_grab_active,
                snap.persist_path,
                snap.active_indicator_keys.into_iter().collect(),
                snap.pulsing_indicator_keys.into_iter().collect(),
            ),
            Err(_) => (
                Vec::new(),
                true,
                Vec::new(),
                false,
                None,
                false,
                None,
                BTreeSet::new(),
                BTreeSet::new(),
            ),
        };
        let keyboard_fn_map: BTreeMap<u8, u8> = client
            .get_fn_overrides()
            .unwrap_or_default()
            .into_iter()
            .map(|o| (o.index, o.target_hid))
            .collect();
        let next_shortcut_id = shortcut_rules
            .iter()
            .map(|r| r.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);

        let mut actuations = BTreeMap::new();
        for e in actuations_vec {
            if let Some(c) = e.config {
                actuations.insert(e.index, c);
            }
        }
        let mut switches = BTreeMap::new();
        for e in switches_vec {
            switches.insert(e.index, e.switch_id);
        }
        let mut deadbands = BTreeMap::new();
        for e in deadbands_vec {
            deadbands.insert(e.index, (e.top_raw, e.bottom_raw));
        }

        let focused = 46; // W key
        let mut selected = BTreeSet::new();
        selected.insert(focused);

        let min_mm = probe.max_travel.map_or(0.08, |m| m.min_mm);
        let max_mm = probe.max_travel.map_or(3.40, |m| m.max_mm);
        let focus_act = actuations.get(&focused).copied();
        let focus_db = deadbands.get(&focused).copied().unwrap_or((0, 0));
        let draft_switch_id = switches.get(&focused).copied().unwrap_or(1);

        let mut state = Self {
            socket_arg: socket,
            client,
            server_cred,
            daemon_status,
            probe,
            unit_mm,
            min_mm,
            max_mm,
            selected_keys: selected,
            focused_key: focused,
            actuations,
            switches,
            deadbands,
            draft_mode: focus_act.map_or(TriggerMode::Fixed, |c| c.mode),
            draft_travel_mm: focus_act.map_or(1.70, |c| f64::from(c.travel_raw) * unit_mm),
            draft_rt_press_mm: focus_act
                .map_or(0.30, |c| f64::from(c.interval1_raw) * unit_mm)
                .max(0.01),
            draft_rt_release_mm: focus_act
                .map_or(0.30, |c| f64::from(c.interval2_raw) * unit_mm)
                .max(0.01),
            draft_switch_id,
            draft_db_top_mm: f64::from(focus_db.0) * 0.01,
            draft_db_bottom_mm: f64::from(focus_db.1) * 0.01,
            light_state,
            light_tab: LightEditorTab::FirmwareEffects,
            active_custom_slot,
            custom_slots: [slot0, slot1],
            custom_paint_mode: false,
            custom_brush: [0, 216, 255],
            custom_brightness: light_state.brightness.clamp(1, LIGHT_MAX_LEVEL),
            poll_rate_khz,
            socd_enabled,
            socd_pairs,
            travel_enabled: true,
            travel_thread: None,
            live_depths: BTreeMap::new(),
            last_sample: None,
            recent_impulses: Vec::new(),
            switch_tests: BTreeMap::new(),
            cal_active: false,
            cal_any_key: false,
            cal_calibrated: BTreeSet::new(),
            cal_status_text: "Ready (idle — keys untouched)".into(),
            cal_thread: None,
            shortcut_rules,
            shortcut_watcher_active,
            shortcut_logs,
            uinput_ready,
            uinput_error,
            evdev_grab_active,
            shortcut_persist_path,
            keyboard_fn_map,
            next_shortcut_id,
            draft_rule_modifier: Some(keymap::idx::FN),
            draft_rule_key: keymap::idx::T,
            draft_rule_depth_mm: 1.50,
            draft_rule_preset: ShortcutActionPreset::EmitVirtualKey,
            draft_virtual_key: VirtualKey::Up,
            draft_rule_command: "notify-send 'Reaula WIN 60 HE' 'Hotkey triggered'".into(),
            shortcut_recording: false,
            record_first_key: None,
            captured_gesture_banner: None,
            daemon_pulses: BTreeMap::new(),
            daemon_active_snapshot,
            daemon_pulsing_snapshot,
            diag_report: None,
            echo_latencies_ms: Vec::new(),
            feedback_log: VecDeque::new(),
            feedback_seq: 0,
            diag_switch_view: SwitchViewMode::ThreeD,
            diag_switch_cam: SwitchCamera::default(),
            diag_switch_preset: Some(SwitchCameraPreset::Iso),
            diag_sweep_active: false,
            diag_sweep_started_at: None,
            diag_follow_press: true,
            camera_3d: Kb3dCamera::from_preset(CameraPreset::Perspective),
            camera_preset: Some(CameraPreset::Perspective),
            light_view_mode: LightViewMode::ThreeD,
            animate_3d: true,
            typing_only_effects: false,
            mascots: MascotEasterEggState::default(),
            audio_sources: build_audio_source_options(),
            audio_source_idx: 0,
            audio_config: AudioLiveConfig {
                style: AudioEffectStyle::Spectrum,
                palette: AudioPalette::Aurora,
                fg: [56, 189, 248],
                bg: [0, 0, 0],
                brightness: light_state.brightness.clamp(1, LIGHT_MAX_LEVEL),
                fps: 35,
                gain: 1.2,
            },
            audio_active: false,
            last_audio_poll: Instant::now(),
            audio_status_text: "Idle (ready to run equalizer in reaulad)".into(),
            audio_source_label: "Not streaming".into(),
            audio_metrics: None,
            audio_preview_frame: None,
            audio_frames_rendered: 0,
            audio_packets_total: 0,
            audio_packets_last: 0,
            audio_actual_fps: 0.0,
            audio_disclaimer_accepted: false,
            audio_disclaimer_modal_open: false,
            audio_disclaimer_opened_at: None,
            audio_disclaimer_checked: false,
            audio_disclaimer_pending_start: false,
        };
        if let Ok(st) = state.client.get_audio_sync_state() {
            let active = st.active;
            state.apply_audio_status(st, active);
        }
        let _ = state.start_travel();
        state.record_feedback("Connect & Probe", 0.0, true, "Connected to reaulad".into());
        Ok(state)
    }

    pub fn keyboard_id(&self) -> Option<&str> {
        self.daemon_status
            .keyboard_id
            .as_deref()
            .or(self.probe.keyboard_id.as_deref())
    }

    pub fn model_label(&self) -> &str {
        self.daemon_status
            .keyboard_name
            .as_deref()
            .or(self.probe.model_name.as_deref())
            .unwrap_or("AULA WIN 60 HE")
    }

    pub fn record_feedback(&mut self, label: impl Into<String>, rtt_ms: f64, ok: bool, detail: String) {
        self.feedback_seq += 1;
        self.feedback_log.push_front(ResponseEvent {
            id: self.feedback_seq,
            label: label.into(),
            rtt_ms,
            ok,
            detail,
        });
        while self.feedback_log.len() > MAX_FEEDBACK_EVENTS {
            self.feedback_log.pop_back();
        }
    }

    fn call_with_reconnect<T>(
        &mut self,
        mut f: impl FnMut(&mut DaemonClient) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        let res = f(&mut self.client);
        match res {
            Ok(v) => Ok(v),
            Err(err) if is_recoverable_ipc_error(&err) => {
                if let Ok(new_client) = DaemonClient::connect(self.socket_arg.as_deref()) {
                    self.server_cred = new_client.server_credentials();
                    self.client = new_client;
                    if self.travel_enabled && !self.cal_active {
                        let _ = self.start_travel();
                    }
                    f(&mut self.client)
                } else {
                    Err(err)
                }
            }
            Err(err) => Err(err),
        }
    }

    pub fn run_measured<T, F>(&mut self, label: &str, f: F) -> Result<T, IpcError>
    where
        F: FnMut(&mut DaemonClient) -> Result<T, IpcError>,
    {
        let t0 = Instant::now();
        let res = self.call_with_reconnect(f);
        let rtt_ms = t0.elapsed().as_secs_f64() * 1000.0;
        match &res {
            Ok(_) => self.record_feedback(label, rtt_ms, true, "OK".into()),
            Err(err) => self.record_feedback(label, rtt_ms, false, err.to_string()),
        }
        res
    }

    pub fn start_travel(&mut self) -> Result<(), String> {
        if self.cal_active {
            return Err("Stop calibration before streaming live key travel.".into());
        }
        self.travel_thread = None;
        let sub_client = DaemonClient::connect(self.socket_arg.as_deref()).map_err(|e| e.to_string())?;
        let mut sub = sub_client
            .subscribe_travel(keymap::all_indices())
            .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);

        thread::Builder::new()
            .name("reaula-gui-travel".into())
            .spawn(move || {
                while !stop_bg.load(Ordering::Relaxed) {
                    match sub.next_sample() {
                        Ok(Some(item)) => {
                            if tx.send(item).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
                sub.close();
            })
            .map_err(|e| e.to_string())?;

        self.travel_thread = Some(TravelThread { stop, rx });
        self.travel_enabled = true;
        Ok(())
    }

    pub fn stop_travel(&mut self) {
        self.travel_thread = None;
        self.travel_enabled = false;
        self.live_depths.clear();
    }

    pub fn clear_stuck_travel(&mut self) {
        self.live_depths.clear();
        self.last_sample = None;
        for st in self.switch_tests.values_mut() {
            st.current_mm = 0.0;
            st.was_pressed = false;
            st.released_zero = true;
        }
    }

    pub fn reset_switch_tests(&mut self) {
        self.switch_tests.clear();
        self.live_depths.clear();
        self.last_sample = None;
    }

    pub fn drain_travel(&mut self, auto_focus_pressed: bool) -> bool {
        let Some(t) = &self.travel_thread else {
            return false;
        };
        let mut batch = Vec::new();
        while let Ok(item) = t.rx.try_recv() {
            batch.push(item);
        }
        let now = Instant::now();
        let mut changed = false;
        for (sample, unit_mm) in batch {
            let mm = sample.depth_mm(unit_mm);
            if mm <= 0.015 {
                self.live_depths.remove(&sample.index);
            } else {
                self.live_depths.insert(sample.index, (mm, now));
            }
            self.last_sample = Some((sample.index, mm));

            let entry = self.switch_tests.entry(sample.index).or_insert(SwitchTestStats {
                current_mm: mm,
                peak_mm: mm,
                released_zero: mm <= 0.02,
                presses: 0,
                was_pressed: false,
                last_seen: now,
            });
            let newly_pressed = !entry.was_pressed && mm >= 0.30;
            entry.current_mm = mm;
            entry.last_seen = now;
            if mm > entry.peak_mm {
                entry.peak_mm = mm;
            }
            if newly_pressed {
                entry.was_pressed = true;
                entry.presses += 1;
                entry.released_zero = false;
                if auto_focus_pressed {
                    self.focused_key = sample.index;
                }
                if let Some(kdef) = keymap::by_index(sample.index) {
                    let nx = ((kdef.x as f32 + kdef.width as f32 * 0.5) - 10.0) / 577.0;
                    let ny = ((kdef.y as f32 + kdef.height as f32 * 0.5) - 10.0) / 207.0;
                    self.recent_impulses.push(TrackedImpulse {
                        index: sample.index,
                        nx,
                        ny,
                        trigger_at: now,
                        strength: ((mm / self.max_mm.max(1.0)) as f32).clamp(0.45, 1.0),
                    });
                    self.mascots
                        .record_code(MascotKeySource::Hardware, kdef.code, now);
                } else {
                    self.mascots
                        .record_code(MascotKeySource::Hardware, "", now);
                }
            } else if entry.was_pressed && mm <= 0.02 {
                entry.was_pressed = false;
                entry.released_zero = true;
            }

            if self.shortcut_watcher_active {
                for rule in &self.shortcut_rules {
                    if !rule.enabled || rule.trigger_key != sample.index {
                        continue;
                    }
                    let mod_ok = rule
                        .modifier_key
                        .is_none_or(|m| self.live_depths.get(&m).is_some_and(|(d, _)| *d >= 0.35));
                    if mod_ok && mm >= rule.trigger_depth_mm {
                        self.daemon_pulses.insert(rule.trigger_key, now);
                        if let Some(m) = rule.modifier_key {
                            self.daemon_pulses.entry(m).or_insert(now);
                        }
                    }
                }
            }

            if self.shortcut_recording {
                self.process_gesture_capture(sample.index, mm);
            }
            changed = true;
        }
        // Expire old typing impulses (> 3.1s for Fireworks tail), daemon pulses (> 650ms), and stale holds.
        let before_imp = self.recent_impulses.len();
        self.recent_impulses
            .retain(|imp| now.duration_since(imp.trigger_at) < Duration::from_millis(3100));
        let before_pulse = self.daemon_pulses.len();
        self.daemon_pulses
            .retain(|_, ts| now.duration_since(*ts) < Duration::from_millis(650));
        let before = self.live_depths.len();
        self.live_depths
            .retain(|_, (_, ts)| now.duration_since(*ts) < Duration::from_millis(1200));
        changed
            || self.live_depths.len() != before
            || self.recent_impulses.len() != before_imp
            || self.daemon_pulses.len() != before_pulse
    }

    pub fn active_impulses(&self) -> Vec<KeyImpulse> {
        let now = Instant::now();
        self.recent_impulses
            .iter()
            .filter_map(|imp| {
                let age_s = now.duration_since(imp.trigger_at).as_secs_f32();
                (age_s <= 3.0).then_some(KeyImpulse {
                    index: imp.index,
                    nx: imp.nx,
                    ny: imp.ny,
                    age_s,
                    strength: imp.strength,
                })
            })
            .collect()
    }

    pub fn is_fn_held(&self) -> bool {
        self.live_depths
            .get(&keymap::idx::FN)
            .is_some_and(|(d, _)| *d >= 0.35)
    }

    /// Active daemon memory-mapping indicator key slots `(glowing_aqua, bright_pulsing)`.
    pub fn active_daemon_indicators(&self) -> (BTreeSet<u8>, BTreeSet<u8>) {
        if !self.shortcut_watcher_active {
            return (BTreeSet::new(), BTreeSet::new());
        }
        let now = Instant::now();
        let mut active: BTreeSet<u8> = self.daemon_active_snapshot.clone();
        let mut pulsing: BTreeSet<u8> = self.daemon_pulsing_snapshot.clone();

        for rule in &self.shortcut_rules {
            if !rule.enabled {
                continue;
            }
            if let Some(m) = rule.modifier_key
                && self.live_depths.get(&m).is_some_and(|(d, _)| *d >= 0.35)
            {
                active.insert(m);
                active.insert(rule.trigger_key);
                if self
                    .live_depths
                    .get(&rule.trigger_key)
                    .is_some_and(|(d, _)| *d >= rule.trigger_depth_mm)
                {
                    pulsing.insert(rule.trigger_key);
                }
            }
        }

        for (&idx, &ts) in &self.daemon_pulses {
            if now.saturating_duration_since(ts) < Duration::from_millis(650) {
                active.insert(idx);
                pulsing.insert(idx);
            }
        }

        (active, pulsing)
    }

    /// Return the live mechanical/Hall-effect inspection state for the currently focused key.
    pub fn focused_switch_inspection(&self) -> SwitchInspectionSnapshot {
        let key_index = self.focused_key;
        let key_name = keymap::by_index(key_index).map_or("Key", |k| k.name);
        let switch_id = self.switches.get(&key_index).copied().unwrap_or(1);
        let switch_label = protocol::switch_profile_name(switch_id);
        let max_mm = self.max_mm.max(0.5);

        let act = self.actuations.get(&key_index).copied();
        let actuation_mm = act
            .map(|c| f64::from(c.travel_raw) * self.unit_mm)
            .unwrap_or(1.70)
            .clamp(self.min_mm, max_mm);
        let rapid_trigger = act.is_some_and(|c| c.mode != TriggerMode::Fixed);
        let rt_press_mm = act
            .map(|c| (f64::from(c.interval1_raw) * self.unit_mm).max(0.01))
            .unwrap_or(0.30);
        let rt_release_mm = act
            .map(|c| {
                if c.mode == TriggerMode::RapidTriggerSplit {
                    (f64::from(c.interval2_raw) * self.unit_mm).max(0.01)
                } else {
                    (f64::from(c.interval1_raw) * self.unit_mm).max(0.01)
                }
            })
            .unwrap_or(0.30);

        let (db_top_raw, db_bot_raw) = self.deadbands.get(&key_index).copied().unwrap_or((0, 0));
        let db_top_mm = f64::from(db_top_raw) * 0.01;
        let db_bottom_mm = f64::from(db_bot_raw) * 0.01;

        let live = self.live_depths.get(&key_index).map(|(m, _)| *m);
        let (travel_mm, sweeping) = if let Some(mm) = live {
            (mm.clamp(0.0, max_mm), false)
        } else if self.diag_sweep_active
            && let Some(t0) = self.diag_sweep_started_at
        {
            let phase = (t0.elapsed().as_secs_f64() * 2.4).cos();
            let norm = (0.5 - 0.5 * phase).clamp(0.0, 1.0);
            (norm * max_mm, true)
        } else {
            (0.0, false)
        };

        let test_stat = self.switch_tests.get(&key_index);
        let peak_mm = test_stat.map_or(travel_mm, |s| s.peak_mm.max(travel_mm));
        let presses = test_stat.map_or(0, |s| s.presses);
        let released_zero = test_stat.is_some_and(|s| s.released_zero);

        let triggered = travel_mm >= actuation_mm;
        let frac = (travel_mm / max_mm).clamp(0.0, 1.0);
        let flux_pct = ((0.22 + 0.78 * frac) * frac * 100.0).clamp(0.0, 100.0);

        SwitchInspectionSnapshot {
            key_index,
            key_name,
            switch_id,
            switch_label,
            travel_mm,
            max_mm,
            peak_mm,
            presses,
            released_zero,
            actuation_mm,
            rapid_trigger,
            rt_press_mm,
            rt_release_mm,
            db_top_mm,
            db_bottom_mm,
            triggered,
            sweeping,
            flux_pct,
        }
    }

    fn process_gesture_capture(&mut self, index: u8, mm: f64) {
        if mm >= 0.85 {
            match self.record_first_key {
                None => {
                    self.record_first_key = Some(index);
                    let first_name = keymap::by_index(index).map_or("Key", |k| k.name);
                    self.captured_gesture_banner = Some(format!(
                        "Holding {first_name}... press 2nd key for '{first_name} + Key' chord, or release {first_name} for single key."
                    ));
                }
                Some(first) if first != index => {
                    self.draft_rule_modifier = Some(first);
                    self.draft_rule_key = index;
                    self.shortcut_recording = false;
                    self.record_first_key = None;
                    let m_name = keymap::by_index(first).map_or("Mod", |k| k.name);
                    let t_name = keymap::by_index(index).map_or("Key", |k| k.name);
                    self.captured_gesture_banner = Some(format!(
                        "Memorized chord gesture: {m_name} + {t_name}!"
                    ));
                }
                Some(_) => {}
            }
        } else if mm <= 0.15 && self.record_first_key == Some(index) {
            self.draft_rule_modifier = None;
            self.draft_rule_key = index;
            self.shortcut_recording = false;
            self.record_first_key = None;
            let t_name = keymap::by_index(index).map_or("Key", |k| k.name);
            self.captured_gesture_banner = Some(format!("Memorized single key gesture: {t_name}!"));
        }
    }

    pub fn live_depths_array(&self) -> [f32; MATRIX_SLOTS] {
        let mut arr = [0.0f32; MATRIX_SLOTS];
        for (&idx, &(mm, _)) in &self.live_depths {
            if let Some(slot) = arr.get_mut(idx as usize) {
                *slot = mm as f32;
            }
        }
        arr
    }

    pub fn start_calibration(&mut self, any_key: bool) -> Result<(), String> {
        self.stop_calibration();
        if self.travel_enabled {
            self.stop_travel();
        }
        self.cal_calibrated.clear();
        self.cal_any_key = any_key;

        let sub_client = DaemonClient::connect(self.socket_arg.as_deref()).map_err(|e| e.to_string())?;
        let mut sub = sub_client
            .start_calibration(any_key)
            .map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);

        thread::Builder::new()
            .name("reaula-gui-calib".into())
            .spawn(move || {
                while !stop_bg.load(Ordering::Relaxed) {
                    match sub.next_update() {
                        Ok(Some(prog)) => {
                            let done = !matches!(prog.status, CalibrationStatus::Progress);
                            let _ = tx.send(prog);
                            if done {
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(_) => break,
                    }
                }
                sub.stop();
            })
            .map_err(|e| e.to_string())?;

        self.cal_thread = Some(CalibThread { stop, rx });
        self.cal_active = true;
        self.cal_status_text = if any_key {
            "Any-Key calibration armed — press key(s) firmly to full bottom-out, then click Finish.".into()
        } else {
            "All-Key calibration armed — press EVERY key smoothly to bottom-out until 61/61.".into()
        };
        self.record_feedback(
            "Start Calibration",
            0.0,
            true,
            if any_key { "Any-key armed".into() } else { "All-key armed".into() },
        );
        Ok(())
    }

    pub fn stop_calibration(&mut self) {
        let was_active = self.cal_active || self.cal_thread.is_some();
        self.cal_thread = None;
        self.cal_active = false;
        if was_active {
            self.cal_status_text = format!(
                "Disarmed ({} / {} keys recorded).",
                self.cal_calibrated.len(),
                keymap::WIN60HE_KEYS.len()
            );
            self.record_feedback("Stop Calibration", 0.0, true, self.cal_status_text.clone());
        }
    }

    pub fn drain_calibration(&mut self) -> bool {
        let Some(t) = &self.cal_thread else {
            return false;
        };
        let mut changed = false;
        let mut finished = false;
        while let Ok(prog) = t.rx.try_recv() {
            match prog.status {
                CalibrationStatus::Progress => {
                    for idx in prog.calibrated_indices {
                        self.cal_calibrated.insert(idx);
                    }
                    self.cal_status_text = format!(
                        "Calibrating: {}/{} keys completed",
                        self.cal_calibrated.len(),
                        keymap::WIN60HE_KEYS.len()
                    );
                }
                CalibrationStatus::Finished => {
                    self.cal_status_text = format!(
                        "Calibration completed ({} keys stored).",
                        self.cal_calibrated.len()
                    );
                    finished = true;
                }
                CalibrationStatus::Failed(code) => {
                    self.cal_status_text = format!("Calibration failed (firmware code {code}).");
                    finished = true;
                }
                CalibrationStatus::Unknown(code) => {
                    self.cal_status_text = format!("Calibration halted (status code {code}).");
                    finished = true;
                }
            }
            changed = true;
        }
        if finished {
            self.cal_thread = None;
            self.cal_active = false;
            self.record_feedback("Calibration Result", 0.0, true, self.cal_status_text.clone());
        }
        changed
    }

    pub fn run_diagnostics(&mut self) -> Result<DiagnosticReport, String> {
        let t0 = Instant::now();
        let rep = self
            .call_with_reconnect(|c| c.run_diagnostics())
            .map_err(|e| e.to_string())?;
        let rtt = t0.elapsed().as_secs_f64() * 1000.0;
        let summary = rep.attribution.summary.clone();
        self.diag_report = Some(rep.clone());
        self.record_feedback("4-Layer Diagnostics", rtt, rep.attribution.primary_fault.is_none(), summary);
        Ok(rep)
    }

    pub fn run_echo_benchmark(&mut self) -> Result<String, String> {
        self.echo_latencies_ms.clear();
        for _ in 0..5 {
            let t0 = Instant::now();
            self.call_with_reconnect(|c| c.get_poll_rate())
                .map_err(|e| e.to_string())?;
            self.echo_latencies_ms
                .push(t0.elapsed().as_secs_f64() * 1000.0);
        }
        let min = self
            .echo_latencies_ms
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let max = self
            .echo_latencies_ms
            .iter()
            .copied()
            .fold(0.0f64, f64::max);
        let avg = self.echo_latencies_ms.iter().sum::<f64>()
            / (self.echo_latencies_ms.len().max(1) as f64);
        let msg = format!("5x HID Echo RTT: min={min:.2} ms  avg={avg:.2} ms  max={max:.2} ms");
        self.record_feedback("HID Echo Burst (5x)", avg, true, msg.clone());
        Ok(msg)
    }

    fn apply_shortcuts_snapshot(&mut self, snap: ShortcutsSnapshot) {
        self.shortcut_rules = snap.rules;
        self.shortcut_watcher_active = snap.watcher_active;
        self.shortcut_logs = snap.recent_triggers;
        self.uinput_ready = snap.uinput_ready;
        self.uinput_error = snap.uinput_error;
        self.evdev_grab_active = snap.evdev_grab_active;
        self.shortcut_persist_path = snap.persist_path;
        self.daemon_active_snapshot = snap.active_indicator_keys.into_iter().collect();
        self.daemon_pulsing_snapshot = snap.pulsing_indicator_keys.into_iter().collect();
    }

    pub fn poll_shortcut_logs_quiet(&mut self) -> bool {
        let Ok(snap) = self.call_with_reconnect(|c| c.get_shortcuts()) else {
            return false;
        };
        let next_active: BTreeSet<u8> = snap.active_indicator_keys.iter().copied().collect();
        let next_pulsing: BTreeSet<u8> = snap.pulsing_indicator_keys.iter().copied().collect();
        let changed = snap.recent_triggers.len() != self.shortcut_logs.len()
            || snap.recent_triggers.first().map(|l| l.unix_ms)
                != self.shortcut_logs.first().map(|l| l.unix_ms)
            || snap.uinput_ready != self.uinput_ready
            || snap.uinput_error != self.uinput_error
            || snap.evdev_grab_active != self.evdev_grab_active
            || next_active != self.daemon_active_snapshot
            || next_pulsing != self.daemon_pulsing_snapshot;
        if changed {
            self.shortcut_logs = snap.recent_triggers;
            self.uinput_ready = snap.uinput_ready;
            self.uinput_error = snap.uinput_error;
            self.evdev_grab_active = snap.evdev_grab_active;
            self.shortcut_persist_path = snap.persist_path;
            self.daemon_active_snapshot = next_active;
            self.daemon_pulsing_snapshot = next_pulsing;
        }
        changed
    }

    pub fn sync_shortcuts(&mut self) -> Result<(), String> {
        let rules = self.shortcut_rules.clone();
        let watcher = self.shortcut_watcher_active;
        let snap = self
            .run_measured("Sync Hotkey Rules", |c| {
                c.set_shortcuts(rules.clone(), watcher)
            })
            .map_err(|e| e.to_string())?;
        self.apply_shortcuts_snapshot(snap);
        Ok(())
    }

    pub fn refresh_shortcuts(&mut self) -> Result<(), String> {
        let snap = self
            .run_measured("Refresh Hotkeys", |c| c.get_shortcuts())
            .map_err(|e| e.to_string())?;
        self.apply_shortcuts_snapshot(snap);
        Ok(())
    }

    pub fn trigger_shortcut_rule(&mut self, id: u32) -> Result<ShortcutTriggerLog, String> {
        let matched = self.shortcut_rules.iter().find(|r| r.id == id).cloned();
        let log = self
            .run_measured("Test Fire Shortcut", |c| c.trigger_shortcut(id))
            .map_err(|e| e.to_string())?;
        if let Some(rule) = matched {
            let now = Instant::now();
            self.daemon_pulses.insert(rule.trigger_key, now);
            if let Some(m) = rule.modifier_key {
                self.daemon_pulses.entry(m).or_insert(now);
            }
        }
        if let Ok(snap) = self.call_with_reconnect(|c| c.get_shortcuts()) {
            self.apply_shortcuts_snapshot(snap);
        }
        Ok(log)
    }

    pub fn build_draft_shortcut_rule(&mut self) -> ShortcutRule {
        let id = self.next_shortcut_id;
        self.next_shortcut_id += 1;
        let trig_name = keymap::by_index(self.draft_rule_key)
            .map_or("Key", |k| k.name);
        let gesture = match self.draft_rule_modifier {
            Some(m) => {
                let mname = keymap::by_index(m).map_or("Mod", |k| k.name);
                format!("{mname}+{trig_name}")
            }
            None => trig_name.to_string(),
        };
        let (action, act_label) = match self.draft_rule_preset {
            ShortcutActionPreset::EmitVirtualKey => (
                ShortcutAction::EmitKey {
                    key: self.draft_virtual_key,
                },
                self.draft_virtual_key.label(),
            ),
            ShortcutActionPreset::FpsRapidTrigger => (
                ShortcutAction::ActuationPreset {
                    name: "FPS RT WASD".into(),
                    mode: TriggerMode::RapidTrigger,
                    indices: vec![keymap::idx::W, keymap::idx::A, keymap::idx::S, keymap::idx::D],
                    depth_mm: 0.30,
                    rt_press_mm: 0.15,
                    rt_release_mm: 0.15,
                },
                "FPS RT WASD",
            ),
            ShortcutActionPreset::TypingComfort => (
                ShortcutAction::ActuationPreset {
                    name: "Typing 1.80mm".into(),
                    mode: TriggerMode::Fixed,
                    indices: keymap::all_indices(),
                    depth_mm: 1.80,
                    rt_press_mm: 0.30,
                    rt_release_mm: 0.30,
                },
                "Typing 1.8mm",
            ),
            ShortcutActionPreset::ToggleSocd => (ShortcutAction::ToggleSocd, "Toggle SOCD"),
            ShortcutActionPreset::ToggleLighting => (ShortcutAction::ToggleLighting, "Toggle LED"),
            ShortcutActionPreset::NeonWaveLighting => (
                ShortcutAction::LightingPreset {
                    name: "Wave Cyan".into(),
                    state: LightState {
                        mode: 2,
                        brightness: 4,
                        speed: 3,
                        fg: [0, 216, 255],
                        bg: [0, 0, 0],
                        direction: 0,
                        full_color: true,
                        power_on: true,
                    },
                },
                "Wave Cyan RGB",
            ),
            ShortcutActionPreset::PollRate8k => (ShortcutAction::SetPollRate { khz: 8 }, "8 kHz Poll"),
            ShortcutActionPreset::RunCommand => (
                ShortcutAction::RunCommand {
                    command: self.draft_rule_command.trim().to_string(),
                },
                "Shell Command",
            ),
        };

        ShortcutRule {
            id,
            name: format!("{gesture} -> {act_label}"),
            enabled: true,
            modifier_key: self.draft_rule_modifier,
            trigger_key: self.draft_rule_key,
            trigger_depth_mm: self.draft_rule_depth_mm,
            action,
        }
    }

    pub fn add_fn_arrow_cluster(&mut self, preset: ArrowClusterPreset) {
        for (trig, vk, label) in arrow_cluster_bindings(preset) {
            if let Some(existing) = self.shortcut_rules.iter_mut().find(|r| {
                r.modifier_key == Some(keymap::idx::FN) && r.trigger_key == trig
            }) {
                existing.enabled = true;
                existing.name = label.into();
                existing.trigger_depth_mm = self.draft_rule_depth_mm;
                existing.action = ShortcutAction::EmitKey { key: vk };
            } else {
                let id = self.next_shortcut_id;
                self.next_shortcut_id += 1;
                self.shortcut_rules.push(ShortcutRule {
                    id,
                    name: label.into(),
                    enabled: true,
                    modifier_key: Some(keymap::idx::FN),
                    trigger_key: trig,
                    trigger_depth_mm: self.draft_rule_depth_mm,
                    action: ShortcutAction::EmitKey { key: vk },
                });
            }
        }
        self.shortcut_watcher_active = true;
    }

    fn apply_fn_overrides_list(&mut self, list: Vec<FnOverride>) {
        self.keyboard_fn_map = list
            .into_iter()
            .map(|o| (o.index, o.target_hid))
            .collect();
    }

    fn mute_overlapping_daemon_fn_emitters(&mut self, indices: &[u8]) {
        let mut touched = false;
        for rule in &mut self.shortcut_rules {
            if rule.enabled
                && rule.modifier_key == Some(keymap::idx::FN)
                && indices.contains(&rule.trigger_key)
                && matches!(rule.action, ShortcutAction::EmitKey { .. })
            {
                rule.enabled = false;
                touched = true;
            }
        }
        if touched {
            let _ = self.sync_shortcuts();
        }
    }

    pub fn refresh_keyboard_fn_map(&mut self) -> Result<(), String> {
        let list = self
            .run_measured("Read KB Fn Flash", |c| c.get_fn_overrides())
            .map_err(|e| e.to_string())?;
        self.apply_fn_overrides_list(list);
        Ok(())
    }

    pub fn save_draft_to_keyboard_fn(&mut self) -> Result<String, String> {
        if self.draft_rule_modifier != Some(keymap::idx::FN) {
            return Err("Hardware KB flash stores Fn + Key chords only. Choose Modifier: Fn, or save to Daemon Backup.".into());
        }
        if self.draft_rule_preset != ShortcutActionPreset::EmitVirtualKey {
            return Err("Hardware KB flash stores key remaps only (Emit OS Key). Use Daemon Backup for Macros/Profiles/Shell.".into());
        }
        let Some(target_hid) = keymap::virtual_key_to_hid_usage(self.draft_virtual_key) else {
            return Err(format!(
                "{} is a media/consumer usage; keep it on Daemon Backup instead.",
                self.draft_virtual_key.label()
            ));
        };
        let idx = self.draft_rule_key;
        let list = self
            .run_measured("Write Fn Rule to KB", |c| {
                c.apply_keyboard_fn_map(
                    vec![FnOverride {
                        index: idx,
                        target_hid,
                    }],
                    Vec::new(),
                )
            })
            .map_err(|e| e.to_string())?;
        self.apply_fn_overrides_list(list);
        self.mute_overlapping_daemon_fn_emitters(&[idx]);
        let key_name = keymap::by_index(idx).map_or("Key", |k| k.name);
        Ok(format!(
            "Saved Fn+{key_name} -> {} to WIN 60 HE onboard flash!",
            self.draft_virtual_key.label()
        ))
    }

    pub fn save_arrow_cluster_to_keyboard(
        &mut self,
        preset: ArrowClusterPreset,
    ) -> Result<String, String> {
        let bindings = arrow_cluster_bindings(preset);
        let mut set = Vec::with_capacity(bindings.len());
        let mut indices = Vec::with_capacity(bindings.len());
        for (idx, vk, _) in bindings {
            let Some(target_hid) = keymap::virtual_key_to_hid_usage(vk) else {
                return Err("Missing USB HID mapping for arrow key".into());
            };
            set.push(FnOverride {
                index: idx,
                target_hid,
            });
            indices.push(idx);
        }
        let list = self
            .run_measured("Write Fn Arrows to KB", |c| {
                c.apply_keyboard_fn_map(set.clone(), Vec::new())
            })
            .map_err(|e| e.to_string())?;
        self.apply_fn_overrides_list(list);
        self.mute_overlapping_daemon_fn_emitters(&indices);
        Ok(match preset {
            ArrowClusterPreset::Rdfg => "Saved Fn+R/D/F/G Arrows to WIN 60 HE flash!".into(),
            ArrowClusterPreset::Tfgh => "Saved Fn+T/F/G/H Arrows to WIN 60 HE flash!".into(),
            ArrowClusterPreset::Wasd => "Saved Fn+W/A/S/D Arrows to WIN 60 HE flash!".into(),
        })
    }

    pub fn clear_keyboard_fn_slot(&mut self, index: u8) -> Result<(), String> {
        let list = self
            .run_measured("Clear KB Fn Slot", |c| {
                c.apply_keyboard_fn_map(Vec::new(), vec![index])
            })
            .map_err(|e| e.to_string())?;
        self.apply_fn_overrides_list(list);
        Ok(())
    }

    pub fn reset_keyboard_fn_map(&mut self) -> Result<(), String> {
        let list = self
            .run_measured("Reset KB Fn Flash", |c| c.reset_keyboard_fn_map())
            .map_err(|e| e.to_string())?;
        self.apply_fn_overrides_list(list);
        Ok(())
    }

    pub fn target_indices(&self) -> Vec<u8> {
        if self.selected_keys.is_empty() {
            vec![self.focused_key]
        } else {
            self.selected_keys.iter().copied().collect()
        }
    }

    /// Return the custom per-key RGB grid to preview in both 2D and 3D views, if applicable.
    pub fn effective_preview_custom_frame(&self) -> Option<&[[u8; 3]; MATRIX_SLOTS]> {
        if self.audio_active
            && (self.light_tab == LightEditorTab::AudioSync
                || self.light_tab.custom_slot_index().is_none())
            && let Some(frame) = &self.audio_preview_frame
        {
            return Some(frame);
        }
        self.light_tab
            .custom_slot_index()
            .map(|s| &self.custom_slots[usize::from(s)])
    }

    pub fn sync_draft_from_key(&mut self, index: u8) {
        self.focused_key = index;
        if let Some(c) = self.actuations.get(&index) {
            self.draft_mode = c.mode;
            self.draft_travel_mm = f64::from(c.travel_raw) * self.unit_mm;
            self.draft_rt_press_mm = (f64::from(c.interval1_raw) * self.unit_mm).max(0.01);
            self.draft_rt_release_mm = (f64::from(c.interval2_raw) * self.unit_mm).max(0.01);
        }
        if let Some(&sw) = self.switches.get(&index) {
            self.draft_switch_id = sw;
        }
        if let Some(&(t, b)) = self.deadbands.get(&index) {
            self.draft_db_top_mm = f64::from(t) * 0.01;
            self.draft_db_bottom_mm = f64::from(b) * 0.01;
        }
    }

    /// Refresh selectable ALSA capture devices while preserving currently picked choice if present.
    pub fn refresh_audio_sources(&mut self) {
        let prev = self.selected_audio_source().clone();
        self.audio_sources = build_audio_source_options();
        self.audio_source_idx = self
            .audio_sources
            .iter()
            .position(|opt| opt == &prev)
            .unwrap_or(0);
    }

    pub fn selected_audio_source(&self) -> &AudioSourceOption {
        self.audio_sources
            .get(self.audio_source_idx)
            .unwrap_or(&self.audio_sources[0])
    }

    /// Open the liability disclaimer modal (starting a fresh 10-second cooldown while unaccepted).
    pub fn open_audio_disclaimer(&mut self, pending_start: bool) {
        self.audio_disclaimer_modal_open = true;
        self.audio_disclaimer_pending_start = pending_start;
        if !self.audio_disclaimer_accepted {
            self.audio_disclaimer_opened_at = Some(Instant::now());
            self.audio_disclaimer_checked = false;
        } else {
            self.audio_disclaimer_checked = true;
        }
    }

    pub fn close_audio_disclaimer(&mut self) {
        self.audio_disclaimer_modal_open = false;
        self.audio_disclaimer_pending_start = false;
    }

    /// Seconds remaining on the mandatory 10s disclaimer cooldown (`0.0` once elapsed).
    pub fn disclaimer_remaining_secs(&self) -> f64 {
        if self.audio_disclaimer_accepted {
            return 0.0;
        }
        let Some(opened_at) = self.audio_disclaimer_opened_at else {
            return AUDIO_DISCLAIMER_COOLDOWN_SECS;
        };
        (AUDIO_DISCLAIMER_COOLDOWN_SECS - opened_at.elapsed().as_secs_f64()).max(0.0)
    }

    pub fn can_confirm_audio_disclaimer(&self) -> bool {
        self.audio_disclaimer_checked && self.disclaimer_remaining_secs() <= 0.0
    }

    /// Record explicit user acknowledgement once both the 10s cooldown and checkbox are satisfied.
    pub fn confirm_audio_disclaimer(&mut self) -> Result<bool, String> {
        let rem = self.disclaimer_remaining_secs();
        if rem > 0.0 {
            return Err(format!(
                "Please wait {:.0}s to finish reading the hardware disclaimer.",
                rem.ceil()
            ));
        }
        if !self.audio_disclaimer_checked {
            return Err("Check the 'I Agree' liability acknowledgement box first.".into());
        }
        self.audio_disclaimer_accepted = true;
        let should_start = self.audio_disclaimer_pending_start;
        self.audio_disclaimer_modal_open = false;
        self.audio_disclaimer_pending_start = false;
        Ok(should_start)
    }

    pub fn select_audio_source_ipc(&mut self, src: &AudioSyncSource) {
        let opt = AudioSourceOption::from_ipc(src);
        if let Some(pos) = self.audio_sources.iter().position(|o| o == &opt) {
            self.audio_source_idx = pos;
        } else {
            self.audio_sources.push(opt);
            self.audio_source_idx = self.audio_sources.len() - 1;
        }
    }

    fn apply_audio_status(&mut self, st: AudioSyncStatus, sync_config: bool) {
        let was_active = self.audio_active;
        self.audio_active = st.active;
        self.audio_source_label = st.source_label;
        self.audio_metrics = if st.active { Some(st.metrics) } else { None };
        if st.active && st.frame.len() == MATRIX_SLOTS {
            let mut arr = [[0u8; 3]; MATRIX_SLOTS];
            arr.copy_from_slice(&st.frame);
            self.audio_preview_frame = Some(arr);
        } else if !st.active {
            self.audio_preview_frame = None;
        }
        self.audio_frames_rendered = st.frames_rendered;
        self.audio_packets_total = st.packets_total;
        self.audio_packets_last = st.packets_last;
        self.audio_actual_fps = st.actual_fps;

        if sync_config {
            self.audio_config.apply_from_ipc(&st.config);
            self.select_audio_source_ipc(&st.config.source);
        }

        if st.active {
            self.audio_status_text = format!(
                "Daemon streaming · {} · {:.1} fps · {} HID pkts",
                self.audio_source_label, self.audio_actual_fps, self.audio_packets_total
            );
        } else if let Some(err) = st.last_error {
            self.audio_status_text = err;
        } else if was_active {
            self.audio_status_text = "Stopped Audio Sync".into();
        }
    }

    /// Sync live effect/palette/gain/fps/brightness/source edits to `reaulad`.
    pub fn push_audio_live_config(&mut self) {
        if !self.audio_active {
            return;
        }
        let cfg = self.audio_config.to_ipc(self.selected_audio_source());
        match self.call_with_reconnect(|c| c.update_audio_sync(cfg.clone())) {
            Ok(st) => self.apply_audio_status(st, false),
            Err(e) => {
                let msg = format!("Failed to update daemon Audio Sync: {e}");
                self.audio_status_text = msg.clone();
                self.record_feedback("Audio Sync Update", 0.0, false, msg);
            }
        }
    }

    /// Start background ALSA/Demo equalizer in `reaulad` if hardware disclaimer is accepted.
    pub fn start_audio_sync(&mut self) -> Result<(), String> {
        if !self.audio_disclaimer_accepted {
            self.open_audio_disclaimer(true);
            return Err("Accept the experimental hardware disclaimer before starting Audio Sync.".into());
        }
        if self.cal_active {
            return Err("Stop sensor calibration before starting Audio Sync.".into());
        }

        let cfg = self.audio_config.to_ipc(self.selected_audio_source());
        let src_label = self.selected_audio_source().label();
        let st = self
            .call_with_reconnect(|c| c.start_audio_sync(cfg.clone()))
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("unknown variant") {
                    "Running reaulad predates daemon-hosted Audio Sync; restart reaulad (`reaula daemon restart`).".into()
                } else {
                    msg
                }
            })?;

        self.apply_audio_status(st, false);
        self.audio_status_text = format!("Daemon streaming · {src_label}");
        self.record_feedback(
            "Audio Sync Start",
            0.0,
            true,
            self.audio_status_text.clone(),
        );
        Ok(())
    }

    pub fn stop_audio_sync(&mut self, restore_previous_lighting: bool) {
        if !self.audio_active {
            return;
        }
        match self.call_with_reconnect(|c| c.stop_audio_sync(restore_previous_lighting)) {
            Ok(st) => {
                self.apply_audio_status(st, false);
                self.audio_status_text = if restore_previous_lighting {
                    "Stopped Audio Sync (restored previous keyboard lighting)".into()
                } else {
                    "Stopped Audio Sync".into()
                };
                self.record_feedback("Audio Sync Stop", 0.0, true, self.audio_status_text.clone());
            }
            Err(e) => {
                self.audio_active = false;
                self.audio_preview_frame = None;
                let msg = format!("Audio Sync stop failed: {e}");
                self.audio_status_text = msg.clone();
                self.record_feedback("Audio Sync Stop", 0.0, false, msg);
            }
        }
    }

    /// Poll `reaulad` for the live audio FFT metrics & preview frame.
    pub fn drain_audio(&mut self) -> (bool, Option<String>) {
        let interval = if self.audio_active {
            Duration::from_millis(33)
        } else {
            Duration::from_millis(500)
        };
        if self.last_audio_poll.elapsed() < interval {
            return (false, None);
        }
        self.last_audio_poll = Instant::now();

        let Ok(st) = self.call_with_reconnect(|c| c.get_audio_sync_state()) else {
            return (false, None);
        };

        let was_active = self.audio_active;
        let stopped_err = if was_active && !st.active {
            st.last_error.clone()
        } else {
            None
        };

        let changed = was_active != st.active
            || st.active
            || st.last_error.as_deref() != Some(self.audio_status_text.as_str())
                && st.last_error.is_some();

        if !changed {
            return (false, None);
        }

        let sync_cfg = st.active;
        self.apply_audio_status(st, sync_cfg);
        if let Some(err) = &stopped_err {
            self.record_feedback("Audio Sync Error", 0.0, false, err.clone());
        }
        (true, stopped_err)
    }
}

fn build_audio_source_options() -> Vec<AudioSourceOption> {
    let mut out = vec![
        AudioSourceOption::Monitor,
        AudioSourceOption::DefaultMic,
        AudioSourceOption::Demo,
    ];
    #[cfg(target_os = "linux")]
    if let Ok(devices) = alsa::list_capture_devices() {
        for dev in devices {
            if matches!(dev.name.as_str(), "default" | "pipewire" | "pulse") {
                continue;
            }
            out.push(AudioSourceOption::AlsaDevice {
                pcm: dev.name,
                label: if dev.description.is_empty() {
                    dev.ioid
                } else {
                    dev.description
                },
            });
        }
    }
    out
}

fn load_custom_slot(client: &mut DaemonClient, slot: u8) -> [[u8; 3]; MATRIX_SLOTS] {
    let mut out = [[0u8; 3]; MATRIX_SLOTS];
    if let Ok(vec) = client.get_custom_rgb(slot) {
        for (dst, src) in out.iter_mut().zip(vec) {
            *dst = src;
        }
    }
    out
}

/// Local fallback diagnostics when `reaulad` is offline/unreachable.
pub fn offline_diagnostic_report(reason: &str) -> DiagnosticReport {
    let mut checks = diagnostics::run_kernel_checks();
    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Daemon,
        name: "Daemon socket connection".into(),
        status: CheckStatus::Fail,
        latency_ms: None,
        detail: reason.to_string(),
        remedy: Some("Start the daemon first: `cargo run -p reaula-cli -- daemon start`.".into()),
    });
    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Keyboard,
        name: "Keyboard HID probe".into(),
        status: CheckStatus::Warn,
        latency_ms: None,
        detail: "Deferred until daemon owns /dev/hidraw*.".into(),
        remedy: None,
    });
    checks.extend(diagnostics::run_software_checks());
    let attribution = diagnostics::compute_attribution(&checks);
    DiagnosticReport {
        generated_at_unix_ms: diagnostics::now_unix_ms(),
        checks,
        attribution,
    }
}

pub fn format_cap_pills(probe: &DeviceProbe) -> Vec<(&'static str, bool)> {
    let Some(c) = probe.init_info.as_ref() else {
        return Vec::new();
    };
    vec![
        ("PRCS/SOCD", c.is_prcs),
        ("Custom RGB", c.is_custom_light),
        ("Switch Profiles", c.is_more_switch),
        ("Poll Rate", c.is_poll_rate),
        ("Deadband", c.is_dead_band),
        ("Music Rhythm", c.is_music_rhythm),
        ("Any-Key Cal", c.is_any_key_calibration),
        ("HighPrecision", c.is_high_precision),
        ("Gamepad", c.is_gamepad),
        ("RS", c.is_rs),
    ]
}

pub fn format_rgb_hex(rgb: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2])
}

pub fn pack_rgb_u32(rgb: [u8; 3]) -> u32 {
    (u32::from(rgb[0]) << 16) | (u32::from(rgb[1]) << 8) | u32::from(rgb[2])
}

/// Convert sRGB `[0..255]` into HSV `(hue in 0..1, sat in 0..1, val in 0..1)`.
pub fn rgb_to_hsv(rgb: [u8; 3]) -> [f32; 3] {
    let r = f32::from(rgb[0]) / 255.0;
    let g = f32::from(rgb[1]) / 255.0;
    let b = f32::from(rgb[2]) / 255.0;
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let delta = max_c - min_c;

    let hue = if delta <= 1e-5 {
        0.0
    } else if (max_c - r).abs() <= 1e-5 {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if (max_c - g).abs() <= 1e-5 {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    let sat = if max_c <= 1e-5 { 0.0 } else { delta / max_c };
    [hue.rem_euclid(1.0), sat.clamp(0.0, 1.0), max_c.clamp(0.0, 1.0)]
}

/// Convert HSV `(hue in 0..1, sat in 0..1, val in 0..1)` back to sRGB `[0..255]`.
pub fn hsv_to_rgb_u8(hue: f32, sat: f32, val: f32) -> [u8; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let s = sat.clamp(0.0, 1.0);
    let v = val.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [
        ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Human-readable color description (used in color picker badges & tooltips).
pub fn describe_rgb_human(rgb: [u8; 3]) -> &'static str {
    let [h, s, v] = rgb_to_hsv(rgb);
    if v < 0.08 {
        return "Off / Black";
    }
    if s < 0.14 {
        return if v > 0.88 {
            "Pure White"
        } else if v > 0.45 {
            "Silver Gray"
        } else {
            "Charcoal"
        };
    }
    let deg = h * 360.0;
    match deg {
        d if !(14.0..344.0).contains(&d) => "Crimson Red",
        d if d < 28.0 => "Coral Orange",
        d if d < 46.0 => "Warm Amber",
        d if d < 66.0 => "Golden Yellow",
        d if d < 105.0 => "Lime Green",
        d if d < 152.0 => "Emerald Green",
        d if d < 176.0 => "Mint Teal",
        d if d < 200.0 => "Aqua Blue",
        d if d < 220.0 => "Sky Blue",
        d if d < 252.0 => "Cobalt Blue",
        d if d < 286.0 => "Electric Violet",
        d if d < 322.0 => "Magenta",
        _ => "Hot Pink",
    }
}

/// Parse hex (`#RRGGBB`, `RRGGBB`, `#RGB`) or `R,G,B` text cleanly (e.g. from Paste Hex).
pub fn parse_hex_or_rgb(raw: &str) -> Option<[u8; 3]> {
    let trimmed = raw.trim();
    if let Some(hex) = trimmed.strip_prefix('#').or_else(|| {
        trimmed
            .chars()
            .all(|c| c.is_ascii_hexdigit())
            .then_some(trimmed)
    }) {
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some([r, g, b]);
        }
        if hex.len() == 3 {
            let d = |s: &str| u8::from_str_radix(s, 16).ok().map(|v| v * 17);
            return Some([d(&hex[0..1])?, d(&hex[1..2])?, d(&hex[2..3])?]);
        }
    }
    let parts: Vec<&str> = trimmed.split([',', ' ']).filter(|s| !s.is_empty()).collect();
    if parts.len() == 3 {
        return Some([
            parts[0].parse().ok()?,
            parts[1].parse().ok()?,
            parts[2].parse().ok()?,
        ]);
    }
    None
}

pub fn protocol_light_summary(ls: &LightState) -> String {
    format!(
        "{} | bri {}/4 | spd {}/4 | {} | {}",
        protocol::light_mode_name(ls.mode),
        ls.brightness,
        ls.speed,
        format_rgb_hex(ls.fg),
        if ls.power_on { "ON" } else { "OFF" }
    )
}

pub(crate) fn is_recoverable_ipc_error(err: &IpcError) -> bool {
    matches!(err, IpcError::Disconnected | IpcError::Io(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn identifies_broken_pipe_and_disconnect_for_auto_reconnect() {
        assert!(is_recoverable_ipc_error(&IpcError::Disconnected));
        assert!(is_recoverable_ipc_error(&IpcError::Io(io::Error::from(
            io::ErrorKind::BrokenPipe
        ))));
        assert!(is_recoverable_ipc_error(&IpcError::Io(io::Error::from(
            io::ErrorKind::ConnectionReset
        ))));
        assert!(!is_recoverable_ipc_error(&IpcError::Remote(
            "HID error".into()
        )));
    }

    #[test]
    fn audio_disclaimer_requires_10s_cooldown_and_checkbox() {
        let rem_from_now = |elapsed_s: u64| -> bool {
            let opened = Instant::now() - Duration::from_secs(elapsed_s);
            let rem = (AUDIO_DISCLAIMER_COOLDOWN_SECS - opened.elapsed().as_secs_f64()).max(0.0);
            rem <= 0.0
        };
        assert!(!rem_from_now(0));
        assert!(!rem_from_now(9));
        assert!(rem_from_now(11));

        assert_eq!(AUDIO_DISCLAIMER_COOLDOWN_SECS, 10.0);
    }

    #[test]
    fn easter_egg_summons_miku_on_fast_3_9_and_teto_on_4_1() {
        let t0 = Instant::now();
        let mut egg = MascotEasterEggState::default();

        // Slow 3 -> 9 (> 420ms) does not summon Miku.
        assert!(!egg.record_code(MascotKeySource::Hardware, "Digit3", t0));
        assert!(!egg.record_code(
            MascotKeySource::Hardware,
            "Digit9",
            t0 + Duration::from_millis(700)
        ));
        assert!(!egg.miku_visible);

        // Intervening unrelated key clears sequence, then fast 3 -> 9 summons Miku.
        let t1 = t0 + Duration::from_secs(2);
        assert!(!egg.record_code(MascotKeySource::Hardware, "Digit3", t1));
        assert!(!egg.record_code(
            MascotKeySource::Hardware,
            "KeyA",
            t1 + Duration::from_millis(50)
        ));
        assert!(!egg.record_code(
            MascotKeySource::Hardware,
            "Digit9",
            t1 + Duration::from_millis(120)
        ));
        assert!(!egg.miku_visible);

        let t2 = t1 + Duration::from_secs(1);
        assert!(!egg.record_code(MascotKeySource::Hardware, "Digit3", t2));
        // Duplicate OS window echo within 90ms is ignored cleanly.
        assert!(!egg.record_code(
            MascotKeySource::Window,
            "3",
            t2 + Duration::from_millis(5)
        ));
        assert!(egg.record_code(
            MascotKeySource::Hardware,
            "Digit9",
            t2 + Duration::from_millis(180)
        ));
        assert!(egg.miku_visible);
        assert!(egg.consume_dirty());

        // Fast repeat within 2s cooldown keeps Miku summoned instead of flickering off.
        assert!(!egg.record_code(
            MascotKeySource::Hardware,
            "Digit3",
            t2 + Duration::from_millis(300)
        ));
        assert!(!egg.record_code(
            MascotKeySource::Hardware,
            "Digit9",
            t2 + Duration::from_millis(450)
        ));
        assert!(egg.miku_visible);

        // Pressing 4 -> 1 summons Teto alongside Miku via window keys too.
        let t3 = t2 + Duration::from_secs(1);
        assert!(!egg.record_code(MascotKeySource::Window, "4", t3));
        assert!(egg.record_code(
            MascotKeySource::Window,
            "1",
            t3 + Duration::from_millis(240)
        ));
        assert!(egg.teto_visible);
        assert!(egg.miku_visible);
        assert!(egg.miku_pose(t3 + Duration::from_millis(500)).scale > 0.9);
        assert!(egg.teto_pose(t3 + Duration::from_millis(500)).scale > 0.9);
    }

    #[test]
    fn color_helpers_roundtrip_hsv_and_parse_formats() {
        for sample in [[0, 215, 255], [245, 158, 11], [168, 85, 247], [255, 0, 85]] {
            let [h, s, v] = rgb_to_hsv(sample);
            let back = hsv_to_rgb_u8(h, s, v);
            assert!(
                (i16::from(sample[0]) - i16::from(back[0])).abs() <= 2
                    && (i16::from(sample[1]) - i16::from(back[1])).abs() <= 2
                    && (i16::from(sample[2]) - i16::from(back[2])).abs() <= 2,
                "roundtrip mismatch {sample:?} -> {back:?}"
            );
        }
        assert_eq!(describe_rgb_human([0, 215, 255]), "Aqua Blue");
        assert_eq!(describe_rgb_human([0, 0, 0]), "Off / Black");
        assert_eq!(describe_rgb_human([255, 255, 255]), "Pure White");

        assert_eq!(parse_hex_or_rgb("#00d7ff"), Some([0, 215, 255]));
        assert_eq!(parse_hex_or_rgb("38bdf8"), Some([56, 189, 248]));
        assert_eq!(parse_hex_or_rgb("#f80"), Some([255, 136, 0]));
        assert_eq!(parse_hex_or_rgb("10, 220, 128"), Some([10, 220, 128]));
        assert_eq!(parse_hex_or_rgb("not-a-color"), None);
    }
}

