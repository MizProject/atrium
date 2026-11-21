//! Secure Unix domain socket IPC protocol between `reaulad` and local clients
//! (`reaula`, `reaula-tui`, `reaula-gui`).
//!
//! Security model (fail-closed):
//! 1. Local-only Unix stream socket (never binds a network/TCP port).
//! 2. Socket directory is restricted (`0700` owned by the sole authorized UID
//!    before `bind()`, preventing other users from even traversing to the socket).
//! 3. Socket file is `0600` owned by the authorized UID (when single-user).
//! 4. On every `accept()`, the daemon verifies kernel `SO_PEERCRED` (`uid`/`pid`)
//!    against an explicit allowlist before reading any message bytes.
//! 5. Clients also check `SO_PEERCRED` on the daemon peer (`uid == 0` or `uid == self`).

use std::ffi::CStr;
use std::fs::{self, Permissions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::audio_fx::{AudioEffectStyle, AudioFrameMetrics, AudioPalette};
use crate::diagnostics::DiagnosticReport;
use crate::driver::DeviceProbe;
use crate::protocol::{
    CalibrationProgress, LIGHT_MAX_LEVEL, LightList, LightState, LiveTravelSample, PrcsPair,
    TriggerConfig, TriggerMode, WinLockFlags,
};

pub const SYSTEM_SOCKET_DIR: &str = "/run/reaula";
pub const SOCKET_FILENAME: &str = "reaulad.sock";
pub const MAX_IPC_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IPC message exceeded maximum length ({MAX_IPC_LINE_BYTES} bytes)")]
    MessageTooLarge,

    #[error("IPC connection closed unexpectedly")]
    Disconnected,

    #[error("unsafe socket path {path}: {reason}")]
    UnsafeSocketPath { path: String, reason: String },

    #[error("untrusted daemon peer UID {uid} (expected root or current UID {expected_uid})")]
    UntrustedDaemon { uid: u32, expected_uid: u32 },

    #[error("daemon returned error: {0}")]
    Remote(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCredentials {
    pub pid: u32,
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub daemon_pid: u32,
    pub daemon_uid: u32,
    pub allowed_uids: Vec<u32>,
    pub device_connected: bool,
    pub device_description: Option<String>,
    #[serde(default)]
    pub keyboard_id: Option<String>,
    #[serde(default)]
    pub keyboard_name: Option<String>,
    pub socket_path: String,
    pub travel_subscribers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyActuationEntry {
    pub index: u8,
    pub config: Option<TriggerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeySwitchEntry {
    pub index: u8,
    pub switch_id: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyDeadbandEntry {
    pub index: u8,
    pub top_raw: u8,
    pub bottom_raw: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VirtualKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Escape,
    Grave,
    PrintScreen,
    Mute,
    VolumeDown,
    VolumeUp,
    PlayPause,
    NextSong,
    PreviousSong,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

impl VirtualKey {
    pub const ALL: [Self; 31] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::Home,
        Self::End,
        Self::PageUp,
        Self::PageDown,
        Self::Insert,
        Self::Delete,
        Self::Escape,
        Self::Grave,
        Self::PrintScreen,
        Self::Mute,
        Self::VolumeDown,
        Self::VolumeUp,
        Self::PlayPause,
        Self::NextSong,
        Self::PreviousSong,
        Self::F1,
        Self::F2,
        Self::F3,
        Self::F4,
        Self::F5,
        Self::F6,
        Self::F7,
        Self::F8,
        Self::F9,
        Self::F10,
        Self::F11,
        Self::F12,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "Arrow Up",
            Self::Down => "Arrow Down",
            Self::Left => "Arrow Left",
            Self::Right => "Arrow Right",
            Self::Home => "Home",
            Self::End => "End",
            Self::PageUp => "Page Up",
            Self::PageDown => "Page Down",
            Self::Insert => "Insert",
            Self::Delete => "Delete",
            Self::Escape => "Esc",
            Self::Grave => "Grave (~)",
            Self::PrintScreen => "Print Screen",
            Self::Mute => "Mute",
            Self::VolumeDown => "Volume -",
            Self::VolumeUp => "Volume +",
            Self::PlayPause => "Play / Pause",
            Self::NextSong => "Next Track",
            Self::PreviousSong => "Prev Track",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
        }
    }

    pub fn linux_code(self) -> u16 {
        match self {
            Self::Up => 103,
            Self::Down => 108,
            Self::Left => 105,
            Self::Right => 106,
            Self::Home => 102,
            Self::End => 107,
            Self::PageUp => 104,
            Self::PageDown => 109,
            Self::Insert => 110,
            Self::Delete => 111,
            Self::Escape => 1,
            Self::Grave => 41,
            Self::PrintScreen => 99,
            Self::Mute => 113,
            Self::VolumeDown => 114,
            Self::VolumeUp => 115,
            Self::PlayPause => 164,
            Self::NextSong => 163,
            Self::PreviousSong => 165,
            Self::F1 => 59,
            Self::F2 => 60,
            Self::F3 => 61,
            Self::F4 => 62,
            Self::F5 => 63,
            Self::F6 => 64,
            Self::F7 => 65,
            Self::F8 => 66,
            Self::F9 => 67,
            Self::F10 => 68,
            Self::F11 => 87,
            Self::F12 => 88,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShortcutAction {
    EmitKey {
        key: VirtualKey,
    },
    ActuationPreset {
        name: String,
        mode: TriggerMode,
        indices: Vec<u8>,
        depth_mm: f64,
        rt_press_mm: f64,
        rt_release_mm: f64,
    },
    ToggleSocd,
    LightingPreset {
        name: String,
        state: LightState,
    },
    ToggleLighting,
    SetPollRate {
        khz: u8,
    },
    RunCommand {
        command: String,
    },
}

impl ShortcutAction {
    pub fn summary(&self) -> String {
        match self {
            Self::EmitKey { key } => format!("Emit OS Key: {}", key.label()),
            Self::ActuationPreset {
                name,
                mode,
                indices,
                depth_mm,
                ..
            } => format!(
                "Actuation '{name}': {mode:?} {depth_mm:.2} mm ({} keys)",
                indices.len()
            ),
            Self::ToggleSocd => "Toggle SOCD / PRCS".into(),
            Self::LightingPreset { name, .. } => format!("Lighting preset '{name}'"),
            Self::ToggleLighting => "Toggle keyboard RGB power".into(),
            Self::SetPollRate { khz } => format!("Set polling rate {khz} kHz"),
            Self::RunCommand { command } => format!("Run `{command}` (user UID)"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutRule {
    pub id: u32,
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub modifier_key: Option<u8>,
    pub trigger_key: u8,
    pub trigger_depth_mm: f64,
    pub action: ShortcutAction,
}

impl ShortcutRule {
    pub fn gesture_label(&self) -> String {
        let trig = crate::keymap::by_index(self.trigger_key)
            .map_or("Key", |k| k.name);
        match self.modifier_key {
            Some(m) => {
                let mname = crate::keymap::by_index(m).map_or("Mod", |k| k.name);
                format!("{mname} + {trig}")
            }
            None => trig.to_string(),
        }
    }
}

/// Soft Aqua Blue LED color used when daemon memory-mapping indicators activate (distinct from firmware indicators).
pub const DAEMON_INDICATOR_AQUA_RGB: [u8; 3] = [0, 215, 255];
/// Bright Electric Aqua flash when a daemon shortcut triggers or latches.
pub const DAEMON_INDICATOR_PULSE_RGB: [u8; 3] = [110, 252, 255];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutsSnapshot {
    pub rules: Vec<ShortcutRule>,
    pub watcher_active: bool,
    pub recent_triggers: Vec<ShortcutTriggerLog>,
    #[serde(default)]
    pub uinput_ready: bool,
    #[serde(default)]
    pub uinput_error: Option<String>,
    #[serde(default)]
    pub evdev_grab_active: bool,
    #[serde(default)]
    pub persist_path: Option<String>,
    #[serde(default)]
    pub active_indicator_keys: Vec<u8>,
    #[serde(default)]
    pub pulsing_indicator_keys: Vec<u8>,
}

/// One hardware Fn-layer keymap override stored in WIN 60 HE flash (`cmd 0x18`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FnOverride {
    pub index: u8,
    pub target_hid: u8,
}

impl From<(u8, u8)> for FnOverride {
    fn from((index, target_hid): (u8, u8)) -> Self {
        Self { index, target_hid }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedShortcuts {
    pub rules: Vec<ShortcutRule>,
    pub watcher_active: bool,
}

pub fn encode_persisted_shortcuts(rules: &[ShortcutRule], watcher_active: bool) -> Result<Vec<u8>, IpcError> {
    Ok(serde_json::to_vec_pretty(&PersistedShortcuts {
        rules: rules.to_vec(),
        watcher_active,
    })?)
}

pub fn decode_persisted_shortcuts(bytes: &[u8]) -> Result<PersistedShortcuts, IpcError> {
    Ok(serde_json::from_slice(bytes)?)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShortcutTriggerLog {
    pub rule_id: u32,
    pub rule_name: String,
    pub trigger_key: u8,
    pub action_summary: String,
    pub unix_ms: u64,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioSyncSource {
    Monitor,
    DefaultMic,
    Demo,
    Device {
        pcm: String,
        #[serde(default)]
        label: String,
    },
}

impl AudioSyncSource {
    pub fn label(&self) -> String {
        match self {
            Self::Monitor => "Desktop Audio Monitor (ALSA)".into(),
            Self::DefaultMic => "Default Mic / Line-In (ALSA)".into(),
            Self::Demo => "120 BPM Built-In Demo Synth".into(),
            Self::Device { pcm, label } => {
                if label.is_empty() {
                    pcm.clone()
                } else {
                    format!("{pcm} — {label}")
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSyncConfig {
    pub source: AudioSyncSource,
    pub style: AudioEffectStyle,
    pub palette: AudioPalette,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub brightness: u8,
    pub fps: u8,
    pub gain: f32,
}

impl Default for AudioSyncConfig {
    fn default() -> Self {
        Self {
            source: AudioSyncSource::Monitor,
            style: AudioEffectStyle::Spectrum,
            palette: AudioPalette::Aurora,
            fg: [56, 189, 248],
            bg: [0, 0, 0],
            brightness: LIGHT_MAX_LEVEL,
            fps: 35,
            gain: 1.2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSyncStatus {
    pub active: bool,
    pub config: AudioSyncConfig,
    pub source_label: String,
    pub metrics: AudioFrameMetrics,
    pub frame: Vec<[u8; 3]>,
    pub frames_rendered: u64,
    pub packets_total: u64,
    pub packets_last: usize,
    pub actual_fps: f32,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl Default for AudioSyncStatus {
    fn default() -> Self {
        Self {
            active: false,
            config: AudioSyncConfig::default(),
            source_label: "Idle".into(),
            metrics: AudioFrameMetrics::default(),
            frame: Vec::new(),
            frames_rendered: 0,
            packets_total: 0,
            packets_last: 0,
            actual_fps: 0.0,
            last_error: None,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum DaemonRequest {
    Ping,
    Shutdown,
    Probe,
    GetActuation {
        indices: Vec<u8>,
    },
    SetActuation {
        indices: Vec<u8>,
        mode: TriggerMode,
        travel_mm: f64,
        rt_press_mm: Option<f64>,
        rt_release_mm: Option<f64>,
    },
    GetLight,
    SetLight {
        state: LightState,
    },
    GetLightList,
    GetCustomRgb {
        slot: u8,
    },
    SetCustomRgb {
        slot: u8,
        colors: Vec<[u8; 3]>,
        brightness: u8,
    },
    BeginCustomStream {
        brightness: u8,
    },
    StreamCustomRgb {
        slot: u8,
        colors: Vec<[u8; 3]>,
        force: bool,
    },
    GetSwitches {
        indices: Vec<u8>,
    },
    SetSwitches {
        indices: Vec<u8>,
        switch_id: u8,
    },
    GetDeadband {
        indices: Vec<u8>,
    },
    SetDeadband {
        indices: Vec<u8>,
        top_raw: u8,
        bottom_raw: u8,
    },
    GetPollRate,
    SetPollRate {
        khz: u8,
    },
    GetSocd,
    SetSocd {
        enabled: bool,
        pairs: Vec<PrcsPair>,
    },
    GetKeymap {
        fn_layer: bool,
    },
    ApplyKeyboardFnMap {
        set: Vec<FnOverride>,
        clear: Vec<u8>,
    },
    ResetKeyboardFnMap,
    SetWinLock {
        flags: WinLockFlags,
    },
    Raw {
        body: Vec<u8>,
    },
    SubscribeTravel {
        indices: Vec<u8>,
    },
    UnsubscribeTravel,
    GetActiveCustomSlot,
    GetShortcuts,
    SetShortcuts {
        rules: Vec<ShortcutRule>,
        watcher_active: bool,
    },
    TriggerShortcut {
        id: u32,
    },
    RunDiagnostics,
    StartCalibration {
        any_key: bool,
    },
    StopCalibration,
    StartAudioSync {
        config: AudioSyncConfig,
    },
    UpdateAudioSync {
        config: AudioSyncConfig,
    },
    StopAudioSync {
        #[serde(default = "default_true")]
        restore_lighting: bool,
    },
    GetAudioSyncState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonResponse {
    Ack,
    Status(DaemonStatus),
    Probe(Box<DeviceProbe>),
    Actuation {
        unit_mm: f64,
        entries: Vec<KeyActuationEntry>,
    },
    Light(LightState),
    LightList(LightList),
    CustomRgb {
        slot: u8,
        colors: Vec<[u8; 3]>,
    },
    CustomStreamed {
        packets_sent: usize,
    },
    ActiveCustomSlot {
        slot: u8,
    },
    Switches {
        entries: Vec<KeySwitchEntry>,
    },
    Deadband {
        entries: Vec<KeyDeadbandEntry>,
    },
    PollRate {
        khz: u8,
    },
    Socd {
        enabled: bool,
        pairs: Vec<PrcsPair>,
    },
    Keymap {
        fn_layer: bool,
        table: Vec<u8>,
    },
    KeyboardFnMap {
        overrides: Vec<FnOverride>,
    },
    RawReply {
        reply: Option<Vec<u8>>,
    },
    TravelSubscribed {
        unit_mm: f64,
    },
    TravelSample {
        sample: LiveTravelSample,
        unit_mm: f64,
    },
    Shortcuts(ShortcutsSnapshot),
    ShortcutTriggered(ShortcutTriggerLog),
    Diagnostics(Box<DiagnosticReport>),
    CalibrationStarted,
    CalibrationUpdate(CalibrationProgress),
    AudioSync(Box<AudioSyncStatus>),
    Error {
        message: String,
    },
}

/// Return the effective user ID of this process.
pub fn current_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and always succeeds.
    unsafe { libc::geteuid() }
}

/// Return the effective group ID of this process.
pub fn current_gid() -> u32 {
    // SAFETY: `getegid` has no preconditions and always succeeds.
    unsafe { libc::getegid() }
}

/// Look up primary GID and home directory for a UID from `/etc/passwd` (via `getpwuid_r`).
pub fn user_passwd_for_uid(uid: u32) -> Option<(u32, PathBuf)> {
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0u8; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `pwd`, `buf`, and `result` are valid mutable buffers for `getpwuid_r`.
    let rc = unsafe {
        libc::getpwuid_r(
            uid,
            pwd.as_mut_ptr(),
            buf.as_mut_ptr().cast::<libc::c_char>(),
            buf.len(),
            &mut result,
        )
    };
    if rc == 0 && !result.is_null() {
        // SAFETY: `getpwuid_r` initialized `pwd` when returning 0 and non-null `result`.
        let p = unsafe { pwd.assume_init() };
        let home = if p.pw_dir.is_null() {
            PathBuf::new()
        } else {
            // SAFETY: `pw_dir` is a valid NUL-terminated string inside `buf`.
            PathBuf::from(unsafe { CStr::from_ptr(p.pw_dir) }.to_string_lossy().as_ref())
        };
        Some((p.pw_gid, home))
    } else {
        None
    }
}

/// Look up primary GID for a UID from `/etc/passwd` (via `getpwuid_r`), if present.
pub fn primary_gid_for_uid(uid: u32) -> Option<u32> {
    user_passwd_for_uid(uid).map(|(gid, _)| gid)
}

/// Look up `(uid, gid)` for a local username via `getpwnam_r`.
pub fn lookup_user(username: &str) -> Option<(u32, u32)> {
    let c_name = std::ffi::CString::new(username).ok()?;
    let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buf = vec![0u8; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: `c_name`, `pwd`, `buf`, and `result` are valid for `getpwnam_r`.
    let rc = unsafe {
        libc::getpwnam_r(
            c_name.as_ptr(),
            pwd.as_mut_ptr(),
            buf.as_mut_ptr().cast::<libc::c_char>(),
            buf.len(),
            &mut result,
        )
    };
    if rc == 0 && !result.is_null() {
        // SAFETY: `getpwnam_r` initialized `pwd` when returning 0 and non-null `result`.
        let p = unsafe { pwd.assume_init() };
        Some((p.pw_uid, p.pw_gid))
    } else {
        None
    }
}

/// Query kernel-verified `SO_PEERCRED` (`pid`, `uid`, `gid`) on a connected Unix stream socket.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub fn peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: u32::MAX,
        gid: u32::MAX,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` match `SO_PEERCRED` (`struct ucred`) on Linux.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    if cred.pid <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "peer PID is missing or zero",
        ));
    }
    Ok(PeerCredentials {
        pid: cred.pid as u32,
        uid: cred.uid,
        gid: cred.gid,
    })
}

/// Fail-closed check that `cred` has a valid PID and belongs to `allowed_uids`.
pub fn is_peer_authorized(cred: &PeerCredentials, allowed_uids: &[u32]) -> bool {
    cred.pid > 0 && allowed_uids.contains(&cred.uid)
}

/// Default socket path for starting `reaulad`.
pub fn default_daemon_socket_path() -> PathBuf {
    if current_uid() == 0 {
        PathBuf::from(SYSTEM_SOCKET_DIR).join(SOCKET_FILENAME)
    } else if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
        && !runtime.is_empty()
    {
        PathBuf::from(runtime).join("reaula").join(SOCKET_FILENAME)
    } else {
        PathBuf::from(format!("/tmp/reaula-{}", current_uid())).join(SOCKET_FILENAME)
    }
}

/// Discover the socket path a client (`reaula`, `reaula-tui`, `reaula-gui`) should connect to.
pub fn discover_client_socket_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    if let Some(env_path) = std::env::var_os("REAULA_SOCKET")
        && !env_path.is_empty()
    {
        return PathBuf::from(env_path);
    }
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR")
        && !runtime.is_empty()
    {
        let user_sock = PathBuf::from(runtime).join("reaula").join(SOCKET_FILENAME);
        if user_sock.exists() {
            return user_sock;
        }
    }
    let sys_sock = PathBuf::from(SYSTEM_SOCKET_DIR).join(SOCKET_FILENAME);
    if sys_sock.exists() {
        return sys_sock;
    }
    sys_sock
}

/// Bind a Unix domain socket listener with strict directory ownership,
/// non-symlink verification, and restricted file permissions (`0700` dir + `0600` socket
/// when a single UID is authorized).
pub fn bind_secure_listener(
    socket_path: &Path,
    allowed_uids: &[u32],
) -> Result<UnixListener, IpcError> {
    let parent = socket_path.parent().ok_or_else(|| IpcError::UnsafeSocketPath {
        path: socket_path.display().to_string(),
        reason: "socket path has no parent directory".into(),
    })?;

    if let Ok(meta) = fs::symlink_metadata(parent)
        && meta.file_type().is_symlink()
    {
        return Err(IpcError::UnsafeSocketPath {
            path: parent.display().to_string(),
            reason: "parent directory must not be a symlink".into(),
        });
    }
    fs::create_dir_all(parent)?;

    let parent_meta = fs::symlink_metadata(parent)?;
    if !parent_meta.is_dir() || parent_meta.file_type().is_symlink() {
        return Err(IpcError::UnsafeSocketPath {
            path: parent.display().to_string(),
            reason: "parent is not a real directory".into(),
        });
    }

    let single_uid = (allowed_uids.len() == 1).then_some(allowed_uids[0]);

    // Lock down the directory BEFORE calling bind() so there is no microsecond
    // window where an unprivileged third-party UID could reach the socket.
    if let Some(uid) = single_uid {
        let gid = primary_gid_for_uid(uid).unwrap_or_else(current_gid);
        if current_uid() == 0 {
            chown_path(parent, uid, gid)?;
        }
        fs::set_permissions(parent, Permissions::from_mode(0o700))?;
    } else {
        fs::set_permissions(parent, Permissions::from_mode(0o755))?;
    }

    // Refuse symlinks or non-sockets at the target socket path before unlinking.
    if let Ok(sock_meta) = fs::symlink_metadata(socket_path) {
        if sock_meta.file_type().is_symlink() || !sock_meta.file_type().is_socket() {
            return Err(IpcError::UnsafeSocketPath {
                path: socket_path.display().to_string(),
                reason: "existing path is not a Unix socket".into(),
            });
        }
        fs::remove_file(socket_path)?;
    }

    let listener = UnixListener::bind(socket_path)?;

    if let Some(uid) = single_uid {
        let gid = primary_gid_for_uid(uid).unwrap_or_else(current_gid);
        if current_uid() == 0 {
            chown_path(socket_path, uid, gid)?;
        }
        fs::set_permissions(socket_path, Permissions::from_mode(0o600))?;
    } else {
        fs::set_permissions(socket_path, Permissions::from_mode(0o660))?;
    }

    Ok(listener)
}

fn chown_path(path: &Path, uid: u32, gid: u32) -> io::Result<()> {
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "interior NUL in path"))?;
    // SAFETY: `c_path` is a NUL-terminated path string; `lchown` avoids following symlinks.
    let rc = unsafe { libc::lchown(c_path.as_ptr(), uid, gid) };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Write a single JSON-encoded message followed by `\n`.
pub fn write_json_line<T: Serialize>(writer: &mut impl Write, msg: &T) -> Result<(), IpcError> {
    let mut line = serde_json::to_vec(msg)?;
    if line.len() > MAX_IPC_LINE_BYTES {
        return Err(IpcError::MessageTooLarge);
    }
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()?;
    Ok(())
}

/// Read one `\n`-delimited JSON line bounded by [`MAX_IPC_LINE_BYTES`].
pub fn read_json_line<R: BufRead, T: for<'de> Deserialize<'de>>(
    reader: &mut R,
) -> Result<T, IpcError> {
    let mut buf = Vec::new();
    let mut limited = reader.by_ref().take((MAX_IPC_LINE_BYTES as u64) + 1);
    let n = limited.read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Err(IpcError::Disconnected);
    }
    if buf.len() > MAX_IPC_LINE_BYTES {
        return Err(IpcError::MessageTooLarge);
    }
    let parsed = serde_json::from_slice(&buf)?;
    Ok(parsed)
}

/// Synchronous client connection to `reaulad`.
pub struct DaemonClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    socket_path: PathBuf,
    server_cred: PeerCredentials,
}

impl DaemonClient {
    /// Connect to `reaulad` at `socket_path` (or auto-discovered path) and verify
    /// that the daemon process belongs to root (`0`) or the current user.
    pub fn connect(socket_path: Option<&Path>) -> Result<Self, IpcError> {
        let path = discover_client_socket_path(socket_path);
        let stream = UnixStream::connect(&path)?;
        let cred = peer_credentials(&stream)?;
        let me = current_uid();
        if cred.uid != 0 && cred.uid != me {
            return Err(IpcError::UntrustedDaemon {
                uid: cred.uid,
                expected_uid: me,
            });
        }
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let reader_stream = stream.try_clone()?;
        Ok(Self {
            writer: stream,
            reader: BufReader::new(reader_stream),
            socket_path: path,
            server_cred: cred,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub fn server_credentials(&self) -> PeerCredentials {
        self.server_cred
    }

    pub fn request(&mut self, req: &DaemonRequest) -> Result<DaemonResponse, IpcError> {
        write_json_line(&mut self.writer, req)?;
        let resp: DaemonResponse = read_json_line(&mut self.reader)?;
        if let DaemonResponse::Error { message } = resp {
            return Err(IpcError::Remote(message));
        }
        Ok(resp)
    }

    pub fn ping(&mut self) -> Result<DaemonStatus, IpcError> {
        match self.request(&DaemonRequest::Ping)? {
            DaemonResponse::Status(s) => Ok(s),
            other => Err(IpcError::Remote(format!("unexpected ping reply: {other:?}"))),
        }
    }

    pub fn shutdown(&mut self) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::Shutdown)? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!("unexpected shutdown reply: {other:?}"))),
        }
    }

    pub fn probe(&mut self) -> Result<DeviceProbe, IpcError> {
        match self.request(&DaemonRequest::Probe)? {
            DaemonResponse::Probe(p) => Ok(*p),
            other => Err(IpcError::Remote(format!("unexpected probe reply: {other:?}"))),
        }
    }

    pub fn get_actuation(
        &mut self,
        indices: Vec<u8>,
    ) -> Result<(f64, Vec<KeyActuationEntry>), IpcError> {
        match self.request(&DaemonRequest::GetActuation { indices })? {
            DaemonResponse::Actuation { unit_mm, entries } => Ok((unit_mm, entries)),
            other => Err(IpcError::Remote(format!(
                "unexpected get_actuation reply: {other:?}"
            ))),
        }
    }

    pub fn set_actuation(
        &mut self,
        indices: Vec<u8>,
        mode: TriggerMode,
        travel_mm: f64,
        rt_press_mm: Option<f64>,
        rt_release_mm: Option<f64>,
    ) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::SetActuation {
            indices,
            mode,
            travel_mm,
            rt_press_mm,
            rt_release_mm,
        })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!(
                "unexpected set_actuation reply: {other:?}"
            ))),
        }
    }

    pub fn get_light(&mut self) -> Result<LightState, IpcError> {
        match self.request(&DaemonRequest::GetLight)? {
            DaemonResponse::Light(s) => Ok(s),
            other => Err(IpcError::Remote(format!("unexpected get_light reply: {other:?}"))),
        }
    }

    pub fn set_light(&mut self, state: LightState) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::SetLight { state })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!("unexpected set_light reply: {other:?}"))),
        }
    }

    pub fn get_light_list(&mut self) -> Result<LightList, IpcError> {
        match self.request(&DaemonRequest::GetLightList)? {
            DaemonResponse::LightList(l) => Ok(l),
            other => Err(IpcError::Remote(format!(
                "unexpected get_light_list reply: {other:?}"
            ))),
        }
    }

    pub fn get_switches(&mut self, indices: Vec<u8>) -> Result<Vec<KeySwitchEntry>, IpcError> {
        match self.request(&DaemonRequest::GetSwitches { indices })? {
            DaemonResponse::Switches { entries } => Ok(entries),
            other => Err(IpcError::Remote(format!(
                "unexpected get_switches reply: {other:?}"
            ))),
        }
    }

    pub fn set_switches(&mut self, indices: Vec<u8>, switch_id: u8) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::SetSwitches { indices, switch_id })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!(
                "unexpected set_switches reply: {other:?}"
            ))),
        }
    }

    pub fn get_deadband(&mut self, indices: Vec<u8>) -> Result<Vec<KeyDeadbandEntry>, IpcError> {
        match self.request(&DaemonRequest::GetDeadband { indices })? {
            DaemonResponse::Deadband { entries } => Ok(entries),
            other => Err(IpcError::Remote(format!(
                "unexpected get_deadband reply: {other:?}"
            ))),
        }
    }

    pub fn set_deadband(
        &mut self,
        indices: Vec<u8>,
        top_raw: u8,
        bottom_raw: u8,
    ) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::SetDeadband {
            indices,
            top_raw,
            bottom_raw,
        })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!(
                "unexpected set_deadband reply: {other:?}"
            ))),
        }
    }

    pub fn get_poll_rate(&mut self) -> Result<u8, IpcError> {
        match self.request(&DaemonRequest::GetPollRate)? {
            DaemonResponse::PollRate { khz } => Ok(khz),
            other => Err(IpcError::Remote(format!(
                "unexpected get_poll_rate reply: {other:?}"
            ))),
        }
    }

    pub fn set_poll_rate(&mut self, khz: u8) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::SetPollRate { khz })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!(
                "unexpected set_poll_rate reply: {other:?}"
            ))),
        }
    }

    pub fn get_socd(&mut self) -> Result<(bool, Vec<PrcsPair>), IpcError> {
        match self.request(&DaemonRequest::GetSocd)? {
            DaemonResponse::Socd { enabled, pairs } => Ok((enabled, pairs)),
            other => Err(IpcError::Remote(format!("unexpected get_socd reply: {other:?}"))),
        }
    }

    pub fn set_socd(&mut self, enabled: bool, pairs: Vec<PrcsPair>) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::SetSocd { enabled, pairs })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!("unexpected set_socd reply: {other:?}"))),
        }
    }

    pub fn get_keymap(&mut self, fn_layer: bool) -> Result<Vec<u8>, IpcError> {
        match self.request(&DaemonRequest::GetKeymap { fn_layer })? {
            DaemonResponse::Keymap { table, .. } => Ok(table),
            other => Err(IpcError::Remote(format!("unexpected get_keymap reply: {other:?}"))),
        }
    }

    pub fn get_fn_overrides(&mut self) -> Result<Vec<FnOverride>, IpcError> {
        let table = self.get_keymap(true)?;
        Ok(crate::protocol::decode_fn_keymap_overrides(&table)
            .into_iter()
            .map(|(index, target_hid)| FnOverride { index, target_hid })
            .collect())
    }

    pub fn apply_keyboard_fn_map(
        &mut self,
        set: Vec<FnOverride>,
        clear: Vec<u8>,
    ) -> Result<Vec<FnOverride>, IpcError> {
        match self.request(&DaemonRequest::ApplyKeyboardFnMap { set, clear })? {
            DaemonResponse::KeyboardFnMap { overrides } => Ok(overrides),
            other => Err(IpcError::Remote(format!(
                "unexpected apply_keyboard_fn_map reply: {other:?}"
            ))),
        }
    }

    pub fn reset_keyboard_fn_map(&mut self) -> Result<Vec<FnOverride>, IpcError> {
        match self.request(&DaemonRequest::ResetKeyboardFnMap)? {
            DaemonResponse::KeyboardFnMap { overrides } => Ok(overrides),
            other => Err(IpcError::Remote(format!(
                "unexpected reset_keyboard_fn_map reply: {other:?}"
            ))),
        }
    }

    pub fn set_win_lock(&mut self, flags: WinLockFlags) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::SetWinLock { flags })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!(
                "unexpected set_win_lock reply: {other:?}"
            ))),
        }
    }

    pub fn raw(&mut self, body: Vec<u8>) -> Result<Option<Vec<u8>>, IpcError> {
        match self.request(&DaemonRequest::Raw { body })? {
            DaemonResponse::RawReply { reply } => Ok(reply),
            other => Err(IpcError::Remote(format!("unexpected raw reply: {other:?}"))),
        }
    }

    pub fn get_active_custom_slot(&mut self) -> Result<u8, IpcError> {
        match self.request(&DaemonRequest::GetActiveCustomSlot)? {
            DaemonResponse::ActiveCustomSlot { slot } => Ok(slot),
            other => Err(IpcError::Remote(format!(
                "unexpected get_active_custom_slot reply: {other:?}"
            ))),
        }
    }

    pub fn get_custom_rgb(&mut self, slot: u8) -> Result<Vec<[u8; 3]>, IpcError> {
        match self.request(&DaemonRequest::GetCustomRgb { slot })? {
            DaemonResponse::CustomRgb { colors, .. } => Ok(colors),
            other => Err(IpcError::Remote(format!(
                "unexpected get_custom_rgb reply: {other:?}"
            ))),
        }
    }

    pub fn set_custom_rgb(
        &mut self,
        slot: u8,
        colors: Vec<[u8; 3]>,
        brightness: u8,
    ) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::SetCustomRgb {
            slot,
            colors,
            brightness,
        })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!(
                "unexpected set_custom_rgb reply: {other:?}"
            ))),
        }
    }

    pub fn begin_custom_stream(&mut self, brightness: u8) -> Result<(), IpcError> {
        match self.request(&DaemonRequest::BeginCustomStream { brightness })? {
            DaemonResponse::Ack => Ok(()),
            other => Err(IpcError::Remote(format!(
                "unexpected begin_custom_stream reply: {other:?}"
            ))),
        }
    }

    pub fn stream_custom_rgb(
        &mut self,
        slot: u8,
        colors: Vec<[u8; 3]>,
        force: bool,
    ) -> Result<usize, IpcError> {
        match self.request(&DaemonRequest::StreamCustomRgb {
            slot,
            colors,
            force,
        })? {
            DaemonResponse::CustomStreamed { packets_sent } => Ok(packets_sent),
            other => Err(IpcError::Remote(format!(
                "unexpected stream_custom_rgb reply: {other:?}"
            ))),
        }
    }

    pub fn start_audio_sync(&mut self, config: AudioSyncConfig) -> Result<AudioSyncStatus, IpcError> {
        match self.request(&DaemonRequest::StartAudioSync { config })? {
            DaemonResponse::AudioSync(status) => Ok(*status),
            other => Err(IpcError::Remote(format!(
                "unexpected start_audio_sync reply: {other:?}"
            ))),
        }
    }

    pub fn update_audio_sync(&mut self, config: AudioSyncConfig) -> Result<AudioSyncStatus, IpcError> {
        match self.request(&DaemonRequest::UpdateAudioSync { config })? {
            DaemonResponse::AudioSync(status) => Ok(*status),
            other => Err(IpcError::Remote(format!(
                "unexpected update_audio_sync reply: {other:?}"
            ))),
        }
    }

    pub fn stop_audio_sync(&mut self, restore_lighting: bool) -> Result<AudioSyncStatus, IpcError> {
        match self.request(&DaemonRequest::StopAudioSync { restore_lighting })? {
            DaemonResponse::AudioSync(status) => Ok(*status),
            other => Err(IpcError::Remote(format!(
                "unexpected stop_audio_sync reply: {other:?}"
            ))),
        }
    }

    pub fn get_audio_sync_state(&mut self) -> Result<AudioSyncStatus, IpcError> {
        match self.request(&DaemonRequest::GetAudioSyncState)? {
            DaemonResponse::AudioSync(status) => Ok(*status),
            other => Err(IpcError::Remote(format!(
                "unexpected get_audio_sync_state reply: {other:?}"
            ))),
        }
    }

    pub fn get_shortcuts(&mut self) -> Result<ShortcutsSnapshot, IpcError> {
        match self.request(&DaemonRequest::GetShortcuts)? {
            DaemonResponse::Shortcuts(snap) => Ok(snap),
            other => Err(IpcError::Remote(format!(
                "unexpected get_shortcuts reply: {other:?}"
            ))),
        }
    }

    pub fn set_shortcuts(
        &mut self,
        rules: Vec<ShortcutRule>,
        watcher_active: bool,
    ) -> Result<ShortcutsSnapshot, IpcError> {
        match self.request(&DaemonRequest::SetShortcuts {
            rules,
            watcher_active,
        })? {
            DaemonResponse::Shortcuts(snap) => Ok(snap),
            other => Err(IpcError::Remote(format!(
                "unexpected set_shortcuts reply: {other:?}"
            ))),
        }
    }

    pub fn trigger_shortcut(&mut self, id: u32) -> Result<ShortcutTriggerLog, IpcError> {
        match self.request(&DaemonRequest::TriggerShortcut { id })? {
            DaemonResponse::ShortcutTriggered(log) => Ok(log),
            other => Err(IpcError::Remote(format!(
                "unexpected trigger_shortcut reply: {other:?}"
            ))),
        }
    }

    pub fn run_diagnostics(&mut self) -> Result<DiagnosticReport, IpcError> {
        match self.request(&DaemonRequest::RunDiagnostics)? {
            DaemonResponse::Diagnostics(report) => Ok(*report),
            other => Err(IpcError::Remote(format!(
                "unexpected run_diagnostics reply: {other:?}"
            ))),
        }
    }

    /// Enter guarded calibration stream mode on this dedicated connection.
    pub fn start_calibration(
        mut self,
        any_key: bool,
    ) -> Result<CalibrationSubscription, IpcError> {
        match self.request(&DaemonRequest::StartCalibration { any_key })? {
            DaemonResponse::CalibrationStarted => {
                self.reader
                    .get_mut()
                    .set_read_timeout(Some(Duration::from_millis(50)))?;
                Ok(CalibrationSubscription {
                    writer: self.writer,
                    reader: self.reader,
                })
            }
            other => Err(IpcError::Remote(format!(
                "unexpected start_calibration reply: {other:?}"
            ))),
        }
    }

    /// Enter live travel subscription mode on this dedicated connection.
    pub fn subscribe_travel(mut self, indices: Vec<u8>) -> Result<TravelSubscription, IpcError> {
        match self.request(&DaemonRequest::SubscribeTravel { indices })? {
            DaemonResponse::TravelSubscribed { unit_mm } => {
                // Use short read timeout so callers can poll stop flags cleanly.
                self.reader
                    .get_mut()
                    .set_read_timeout(Some(Duration::from_millis(50)))?;
                Ok(TravelSubscription {
                    writer: self.writer,
                    reader: self.reader,
                    unit_mm,
                })
            }
            other => Err(IpcError::Remote(format!(
                "unexpected subscribe_travel reply: {other:?}"
            ))),
        }
    }
}

/// Dedicated streaming socket connection receiving live travel events from `reaulad`.
pub struct TravelSubscription {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    pub unit_mm: f64,
}

impl TravelSubscription {
    /// Poll the next travel sample, returning `Ok(None)` on read timeout.
    pub fn next_sample(&mut self) -> Result<Option<(LiveTravelSample, f64)>, IpcError> {
        loop {
            match read_json_line::<_, DaemonResponse>(&mut self.reader) {
                Ok(DaemonResponse::TravelSample { sample, unit_mm }) => {
                    return Ok(Some((sample, unit_mm)));
                }
                Ok(DaemonResponse::Ack) => continue,
                Ok(DaemonResponse::Error { message }) => return Err(IpcError::Remote(message)),
                Ok(_) => continue,
                Err(IpcError::Io(ref e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn close(mut self) {
        let _ = write_json_line(&mut self.writer, &DaemonRequest::UnsubscribeTravel);
    }
}

impl Drop for TravelSubscription {
    fn drop(&mut self) {
        let _ = write_json_line(&mut self.writer, &DaemonRequest::UnsubscribeTravel);
    }
}

/// Dedicated streaming socket connection monitoring guarded key calibration.
pub struct CalibrationSubscription {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl CalibrationSubscription {
    /// Poll the next calibration status packet, returning `Ok(None)` on read timeout.
    pub fn next_update(&mut self) -> Result<Option<CalibrationProgress>, IpcError> {
        loop {
            match read_json_line::<_, DaemonResponse>(&mut self.reader) {
                Ok(DaemonResponse::CalibrationUpdate(prog)) => return Ok(Some(prog)),
                Ok(DaemonResponse::Ack) | Ok(DaemonResponse::CalibrationStarted) => continue,
                Ok(DaemonResponse::Error { message }) => return Err(IpcError::Remote(message)),
                Ok(_) => continue,
                Err(IpcError::Io(ref e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Explicitly disarm firmware calibration mode before closing.
    pub fn stop(mut self) {
        let _ = write_json_line(&mut self.writer, &DaemonRequest::StopCalibration);
    }
}

impl Drop for CalibrationSubscription {
    fn drop(&mut self) {
        let _ = write_json_line(&mut self.writer, &DaemonRequest::StopCalibration);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn json_line_roundtrip_and_size_limit() {
        let mut buf = Vec::new();
        let req = DaemonRequest::SetLight {
            state: LightState::default(),
        };
        write_json_line(&mut buf, &req).unwrap();

        let mut cur = Cursor::new(buf);
        let decoded: DaemonRequest = read_json_line(&mut cur).unwrap();
        assert!(matches!(decoded, DaemonRequest::SetLight { .. }));

        let oversized = vec![b'a'; MAX_IPC_LINE_BYTES + 10];
        let mut cur_big = Cursor::new(oversized);
        let err = read_json_line::<_, DaemonRequest>(&mut cur_big).unwrap_err();
        assert!(matches!(err, IpcError::MessageTooLarge));
    }

    #[test]
    fn peer_cred_over_socketpair() {
        let (a, _b) = UnixStream::pair().unwrap();
        let cred = peer_credentials(&a).unwrap();
        assert_eq!(cred.uid, current_uid());
        assert_eq!(cred.pid, std::process::id());
    }

    #[test]
    fn bind_secure_listener_creates_0700_dir_and_0600_socket() {
        let base = std::env::temp_dir().join(format!("reaula-ipc-test-{}", std::process::id()));
        let sock = base.join("reaulad.sock");
        let uid = current_uid();
        let listener = bind_secure_listener(&sock, &[uid]).unwrap();

        let dir_mode = fs::metadata(&base).unwrap().permissions().mode() & 0o777;
        let sock_mode = fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(sock_mode, 0o600);

        drop(listener);
        let _ = fs::remove_file(&sock);
        let _ = fs::remove_dir(&base);
    }

    #[test]
    fn peer_authorization_allowlist_is_fail_closed() {
        let cred = PeerCredentials {
            pid: 4242,
            uid: 1000,
            gid: 1000,
        };
        assert!(is_peer_authorized(&cred, &[1000]));
        assert!(!is_peer_authorized(&cred, &[]));
        assert!(!is_peer_authorized(&cred, &[0, 1001]));

        let zero_pid = PeerCredentials {
            pid: 0,
            uid: 1000,
            gid: 1000,
        };
        assert!(!is_peer_authorized(&zero_pid, &[1000]));
    }

    #[test]
    fn bind_secure_listener_rejects_symlink_socket() {
        let base = std::env::temp_dir().join(format!("reaula-ipc-sym-{}", std::process::id()));
        let _ = fs::create_dir_all(&base);
        let target = base.join("real");
        let sock = base.join("reaulad.sock");
        fs::write(&target, b"x").unwrap();
        std::os::unix::fs::symlink(&target, &sock).unwrap();

        let err = bind_secure_listener(&sock, &[current_uid()]).unwrap_err();
        assert!(matches!(err, IpcError::UnsafeSocketPath { .. }));

        let _ = fs::remove_file(&sock);
        let _ = fs::remove_file(&target);
        let _ = fs::remove_dir(&base);
    }

    #[test]
    fn shortcut_rule_and_calibration_roundtrip() {
        let rule = ShortcutRule {
            id: 1,
            name: "Fn+T -> Up".into(),
            enabled: true,
            modifier_key: Some(crate::keymap::idx::FN),
            trigger_key: crate::keymap::idx::T,
            trigger_depth_mm: 1.6,
            action: ShortcutAction::EmitKey {
                key: VirtualKey::Up,
            },
        };
        let req = DaemonRequest::SetShortcuts {
            rules: vec![rule.clone()],
            watcher_active: true,
        };
        let mut buf = Vec::new();
        write_json_line(&mut buf, &req).unwrap();
        let mut cur = Cursor::new(buf);
        let decoded: DaemonRequest = read_json_line(&mut cur).unwrap();
        match decoded {
            DaemonRequest::SetShortcuts {
                rules,
                watcher_active,
            } => {
                assert!(watcher_active);
                assert_eq!(rules.len(), 1);
                assert_eq!(rules[0].modifier_key, Some(crate::keymap::idx::FN));
                assert_eq!(rules[0].trigger_key, crate::keymap::idx::T);
                assert_eq!(rules[0].gesture_label(), "Fn + T");
                assert!(rules[0].action.summary().contains("Arrow Up"));
            }
            other => panic!("unexpected req: {other:?}"),
        }

        let resp = DaemonResponse::CalibrationUpdate(CalibrationProgress {
            status: crate::protocol::CalibrationStatus::Progress,
            calibrated_indices: vec![0, 46],
        });
        let mut rbuf = Vec::new();
        write_json_line(&mut rbuf, &resp).unwrap();
        let mut rcur = Cursor::new(rbuf);
        let rdecoded: DaemonResponse = read_json_line(&mut rcur).unwrap();
        assert!(matches!(rdecoded, DaemonResponse::CalibrationUpdate(_)));

        let areq = DaemonRequest::StartAudioSync {
            config: AudioSyncConfig::default(),
        };
        let mut abuf = Vec::new();
        write_json_line(&mut abuf, &areq).unwrap();
        let mut acur = Cursor::new(abuf);
        let adecoded: DaemonRequest = read_json_line(&mut acur).unwrap();
        assert!(matches!(adecoded, DaemonRequest::StartAudioSync { .. }));
    }
}

