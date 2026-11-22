//! Complete interactive state, hit-testing model, IPC reconnect wrapper,
//! modals, keymap draft engine, switch diagnostics, and audio sync state for `atrium-tui`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use ratatui::layout::Rect;

#[cfg(target_os = "linux")]
use atrium_core::alsa;
use atrium_core::audio_fx::{AudioEffectStyle, AudioPalette};
use atrium_core::diagnostics::DiagnosticReport;
use atrium_core::driver::DeviceProbe;
use atrium_core::ipc::{
    AudioSyncConfig, AudioSyncSource, AudioSyncStatus, DaemonClient, DaemonStatus, FnOverride,
    IpcError, PeerCredentials, ShortcutAction, ShortcutRule, ShortcutTriggerLog,
    ShortcutsSnapshot, VirtualKey,
};
use atrium_core::keymap::{self, KeyDef, WIN60HE_KEYS};
use atrium_core::protocol::{
    self, CalibrationProgress, CalibrationStatus, LIGHT_MAX_LEVEL, LightState, LiveTravelSample,
    MATRIX_SLOTS, PollRate, PrcsPair, TriggerConfig, TriggerMode, WinLockFlags,
};

use crate::theme::HUMAN_COLOR_PRESETS;

pub const MAX_FEEDBACK_EVENTS: usize = 30;
pub const AUDIO_DISCLAIMER_COOLDOWN_SECS: f64 = 10.0;

pub const LIGHT_MODE_OPTIONS: [u8; 16] = [
    0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 100,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavSection {
    Main = 0,
    Guide = 1,
    Calibration = 2,
    ColorLight = 3,
    Keymaps = 4,
    Diagnostics = 5,
}

impl NavSection {
    pub const ALL: [Self; 6] = [
        Self::Main,
        Self::Guide,
        Self::Calibration,
        Self::ColorLight,
        Self::Keymaps,
        Self::Diagnostics,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "Main",
            Self::Guide => "Guide",
            Self::Calibration => "Calibration",
            Self::ColorLight => "Color & Light",
            Self::Keymaps => "Keymaps",
            Self::Diagnostics => "Diagnostics",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Main => "Actuation, Switch, Deadband & SOCD",
            Self::Guide => "Hardware Reference & Keymaps",
            Self::Calibration => "Guarded Sensor Calibration",
            Self::ColorLight => "RGB Effects, Custom Slots & Audio EQ",
            Self::Keymaps => "Firmware Flash vs Daemon Layer",
            Self::Diagnostics => "4-Layer Fault Isolation & Switch",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Main => "1:Main",
            Self::Guide => "2:Guide",
            Self::Calibration => "3:Calib",
            Self::ColorLight => "4:Light",
            Self::Keymaps => "5:Keymaps",
            Self::Diagnostics => "6:Diag",
        }
    }

    pub fn from_digit(ch: char) -> Option<Self> {
        match ch {
            '1' => Some(Self::Main),
            '2' => Some(Self::Guide),
            '3' => Some(Self::Calibration),
            '4' => Some(Self::ColorLight),
            '5' => Some(Self::Keymaps),
            '6' => Some(Self::Diagnostics),
            _ => None,
        }
    }

    pub fn default_focus(self) -> FocusPane {
        match self {
            Self::Main => FocusPane::MainKeyboard,
            Self::Guide => FocusPane::GuideBody,
            Self::Calibration => FocusPane::CalActions,
            Self::ColorLight => FocusPane::LightEffectsList,
            Self::Keymaps => FocusPane::KeymapsComposer,
            Self::Diagnostics => FocusPane::DiagActions,
        }
    }

    pub fn focus_cycle(self) -> &'static [FocusPane] {
        match self {
            Self::Main => &[
                FocusPane::MainKeyboard,
                FocusPane::MainActuation,
                FocusPane::MainSwitchDeadband,
                FocusPane::MainSocdPoll,
            ],
            Self::Guide => &[FocusPane::GuideBody],
            Self::Calibration => &[FocusPane::CalActions, FocusPane::CalMap],
            Self::ColorLight => &[
                FocusPane::LightEffectsList,
                FocusPane::LightControls,
                FocusPane::LightKeyboard,
            ],
            Self::Keymaps => &[
                FocusPane::KeymapsComposer,
                FocusPane::KeymapsLayerMatrix,
                FocusPane::KeymapsFirmwareList,
                FocusPane::KeymapsDaemonList,
            ],
            Self::Diagnostics => &[
                FocusPane::DiagActions,
                FocusPane::DiagSwitchInspector,
                FocusPane::DiagTestTable,
                FocusPane::DiagFeedbackLog,
            ],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FocusPane {
    MainKeyboard,
    MainActuation,
    MainSwitchDeadband,
    MainSocdPoll,
    GuideBody,
    CalActions,
    CalMap,
    LightEffectsList,
    LightControls,
    LightKeyboard,
    KeymapsLayerMatrix,
    KeymapsComposer,
    KeymapsFirmwareList,
    KeymapsDaemonList,
    DiagActions,
    DiagSwitchInspector,
    DiagTestTable,
    DiagFeedbackLog,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightEditorTab {
    FirmwareEffects,
    CustomSlot1,
    CustomSlot2,
    AudioSync,
}

impl LightEditorTab {
    pub const ALL: [Self; 4] = [
        Self::FirmwareEffects,
        Self::CustomSlot1,
        Self::CustomSlot2,
        Self::AudioSync,
    ];

    pub fn custom_slot_index(self) -> Option<u8> {
        match self {
            Self::CustomSlot1 => Some(0),
            Self::CustomSlot2 => Some(1),
            Self::FirmwareEffects | Self::AudioSync => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArrowClusterPreset {
    Rdfg,
    Tfgh,
    Wasd,
}

impl ArrowClusterPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rdfg => "R/D/F/G Arrows",
            Self::Tfgh => "T/F/G/H Arrows",
            Self::Wasd => "W/A/S/D Arrows",
        }
    }
}

fn arrow_cluster_bindings(preset: ArrowClusterPreset) -> [(u8, VirtualKey, &'static str); 4] {
    match preset {
        ArrowClusterPreset::Rdfg => [
            (keymap::idx::R, VirtualKey::Up, "R -> Up"),
            (keymap::idx::F, VirtualKey::Down, "F -> Down"),
            (keymap::idx::D, VirtualKey::Left, "D -> Left"),
            (keymap::idx::G, VirtualKey::Right, "G -> Right"),
        ],
        ArrowClusterPreset::Tfgh => [
            (keymap::idx::T, VirtualKey::Up, "T -> Up"),
            (keymap::idx::G, VirtualKey::Down, "G -> Down"),
            (keymap::idx::F, VirtualKey::Left, "F -> Left"),
            (keymap::idx::H, VirtualKey::Right, "H -> Right"),
        ],
        ArrowClusterPreset::Wasd => [
            (keymap::idx::W, VirtualKey::Up, "W -> Up"),
            (keymap::idx::S, VirtualKey::Down, "S -> Down"),
            (keymap::idx::A, VirtualKey::Left, "A -> Left"),
            (keymap::idx::D, VirtualKey::Right, "D -> Right"),
        ],
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeymapCategory {
    FirmwareFnFlash,
    DaemonMemory,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeymapLayerView {
    Both,
    FirmwareOnly,
    DaemonOnly,
}

impl KeymapLayerView {
    pub const ALL: [Self; 3] = [Self::Both, Self::FirmwareOnly, Self::DaemonOnly];
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

pub const DAEMON_MODIFIERS: [Option<u8>; 7] = [
    None,
    Some(keymap::idx::CAPS),
    Some(keymap::idx::L_CTRL),
    Some(keymap::idx::R_CTRL),
    Some(keymap::idx::L_ALT),
    Some(keymap::idx::R_ALT),
    Some(keymap::idx::MENU),
];

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
            Self::AlsaDevice { pcm, label } => format!("{pcm} - {label}"),
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

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Debug)]
pub struct SwitchTestStats {
    pub current_mm: f64,
    pub peak_mm: f64,
    pub released_zero: bool,
    pub presses: u32,
    pub was_pressed: bool,
    pub last_seen: Instant,
}

#[derive(Clone, Debug)]
pub struct SwitchInspectionSnapshot {
    pub key_index: u8,
    pub key_name: &'static str,
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

#[derive(Clone, Debug)]
pub struct ResponseEvent {
    pub id: u64,
    pub label: String,
    pub rtt_ms: f64,
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromptPurpose {
    ShellCommand,
    LightFgHex,
    LightBgHex,
    CustomBrushHex,
    SocdCustomPair,
}

#[derive(Clone, Debug)]
pub struct TextPromptModal {
    pub purpose: PromptPurpose,
    pub title: String,
    pub hint: String,
    pub buffer: String,
    pub cursor: usize,
    pub error: Option<String>,
}

impl TextPromptModal {
    pub fn new(purpose: PromptPurpose, title: &str, hint: &str, initial: &str) -> Self {
        let len = initial.chars().count();
        Self {
            purpose,
            title: title.to_string(),
            hint: hint.to_string(),
            buffer: initial.to_string(),
            cursor: len,
            error: None,
        }
    }

    pub fn insert_str(&mut self, s: &str) {
        let clean: String = s.chars().filter(|c| !matches!(c, '\r' | '\n' | '\0')).collect();
        if clean.is_empty() {
            return;
        }
        let byte_idx = char_to_byte_idx(&self.buffer, self.cursor);
        self.buffer.insert_str(byte_idx, &clean);
        self.cursor += clean.chars().count();
        self.error = None;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = char_to_byte_idx(&self.buffer, self.cursor);
        let start = char_to_byte_idx(&self.buffer, self.cursor - 1);
        self.buffer.replace_range(start..end, "");
        self.cursor -= 1;
        self.error = None;
    }

    pub fn delete(&mut self) {
        let len = self.buffer.chars().count();
        if self.cursor >= len {
            return;
        }
        let start = char_to_byte_idx(&self.buffer, self.cursor);
        let end = char_to_byte_idx(&self.buffer, self.cursor + 1);
        self.buffer.replace_range(start..end, "");
        self.error = None;
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        let len = self.buffer.chars().count();
        self.cursor = (self.cursor + 1).min(len);
    }

    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor = self.buffer.chars().count();
    }
}

fn char_to_byte_idx(s: &str, char_pos: usize) -> usize {
    s.char_indices()
        .nth(char_pos)
        .map_or(s.len(), |(idx, _)| idx)
}

/// Hidden mascot sequence state (fast 3+9 for Miku, 4+1 for Teto).
#[derive(Debug, Default, Clone)]
pub struct MascotEasterEggState {
    pub miku_visible: bool,
    pub teto_visible: bool,
    last_digit1_at: Option<Instant>,
    last_digit3_at: Option<Instant>,
    last_digit4_at: Option<Instant>,
    last_digit9_at: Option<Instant>,
    last_miku_toggle_at: Option<Instant>,
    last_teto_toggle_at: Option<Instant>,
}

impl MascotEasterEggState {
    const MIKU_FAST_WINDOW: Duration = Duration::from_millis(420);
    const TETO_PRESS_WINDOW: Duration = Duration::from_millis(750);
    const TOGGLE_COOLDOWN: Duration = Duration::from_millis(2000);

    pub fn any_visible(&self) -> bool {
        self.miku_visible || self.teto_visible
    }

    pub fn record_code(&mut self, raw: &str, now: Instant) -> bool {
        let canon = match raw {
            "3" | "Digit3" => Some("Digit3"),
            "9" | "Digit9" => Some("Digit9"),
            "4" | "Digit4" => Some("Digit4"),
            "1" | "Digit1" => Some("Digit1"),
            _ => None,
        };
        let Some(digit) = canon else {
            self.last_digit1_at = None;
            self.last_digit3_at = None;
            self.last_digit4_at = None;
            self.last_digit9_at = None;
            return false;
        };
        match digit {
            "Digit3" => {
                self.last_digit3_at = Some(now);
                if self
                    .last_digit9_at
                    .is_some_and(|t| now.saturating_duration_since(t) <= Duration::from_millis(220))
                {
                    self.last_digit9_at = None;
                    self.last_digit3_at = None;
                    return self.toggle_miku(now);
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
                    return self.toggle_miku(now);
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
                    return self.toggle_teto(now);
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
                    return self.toggle_teto(now);
                }
            }
            _ => {}
        }
        false
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
        self.last_miku_toggle_at = Some(now);
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
        self.last_teto_toggle_at = Some(now);
        true
    }
}

#[derive(Clone, Debug)]
pub enum HitTarget {
    Nav(NavSection),
    Focus(FocusPane),
    SelectKey(u8),
    SelectAll,
    SelectWasd,
    SelectFocusOnly,
    TravelToggle,
    ZeroStuck,
    RefreshAll,
    HelpToggle,
    Quit,
    // Main controls
    ActField(usize),
    ActMode(TriggerMode),
    ActTravelStep(f64),
    ActRtPressStep(f64),
    ActRtRelStep(f64),
    ActApply,
    SwPick(u8),
    SwApply,
    DbField(usize),
    DbTopStep(f64),
    DbBottomStep(f64),
    DbApply,
    PollPick(u8),
    SocdToggle,
    SocdPresetClear,
    SocdPresetAd,
    SocdPresetAdWs,
    SocdPromptPair,
    SocdApply,
    WinLockFlag(usize),
    WinLockApply,
    // Guide
    GuideScroll(isize),
    // Calibration
    CalArmAll,
    CalArmAny,
    CalStop,
    // Lighting & Audio Sync
    LightTab(LightEditorTab),
    LightEffectPick(usize),
    LightBri(u8),
    LightSpd(u8),
    LightDirNext,
    LightFullColor,
    LightPower,
    LightPresetPick(bool, usize),
    LightHsvStep(bool, usize, f32),
    LightHexPrompt(bool),
    LightApply,
    LightReload,
    CustomBrushToggle,
    CustomBrushPreset(usize),
    CustomBrushPrompt,
    CustomFillAll,
    CustomClearAll,
    CustomBri(u8),
    CustomUpload,
    CustomReload,
    AudioSourcePick(usize),
    AudioStylePick(AudioEffectStyle),
    AudioPalettePick(AudioPalette),
    AudioGainStep(f32),
    AudioFpsStep(i16),
    AudioBriStep(i16),
    AudioToggle,
    DisclaimerCheckToggle,
    DisclaimerConfirm,
    DisclaimerCancel,
    // Keymaps
    KmLayerView(KeymapLayerView),
    KmCategory(KeymapCategory),
    KmField(usize),
    KmPresetFw(ArrowClusterPreset),
    KmPresetDm(ArrowClusterPreset),
    KmRecordToggle,
    KmWatcherToggle,
    KmModCycle(isize),
    KmTriggerCycle(isize),
    KmDepthStep(f64),
    KmPresetCycle(isize),
    KmVirtualCycle(isize),
    KmShellPrompt,
    KmSave,
    KmReplaceConflict,
    KmLoadConflict(u32),
    KmCancelEdit,
    KmResetFwAll,
    KmFwLoad(u8),
    KmFwClear(u8),
    KmDmToggle(u32),
    KmDmEdit(u32),
    KmDmFire(u32),
    KmDmDel(u32),
    // Diagnostics
    DiagRun,
    DiagEcho,
    DiagSweepToggle,
    DiagFollowToggle,
    DiagClearPeaks,
    DiagPickKey(u8),
    // Modals / Popups
    PromptSubmit,
    ModalClose,
    MascotDismiss,
}

#[derive(Clone, Debug)]
pub struct HitRegion {
    pub rect: Rect,
    pub action: HitTarget,
}

pub struct TravelThreadHandle {
    stop: Arc<AtomicBool>,
    rx: Receiver<(LiveTravelSample, f64)>,
}

impl Drop for TravelThreadHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct CalibThreadHandle {
    stop: Arc<AtomicBool>,
    rx: Receiver<CalibrationProgress>,
}

impl Drop for CalibThreadHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct App {
    pub socket_arg: Option<PathBuf>,
    pub client: DaemonClient,
    pub server_cred: PeerCredentials,
    pub daemon_status: DaemonStatus,
    pub probe: DeviceProbe,
    pub unit_mm: f64,
    pub min_mm: f64,
    pub max_mm: f64,

    pub section: NavSection,
    pub focus: FocusPane,
    pub hit_regions: Vec<HitRegion>,
    pub show_help: bool,
    pub prompt_modal: Option<TextPromptModal>,

    // Selection & keyboard state
    pub cursor_pos: usize,
    pub selected_indices: BTreeSet<u8>,
    pub actuations: BTreeMap<u8, TriggerConfig>,
    pub switches: BTreeMap<u8, u8>,
    pub deadbands: BTreeMap<u8, (u8, u8)>,

    // Main editor drafts + form cursors
    pub main_act_row: usize,
    pub main_sw_row: usize,
    pub main_socd_row: usize,
    pub draft_mode: TriggerMode,
    pub draft_travel_mm: f64,
    pub draft_rt_press_mm: f64,
    pub draft_rt_release_mm: f64,
    pub draft_switch_id: u8,
    pub draft_db_top_mm: f64,
    pub draft_db_bottom_mm: f64,
    pub win_lock: WinLockFlags,

    // Guide scroll
    pub guide_scroll: u16,

    // Calibration
    pub cal_active: bool,
    pub cal_any_key: bool,
    pub cal_action_cursor: usize,
    pub cal_calibrated: BTreeSet<u8>,
    pub cal_status_text: String,
    pub cal_handle: Option<CalibThreadHandle>,

    // Lighting, custom slots & Audio EQ
    pub light_tab: LightEditorTab,
    pub light_state: LightState,
    pub light_mode_cursor: usize,
    pub light_ctrl_cursor: usize,
    pub light_edit_bg: bool,
    pub active_custom_slot: u8,
    pub custom_slots: [[[u8; 3]; MATRIX_SLOTS]; 2],
    pub custom_paint_mode: bool,
    pub custom_brush: [u8; 3],
    pub custom_brightness: u8,

    pub audio_sources: Vec<AudioSourceOption>,
    pub audio_source_idx: usize,
    pub audio_config: AudioLiveConfig,
    pub audio_ctrl_cursor: usize,
    pub audio_status: AudioSyncStatus,
    pub last_audio_poll: Instant,
    pub audio_disclaimer_accepted: bool,
    pub audio_disclaimer_modal_open: bool,
    pub audio_disclaimer_opened_at: Option<Instant>,
    pub audio_disclaimer_checked: bool,
    pub audio_disclaimer_pending_start: bool,

    // Keymaps (FW + DM)
    pub keymap_view: KeymapLayerView,
    pub draft_category: KeymapCategory,
    pub editing_daemon_rule_id: Option<u32>,
    pub draft_rule_modifier: Option<u8>,
    pub draft_rule_key: u8,
    pub draft_rule_depth_mm: f64,
    pub draft_rule_preset: ShortcutActionPreset,
    pub draft_virtual_key: VirtualKey,
    pub draft_rule_command: String,
    pub km_composer_row: usize,
    pub km_fw_cursor: usize,
    pub km_dm_cursor: usize,
    pub shortcut_rules: Vec<ShortcutRule>,
    pub shortcut_watcher_active: bool,
    pub shortcut_logs: Vec<ShortcutTriggerLog>,
    pub uinput_ready: bool,
    pub uinput_error: Option<String>,
    pub evdev_grab_active: bool,
    pub shortcut_persist_path: Option<String>,
    pub keyboard_fn_map: BTreeMap<u8, u8>,
    pub next_shortcut_id: u32,
    pub shortcut_recording: bool,
    pub record_first_key: Option<u8>,
    pub captured_gesture_banner: Option<String>,
    pub daemon_pulses: BTreeMap<u8, Instant>,
    pub daemon_active_snapshot: BTreeSet<u8>,
    pub daemon_pulsing_snapshot: BTreeSet<u8>,
    pub last_shortcut_poll: Instant,

    // SOCD / Poll Rate
    pub poll_rate_khz: u8,
    pub socd_enabled: bool,
    pub socd_pairs: Vec<PrcsPair>,

    // Diagnostics & switch cutaway
    pub diag_report: Option<DiagnosticReport>,
    pub echo_latencies_ms: Vec<f64>,
    pub diag_action_cursor: usize,
    pub diag_table_cursor: usize,
    pub diag_log_scroll: u16,
    pub diag_sweep_active: bool,
    pub diag_sweep_started_at: Option<Instant>,
    pub diag_follow_press: bool,
    pub feedback_log: VecDeque<ResponseEvent>,
    pub feedback_seq: u64,

    // Live travel & switch verification
    pub travel_enabled: bool,
    pub travel_handle: Option<TravelThreadHandle>,
    pub live_depths: BTreeMap<u8, (f64, Instant)>,
    pub last_sample: Option<(u8, f64)>,
    pub switch_tests: BTreeMap<u8, SwitchTestStats>,
    pub mascots: MascotEasterEggState,

    pub status_msg: String,
}

impl App {
    pub fn connect(socket_arg: Option<PathBuf>) -> Result<Self, String> {
        let mut client = DaemonClient::connect(socket_arg.as_deref()).map_err(|e| {
            format!(
                "could not connect to atriumd ({e}).\n\
                 Start the secure daemon first:\n  atrium daemon start\n\
                 Or via systemd:\n  atrium daemon start --systemd"
            )
        })?;
        let server_cred = client.server_credentials();
        let daemon_status = client.ping().map_err(|e| e.to_string())?;
        let probe = client.probe().map_err(|e| e.to_string())?;

        let (unit_mm, min_mm, max_mm) = probe
            .max_travel
            .map_or((0.01, 0.08, 3.40), |m| (m.unit_mm, m.min_mm, m.max_mm));

        let light_state = client.get_light().unwrap_or_else(|_| probe.light_state.unwrap_or_default());
        let light_mode_cursor = LIGHT_MODE_OPTIONS
            .iter()
            .position(|&m| m == light_state.mode)
            .unwrap_or(0);
        let active_custom_slot = client.get_active_custom_slot().unwrap_or(0).min(1);
        let custom_slots = [load_custom_slot(&mut client, 0), load_custom_slot(&mut client, 1)];

        let all_idx = keymap::all_indices();
        let mut actuations = BTreeMap::new();
        if let Ok((_, entries)) = client.get_actuation(all_idx.clone()) {
            for e in entries {
                if let Some(cfg) = e.config {
                    actuations.insert(e.index, cfg);
                }
            }
        }
        let mut switches = BTreeMap::new();
        if let Ok(entries) = client.get_switches(all_idx.clone()) {
            for e in entries {
                switches.insert(e.index, e.switch_id);
            }
        }
        let mut deadbands = BTreeMap::new();
        if let Ok(entries) = client.get_deadband(all_idx) {
            for e in entries {
                deadbands.insert(e.index, (e.top_raw, e.bottom_raw));
            }
        }

        let poll_rate_khz = client
            .get_poll_rate()
            .unwrap_or_else(|_| probe.poll_rate_khz.unwrap_or(8));
        let (socd_enabled, socd_pairs) = client
            .get_socd()
            .unwrap_or((probe.prcs_enabled.unwrap_or(false), Vec::new()));

        let snap = client.get_shortcuts().ok();
        let keyboard_fn_map: BTreeMap<u8, u8> = client
            .get_fn_overrides()
            .unwrap_or_default()
            .into_iter()
            .map(|o| (o.index, o.target_hid))
            .collect();

        let focused_idx = keymap::idx::W;
        let mut selected_indices = BTreeSet::new();
        selected_indices.insert(focused_idx);

        let focus_act = actuations.get(&focused_idx).copied();
        let focus_db = deadbands.get(&focused_idx).copied().unwrap_or((0, 0));
        let draft_switch_id = switches.get(&focused_idx).copied().unwrap_or(1);

        let audio_status = client.get_audio_sync_state().unwrap_or_default();
        let mut audio_config = AudioLiveConfig {
            style: AudioEffectStyle::Spectrum,
            palette: AudioPalette::Aurora,
            fg: [56, 189, 248],
            bg: [0, 0, 0],
            brightness: light_state.brightness.clamp(1, LIGHT_MAX_LEVEL),
            fps: 35,
            gain: 1.2,
        };
        if audio_status.active {
            audio_config.apply_from_ipc(&audio_status.config);
        }
        let audio_sources = build_audio_source_options();

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
        ) = match snap {
            Some(s) => (
                s.rules,
                s.watcher_active,
                s.recent_triggers,
                s.uinput_ready,
                s.uinput_error,
                s.evdev_grab_active,
                s.persist_path,
                s.active_indicator_keys.into_iter().collect(),
                s.pulsing_indicator_keys.into_iter().collect(),
            ),
            None => (
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
        let next_shortcut_id = shortcut_rules
            .iter()
            .map(|r| r.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let default_category = if shortcut_rules.is_empty() && !keyboard_fn_map.is_empty() {
            KeymapCategory::FirmwareFnFlash
        } else {
            KeymapCategory::DaemonMemory
        };

        let mut app = Self {
            socket_arg,
            client,
            server_cred,
            daemon_status,
            probe,
            unit_mm,
            min_mm,
            max_mm,
            section: NavSection::Main,
            focus: FocusPane::MainKeyboard,
            hit_regions: Vec::with_capacity(128),
            show_help: false,
            prompt_modal: None,
            cursor_pos: WIN60HE_KEYS
                .iter()
                .position(|k| k.index == focused_idx)
                .unwrap_or(0),
            selected_indices,
            actuations,
            switches,
            deadbands,
            main_act_row: 0,
            main_sw_row: 0,
            main_socd_row: 0,
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
            win_lock: WinLockFlags::default(),
            guide_scroll: 0,
            cal_active: false,
            cal_any_key: false,
            cal_action_cursor: 0,
            cal_calibrated: BTreeSet::new(),
            cal_status_text: "Ready (idle - leave all keys untouched before arming)".into(),
            cal_handle: None,
            light_tab: LightEditorTab::FirmwareEffects,
            light_state,
            light_mode_cursor,
            light_ctrl_cursor: 0,
            light_edit_bg: false,
            active_custom_slot,
            custom_slots,
            custom_paint_mode: false,
            custom_brush: [0, 215, 255],
            custom_brightness: light_state.brightness.clamp(1, LIGHT_MAX_LEVEL),
            audio_sources,
            audio_source_idx: 0,
            audio_config,
            audio_ctrl_cursor: 0,
            audio_status,
            last_audio_poll: Instant::now(),
            audio_disclaimer_accepted: false,
            audio_disclaimer_modal_open: false,
            audio_disclaimer_opened_at: None,
            audio_disclaimer_checked: false,
            audio_disclaimer_pending_start: false,
            keymap_view: KeymapLayerView::Both,
            draft_category: default_category,
            editing_daemon_rule_id: None,
            draft_rule_modifier: Some(keymap::idx::CAPS),
            draft_rule_key: keymap::idx::T,
            draft_rule_depth_mm: 1.50,
            draft_rule_preset: ShortcutActionPreset::EmitVirtualKey,
            draft_virtual_key: VirtualKey::Up,
            draft_rule_command: "notify-send 'Atrium WIN 60 HE' 'Hotkey triggered'".into(),
            km_composer_row: 0,
            km_fw_cursor: 0,
            km_dm_cursor: 0,
            shortcut_rules,
            shortcut_watcher_active,
            shortcut_logs,
            uinput_ready,
            uinput_error,
            evdev_grab_active,
            shortcut_persist_path,
            keyboard_fn_map,
            next_shortcut_id,
            shortcut_recording: false,
            record_first_key: None,
            captured_gesture_banner: None,
            daemon_pulses: BTreeMap::new(),
            daemon_active_snapshot,
            daemon_pulsing_snapshot,
            last_shortcut_poll: Instant::now(),
            poll_rate_khz,
            socd_enabled,
            socd_pairs,
            diag_report: None,
            echo_latencies_ms: Vec::new(),
            diag_action_cursor: 0,
            diag_table_cursor: 0,
            diag_log_scroll: 0,
            diag_sweep_active: false,
            diag_sweep_started_at: None,
            diag_follow_press: true,
            feedback_log: VecDeque::new(),
            feedback_seq: 0,
            travel_enabled: false,
            travel_handle: None,
            live_depths: BTreeMap::new(),
            last_sample: None,
            switch_tests: BTreeMap::new(),
            mascots: MascotEasterEggState::default(),
            status_msg: "Connected. 1-6 switch workspaces · Tab switches focus · Mouse & scroll supported · '?' help."
                .into(),
        };
        if app.audio_status.active {
            let src = app.audio_status.config.source.clone();
            app.select_audio_source_ipc(&src);
        }
        app.record_feedback("Connect & Probe", 0.0, true, "Connected to atriumd".into());
        app.set_travel_streaming(true);
        Ok(app)
    }

    pub fn switch_section(&mut self, section: NavSection) {
        self.section = section;
        self.focus = section.default_focus();
    }

    pub fn cycle_focus(&mut self, forward: bool) {
        let cycle = self.section.focus_cycle();
        if cycle.is_empty() {
            return;
        }
        let pos = cycle.iter().position(|&f| f == self.focus).unwrap_or(0);
        let next = if forward {
            (pos + 1) % cycle.len()
        } else {
            (pos + cycle.len() - 1) % cycle.len()
        };
        self.focus = cycle[next];
    }

    pub fn current_key(&self) -> KeyDef {
        WIN60HE_KEYS[self.cursor_pos]
    }

    pub fn focus_key_index(&mut self, index: u8) {
        if let Some(pos) = WIN60HE_KEYS.iter().position(|k| k.index == index) {
            self.cursor_pos = pos;
            self.sync_draft_from_cursor();
        }
    }

    pub fn target_indices(&self) -> Vec<u8> {
        if self.selected_indices.is_empty() {
            vec![self.current_key().index]
        } else {
            self.selected_indices.iter().copied().collect()
        }
    }

    pub fn sync_draft_from_cursor(&mut self) {
        let idx = self.current_key().index;
        if let Some(cfg) = self.actuations.get(&idx) {
            self.draft_mode = cfg.mode;
            self.draft_travel_mm = f64::from(cfg.travel_raw) * self.unit_mm;
            self.draft_rt_press_mm = (f64::from(cfg.interval1_raw) * self.unit_mm).max(0.01);
            self.draft_rt_release_mm = (f64::from(cfg.interval2_raw) * self.unit_mm).max(0.01);
        }
        if let Some(&sw) = self.switches.get(&idx) {
            self.draft_switch_id = sw;
        }
        if let Some(&(t, b)) = self.deadbands.get(&idx) {
            self.draft_db_top_mm = f64::from(t) * 0.01;
            self.draft_db_bottom_mm = f64::from(b) * 0.01;
        }
    }

    pub fn move_key_cursor(&mut self, dr: i32, dc: i32) {
        let cur = self.current_key();
        let target_row = (i32::from(cur.row()) + dr).clamp(1, 5) as u8;
        let cur_center = i32::from(cur.x) + i32::from(cur.width) / 2;

        if dr == 0 && dc != 0 {
            let row_keys: Vec<usize> = WIN60HE_KEYS
                .iter()
                .enumerate()
                .filter(|(_, k)| k.row() == cur.row())
                .map(|(i, _)| i)
                .collect();
            if let Some(pos) = row_keys.iter().position(|&i| i == self.cursor_pos) {
                let next = (pos as i32 + dc).clamp(0, (row_keys.len() - 1) as i32) as usize;
                self.cursor_pos = row_keys[next];
            }
        } else {
            let best = WIN60HE_KEYS
                .iter()
                .enumerate()
                .filter(|(_, k)| k.row() == target_row)
                .min_by_key(|(_, k)| {
                    let center = i32::from(k.x) + i32::from(k.width) / 2;
                    (center - cur_center).abs()
                })
                .map(|(i, _)| i);
            if let Some(idx) = best {
                self.cursor_pos = idx;
            }
        }
        self.sync_draft_from_cursor();
    }

    pub fn record_feedback(
        &mut self,
        label: impl Into<String>,
        rtt_ms: f64,
        ok: bool,
        detail: String,
    ) {
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

    pub fn call_with_reconnect<T>(
        &mut self,
        mut f: impl FnMut(&mut DaemonClient) -> Result<T, IpcError>,
    ) -> Result<T, IpcError> {
        let res = f(&mut self.client);
        match res {
            Ok(v) => Ok(v),
            Err(err) if matches!(err, IpcError::Disconnected | IpcError::Io(_)) => {
                if let Ok(new_client) = DaemonClient::connect(self.socket_arg.as_deref()) {
                    self.server_cred = new_client.server_credentials();
                    self.client = new_client;
                    if self.travel_enabled && !self.cal_active {
                        self.travel_handle = None;
                        self.travel_enabled = false;
                        self.set_travel_streaming(true);
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

    pub fn set_travel_streaming(&mut self, enable: bool) {
        if enable == self.travel_enabled {
            return;
        }
        if !enable {
            self.travel_handle = None;
            self.travel_enabled = false;
            self.live_depths.clear();
            self.status_msg = "Live travel streaming paused.".into();
            return;
        }
        if self.cal_active {
            self.status_msg = "Disarm Calibration before starting Live Travel.".into();
            return;
        }

        let sock = self.socket_arg.clone();
        let Ok(stream_client) = DaemonClient::connect(sock.as_deref()) else {
            self.status_msg = "Failed to open travel subscription socket.".into();
            return;
        };
        let Ok(mut sub) = stream_client.subscribe_travel(keymap::all_indices()) else {
            self.status_msg = "Daemon refused travel subscription.".into();
            return;
        };

        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let (tx, rx) = mpsc::channel();
        let _ = thread::Builder::new()
            .name("tui-travel".into())
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
            });

        self.travel_handle = Some(TravelThreadHandle { stop, rx });
        self.travel_enabled = true;
        self.status_msg = "Live travel streaming ON.".into();
    }

    pub fn clear_stuck_travel(&mut self) {
        self.live_depths.clear();
        self.last_sample = None;
        for st in self.switch_tests.values_mut() {
            st.current_mm = 0.0;
            st.was_pressed = false;
            st.released_zero = true;
        }
        self.status_msg = "Cleared stuck live-travel depths.".into();
    }

    pub fn drain_travel_samples(&mut self) {
        let mut batch = Vec::new();
        if let Some(handle) = &self.travel_handle {
            while let Ok(item) = handle.rx.try_recv() {
                batch.push(item);
            }
        }
        let now = Instant::now();
        let follow_diag = self.section == NavSection::Diagnostics && self.diag_follow_press;
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
                if follow_diag {
                    self.focus_key_index(sample.index);
                    if let Some(pos) = WIN60HE_KEYS.iter().position(|k| k.index == sample.index) {
                        self.diag_table_cursor = pos;
                    }
                }
                if let Some(kdef) = keymap::by_index(sample.index) {
                    self.mascots.record_code(kdef.code, now);
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
        }
        self.live_depths
            .retain(|_, (_, ts)| now.duration_since(*ts) < Duration::from_millis(1200));
        self.daemon_pulses
            .retain(|_, ts| now.duration_since(*ts) < Duration::from_millis(650));
    }

    pub fn is_fn_held(&self) -> bool {
        self.live_depths
            .get(&keymap::idx::FN)
            .is_some_and(|(d, _)| *d >= 0.35)
    }

    pub fn active_daemon_indicators(&self) -> (BTreeSet<u8>, BTreeSet<u8>) {
        if !self.shortcut_watcher_active {
            return (BTreeSet::new(), BTreeSet::new());
        }
        let now = Instant::now();
        let mut active = self.daemon_active_snapshot.clone();
        let mut pulsing = self.daemon_pulsing_snapshot.clone();
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

    pub fn refresh_all(&mut self) {
        match self.run_measured("Probe", |c| c.probe()) {
            Ok(p) => self.probe = p,
            Err(e) => {
                self.status_msg = format!("Refresh/Reconnect failed: {e}");
                return;
            }
        }
        let all_idx = keymap::all_indices();
        if let Ok((_, entries)) = self.call_with_reconnect(|c| c.get_actuation(all_idx.clone())) {
            for e in entries {
                if let Some(cfg) = e.config {
                    self.actuations.insert(e.index, cfg);
                }
            }
        }
        if let Ok(entries) = self.call_with_reconnect(|c| c.get_switches(all_idx.clone())) {
            for e in entries {
                self.switches.insert(e.index, e.switch_id);
            }
        }
        if let Ok(entries) = self.call_with_reconnect(|c| c.get_deadband(all_idx.clone())) {
            for e in entries {
                self.deadbands.insert(e.index, (e.top_raw, e.bottom_raw));
            }
        }
        if let Ok(ls) = self.call_with_reconnect(|c| c.get_light()) {
            self.light_state = ls;
            if let Some(pos) = LIGHT_MODE_OPTIONS.iter().position(|&m| m == ls.mode) {
                self.light_mode_cursor = pos;
            }
        }
        if let Ok(s0) = self.call_with_reconnect(|c| c.get_custom_rgb(0)) {
            for (dst, src) in self.custom_slots[0].iter_mut().zip(s0) {
                *dst = src;
            }
        }
        if let Ok(s1) = self.call_with_reconnect(|c| c.get_custom_rgb(1)) {
            for (dst, src) in self.custom_slots[1].iter_mut().zip(s1) {
                *dst = src;
            }
        }
        if let Ok(khz) = self.call_with_reconnect(|c| c.get_poll_rate()) {
            self.poll_rate_khz = khz;
        }
        if let Ok((en, pairs)) = self.call_with_reconnect(|c| c.get_socd()) {
            self.socd_enabled = en;
            self.socd_pairs = pairs;
        }
        if let Ok(st) = self.call_with_reconnect(|c| c.ping()) {
            self.daemon_status = st;
        }
        if let Ok(snap) = self.call_with_reconnect(|c| c.get_shortcuts()) {
            self.apply_shortcuts_snapshot(snap);
        }
        if let Ok(list) = self.call_with_reconnect(|c| c.get_fn_overrides()) {
            self.apply_fn_overrides_list(list);
        }
        if let Ok(st) = self.call_with_reconnect(|c| c.get_audio_sync_state()) {
            let sync = st.active;
            self.apply_audio_status(st, sync);
        }
        self.sync_draft_from_cursor();
        self.status_msg = "Refreshed full keyboard & daemon state.".into();
    }

    // ---- Main Actuation / Switch / Deadband / Poll / SOCD / WinLock ----

    pub fn apply_actuation(&mut self) {
        let indices = self.target_indices();
        let (rt_p, rt_r) = match self.draft_mode {
            TriggerMode::Fixed | TriggerMode::Unknown(_) => (None, None),
            TriggerMode::RapidTrigger => (Some(self.draft_rt_press_mm), None),
            TriggerMode::RapidTriggerSplit => {
                (Some(self.draft_rt_press_mm), Some(self.draft_rt_release_mm))
            }
        };
        let mode = self.draft_mode;
        let travel = self.draft_travel_mm;
        match self.run_measured("Set Actuation", |c| {
            c.set_actuation(indices.clone(), mode, travel, rt_p, rt_r)
        }) {
            Ok(()) => {
                if let Ok((_, entries)) =
                    self.call_with_reconnect(|c| c.get_actuation(indices.clone()))
                {
                    for e in entries {
                        if let Some(cfg) = e.config {
                            self.actuations.insert(e.index, cfg);
                        }
                    }
                }
                self.status_msg = format!(
                    "Applied {} @ {:.2} mm to {} key(s).",
                    mode.label(),
                    travel,
                    indices.len()
                );
            }
            Err(e) => self.status_msg = format!("Actuation write failed: {e}"),
        }
    }

    pub fn apply_switch(&mut self) {
        let indices = self.target_indices();
        let sw = self.draft_switch_id;
        match self.run_measured("Set Switch", |c| c.set_switches(indices.clone(), sw)) {
            Ok(()) => {
                for &idx in &indices {
                    self.switches.insert(idx, sw);
                }
                self.status_msg = format!(
                    "Set switch profile -> {} for {} key(s).",
                    protocol::switch_profile_name(sw),
                    indices.len()
                );
            }
            Err(e) => self.status_msg = format!("Switch write failed: {e}"),
        }
    }

    pub fn apply_deadband(&mut self) {
        let indices = self.target_indices();
        let top_raw = (self.draft_db_top_mm * 100.0).round().clamp(0.0, 255.0) as u8;
        let bot_raw = (self.draft_db_bottom_mm * 100.0).round().clamp(0.0, 255.0) as u8;
        match self.run_measured("Set Deadband", |c| {
            c.set_deadband(indices.clone(), top_raw, bot_raw)
        }) {
            Ok(()) => {
                for &idx in &indices {
                    self.deadbands.insert(idx, (top_raw, bot_raw));
                }
                self.status_msg = format!(
                    "Updated deadband ({:.2}/{:.2} mm) for {} key(s).",
                    f64::from(top_raw) * 0.01,
                    f64::from(bot_raw) * 0.01,
                    indices.len()
                );
            }
            Err(e) => self.status_msg = format!("Deadband write failed: {e}"),
        }
    }

    pub fn apply_poll_rate(&mut self, khz: u8) {
        if PollRate::from_khz(khz).is_err() {
            return;
        }
        match self.run_measured("Set Poll Rate", |c| c.set_poll_rate(khz)) {
            Ok(()) => {
                self.poll_rate_khz = khz;
                self.status_msg = format!("Set USB polling rate to {khz} kHz.");
            }
            Err(e) => self.status_msg = format!("Poll rate failed: {e}"),
        }
    }

    pub fn apply_socd(&mut self) {
        let enabled = self.socd_enabled;
        let pairs = self.socd_pairs.clone();
        match self.run_measured("Set SOCD / PRCS", |c| c.set_socd(enabled, pairs.clone())) {
            Ok(()) => {
                self.status_msg = format!(
                    "SOCD {} ({} pair(s)).",
                    if enabled { "enabled" } else { "disabled" },
                    self.socd_pairs.len()
                );
            }
            Err(e) => self.status_msg = format!("SOCD write failed: {e}"),
        }
    }

    pub fn apply_win_lock(&mut self) {
        let flags = self.win_lock;
        match self.run_measured("Set Win-Lock", |c| c.set_win_lock(flags)) {
            Ok(()) => {
                self.status_msg = format!(
                    "Applied Win-Lock (Win={}, Shift+Tab={}, Alt+Tab={}, Alt+F4={}).",
                    flags.disable_win,
                    flags.disable_shift_tab,
                    flags.disable_alt_tab,
                    flags.disable_alt_f4
                );
            }
            Err(e) => self.status_msg = format!("Win-Lock failed: {e}"),
        }
    }

    // ---- Calibration ----

    pub fn start_calibration(&mut self, any_key: bool) {
        self.stop_calibration();
        if self.travel_enabled {
            self.set_travel_streaming(false);
        }
        self.cal_calibrated.clear();
        self.cal_any_key = any_key;

        let Ok(sub_client) = DaemonClient::connect(self.socket_arg.as_deref()) else {
            self.status_msg = "Failed to connect calibration stream socket.".into();
            return;
        };
        let mut sub = match sub_client.start_calibration(any_key) {
            Ok(s) => s,
            Err(e) => {
                self.status_msg = format!("Calibration failed to arm: {e}");
                return;
            }
        };

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let _ = thread::Builder::new()
            .name("tui-calib".into())
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
            });

        self.cal_handle = Some(CalibThreadHandle { stop, rx });
        self.cal_active = true;
        self.cal_status_text = if any_key {
            "Any-Key ARMED - press target key(s) to bottom-out, then press Disarm / Finish.".into()
        } else {
            "All-Key ARMED - press EVERY key smoothly to bottom-out until 61/61.".into()
        };
        self.record_feedback(
            "Start Calibration",
            0.0,
            true,
            if any_key { "Any-key armed" } else { "All-key armed" }.into(),
        );
        self.status_msg = self.cal_status_text.clone();
    }

    pub fn stop_calibration(&mut self) {
        let was_active = self.cal_active || self.cal_handle.is_some();
        self.cal_handle = None;
        self.cal_active = false;
        if was_active {
            self.cal_status_text = format!(
                "Disarmed ({}/{} keys recorded).",
                self.cal_calibrated.len(),
                WIN60HE_KEYS.len()
            );
            self.record_feedback("Stop Calibration", 0.0, true, self.cal_status_text.clone());
            self.status_msg = self.cal_status_text.clone();
        }
    }

    pub fn drain_calibration(&mut self) {
        let Some(handle) = &self.cal_handle else {
            return;
        };
        let mut finished = false;
        while let Ok(prog) = handle.rx.try_recv() {
            match prog.status {
                CalibrationStatus::Progress => {
                    for idx in prog.calibrated_indices {
                        self.cal_calibrated.insert(idx);
                    }
                    self.cal_status_text = format!(
                        "Calibrating: {}/{} keys completed",
                        self.cal_calibrated.len(),
                        WIN60HE_KEYS.len()
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
        }
        if finished {
            self.cal_handle = None;
            self.cal_active = false;
            self.record_feedback("Calibration Result", 0.0, true, self.cal_status_text.clone());
            self.status_msg = self.cal_status_text.clone();
        }
    }

    // ---- Lighting, Custom Slots & Audio Sync ----

    pub fn apply_lighting(&mut self) {
        if self.audio_status.active {
            self.stop_audio_sync(false);
        }
        let ls = self.light_state;
        match self.run_measured("Set Lighting", |c| c.set_light(ls)) {
            Ok(()) => {
                self.status_msg = format!(
                    "Applied lighting: {} (bri {}/4, spd {}/4, #{:02X}{:02X}{:02X}).",
                    protocol::light_mode_name(ls.mode),
                    ls.brightness,
                    ls.speed,
                    ls.fg[0],
                    ls.fg[1],
                    ls.fg[2],
                );
            }
            Err(e) => self.status_msg = format!("Lighting write failed: {e}"),
        }
    }

    pub fn reload_lighting(&mut self) {
        match self.run_measured("Read Lighting", |c| c.get_light()) {
            Ok(ls) => {
                self.light_state = ls;
                if let Some(pos) = LIGHT_MODE_OPTIONS.iter().position(|&m| m == ls.mode) {
                    self.light_mode_cursor = pos;
                }
                self.status_msg = "Reloaded current lighting from WIN 60 HE.".into();
            }
            Err(e) => self.status_msg = format!("Lighting read failed: {e}"),
        }
    }

    pub fn paint_custom_key(&mut self, index: u8, clear: bool) {
        let Some(slot) = self.light_tab.custom_slot_index() else {
            return;
        };
        let slot_idx = usize::from(slot);
        if usize::from(index) < MATRIX_SLOTS {
            self.custom_slots[slot_idx][usize::from(index)] =
                if clear { [0, 0, 0] } else { self.custom_brush };
            let name = keymap::by_index(index).map_or("Key", |k| k.name);
            self.status_msg = format!(
                "{} {name} on Custom Slot {}.",
                if clear { "Cleared" } else { "Painted" },
                slot + 1
            );
        }
    }

    pub fn fill_custom_slot(&mut self, rgb: [u8; 3]) {
        let Some(slot) = self.light_tab.custom_slot_index() else {
            return;
        };
        let slot_idx = usize::from(slot);
        for key in WIN60HE_KEYS {
            if usize::from(key.index) < MATRIX_SLOTS {
                self.custom_slots[slot_idx][usize::from(key.index)] = rgb;
            }
        }
        self.status_msg = format!(
            "Filled all 61 keys on Custom Slot {} with #{:02X}{:02X}{:02X}.",
            slot + 1,
            rgb[0],
            rgb[1],
            rgb[2]
        );
    }

    pub fn upload_custom_slot(&mut self) {
        let Some(slot) = self.light_tab.custom_slot_index() else {
            return;
        };
        if self.audio_status.active {
            self.stop_audio_sync(false);
        }
        let colors = self.custom_slots[usize::from(slot)].to_vec();
        let bri = self.custom_brightness.clamp(1, LIGHT_MAX_LEVEL);
        match self.run_measured("Upload Custom RGB", |c| {
            c.set_custom_rgb(slot, colors.clone(), bri)
        }) {
            Ok(()) => {
                self.active_custom_slot = slot;
                self.status_msg = format!("Uploaded Custom RGB Slot {} to WIN 60 HE.", slot + 1);
            }
            Err(e) => self.status_msg = format!("Custom RGB upload failed: {e}"),
        }
    }

    pub fn reload_custom_slot(&mut self) {
        let Some(slot) = self.light_tab.custom_slot_index() else {
            return;
        };
        match self.run_measured("Read Custom RGB", |c| c.get_custom_rgb(slot)) {
            Ok(vec) => {
                for (dst, src) in self.custom_slots[usize::from(slot)].iter_mut().zip(vec) {
                    *dst = src;
                }
                self.status_msg = format!("Reloaded Custom Slot {} from keyboard.", slot + 1);
            }
            Err(e) => self.status_msg = format!("Custom RGB read failed: {e}"),
        }
    }

    pub fn selected_audio_source(&self) -> &AudioSourceOption {
        self.audio_sources
            .get(self.audio_source_idx)
            .unwrap_or(&self.audio_sources[0])
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

    pub fn disclaimer_remaining_secs(&self) -> f64 {
        if self.audio_disclaimer_accepted {
            return 0.0;
        }
        let Some(opened_at) = self.audio_disclaimer_opened_at else {
            return AUDIO_DISCLAIMER_COOLDOWN_SECS;
        };
        (AUDIO_DISCLAIMER_COOLDOWN_SECS - opened_at.elapsed().as_secs_f64()).max(0.0)
    }

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

    pub fn confirm_audio_disclaimer(&mut self) {
        let rem = self.disclaimer_remaining_secs();
        if rem > 0.0 {
            self.status_msg = format!("Please wait {:.0}s before confirming disclaimer.", rem.ceil());
            return;
        }
        if !self.audio_disclaimer_checked {
            self.status_msg = "Check '[x] I Agree' before starting Audio Sync.".into();
            return;
        }
        self.audio_disclaimer_accepted = true;
        let should_start = self.audio_disclaimer_pending_start;
        self.audio_disclaimer_modal_open = false;
        self.audio_disclaimer_pending_start = false;
        if should_start {
            self.start_audio_sync();
        }
    }

    pub fn start_audio_sync(&mut self) {
        if !self.audio_disclaimer_accepted {
            self.open_audio_disclaimer(true);
            return;
        }
        if self.cal_active {
            self.status_msg = "Stop sensor calibration before starting Audio Sync.".into();
            return;
        }
        let cfg = self.audio_config.to_ipc(self.selected_audio_source());
        match self.run_measured("Audio Sync Start", |c| c.start_audio_sync(cfg.clone())) {
            Ok(st) => {
                self.apply_audio_status(st, false);
                self.status_msg = format!(
                    "Started daemon Audio Sync ({}).",
                    self.selected_audio_source().label()
                );
            }
            Err(e) => self.status_msg = format!("Audio Sync start failed: {e}"),
        }
    }

    pub fn stop_audio_sync(&mut self, restore_lighting: bool) {
        if !self.audio_status.active {
            return;
        }
        match self.run_measured("Audio Sync Stop", |c| c.stop_audio_sync(restore_lighting)) {
            Ok(st) => {
                self.apply_audio_status(st, false);
                self.status_msg = "Stopped daemon Audio Sync (restored lighting).".into();
            }
            Err(e) => self.status_msg = format!("Audio Sync stop failed: {e}"),
        }
    }

    pub fn toggle_audio_sync(&mut self) {
        if self.audio_status.active {
            self.stop_audio_sync(true);
        } else {
            self.start_audio_sync();
        }
    }

    pub fn push_audio_live_update(&mut self, msg: &str) {
        self.status_msg = msg.to_string();
        if !self.audio_status.active {
            return;
        }
        let cfg = self.audio_config.to_ipc(self.selected_audio_source());
        if let Ok(st) = self.call_with_reconnect(|c| c.update_audio_sync(cfg.clone())) {
            self.apply_audio_status(st, false);
        }
    }

    fn apply_audio_status(&mut self, st: AudioSyncStatus, sync_cfg: bool) {
        let was_active = self.audio_active_before();
        if sync_cfg {
            self.audio_config.apply_from_ipc(&st.config);
            self.select_audio_source_ipc(&st.config.source);
        }
        if !st.active && was_active && let Some(err) = &st.last_error {
            self.status_msg = format!("Audio Sync stopped: {err}");
        }
        self.audio_status = st;
    }

    fn audio_active_before(&self) -> bool {
        self.audio_status.active
    }

    pub fn poll_audio_sync(&mut self) {
        let interval = if self.audio_status.active || self.section == NavSection::ColorLight {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(500)
        };
        if self.last_audio_poll.elapsed() < interval {
            return;
        }
        self.last_audio_poll = Instant::now();
        if let Ok(st) = self.call_with_reconnect(|c| c.get_audio_sync_state()) {
            let sync_cfg = !self.audio_status.active && st.active;
            self.apply_audio_status(st, sync_cfg);
        }
    }

    // ---- Keymaps ([FW] Flash + [DM] Daemon) ----

    pub fn apply_shortcuts_snapshot(&mut self, snap: ShortcutsSnapshot) {
        self.shortcut_rules = snap.rules;
        self.shortcut_watcher_active = snap.watcher_active;
        self.shortcut_logs = snap.recent_triggers;
        self.uinput_ready = snap.uinput_ready;
        self.uinput_error = snap.uinput_error;
        self.evdev_grab_active = snap.evdev_grab_active;
        self.shortcut_persist_path = snap.persist_path;
        self.daemon_active_snapshot = snap.active_indicator_keys.into_iter().collect();
        self.daemon_pulsing_snapshot = snap.pulsing_indicator_keys.into_iter().collect();
        if let Some(edit_id) = self.editing_daemon_rule_id
            && !self.shortcut_rules.iter().any(|r| r.id == edit_id)
        {
            self.editing_daemon_rule_id = None;
        }
        if !self.shortcut_rules.is_empty() {
            self.km_dm_cursor = self.km_dm_cursor.min(self.shortcut_rules.len() - 1);
        } else {
            self.km_dm_cursor = 0;
        }
    }

    pub fn poll_shortcut_logs_quiet(&mut self) {
        let interval = if self.section == NavSection::Keymaps {
            Duration::from_millis(120)
        } else {
            Duration::from_millis(800)
        };
        if self.last_shortcut_poll.elapsed() < interval {
            return;
        }
        self.last_shortcut_poll = Instant::now();
        if let Ok(snap) = self.call_with_reconnect(|c| c.get_shortcuts()) {
            self.apply_shortcuts_snapshot(snap);
        }
    }

    pub fn sync_shortcuts(&mut self) -> Result<(), String> {
        let rules = self.shortcut_rules.clone();
        let watcher = self.shortcut_watcher_active;
        let snap = self
            .run_measured("Sync Hotkeys", |c| c.set_shortcuts(rules.clone(), watcher))
            .map_err(|e| e.to_string())?;
        self.apply_shortcuts_snapshot(snap);
        Ok(())
    }

    pub fn firmware_compatible_virtual_keys() -> Vec<VirtualKey> {
        VirtualKey::ALL
            .iter()
            .copied()
            .filter(|&vk| keymap::virtual_key_to_hid_usage(vk).is_some())
            .collect()
    }

    pub fn set_keymap_draft_category(&mut self, cat: KeymapCategory) {
        self.draft_category = cat;
        if cat == KeymapCategory::FirmwareFnFlash
            && keymap::virtual_key_to_hid_usage(self.draft_virtual_key).is_none()
        {
            self.draft_virtual_key = VirtualKey::Up;
        }
        if cat == KeymapCategory::DaemonMemory && self.draft_rule_modifier == Some(keymap::idx::FN) {
            self.draft_rule_modifier = Some(keymap::idx::CAPS);
        }
        if self.draft_rule_key == keymap::idx::FN {
            self.draft_rule_key = keymap::idx::T;
        }
    }

    pub fn load_firmware_slot_into_draft(&mut self, index: u8, target_hid: u8) {
        self.draft_category = KeymapCategory::FirmwareFnFlash;
        self.editing_daemon_rule_id = None;
        self.draft_rule_key = index;
        if let Some(vk) = keymap::hid_usage_to_virtual_key(target_hid) {
            self.draft_virtual_key = vk;
        } else if keymap::virtual_key_to_hid_usage(self.draft_virtual_key).is_none() {
            self.draft_virtual_key = VirtualKey::Up;
        }
        let kname = keymap::by_index(index).map_or("Key", |k| k.name);
        self.status_msg = format!("Loaded [FW] slot Fn+{kname} into Composer.");
    }

    pub fn load_daemon_rule_into_draft(&mut self, rule: &ShortcutRule) {
        self.draft_category = KeymapCategory::DaemonMemory;
        self.editing_daemon_rule_id = Some(rule.id);
        self.draft_rule_modifier = rule.modifier_key.filter(|&m| m != keymap::idx::FN);
        self.draft_rule_key = if rule.trigger_key == keymap::idx::FN {
            keymap::idx::T
        } else {
            rule.trigger_key
        };
        self.draft_rule_depth_mm = rule.trigger_depth_mm;
        match &rule.action {
            ShortcutAction::EmitKey { key } => {
                self.draft_rule_preset = ShortcutActionPreset::EmitVirtualKey;
                self.draft_virtual_key = *key;
            }
            ShortcutAction::ActuationPreset {
                mode: TriggerMode::RapidTrigger,
                ..
            } => self.draft_rule_preset = ShortcutActionPreset::FpsRapidTrigger,
            ShortcutAction::ActuationPreset { .. } => {
                self.draft_rule_preset = ShortcutActionPreset::TypingComfort
            }
            ShortcutAction::ToggleSocd => self.draft_rule_preset = ShortcutActionPreset::ToggleSocd,
            ShortcutAction::ToggleLighting => {
                self.draft_rule_preset = ShortcutActionPreset::ToggleLighting
            }
            ShortcutAction::LightingPreset { .. } => {
                self.draft_rule_preset = ShortcutActionPreset::NeonWaveLighting
            }
            ShortcutAction::SetPollRate { .. } => {
                self.draft_rule_preset = ShortcutActionPreset::PollRate8k
            }
            ShortcutAction::RunCommand { command } => {
                self.draft_rule_preset = ShortcutActionPreset::RunCommand;
                self.draft_rule_command = command.clone();
            }
        }
        self.status_msg = format!("Editing [DM] rule #{} in place.", rule.id);
    }

    pub fn draft_gesture_label(&self) -> String {
        let trig_name = keymap::by_index(self.draft_rule_key).map_or("Key", |k| k.name);
        match self.draft_rule_modifier.filter(|&m| m != keymap::idx::FN) {
            Some(m) => {
                let mname = keymap::by_index(m).map_or("Mod", |k| k.name);
                format!("{mname} + {trig_name}")
            }
            None => trig_name.to_string(),
        }
    }

    pub fn conflicting_daemon_rule_for_draft(&self) -> Option<&ShortcutRule> {
        let dm_mod = self.draft_rule_modifier.filter(|&m| m != keymap::idx::FN);
        self.shortcut_rules.iter().find(|r| {
            Some(r.id) != self.editing_daemon_rule_id
                && r.modifier_key == dm_mod
                && r.trigger_key == self.draft_rule_key
        })
    }

    pub fn build_shortcut_rule_with_id(&self, id: u32) -> ShortcutRule {
        let modifier_key = self.draft_rule_modifier.filter(|&m| m != keymap::idx::FN);
        let trigger_key = if self.draft_rule_key == keymap::idx::FN {
            keymap::idx::T
        } else {
            self.draft_rule_key
        };
        let trig_name = keymap::by_index(trigger_key).map_or("Key", |k| k.name);
        let gesture = match modifier_key {
            Some(m) => format!("{}+{trig_name}", keymap::by_index(m).map_or("Mod", |k| k.name)),
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
            ShortcutActionPreset::ToggleLighting => {
                (ShortcutAction::ToggleLighting, "Toggle LED")
            }
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
            ShortcutActionPreset::PollRate8k => {
                (ShortcutAction::SetPollRate { khz: 8 }, "8 kHz Poll")
            }
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
            modifier_key,
            trigger_key,
            trigger_depth_mm: self.draft_rule_depth_mm,
            action,
        }
    }

    pub fn validate_draft_daemon_rule(&self) -> Result<(), String> {
        if self.draft_rule_modifier == Some(keymap::idx::FN)
            || self.draft_rule_key == keymap::idx::FN
        {
            return Err("Fn (#122) is reserved for [FW] Flash - pick another key.".into());
        }
        if self.draft_rule_preset == ShortcutActionPreset::RunCommand
            && self.draft_rule_command.trim().is_empty()
        {
            return Err("Shell command cannot be empty.".into());
        }
        Ok(())
    }

    pub fn commit_draft_daemon_rule(&mut self, allow_replace_conflict: bool) {
        if let Err(e) = self.validate_draft_daemon_rule() {
            self.status_msg = e;
            return;
        }
        let conflict_id = self.conflicting_daemon_rule_for_draft().map(|r| r.id);
        let gesture = self.draft_gesture_label();
        if let Some(cid) = conflict_id
            && !allow_replace_conflict
        {
            self.status_msg = format!(
                "Conflict: rule #{cid} already uses {gesture}. Load #{cid} or Replace #{cid}."
            );
            return;
        }

        let prev_rules = self.shortcut_rules.clone();
        let prev_watcher = self.shortcut_watcher_active;
        let prev_next = self.next_shortcut_id;
        let prev_edit = self.editing_daemon_rule_id;

        let ok_msg = if let Some(edit_id) = self.editing_daemon_rule_id {
            if allow_replace_conflict && let Some(cid) = conflict_id {
                self.shortcut_rules.retain(|r| r.id != cid);
            }
            if self.shortcut_rules.iter().any(|r| r.id == edit_id) {
                let mut updated = self.build_shortcut_rule_with_id(edit_id);
                if let Some(slot) = self.shortcut_rules.iter_mut().find(|r| r.id == edit_id) {
                    updated.enabled = slot.enabled;
                    *slot = updated;
                }
                self.shortcut_watcher_active = true;
                self.editing_daemon_rule_id = None;
                format!("Updated [DM] rule #{edit_id} ({gesture}) in place.")
            } else {
                self.editing_daemon_rule_id = None;
                let id = self.next_shortcut_id;
                self.next_shortcut_id += 1;
                self.shortcut_rules.push(self.build_shortcut_rule_with_id(id));
                self.shortcut_watcher_active = true;
                format!("Added [DM] rule #{id} ({gesture}).")
            }
        } else if allow_replace_conflict && let Some(cid) = conflict_id {
            let updated = self.build_shortcut_rule_with_id(cid);
            if let Some(slot) = self.shortcut_rules.iter_mut().find(|r| r.id == cid) {
                *slot = updated;
            }
            self.shortcut_watcher_active = true;
            format!("Replaced existing [DM] rule #{cid} with {gesture}.")
        } else {
            let id = self.next_shortcut_id;
            self.next_shortcut_id += 1;
            self.shortcut_rules.push(self.build_shortcut_rule_with_id(id));
            self.shortcut_watcher_active = true;
            format!("Added [DM] rule #{id} ({gesture}).")
        };

        if let Err(err) = self.sync_shortcuts() {
            self.shortcut_rules = prev_rules;
            self.shortcut_watcher_active = prev_watcher;
            self.next_shortcut_id = prev_next;
            self.editing_daemon_rule_id = prev_edit;
            self.status_msg = format!("Daemon rule save failed: {err}");
        } else {
            self.status_msg = ok_msg;
        }
    }

    pub fn save_draft_to_keyboard_fn(&mut self) {
        if self.draft_rule_key == keymap::idx::FN {
            self.status_msg = "Pick a non-Fn trigger key for Fn+Key firmware flash.".into();
            return;
        }
        let Some(target_hid) = keymap::virtual_key_to_hid_usage(self.draft_virtual_key) else {
            self.status_msg = format!(
                "{} is media/consumer; use Daemon Level instead.",
                self.draft_virtual_key.label()
            );
            return;
        };
        let idx = self.draft_rule_key;
        match self.run_measured("Write [FW] Fn Slot", |c| {
            c.apply_keyboard_fn_map(
                vec![FnOverride {
                    index: idx,
                    target_hid,
                }],
                Vec::new(),
            )
        }) {
            Ok(list) => {
                self.apply_fn_overrides_list(list);
                let kname = keymap::by_index(idx).map_or("Key", |k| k.name);
                self.status_msg = format!(
                    "Saved [FW] Fn+{kname} -> {} to onboard flash.",
                    self.draft_virtual_key.label()
                );
            }
            Err(e) => self.status_msg = format!("FW write failed: {e}"),
        }
    }

    pub fn save_arrow_cluster_to_keyboard(&mut self, preset: ArrowClusterPreset) {
        let bindings = arrow_cluster_bindings(preset);
        let mut set = Vec::with_capacity(bindings.len());
        for (idx, vk, _) in bindings {
            let Some(target_hid) = keymap::virtual_key_to_hid_usage(vk) else {
                self.status_msg = "Missing HID mapping for arrow preset.".into();
                return;
            };
            set.push(FnOverride {
                index: idx,
                target_hid,
            });
        }
        match self.run_measured("Write [FW] Arrows", |c| {
            c.apply_keyboard_fn_map(set.clone(), Vec::new())
        }) {
            Ok(list) => {
                self.apply_fn_overrides_list(list);
                self.status_msg = format!("Flashed [FW] Fn+{} to keyboard.", preset.label());
            }
            Err(e) => self.status_msg = format!("FW arrow cluster failed: {e}"),
        }
    }

    pub fn add_daemon_arrow_cluster(&mut self, preset: ArrowClusterPreset) {
        let mod_idx = self
            .draft_rule_modifier
            .filter(|&m| m != keymap::idx::FN)
            .unwrap_or(keymap::idx::CAPS);
        let mod_name = keymap::by_index(mod_idx).map_or("Caps", |k| k.name);
        let prev = self.shortcut_rules.clone();
        for (trig, vk, _) in arrow_cluster_bindings(preset) {
            let label = format!(
                "{mod_name}+{} -> {}",
                keymap::by_index(trig).map_or("Key", |k| k.name),
                vk.label()
            );
            if let Some(existing) = self
                .shortcut_rules
                .iter_mut()
                .find(|r| r.modifier_key == Some(mod_idx) && r.trigger_key == trig)
            {
                existing.enabled = true;
                existing.name = label;
                existing.trigger_depth_mm = self.draft_rule_depth_mm;
                existing.action = ShortcutAction::EmitKey { key: vk };
            } else {
                let id = self.next_shortcut_id;
                self.next_shortcut_id += 1;
                self.shortcut_rules.push(ShortcutRule {
                    id,
                    name: label,
                    enabled: true,
                    modifier_key: Some(mod_idx),
                    trigger_key: trig,
                    trigger_depth_mm: self.draft_rule_depth_mm,
                    action: ShortcutAction::EmitKey { key: vk },
                });
            }
        }
        self.shortcut_watcher_active = true;
        if let Err(e) = self.sync_shortcuts() {
            self.shortcut_rules = prev;
            self.status_msg = format!("Daemon arrows failed: {e}");
        } else {
            self.status_msg = format!("Applied [DM] {mod_name}+{} preset.", preset.label());
        }
    }

    pub fn clear_keyboard_fn_slot(&mut self, index: u8) {
        match self.run_measured("Clear [FW] Slot", |c| {
            c.apply_keyboard_fn_map(Vec::new(), vec![index])
        }) {
            Ok(list) => {
                self.apply_fn_overrides_list(list);
                self.status_msg = "Cleared [FW] Fn slot.".into();
            }
            Err(e) => self.status_msg = format!("Clear FW slot failed: {e}"),
        }
    }

    pub fn reset_keyboard_fn_map(&mut self) {
        match self.run_measured("Reset [FW] Fn", |c| c.reset_keyboard_fn_map()) {
            Ok(list) => {
                self.apply_fn_overrides_list(list);
                self.status_msg = "Reset all [FW] Fn flash overrides to factory default.".into();
            }
            Err(e) => self.status_msg = format!("Reset FW map failed: {e}"),
        }
    }

    pub fn trigger_shortcut_rule(&mut self, id: u32) {
        let matched = self.shortcut_rules.iter().find(|r| r.id == id).cloned();
        match self.run_measured("Test Fire [DM]", |c| c.trigger_shortcut(id)) {
            Ok(log) => {
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
                self.status_msg = format!("Fired #{}: {}", log.rule_id, log.message);
            }
            Err(e) => self.status_msg = format!("Test fire failed: {e}"),
        }
    }

    fn apply_fn_overrides_list(&mut self, list: Vec<FnOverride>) {
        self.keyboard_fn_map = list.into_iter().map(|o| (o.index, o.target_hid)).collect();
        let len = self.keyboard_fn_map.len();
        self.km_fw_cursor = if len > 0 { self.km_fw_cursor.min(len - 1) } else { 0 };
    }

    fn process_gesture_capture(&mut self, index: u8, mm: f64) {
        if self.draft_category == KeymapCategory::DaemonMemory && index == keymap::idx::FN {
            if mm >= 0.85 {
                self.captured_gesture_banner = Some(
                    "Fn (#122) is reserved for Firmware [FW] Flash - use Caps/Ctrl/Alt/Menu for [DM]."
                        .into(),
                );
            }
            return;
        }
        if mm >= 0.85 {
            match self.record_first_key {
                None => {
                    self.record_first_key = Some(index);
                    let first_name = keymap::by_index(index).map_or("Key", |k| k.name);
                    self.captured_gesture_banner = Some(format!(
                        "Holding {first_name}... press 2nd key for '{first_name}+Key' chord or release for single."
                    ));
                }
                Some(first) if first != index => {
                    if first == keymap::idx::FN || index == keymap::idx::FN {
                        let trig = if first == keymap::idx::FN { index } else { first };
                        self.set_keymap_draft_category(KeymapCategory::FirmwareFnFlash);
                        self.draft_rule_key = trig;
                        self.shortcut_recording = false;
                        self.record_first_key = None;
                        let t_name = keymap::by_index(trig).map_or("Key", |k| k.name);
                        self.captured_gesture_banner =
                            Some(format!("Captured [FW] chord: Fn + {t_name}!"));
                    } else {
                        self.draft_rule_modifier = Some(first);
                        self.draft_rule_key = index;
                        self.shortcut_recording = false;
                        self.record_first_key = None;
                        let m_name = keymap::by_index(first).map_or("Mod", |k| k.name);
                        let t_name = keymap::by_index(index).map_or("Key", |k| k.name);
                        self.captured_gesture_banner =
                            Some(format!("Captured chord: {m_name} + {t_name}!"));
                    }
                }
                Some(_) => {}
            }
        } else if mm <= 0.15 && self.record_first_key == Some(index) {
            if index == keymap::idx::FN {
                self.shortcut_recording = false;
                self.record_first_key = None;
                self.captured_gesture_banner =
                    Some("Fn cannot be bound alone; select a trigger key for Fn+Key.".into());
                return;
            }
            if self.draft_category == KeymapCategory::DaemonMemory {
                self.draft_rule_modifier = None;
            }
            self.draft_rule_key = index;
            self.shortcut_recording = false;
            self.record_first_key = None;
            let t_name = keymap::by_index(index).map_or("Key", |k| k.name);
            self.captured_gesture_banner = Some(format!("Captured single key: {t_name}!"));
        }
    }

    // ---- Diagnostics ----

    pub fn run_diagnostics(&mut self) {
        match self.run_measured("4-Layer Diagnostics", |c| c.run_diagnostics()) {
            Ok(rep) => {
                self.status_msg = format!("Diagnostics: {}", rep.attribution.summary);
                self.diag_report = Some(rep);
            }
            Err(e) => self.status_msg = format!("Diagnostics failed: {e}"),
        }
    }

    pub fn run_echo_benchmark(&mut self) {
        self.echo_latencies_ms.clear();
        for _ in 0..5 {
            let t0 = Instant::now();
            match self.call_with_reconnect(|c| c.get_poll_rate()) {
                Ok(_) => self
                    .echo_latencies_ms
                    .push(t0.elapsed().as_secs_f64() * 1000.0),
                Err(e) => {
                    self.status_msg = format!("Echo burst failed: {e}");
                    return;
                }
            }
        }
        let min = self.echo_latencies_ms.iter().copied().fold(f64::INFINITY, f64::min);
        let max = self.echo_latencies_ms.iter().copied().fold(0.0f64, f64::max);
        let avg =
            self.echo_latencies_ms.iter().sum::<f64>() / (self.echo_latencies_ms.len().max(1) as f64);
        let msg = format!("5x HID Echo RTT: min={min:.2} avg={avg:.2} max={max:.2} ms");
        self.record_feedback("HID Echo Burst (5x)", avg, true, msg.clone());
        self.status_msg = msg;
    }

    pub fn focused_switch_inspection(&self) -> SwitchInspectionSnapshot {
        let key_index = self.current_key().index;
        let key_name = self.current_key().name;
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

        let stat = self.switch_tests.get(&key_index);
        let peak_mm = stat.map_or(travel_mm, |s| s.peak_mm.max(travel_mm));
        let presses = stat.map_or(0, |s| s.presses);
        let released_zero = stat.is_some_and(|s| s.released_zero);
        let triggered = travel_mm >= actuation_mm;
        let frac = (travel_mm / max_mm).clamp(0.0, 1.0);
        let flux_pct = ((0.22 + 0.78 * frac) * frac * 100.0).clamp(0.0, 100.0);

        SwitchInspectionSnapshot {
            key_index,
            key_name,
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

    pub fn submit_prompt(&mut self) {
        let Some(mut modal) = self.prompt_modal.take() else {
            return;
        };
        let raw = modal.buffer.trim().to_string();
        match modal.purpose {
            PromptPurpose::ShellCommand => {
                if raw.is_empty() {
                    modal.error = Some("Shell command cannot be empty.".into());
                    self.prompt_modal = Some(modal);
                    return;
                }
                self.draft_rule_command = raw;
                self.draft_rule_preset = ShortcutActionPreset::RunCommand;
                self.status_msg = "Updated draft shell command.".into();
            }
            PromptPurpose::LightFgHex => {
                if let Some(rgb) = crate::theme::parse_hex_or_rgb(&raw) {
                    self.light_state.fg = rgb;
                    self.light_state.full_color = false;
                    self.status_msg = format!(
                        "Foreground -> {}",
                        crate::theme::format_rgb_hex(rgb)
                    );
                } else {
                    modal.error = Some("Invalid color (use #RRGGBB or R,G,B).".into());
                    self.prompt_modal = Some(modal);
                }
            }
            PromptPurpose::LightBgHex => {
                if let Some(rgb) = crate::theme::parse_hex_or_rgb(&raw) {
                    self.light_state.bg = rgb;
                    self.status_msg = format!(
                        "Background -> {}",
                        crate::theme::format_rgb_hex(rgb)
                    );
                } else {
                    modal.error = Some("Invalid color (use #RRGGBB or R,G,B).".into());
                    self.prompt_modal = Some(modal);
                }
            }
            PromptPurpose::CustomBrushHex => {
                if let Some(rgb) = crate::theme::parse_hex_or_rgb(&raw) {
                    self.custom_brush = rgb;
                    self.status_msg = format!(
                        "Brush color -> {}",
                        crate::theme::format_rgb_hex(rgb)
                    );
                } else {
                    modal.error = Some("Invalid color (use #RRGGBB or R,G,B).".into());
                    self.prompt_modal = Some(modal);
                }
            }
            PromptPurpose::SocdCustomPair => {
                if let Some(pairs) = parse_socd_pairs_input(&raw) {
                    self.socd_pairs = pairs;
                    self.socd_enabled = true;
                    self.status_msg = format!(
                        "Draft SOCD updated ({} pair(s)) - press Apply SOCD to write.",
                        self.socd_pairs.len()
                    );
                } else {
                    modal.error = Some("Expected e.g. A:D:2 or W:S:1,A:D:1".into());
                    self.prompt_modal = Some(modal);
                }
            }
        }
    }

    pub fn preset_color_name(rgb: [u8; 3]) -> &'static str {
        HUMAN_COLOR_PRESETS
            .iter()
            .find(|(_, c)| *c == rgb)
            .map_or_else(|| crate::theme::describe_rgb_human(rgb), |(n, _)| *n)
    }
}

pub fn parse_socd_pairs_input(raw: &str) -> Option<Vec<PrcsPair>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(Vec::new());
    }
    let mut pairs = Vec::new();
    for seg in trimmed.split([',', ';']).filter(|s| !s.trim().is_empty()) {
        let parts: Vec<&str> = seg.trim().split(':').collect();
        if parts.len() < 2 || parts.len() > 3 {
            return None;
        }
        let k1 = keymap::resolve_key(parts[0].trim())?;
        let k2 = keymap::resolve_key(parts[1].trim())?;
        let model = if parts.len() == 3 {
            parts[2].trim().parse::<u8>().ok()?.saturating_sub(1).min(3)
        } else {
            1
        };
        pairs.push(PrcsPair {
            key1_hid: k1.hid_code,
            key2_hid: k2.hid_code,
            model,
        });
    }
    Some(pairs)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_socd_pair_expressions() {
        let pairs = parse_socd_pairs_input("A:D:2, W:S:1").unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].key1_hid, 0x04);
        assert_eq!(pairs[0].key2_hid, 0x07);
        assert_eq!(pairs[0].model, 1);
        assert!(parse_socd_pairs_input("INVALIDKEY:D").is_none());
    }

    #[test]
    fn mascot_easter_egg_triggers_on_fast_digits() {
        let mut st = MascotEasterEggState::default();
        let t0 = Instant::now();
        assert!(!st.record_code("3", t0));
        assert!(st.record_code("9", t0 + Duration::from_millis(120)));
        assert!(st.miku_visible);
        assert!(!st.record_code("4", t0 + Duration::from_millis(200)));
        assert!(st.record_code("1", t0 + Duration::from_millis(350)));
        assert!(st.teto_visible);
    }

    #[test]
    fn prompt_modal_handles_cursor_insert_and_backspace() {
        let mut modal = TextPromptModal::new(PromptPurpose::ShellCommand, "Cmd", "", "konsole");
        modal.move_left();
        modal.move_left();
        modal.insert_str(" -e ");
        assert_eq!(modal.buffer, "konso -e le");
        modal.backspace();
        assert_eq!(modal.buffer, "konso -ele");
    }
}
