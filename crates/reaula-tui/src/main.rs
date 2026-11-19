use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Tabs};

use reaula_core::audio_fx::{AudioEffectStyle, AudioPalette};
use reaula_core::driver::DeviceProbe;
use reaula_core::ipc::{
    AudioSyncConfig, AudioSyncSource, AudioSyncStatus, DaemonClient, DaemonStatus, PeerCredentials,
};
use reaula_core::keymap::{self, KeyDef, KEYBOARD_CANVAS_WIDTH, WIN60HE_KEYS};
use reaula_core::protocol::{
    self, LightState, LiveTravelSample, PrcsPair, TriggerConfig, TriggerMode,
};

#[derive(Parser)]
#[command(name = "reaula-tui", version, about = "Ratatui TUI for Aula WIN 60 HE")]
struct Args {
    /// Connect to a specific reaulad Unix domain socket
    #[arg(long)]
    socket: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Keyboard = 0,
    Actuation = 1,
    Lighting = 2,
    System = 3,
}

impl ActiveTab {
    fn from_index(i: usize) -> Self {
        match i % 4 {
            0 => Self::Keyboard,
            1 => Self::Actuation,
            2 => Self::Lighting,
            _ => Self::System,
        }
    }
}

const LIGHT_MODE_OPTIONS: [u8; 16] = [
    0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12, 14, 15, 16, 100,
];

const COLOR_PRESETS: [(&str, [u8; 3]); 7] = [
    ("Red", [255, 0, 0]),
    ("Cyan", [0, 220, 255]),
    ("Mint", [0, 255, 160]),
    ("Violet", [170, 70, 255]),
    ("Amber", [255, 170, 0]),
    ("Pink", [255, 50, 150]),
    ("White", [255, 255, 255]),
];

struct TravelThreadHandle {
    stop: Arc<AtomicBool>,
    rx: Receiver<(LiveTravelSample, f64)>,
}

impl Drop for TravelThreadHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

struct App {
    socket_arg: Option<PathBuf>,
    client: DaemonClient,
    server_cred: PeerCredentials,
    daemon_status: DaemonStatus,
    probe: DeviceProbe,
    unit_mm: f64,
    min_mm: f64,
    max_mm: f64,
    tab: ActiveTab,

    // Selection & keyboard state
    cursor_pos: usize,
    selected_indices: BTreeSet<u8>,
    actuations: BTreeMap<u8, TriggerConfig>,
    switches: BTreeMap<u8, u8>,
    deadbands: BTreeMap<u8, (u8, u8)>,

    // Actuation editor draft
    draft_mode: TriggerMode,
    draft_travel_mm: f64,
    draft_rt_press_mm: f64,
    draft_rt_release_mm: f64,
    draft_switch_id: u8,

    // Lighting draft
    light_state: LightState,
    light_mode_cursor: usize,
    color_preset_idx: usize,

    // Daemon-hosted Audio Equalizer telemetry & controls
    audio_status: AudioSyncStatus,
    audio_config: AudioSyncConfig,
    last_audio_poll: Instant,

    // SOCD / Poll rate
    poll_rate_khz: u8,
    socd_enabled: bool,
    socd_pairs: Vec<PrcsPair>,

    // Live travel
    travel_enabled: bool,
    travel_handle: Option<TravelThreadHandle>,
    live_depths: BTreeMap<u8, (f64, Instant)>,
    last_sample: Option<(u8, f64)>,

    status_msg: String,
}

impl App {
    fn connect(socket_arg: Option<PathBuf>) -> Result<Self, String> {
        let mut client = DaemonClient::connect(socket_arg.as_deref()).map_err(|e| {
            format!(
                "could not connect to reaulad ({e}).\n\
                 Start the secure daemon first:\n  reaula daemon start"
            )
        })?;
        let server_cred = client.server_credentials();
        let daemon_status = client.ping().map_err(|e| e.to_string())?;
        let probe = client.probe().map_err(|e| e.to_string())?;

        let (unit_mm, min_mm, max_mm) = probe
            .max_travel
            .map_or((0.01, 0.08, 3.40), |m| (m.unit_mm, m.min_mm, m.max_mm));

        let light_state = probe.light_state.unwrap_or_default();
        let light_mode_cursor = LIGHT_MODE_OPTIONS
            .iter()
            .position(|&m| m == light_state.mode)
            .unwrap_or(0);

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

        let poll_rate_khz = probe.poll_rate_khz.unwrap_or(8);
        let (socd_enabled, socd_pairs) = client.get_socd().unwrap_or((false, Vec::new()));

        let mut selected_indices = BTreeSet::new();
        selected_indices.insert(46); // W key default selection

        let (draft_mode, draft_travel_mm, draft_rt_press_mm, draft_rt_release_mm) =
            if let Some(cfg) = actuations.get(&46) {
                (
                    cfg.mode,
                    f64::from(cfg.travel_raw) * unit_mm,
                    (f64::from(cfg.interval1_raw) * unit_mm).max(0.05),
                    (f64::from(cfg.interval2_raw) * unit_mm).max(0.05),
                )
            } else {
                (TriggerMode::Fixed, 1.70, 0.30, 0.30)
            };

        let draft_switch_id = switches.get(&46).copied().unwrap_or(1);

        let audio_status = client.get_audio_sync_state().unwrap_or_default();
        let audio_config = if audio_status.active {
            audio_status.config.clone()
        } else {
            AudioSyncConfig {
                brightness: light_state.brightness.max(1),
                ..AudioSyncConfig::default()
            }
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
            tab: ActiveTab::Keyboard,
            cursor_pos: WIN60HE_KEYS.iter().position(|k| k.index == 46).unwrap_or(0),
            selected_indices,
            actuations,
            switches,
            deadbands,
            draft_mode,
            draft_travel_mm,
            draft_rt_press_mm,
            draft_rt_release_mm,
            draft_switch_id,
            light_state,
            light_mode_cursor,
            color_preset_idx: 0,
            audio_status,
            audio_config,
            last_audio_poll: Instant::now(),
            poll_rate_khz,
            socd_enabled,
            socd_pairs,
            travel_enabled: false,
            travel_handle: None,
            live_depths: BTreeMap::new(),
            last_sample: None,
            status_msg: "Connected to reaulad. Press 't' for live Hall-effect travel, '1-4' to switch tabs, 'q' to quit.".into(),
        };
        app.set_travel_streaming(true);
        Ok(app)
    }

    fn current_key(&self) -> KeyDef {
        WIN60HE_KEYS[self.cursor_pos]
    }

    fn target_indices(&self) -> Vec<u8> {
        if self.selected_indices.is_empty() {
            vec![self.current_key().index]
        } else {
            self.selected_indices.iter().copied().collect()
        }
    }

    fn sync_draft_from_cursor(&mut self) {
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
    }

    fn set_travel_streaming(&mut self, enable: bool) {
        if enable == self.travel_enabled {
            return;
        }
        if !enable {
            self.travel_handle = None;
            self.travel_enabled = false;
            self.status_msg = "Live travel streaming paused.".into();
            return;
        }

        let sock = self.socket_arg.clone();
        let Ok(stream_client) = DaemonClient::connect(sock.as_deref()) else {
            self.status_msg = "Failed to open travel stream socket.".into();
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
        self.status_msg = "Live travel streaming ON (press keys on WIN 60 HE).".into();
    }

    fn drain_travel_samples(&mut self) {
        let Some(handle) = &self.travel_handle else {
            return;
        };
        let now = Instant::now();
        while let Ok((sample, unit_mm)) = handle.rx.try_recv() {
            let mm = sample.depth_mm(unit_mm);
            self.live_depths.insert(sample.index, (mm, now));
            self.last_sample = Some((sample.index, mm));
        }
        self.live_depths
            .retain(|_, (mm, ts)| *mm > 0.01 || now.duration_since(*ts) < Duration::from_millis(600));
    }

    fn refresh_all(&mut self) {
        match self.client.probe() {
            Ok(p) => self.probe = p,
            Err(e) => {
                self.status_msg = format!("Probe failed: {e}");
                return;
            }
        }
        let all_idx = keymap::all_indices();
        if let Ok((_, entries)) = self.client.get_actuation(all_idx.clone()) {
            for e in entries {
                if let Some(cfg) = e.config {
                    self.actuations.insert(e.index, cfg);
                }
            }
        }
        if let Ok(entries) = self.client.get_switches(all_idx) {
            for e in entries {
                self.switches.insert(e.index, e.switch_id);
            }
        }
        if let Ok((en, pairs)) = self.client.get_socd() {
            self.socd_enabled = en;
            self.socd_pairs = pairs;
        }
        if let Ok(st) = self.client.ping() {
            self.daemon_status = st;
        }
        if let Ok(st) = self.client.get_audio_sync_state() {
            if st.active {
                self.audio_config = st.config.clone();
            }
            self.audio_status = st;
        }
        self.status_msg = "Refreshed device state from reaulad.".into();
    }

    fn poll_audio_sync(&mut self) {
        let interval = if self.audio_status.active || self.tab == ActiveTab::Lighting {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(500)
        };
        if self.last_audio_poll.elapsed() < interval {
            return;
        }
        self.last_audio_poll = Instant::now();
        if let Ok(st) = self.client.get_audio_sync_state() {
            let was_active = self.audio_status.active;
            if st.active {
                self.audio_config = st.config.clone();
            } else if was_active && let Some(err) = &st.last_error {
                self.status_msg = format!("Audio Sync stopped: {err}");
            }
            self.audio_status = st;
        }
    }

    fn toggle_audio_sync(&mut self) {
        if self.audio_status.active {
            match self.client.stop_audio_sync(true) {
                Ok(st) => {
                    self.audio_status = st;
                    self.status_msg = "Stopped daemon Audio Sync (restored lighting).".into();
                }
                Err(e) => self.status_msg = format!("Audio Sync stop failed: {e}"),
            }
        } else {
            match self.client.start_audio_sync(self.audio_config.clone()) {
                Ok(st) => {
                    self.audio_status = st;
                    self.status_msg = format!(
                        "Started daemon Audio Sync ({}).",
                        self.audio_config.source.label()
                    );
                }
                Err(e) => self.status_msg = format!("Audio Sync start failed: {e}"),
            }
        }
    }

    fn push_audio_update(&mut self, msg: &str) {
        if !self.audio_status.active {
            self.status_msg = msg.into();
            return;
        }
        match self.client.update_audio_sync(self.audio_config.clone()) {
            Ok(st) => {
                self.audio_status = st;
                self.status_msg = msg.into();
            }
            Err(e) => self.status_msg = format!("Audio Sync update failed: {e}"),
        }
    }

    fn cycle_audio_source(&mut self) {
        self.audio_config.source = match self.audio_config.source {
            AudioSyncSource::Monitor => AudioSyncSource::DefaultMic,
            AudioSyncSource::DefaultMic => AudioSyncSource::Demo,
            AudioSyncSource::Demo | AudioSyncSource::Device { .. } => AudioSyncSource::Monitor,
        };
        let label = self.audio_config.source.label();
        self.push_audio_update(&format!("Audio source -> {label}"));
    }

    fn cycle_audio_style(&mut self) {
        let pos = AudioEffectStyle::ALL
            .iter()
            .position(|&s| s == self.audio_config.style)
            .unwrap_or(0);
        let next = AudioEffectStyle::ALL[(pos + 1) % AudioEffectStyle::ALL.len()];
        self.audio_config.style = next;
        self.push_audio_update(&format!("Audio effect -> {}", next.label()));
    }

    fn cycle_audio_palette(&mut self) {
        let pos = AudioPalette::ALL
            .iter()
            .position(|&p| p == self.audio_config.palette)
            .unwrap_or(0);
        let next = AudioPalette::ALL[(pos + 1) % AudioPalette::ALL.len()];
        self.audio_config.palette = next;
        self.push_audio_update(&format!("Audio palette -> {}", next.label()));
    }

    fn move_cursor(&mut self, dr: i32, dc: i32) {
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

    fn apply_actuation(&mut self) {
        let indices = self.target_indices();
        let (rt_p, rt_r) = match self.draft_mode {
            TriggerMode::Fixed => (None, None),
            TriggerMode::RapidTrigger => (Some(self.draft_rt_press_mm), None),
            TriggerMode::RapidTriggerSplit => {
                (Some(self.draft_rt_press_mm), Some(self.draft_rt_release_mm))
            }
            TriggerMode::Unknown(_) => (None, None),
        };
        match self.client.set_actuation(
            indices.clone(),
            self.draft_mode,
            self.draft_travel_mm,
            rt_p,
            rt_r,
        ) {
            Ok(()) => {
                if let Ok((_, entries)) = self.client.get_actuation(indices.clone()) {
                    for e in entries {
                        if let Some(cfg) = e.config {
                            self.actuations.insert(e.index, cfg);
                        }
                    }
                }
                self.status_msg = format!(
                    "Applied {} @ {:.2} mm to {} key(s).",
                    self.draft_mode.label(),
                    self.draft_travel_mm,
                    indices.len()
                );
            }
            Err(e) => self.status_msg = format!("Actuation write failed: {e}"),
        }
    }

    fn apply_switch(&mut self) {
        let indices = self.target_indices();
        match self.client.set_switches(indices.clone(), self.draft_switch_id) {
            Ok(()) => {
                for idx in &indices {
                    self.switches.insert(*idx, self.draft_switch_id);
                }
                self.status_msg = format!(
                    "Set switch {} on {} key(s).",
                    protocol::switch_profile_name(self.draft_switch_id),
                    indices.len()
                );
            }
            Err(e) => self.status_msg = format!("Switch write failed: {e}"),
        }
    }

    fn apply_lighting(&mut self) {
        match self.client.set_light(self.light_state) {
            Ok(()) => {
                self.audio_status.active = false;
                self.status_msg = format!(
                    "Applied lighting: {} (bri={}, spd={}, #{:02X}{:02X}{:02X}).",
                    protocol::light_mode_name(self.light_state.mode),
                    self.light_state.brightness,
                    self.light_state.speed,
                    self.light_state.fg[0],
                    self.light_state.fg[1],
                    self.light_state.fg[2],
                );
            }
            Err(e) => self.status_msg = format!("Lighting write failed: {e}"),
        }
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if matches!(code, KeyCode::Char('q'))
            || (matches!(code, KeyCode::Char('c')) && modifiers.contains(KeyModifiers::CONTROL))
        {
            return true;
        }

        match code {
            KeyCode::Char('1') => self.tab = ActiveTab::Keyboard,
            KeyCode::Char('2') => self.tab = ActiveTab::Actuation,
            KeyCode::Char('3') => self.tab = ActiveTab::Lighting,
            KeyCode::Char('4') => self.tab = ActiveTab::System,
            KeyCode::Tab => self.tab = ActiveTab::from_index((self.tab as usize) + 1),
            KeyCode::BackTab => self.tab = ActiveTab::from_index((self.tab as usize) + 3),
            KeyCode::Char('t') => self.set_travel_streaming(!self.travel_enabled),
            KeyCode::Char('r') => self.refresh_all(),
            _ => match self.tab {
                ActiveTab::Keyboard => self.handle_keyboard_key(code),
                ActiveTab::Actuation => self.handle_actuation_key(code),
                ActiveTab::Lighting => self.handle_lighting_key(code),
                ActiveTab::System => self.handle_system_key(code),
            },
        }
        false
    }

    fn handle_keyboard_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Left | KeyCode::Char('h') => self.move_cursor(0, -1),
            KeyCode::Right | KeyCode::Char('l') => self.move_cursor(0, 1),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1, 0),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1, 0),
            KeyCode::Char(' ') => {
                let idx = self.current_key().index;
                if !self.selected_indices.remove(&idx) {
                    self.selected_indices.insert(idx);
                }
            }
            KeyCode::Char('a') => {
                self.selected_indices = keymap::all_indices().into_iter().collect();
                self.status_msg = "Selected all 61 keys.".into();
            }
            KeyCode::Char('w') => {
                self.selected_indices = [46, 68, 69, 70].into_iter().collect();
                self.status_msg = "Selected WASD.".into();
            }
            KeyCode::Char('x') => {
                self.selected_indices.clear();
                self.selected_indices.insert(self.current_key().index);
            }
            _ => {}
        }
    }

    fn handle_actuation_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Left | KeyCode::Char('[') | KeyCode::Char('-') => {
                self.draft_travel_mm = (self.draft_travel_mm - 0.05).max(self.min_mm);
            }
            KeyCode::Right | KeyCode::Char(']') | KeyCode::Char('+') | KeyCode::Char('=') => {
                self.draft_travel_mm = (self.draft_travel_mm + 0.05).min(self.max_mm);
            }
            KeyCode::Char('m') => {
                self.draft_mode = match self.draft_mode {
                    TriggerMode::Fixed => TriggerMode::RapidTrigger,
                    TriggerMode::RapidTrigger => TriggerMode::RapidTriggerSplit,
                    _ => TriggerMode::Fixed,
                };
            }
            KeyCode::Char(',') => {
                self.draft_rt_press_mm = (self.draft_rt_press_mm - 0.05).max(0.05);
            }
            KeyCode::Char('.') => {
                self.draft_rt_press_mm = (self.draft_rt_press_mm + 0.05).min(2.00);
            }
            KeyCode::Char(';') => {
                self.draft_rt_release_mm = (self.draft_rt_release_mm - 0.05).max(0.05);
            }
            KeyCode::Char('\'') => {
                self.draft_rt_release_mm = (self.draft_rt_release_mm + 0.05).min(2.00);
            }
            KeyCode::Char('s') => {
                self.draft_switch_id = match self.draft_switch_id {
                    1 => 2,
                    2 => 3,
                    3 => 5,
                    _ => 1,
                };
            }
            KeyCode::Char('S') => self.apply_switch(),
            KeyCode::Enter | KeyCode::Char('p') => self.apply_actuation(),
            _ => {}
        }
    }

    fn handle_lighting_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.light_mode_cursor = self.light_mode_cursor.saturating_sub(1);
                self.light_state.mode = LIGHT_MODE_OPTIONS[self.light_mode_cursor];
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.light_mode_cursor + 1 < LIGHT_MODE_OPTIONS.len() {
                    self.light_mode_cursor += 1;
                }
                self.light_state.mode = LIGHT_MODE_OPTIONS[self.light_mode_cursor];
            }
            KeyCode::Char('b') => {
                self.light_state.brightness = self.light_state.brightness.saturating_sub(1);
                if self.audio_status.active {
                    self.audio_config.brightness = self.light_state.brightness.max(1);
                    self.push_audio_update(&format!(
                        "Audio brightness -> {}/4",
                        self.audio_config.brightness
                    ));
                }
            }
            KeyCode::Char('B') => {
                self.light_state.brightness = (self.light_state.brightness + 1).min(4);
                if self.audio_status.active {
                    self.audio_config.brightness = self.light_state.brightness.max(1);
                    self.push_audio_update(&format!(
                        "Audio brightness -> {}/4",
                        self.audio_config.brightness
                    ));
                }
            }
            KeyCode::Char('v') => {
                self.light_state.speed = self.light_state.speed.saturating_sub(1);
            }
            KeyCode::Char('V') => {
                self.light_state.speed = (self.light_state.speed + 1).min(4);
            }
            KeyCode::Char('d') => {
                self.light_state.direction = (self.light_state.direction + 1) % 6;
            }
            KeyCode::Char('c') => {
                self.color_preset_idx = (self.color_preset_idx + 1) % COLOR_PRESETS.len();
                self.light_state.fg = COLOR_PRESETS[self.color_preset_idx].1;
            }
            KeyCode::Char('o') => {
                self.light_state.power_on = !self.light_state.power_on;
            }
            KeyCode::Char('a') => self.toggle_audio_sync(),
            KeyCode::Char('A') => self.cycle_audio_source(),
            KeyCode::Char('s') => self.cycle_audio_style(),
            KeyCode::Char('p') => self.cycle_audio_palette(),
            KeyCode::Enter => self.apply_lighting(),
            _ => {}
        }
    }

    fn handle_system_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('e') => {
                let next = !self.socd_enabled;
                match self.client.set_socd(next, self.socd_pairs.clone()) {
                    Ok(()) => {
                        self.socd_enabled = next;
                        self.status_msg = format!("PRCS/SOCD {}", if next { "enabled" } else { "disabled" });
                    }
                    Err(e) => self.status_msg = format!("SOCD update failed: {e}"),
                }
            }
            KeyCode::Char('d') => {
                let ad = PrcsPair {
                    model: 1,
                    key1_hid: 0x04, // A
                    key2_hid: 0x07, // D
                };
                let pairs = if self.socd_pairs.is_empty() {
                    vec![ad]
                } else {
                    Vec::new()
                };
                match self.client.set_socd(self.socd_enabled, pairs.clone()) {
                    Ok(()) => {
                        self.socd_pairs = pairs;
                        self.status_msg = "Updated SOCD pairs.".into();
                    }
                    Err(e) => self.status_msg = format!("SOCD pair update failed: {e}"),
                }
            }
            KeyCode::Char('p') => {
                let next = match self.poll_rate_khz {
                    1 => 2,
                    2 => 4,
                    4 => 8,
                    _ => 1,
                };
                match self.client.set_poll_rate(next) {
                    Ok(()) => {
                        self.poll_rate_khz = next;
                        self.status_msg = format!("Set poll rate to {next} kHz (keyboard restarts).");
                    }
                    Err(e) => self.status_msg = format!("Poll rate failed: {e}"),
                }
            }
            _ => {}
        }
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("reaula-tui error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<(), String> {
    let mut app = App::connect(args.socket)?;

    enable_raw_mode().map_err(|e| format!("failed to enable raw terminal: {e}"))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).map_err(|e| e.to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(|e| e.to_string())?;

    let loop_res = run_event_loop(&mut terminal, &mut app);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    loop_res
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), String> {
    loop {
        app.drain_travel_samples();
        app.poll_audio_sync();
        terminal
            .draw(|f| draw_ui(f, app))
            .map_err(|e| e.to_string())?;

        if event::poll(Duration::from_millis(20)).map_err(|e| e.to_string())?
            && let Event::Key(key) = event::read().map_err(|e| e.to_string())?
            && key.kind == KeyEventKind::Press
            && app.handle_key(key.code, key.modifiers)
        {
            return Ok(());
        }
    }
}

fn draw_ui(f: &mut ratatui::Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(14),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Header + tabs
    let fw = app.probe.firmware_string.as_deref().unwrap_or("WIN 60 HE");
    let audio_badge = if app.audio_status.active {
        format!("ON ({:.0} fps)", app.audio_status.actual_fps)
    } else {
        "OFF".into()
    };
    let header_title = format!(
        " reaula-tui | {} | {} kHz | uid={} (pid={}) | Travel: {} | Audio EQ: {} ",
        fw,
        app.poll_rate_khz,
        app.server_cred.uid,
        app.daemon_status.daemon_pid,
        if app.travel_enabled { "ON" } else { "OFF" },
        audio_badge,
    );
    let tabs = Tabs::new(vec![
        "1: Keyboard & Live Travel",
        "2: Actuation & Switch",
        "3: Lighting",
        "4: SOCD & Security",
    ])
    .block(Block::default().borders(Borders::ALL).title(header_title))
    .select(app.tab as usize)
    .style(Style::default().fg(Color::Gray))
    .highlight_style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(tabs, chunks[0]);

    match app.tab {
        ActiveTab::Keyboard => draw_keyboard_tab(f, chunks[1], app),
        ActiveTab::Actuation => draw_actuation_tab(f, chunks[1], app),
        ActiveTab::Lighting => draw_lighting_tab(f, chunks[1], app),
        ActiveTab::System => draw_system_tab(f, chunks[1], app),
    }

    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" STATUS: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(&app.status_msg),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

fn draw_keyboard_tab(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(17), Constraint::Length(6)])
        .split(area);

    let kb_block = Block::default()
        .borders(Borders::ALL)
        .title(" WIN 60 HE 60% Layout (Arrows/hjkl: move, Space: toggle select, a: all, w: WASD, t: live travel) ");
    let inner = kb_block.inner(split[0]);
    f.render_widget(kb_block, split[0]);

    let row_h = (inner.height / 5).max(3);
    for key in WIN60HE_KEYS {
        let row_idx = u16::from(key.row().saturating_sub(1));
        if row_idx >= 5 {
            continue;
        }
        let x0 = inner.x + ((u32::from(key.x.saturating_sub(10)) * u32::from(inner.width)) / u32::from(KEYBOARD_CANVAS_WIDTH - 20)) as u16;
        let x1 = inner.x + ((u32::from(key.x.saturating_sub(10) + key.width) * u32::from(inner.width)) / u32::from(KEYBOARD_CANVAS_WIDTH - 20)) as u16;
        let y0 = inner.y + row_idx * row_h;
        if x1 <= x0 || y0 + 2 >= inner.y + inner.height {
            continue;
        }
        let rect = Rect::new(x0, y0, (x1 - x0).max(3), row_h.min(inner.y + inner.height - y0));

        let is_cursor = key.index == app.current_key().index;
        let is_selected = app.selected_indices.contains(&key.index);
        let depth = app.live_depths.get(&key.index).map(|(d, _)| *d).unwrap_or(0.0);

        let border_color = if is_cursor {
            Color::Yellow
        } else if depth > 0.15 {
            Color::LightGreen
        } else if is_selected {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        let bg = if depth > 0.15 {
            Color::Rgb(18, (60.0 + (depth / app.max_mm).min(1.0) * 140.0) as u8, 70)
        } else if is_selected {
            Color::Rgb(20, 42, 68)
        } else {
            Color::Reset
        };

        let sub_text = if depth > 0.02 {
            format!("{depth:.2}")
        } else if let Some(cfg) = app.actuations.get(&key.index) {
            format!("{:.1}", f64::from(cfg.travel_raw) * app.unit_mm)
        } else {
            String::new()
        };

        let widget = Paragraph::new(vec![
            Line::from(Span::styled(
                key.name,
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(sub_text, Style::default().fg(Color::Gray))),
        ])
        .style(Style::default().bg(bg))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        );
        f.render_widget(widget, rect);
    }

    // Key Inspector + Live Depth Gauge
    let cur = app.current_key();
    let cur_cfg = app.actuations.get(&cur.index);
    let cur_sw = app.switches.get(&cur.index).copied().unwrap_or(0);
    let cur_db = app.deadbands.get(&cur.index).copied().unwrap_or((0, 0));
    let (live_idx, live_mm) = app.last_sample.unwrap_or((cur.index, 0.0));
    let live_key_name = keymap::by_index(live_idx).map_or("?", |k| k.name);

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(split[1]);

    let info_lines = vec![
        Line::from(format!(
            "Focused: {} (slot {}, HID 0x{:02X}) | Selected count: {}",
            cur.name,
            cur.index,
            cur.hid_code,
            app.selected_indices.len()
        )),
        Line::from(match cur_cfg {
            Some(c) => format!(
                "Actuation: {} @ {:.2} mm | RT: press {:.2} / rel {:.2} mm",
                c.mode.label(),
                f64::from(c.travel_raw) * app.unit_mm,
                f64::from(c.interval1_raw) * app.unit_mm,
                f64::from(c.interval2_raw) * app.unit_mm,
            ),
            None => "Actuation: <not read>".into(),
        }),
        Line::from(format!(
            "Switch: {} | Deadband: top {:.2} / bottom {:.2} mm",
            protocol::switch_profile_name(cur_sw),
            f64::from(cur_db.0) * 0.01,
            f64::from(cur_db.1) * 0.01
        )),
    ];
    f.render_widget(
        Paragraph::new(info_lines).block(Block::default().borders(Borders::ALL).title(" Key Inspector ")),
        bottom[0],
    );

    let ratio = (live_mm / app.max_mm.max(0.1)).clamp(0.0, 1.0);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Live Travel ({live_key_name}) ")),
        )
        .gauge_style(Style::default().fg(Color::LightGreen))
        .ratio(ratio)
        .label(format!("{live_mm:.2} / {:.2} mm", app.max_mm));
    f.render_widget(gauge, bottom[1]);
}

fn draw_actuation_tab(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let lines = vec![
        Line::from(format!(
            "Target keys ({}): {:?}",
            app.target_indices().len(),
            app.target_indices()
                .iter()
                .take(12)
                .map(|&i| keymap::by_index(i).map_or("?", |k| k.name))
                .collect::<Vec<_>>()
        )),
        Line::from(""),
        Line::from(format!(
            "  Trigger Mode       : {}   [press 'm' to cycle Fixed / RT / RT Split]",
            app.draft_mode.label()
        )),
        Line::from(format!(
            "  Actuation Travel   : {:.2} mm   [press '[' / ']' or Left / Right to adjust, range {:.2}..={:.2}]",
            app.draft_travel_mm, app.min_mm, app.max_mm
        )),
        Line::from(format!(
            "  RT Press Sens      : {:.2} mm   [press ',' / '.' to adjust]",
            app.draft_rt_press_mm
        )),
        Line::from(format!(
            "  RT Release Sens    : {:.2} mm   [press ';' / '\\'' to adjust]",
            app.draft_rt_release_mm
        )),
        Line::from(""),
        Line::from("  [Enter] or 'p' -> Apply Actuation to selected key(s)"),
        Line::from(""),
        Line::from(format!(
            "  Switch Profile     : {}   [press 's' to cycle, 'S' to apply]",
            protocol::switch_profile_name(app.draft_switch_id)
        )),
    ];
    let para = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Actuation, Rapid Trigger & Switch Profile "),
    );
    f.render_widget(para, area);
}

fn draw_lighting_tab(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let items: Vec<ListItem> = LIGHT_MODE_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, &m)| {
            let marker = if i == app.light_mode_cursor { "▶ " } else { "  " };
            let style = if i == app.light_mode_cursor {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("{marker}{:>3} - {}", m, protocol::light_mode_name(m))).style(style)
        })
        .collect();
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Firmware Effects (Up/Down) "),
        ),
        split[0],
    );

    let right_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(10)])
        .split(split[1]);

    let details = vec![
        Line::from(format!(
            "Mode       : {} ({})",
            app.light_state.mode,
            protocol::light_mode_name(app.light_state.mode)
        )),
        Line::from(format!(
            "Brightness : {}/4   [b / B]",
            app.light_state.brightness
        )),
        Line::from(format!("Speed      : {}/4   [v / V]", app.light_state.speed)),
        Line::from(format!(
            "Direction  : {} ({})   [d]",
            app.light_state.direction,
            protocol::direction_name(app.light_state.direction)
        )),
        Line::from(format!(
            "Foreground : #{:02X}{:02X}{:02X} ({})   [c]",
            app.light_state.fg[0],
            app.light_state.fg[1],
            app.light_state.fg[2],
            COLOR_PRESETS[app.color_preset_idx].0
        )),
        Line::from(format!(
            "LED Power  : {}   [o]",
            if app.light_state.power_on { "ON" } else { "OFF" }
        )),
        Line::from(""),
        Line::from("[Enter] -> Apply Lighting to WIN 60 HE"),
    ];
    f.render_widget(
        Paragraph::new(details).block(Block::default().borders(Borders::ALL).title(" Controls ")),
        right_split[0],
    );

    const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let band_spans: Vec<Span<'static>> = app
        .audio_status
        .metrics
        .bands
        .iter()
        .map(|&v| {
            let lvl = v.clamp(0.0, 1.0);
            let idx = (lvl * 8.0).round() as usize;
            let ch = BARS[idx.min(8)];
            let color = if lvl > 0.72 {
                Color::Cyan
            } else if lvl > 0.25 {
                Color::Blue
            } else {
                Color::DarkGray
            };
            Span::styled(format!("{ch}{ch} "), Style::default().fg(color))
        })
        .collect();
    let mut eq_spans = vec![Span::raw("14-Band EQ : ")];
    eq_spans.extend(band_spans);

    let eq_status = if app.audio_status.active {
        format!(
            "ACTIVE ({}) · {:.1} fps · {} HID pkts",
            app.audio_status.source_label,
            app.audio_status.actual_fps,
            app.audio_status.packets_total
        )
    } else if let Some(err) = &app.audio_status.last_error {
        format!("IDLE (error: {err})")
    } else {
        "IDLE (runs inside reaulad · persists after closing UI)".into()
    };

    let mut eq_lines = vec![
        Line::from(format!("State      : {eq_status}   [a] toggle")),
        Line::from(format!(
            "Source [A] : {}",
            app.audio_config.source.label()
        )),
        Line::from(format!(
            "Style  [s] : {}   |   Palette [p] : {}",
            app.audio_config.style.label(),
            app.audio_config.palette.label()
        )),
        Line::from(format!(
            "RMS/Peak   : {:.1}/{:.1} dBFS · Bass {:.2} Mid {:.2} Treble {:.2} {}",
            app.audio_status.metrics.rms_dbfs,
            app.audio_status.metrics.peak_dbfs,
            app.audio_status.metrics.bass,
            app.audio_status.metrics.mid,
            app.audio_status.metrics.treble,
            if app.audio_status.metrics.beat || app.audio_status.metrics.beat_env > 0.35 {
                "[BEAT]"
            } else {
                ""
            }
        )),
        Line::from(eq_spans),
    ];
    if !app.audio_status.active {
        eq_lines.push(Line::from("Note: experimental hack; continuous streaming may reduce LED life."));
    }
    f.render_widget(
        Paragraph::new(eq_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Daemon Audio Equalizer (a=Start/Stop, A=Source, s=Effect, p=Palette) "),
        ),
        right_split[1],
    );
}

fn draw_system_tab(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let pair_strs: Vec<String> = app
        .socd_pairs
        .iter()
        .map(|p| {
            let k1 = WIN60HE_KEYS
                .iter()
                .find(|k| k.hid_code == p.key1_hid)
                .map_or("?", |k| k.name);
            let k2 = WIN60HE_KEYS
                .iter()
                .find(|k| k.hid_code == p.key2_hid)
                .map_or("?", |k| k.name);
            format!("{k1}<->{k2} (Model {})", p.model + 1)
        })
        .collect();

    let lines = vec![
        Line::from("Daemon Security (SO_PEERCRED + 0700/0600 Unix Socket):"),
        Line::from(format!("  Socket Path       : {}", app.daemon_status.socket_path)),
        Line::from(format!(
            "  Daemon Identity   : pid={} uid={} (verified via SO_PEERCRED)",
            app.server_cred.pid, app.server_cred.uid
        )),
        Line::from(format!(
            "  Allowed UIDs      : {:?}",
            app.daemon_status.allowed_uids
        )),
        Line::from(""),
        Line::from("Hardware:"),
        Line::from(format!(
            "  Firmware          : {}",
            app.probe.firmware_string.as_deref().unwrap_or("<unknown>")
        )),
        Line::from(format!(
            "  Build Date        : {}",
            app.probe.build_date.as_deref().unwrap_or("<unknown>")
        )),
        Line::from(format!(
            "  USB Polling Rate  : {} kHz ({} Hz)   [press 'p' to cycle]",
            app.poll_rate_khz,
            u16::from(app.poll_rate_khz) * 1000
        )),
        Line::from(""),
        Line::from(format!(
            "PRCS / SOCD         : {}   [press 'e' to toggle, 'd' to toggle A<->D pair]",
            if app.socd_enabled { "ENABLED" } else { "DISABLED" }
        )),
        Line::from(format!(
            "  Pairs             : {}",
            if pair_strs.is_empty() {
                "<none>".into()
            } else {
                pair_strs.join(", ")
            }
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" SOCD, Poll Rate & Daemon Security "),
        ),
        area,
    );
}

