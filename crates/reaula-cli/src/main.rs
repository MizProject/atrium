use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand, ValueEnum};

#[cfg(target_os = "linux")]
use reaula_core::alsa::{self, AlsaCapture, AlsaSourceMode};
use reaula_core::audio_fx::{
    self, AudioEffectStyle, AudioPalette, AudioVisualizer, DEFAULT_SAMPLE_RATE, DemoAudioSynth,
};
use reaula_core::diagnostics::{
    self, CheckStatus, DiagnosticCheck, DiagnosticLayer, DiagnosticReport,
};
use reaula_core::driver::{DeviceProbe, Win60Driver};
use reaula_core::ipc::{
    self, AudioSyncConfig, AudioSyncSource, AudioSyncStatus, DaemonClient, FnOverride,
    KeyActuationEntry, VirtualKey,
};
use reaula_core::keyboards;
use reaula_core::keymap;
use reaula_core::protocol::{
    self, BODY_LEN, LightState, PID, PollRate, PrcsPair, TriggerMode, VID, WinLockFlags, mm_to_raw,
};
use reaula_core::transport::{HidApiTransport, TransportError, scan_devices};

#[derive(Parser)]
#[command(
    name = "reaula",
    version,
    about = "Userspace driver CLI for the Aula WIN 60 HE Hall-effect keyboard"
)]
struct Cli {
    /// Connect through a specific reaulad Unix socket path
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    /// Require connecting via reaulad (fail if daemon is not running)
    #[arg(long, global = true, conflicts_with = "direct")]
    daemon: bool,

    /// Bypass reaulad and talk directly to the HID device
    #[arg(long, global = true)]
    direct: bool,

    /// Re-run direct HID commands under sudo if not already root
    #[arg(long, global = true, conflicts_with = "no_sudo")]
    sudo: bool,

    /// Never prompt to elevate via sudo on permission errors
    #[arg(long, global = true)]
    no_sudo: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy)]
struct ConnectionPolicy<'a> {
    socket: Option<&'a Path>,
    require_daemon: bool,
    force_direct: bool,
    force_sudo: bool,
    no_sudo: bool,
}

enum CliError {
    Message(String),
    ChildExit(ExitCode),
}

impl From<String> for CliError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

#[derive(Subcommand)]
enum Command {
    /// Scan for connected Aula WIN 60 HE interfaces and check permissions
    Scan,

    /// Manage the secure reaulad background daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Launch the Ratatui terminal UI (connects to reaulad)
    Tui,

    /// Launch the GPUI desktop window (connects to reaulad)
    Gui,

    /// Connect to the keyboard and query firmware, capabilities, lighting, and travel limits
    Probe,

    /// Read or configure key actuation & Rapid Trigger settings
    Actuation {
        #[command(subcommand)]
        action: ActuationAction,
    },

    /// Read or set keyboard RGB lighting
    Light {
        #[command(subcommand)]
        action: LightAction,
    },

    /// Read or set magnetic switch profiles (HM1, HH1, CY1, TC1)
    Switch {
        #[command(subcommand)]
        action: SwitchAction,
    },

    /// Read or set top/bottom key deadbands
    Deadband {
        #[command(subcommand)]
        action: DeadbandAction,
    },

    /// Read or set USB polling rate (1, 2, 4, or 8 kHz)
    PollRate {
        #[command(subcommand)]
        action: PollRateAction,
    },

    /// Read or configure PRCS / SOCD key pairs
    Socd {
        #[command(subcommand)]
        action: SocdAction,
    },

    /// Configure Win-lock / shortcut-disable flags
    WinLock {
        #[arg(long)]
        disable_win: bool,
        #[arg(long)]
        disable_shift_tab: bool,
        #[arg(long)]
        disable_alt_tab: bool,
        #[arg(long)]
        disable_alt_f4: bool,
        #[arg(long)]
        off: bool,
    },

    /// Stream live Hall-effect key travel depth in real time
    Travel {
        /// Key selector: 'all', 'wasd', or comma-separated keys (e.g. W,A,S,D,Space)
        #[arg(long, default_value = "all")]
        keys: String,

        /// Optional timeout in seconds (runs until Ctrl+C if omitted)
        #[arg(long)]
        duration: Option<u64>,
    },

    /// Inspect or modify keyboard keymap tables (`cmd 0x18` base or Fn layer)
    Keymap {
        /// Read Fn layer instead of base layer
        #[arg(long)]
        r#fn: bool,

        /// Write Fn-layer remaps directly to keyboard flash, e.g. `R=Up,D=Left,F=Down,G=Right`
        #[arg(long, value_name = "KEY=TARGET,...")]
        set_fn: Option<String>,

        /// Clear specific Fn-layer remaps back to factory default, e.g. `R,D,F,G`
        #[arg(long, value_name = "KEYS")]
        clear_fn: Option<String>,

        /// Clear ALL custom Fn-layer bindings in keyboard flash
        #[arg(long)]
        reset_fn: bool,
    },

    /// Send a raw 63-byte command body in hex (destructive commands are blocked)
    Raw {
        /// Hex bytes (spaces optional, zero-padded to 63 bytes)
        hex_bytes: Vec<String>,
    },

    /// Experiment with live ALSA audio capture, FFT processing, and keyboard RGB effects
    Audio {
        #[command(subcommand)]
        action: AudioAction,
    },

    /// Run 4-layer diagnostics isolating Kernel, Daemon, Keyboard, and Software faults
    Doctor,
}

#[derive(clap::Args, Clone)]
struct AudioSyncArgs {
    /// Audio input source: monitor (desktop loopback), mic, device, stdin, or demo
    #[arg(long, value_enum, default_value_t = AudioSourceArg::Monitor)]
    source: AudioSourceArg,
    /// Explicit ALSA PCM name when --source device is used (e.g. hw:0,0, plughw:0,0, pulse, default)
    #[arg(long)]
    device: Option<String>,
    /// Reactive lighting effect rendered onto the 61 keys
    #[arg(long, value_enum, default_value_t = AudioEffectArg::Spectrum)]
    effect: AudioEffectArg,
    /// Color palette for the audio effect
    #[arg(long, value_enum, default_value_t = AudioPaletteArg::Aurora)]
    palette: AudioPaletteArg,
    /// Foreground RGB when --palette custom is used (R,G,B or #RRGGBB)
    #[arg(long, default_value = "38bdf8")]
    fg: String,
    /// Background RGB behind active bars/ripples
    #[arg(long, default_value = "000000")]
    bg: String,
    /// Firmware Custom-mode LED brightness (0..=4)
    #[arg(long, default_value_t = 4)]
    brightness: u8,
    /// Target LED refresh rate in frames per second (5..=60)
    #[arg(long, default_value_t = 35)]
    fps: u8,
    /// Input gain multiplier (0.1..=10.0)
    #[arg(long, default_value_t = 1.2)]
    gain: f32,
}

#[derive(Subcommand)]
enum AudioAction {
    /// List available ALSA PCM capture devices and monitor targets
    Devices,
    /// Start audio equalizer inside reaulad and return immediately (persists across GUI/CLI exits)
    Start {
        #[command(flatten)]
        cfg: AudioSyncArgs,
    },
    /// Stop the running daemon-hosted audio equalizer
    Stop {
        /// Do not restore the previous keyboard lighting state after stopping
        #[arg(long)]
        no_restore: bool,
    },
    /// Report status and live telemetry from reaulad's audio equalizer
    Status {
        /// Continuously stream live FFT meter and telemetry from reaulad until Ctrl+C
        #[arg(long)]
        watch: bool,
    },
    /// Run audio equalizer (uses reaulad with live watch unless --dry-run, --direct, or --source stdin)
    Run {
        #[command(flatten)]
        cfg: AudioSyncArgs,
        /// Capture sample rate in Hz (for --direct / --dry-run / --source stdin s16le mono)
        #[arg(long, default_value_t = DEFAULT_SAMPLE_RATE)]
        rate: u32,
        /// Optional duration in seconds (runs until Ctrl+C if omitted)
        #[arg(long)]
        duration: Option<u64>,
        /// Process audio & print live spectrum telemetry without writing LED frames to the keyboard
        #[arg(long)]
        dry_run: bool,
        /// Do not save/restore the previous keyboard lighting state on exit
        #[arg(long)]
        no_restore: bool,
        /// Start in reaulad and exit immediately instead of waiting/stopping on Ctrl+C
        #[arg(long)]
        detach: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum AudioSourceArg {
    Monitor,
    Mic,
    Device,
    Stdin,
    Demo,
}

#[derive(Clone, Copy, ValueEnum)]
enum AudioEffectArg {
    Spectrum,
    Pulse,
    Wave,
    Meter,
}

impl From<AudioEffectArg> for AudioEffectStyle {
    fn from(value: AudioEffectArg) -> Self {
        match value {
            AudioEffectArg::Spectrum => Self::Spectrum,
            AudioEffectArg::Pulse => Self::Pulse,
            AudioEffectArg::Wave => Self::Wave,
            AudioEffectArg::Meter => Self::Meter,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum AudioPaletteArg {
    Aurora,
    Regal,
    Neon,
    Inferno,
    Custom,
}

impl From<AudioPaletteArg> for AudioPalette {
    fn from(value: AudioPaletteArg) -> Self {
        match value {
            AudioPaletteArg::Aurora => Self::Aurora,
            AudioPaletteArg::Regal => Self::Regal,
            AudioPaletteArg::Neon => Self::Neon,
            AudioPaletteArg::Inferno => Self::Inferno,
            AudioPaletteArg::Custom => Self::Custom,
        }
    }
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Check whether reaulad is running and display peer-credential security state
    Status,
    /// Start reaulad detached in the background (prompts for sudo if hidraw requires root)
    Start {
        /// Run in foreground instead of detaching
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the running reaulad instance cleanly
    Stop,
}

#[derive(Subcommand)]
enum ActuationAction {
    /// Read actuation & Rapid Trigger config for selected keys
    Get {
        #[arg(long, default_value = "all")]
        keys: String,
    },
    /// Set actuation travel and/or Rapid Trigger mode for selected keys
    Set {
        #[arg(long, default_value = "all")]
        keys: String,
        /// Actuation travel in millimeters (e.g. 1.70, valid 0.08..=3.40)
        #[arg(long)]
        travel: f64,
        #[arg(long, value_enum, default_value_t = CliTriggerMode::Fixed)]
        mode: CliTriggerMode,
        /// Rapid Trigger press sensitivity in mm (for rt / rt-split)
        #[arg(long)]
        rt_press: Option<f64>,
        /// Rapid Trigger release sensitivity in mm (for rt-split)
        #[arg(long)]
        rt_release: Option<f64>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CliTriggerMode {
    Fixed,
    Rt,
    RtSplit,
}

impl From<CliTriggerMode> for TriggerMode {
    fn from(value: CliTriggerMode) -> Self {
        match value {
            CliTriggerMode::Fixed => Self::Fixed,
            CliTriggerMode::Rt => Self::RapidTrigger,
            CliTriggerMode::RtSplit => Self::RapidTriggerSplit,
        }
    }
}

#[derive(Subcommand)]
enum LightAction {
    /// Read current lighting configuration and supported firmware modes
    Get,
    /// Set lighting mode, color, brightness, and speed
    Set {
        #[arg(long, default_value = "static")]
        mode: String,
        #[arg(long, default_value = "255,255,255")]
        color: String,
        #[arg(long, default_value = "0,0,0")]
        bg: String,
        #[arg(long, default_value_t = 4)]
        brightness: u8,
        #[arg(long, default_value_t = 4)]
        speed: u8,
        #[arg(long, default_value_t = 0)]
        direction: u8,
        #[arg(long)]
        full_color: bool,
        #[arg(long)]
        off: bool,
    },
}

#[derive(Subcommand)]
enum SwitchAction {
    /// Read per-key magnetic switch profiles
    Get {
        #[arg(long, default_value = "all")]
        keys: String,
    },
    /// Set magnetic switch profile (read-modify-write)
    Set {
        #[arg(long, default_value = "all")]
        keys: String,
        #[arg(long)]
        profile: String,
    },
}

#[derive(Subcommand)]
enum DeadbandAction {
    /// Read per-key top/bottom deadbands
    Get {
        #[arg(long, default_value = "all")]
        keys: String,
    },
    /// Set top/bottom deadband in mm (read-modify-write, e.g. 0.02)
    Set {
        #[arg(long, default_value = "all")]
        keys: String,
        #[arg(long)]
        top: f64,
        #[arg(long)]
        bottom: f64,
    },
}

#[derive(Subcommand)]
enum PollRateAction {
    /// Read current USB polling rate
    Get,
    /// Set USB polling rate in kHz (1, 2, 4, or 8; restarts keyboard)
    Set { khz: u8 },
}

#[derive(Subcommand)]
enum SocdAction {
    /// Read current PRCS / SOCD enabled state and configured pairs
    Get,
    /// Set PRCS / SOCD power and pairs (e.g. --on --pairs A:D:2)
    Set {
        #[arg(long, conflicts_with = "off")]
        on: bool,
        #[arg(long)]
        off: bool,
        /// Comma-separated KEY1:KEY2[:MODEL1..4] specs (e.g. A:D:2,W:S:2)
        #[arg(long, default_value = "")]
        pairs: String,
    },
}

enum Backend {
    Daemon(DaemonClient),
    Direct(Win60Driver<HidApiTransport>),
}

impl Backend {
    fn source_label(&self) -> String {
        match self {
            Self::Daemon(c) => format!(
                "daemon {} (uid={})",
                c.socket_path().display(),
                c.server_credentials().uid
            ),
            Self::Direct(d) => format!("direct HID ({})", d.device_description()),
        }
    }

    fn probe(&mut self) -> Result<DeviceProbe, String> {
        match self {
            Self::Daemon(c) => c.probe().map_err(|e| e.to_string()),
            Self::Direct(d) => d.probe().map_err(|e| e.to_string()),
        }
    }

    fn get_actuation(&mut self, indices: Vec<u8>) -> Result<(f64, Vec<KeyActuationEntry>), String> {
        match self {
            Self::Daemon(c) => c.get_actuation(indices).map_err(|e| e.to_string()),
            Self::Direct(d) => {
                let unit_mm = direct_unit_mm(d);
                let map = d
                    .read_trigger_configs(&indices)
                    .map_err(|e| e.to_string())?;
                let entries = indices
                    .into_iter()
                    .map(|index| KeyActuationEntry {
                        index,
                        config: map.get(&index).copied(),
                    })
                    .collect();
                Ok((unit_mm, entries))
            }
        }
    }

    fn set_actuation(
        &mut self,
        indices: Vec<u8>,
        mode: TriggerMode,
        travel_mm: f64,
        rt_press_mm: Option<f64>,
        rt_release_mm: Option<f64>,
    ) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c
                .set_actuation(indices, mode, travel_mm, rt_press_mm, rt_release_mm)
                .map_err(|e| e.to_string()),
            Self::Direct(d) => {
                let hp = d
                    .read_init_info()
                    .map(|i| i.is_high_precision)
                    .unwrap_or(false);
                let limits = d.read_max_trigger_travel(hp).ok();
                d.set_actuation(
                    &indices,
                    mode,
                    travel_mm,
                    rt_press_mm,
                    rt_release_mm,
                    limits,
                )
                .map_err(|e| e.to_string())
            }
        }
    }

    fn get_light(&mut self) -> Result<LightState, String> {
        match self {
            Self::Daemon(c) => c.get_light().map_err(|e| e.to_string()),
            Self::Direct(d) => d.read_light().map_err(|e| e.to_string()),
        }
    }

    fn set_light(&mut self, state: LightState) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c.set_light(state).map_err(|e| e.to_string()),
            Self::Direct(d) => d.set_light(&state).map_err(|e| e.to_string()),
        }
    }

    fn get_switches(&mut self, indices: Vec<u8>) -> Result<Vec<(u8, u8)>, String> {
        match self {
            Self::Daemon(c) => c
                .get_switches(indices)
                .map(|v| v.into_iter().map(|e| (e.index, e.switch_id)).collect())
                .map_err(|e| e.to_string()),
            Self::Direct(d) => {
                let table = d.read_switch_table().map_err(|e| e.to_string())?;
                Ok(indices
                    .into_iter()
                    .map(|idx| (idx, table.get(idx as usize).copied().unwrap_or(0)))
                    .collect())
            }
        }
    }

    fn set_switches(&mut self, indices: Vec<u8>, id: u8) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c.set_switches(indices, id).map_err(|e| e.to_string()),
            Self::Direct(d) => d
                .set_switch_profile(&indices, id)
                .map_err(|e| e.to_string()),
        }
    }

    fn get_deadband(&mut self, indices: Vec<u8>) -> Result<Vec<(u8, u8, u8)>, String> {
        match self {
            Self::Daemon(c) => c
                .get_deadband(indices)
                .map(|v| {
                    v.into_iter()
                        .map(|e| (e.index, e.top_raw, e.bottom_raw))
                        .collect()
                })
                .map_err(|e| e.to_string()),
            Self::Direct(d) => {
                let table = d.read_deadband_table().map_err(|e| e.to_string())?;
                Ok(indices
                    .into_iter()
                    .map(|idx| {
                        let off = (idx as usize) * 2;
                        (
                            idx,
                            table.get(off).copied().unwrap_or(0),
                            table.get(off + 1).copied().unwrap_or(0),
                        )
                    })
                    .collect())
            }
        }
    }

    fn set_deadband(
        &mut self,
        indices: Vec<u8>,
        top_raw: u8,
        bottom_raw: u8,
    ) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c
                .set_deadband(indices, top_raw, bottom_raw)
                .map_err(|e| e.to_string()),
            Self::Direct(d) => d
                .set_deadband(&indices, top_raw, bottom_raw)
                .map_err(|e| e.to_string()),
        }
    }

    fn get_poll_rate(&mut self) -> Result<u8, String> {
        match self {
            Self::Daemon(c) => c.get_poll_rate().map_err(|e| e.to_string()),
            Self::Direct(d) => d.read_poll_rate().map_err(|e| e.to_string()),
        }
    }

    fn set_poll_rate(&mut self, khz: u8) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c.set_poll_rate(khz).map_err(|e| e.to_string()),
            Self::Direct(d) => {
                let rate = PollRate::from_khz(khz).map_err(|e| e.to_string())?;
                d.set_poll_rate(rate).map_err(|e| e.to_string())
            }
        }
    }

    fn get_socd(&mut self) -> Result<(bool, Vec<PrcsPair>), String> {
        match self {
            Self::Daemon(c) => c.get_socd().map_err(|e| e.to_string()),
            Self::Direct(d) => {
                let enabled = d.read_prcs_power().map_err(|e| e.to_string())?;
                let pairs = d.read_prcs_pairs().map_err(|e| e.to_string())?;
                Ok((enabled, pairs))
            }
        }
    }

    fn set_socd(&mut self, enabled: bool, pairs: Vec<PrcsPair>) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c.set_socd(enabled, pairs).map_err(|e| e.to_string()),
            Self::Direct(d) => d.set_socd(enabled, &pairs).map_err(|e| e.to_string()),
        }
    }

    fn get_keymap(&mut self, fn_layer: bool) -> Result<Vec<u8>, String> {
        match self {
            Self::Daemon(c) => c.get_keymap(fn_layer).map_err(|e| e.to_string()),
            Self::Direct(d) => d
                .read_keymap(fn_layer)
                .map(|arr| arr.to_vec())
                .map_err(|e| e.to_string()),
        }
    }

    fn apply_keyboard_fn_map(
        &mut self,
        set: Vec<FnOverride>,
        clear: Vec<u8>,
    ) -> Result<Vec<FnOverride>, String> {
        match self {
            Self::Daemon(c) => c
                .apply_keyboard_fn_map(set, clear)
                .map_err(|e| e.to_string()),
            Self::Direct(d) => {
                let pairs: Vec<(u8, u8)> =
                    set.into_iter().map(|o| (o.index, o.target_hid)).collect();
                d.apply_fn_keymap_overrides(&pairs, &clear)
                    .map(|list| list.into_iter().map(FnOverride::from).collect())
                    .map_err(|e| e.to_string())
            }
        }
    }

    fn reset_keyboard_fn_map(&mut self) -> Result<Vec<FnOverride>, String> {
        match self {
            Self::Daemon(c) => c.reset_keyboard_fn_map().map_err(|e| e.to_string()),
            Self::Direct(d) => d
                .reset_fn_keymap()
                .map(|list| list.into_iter().map(FnOverride::from).collect())
                .map_err(|e| e.to_string()),
        }
    }

    fn set_win_lock(&mut self, flags: WinLockFlags) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c.set_win_lock(flags).map_err(|e| e.to_string()),
            Self::Direct(d) => d.set_win_lock(flags).map_err(|e| e.to_string()),
        }
    }

    fn raw(&mut self, body: [u8; BODY_LEN]) -> Result<Option<Vec<u8>>, String> {
        match self {
            Self::Daemon(c) => c.raw(body.to_vec()).map_err(|e| e.to_string()),
            Self::Direct(d) => d.send_raw_checked(&body).map_err(|e| e.to_string()),
        }
    }

    fn get_custom_rgb(&mut self, slot: u8) -> Result<Vec<[u8; 3]>, String> {
        match self {
            Self::Daemon(c) => c.get_custom_rgb(slot).map_err(|e| e.to_string()),
            Self::Direct(d) => d
                .read_custom_light(slot)
                .map(|arr| arr.to_vec())
                .map_err(|e| e.to_string()),
        }
    }

    fn set_custom_rgb(
        &mut self,
        slot: u8,
        colors: Vec<[u8; 3]>,
        brightness: u8,
    ) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c
                .set_custom_rgb(slot, colors, brightness)
                .map_err(|e| e.to_string()),
            Self::Direct(d) => {
                if colors.len() != protocol::MATRIX_SLOTS {
                    return Err(format!(
                        "expected {} RGB slots, got {}",
                        protocol::MATRIX_SLOTS,
                        colors.len()
                    ));
                }
                let mut arr = [[0u8; 3]; protocol::MATRIX_SLOTS];
                arr.copy_from_slice(&colors);
                d.set_custom_light(slot, &arr, brightness)
                    .map_err(|e| e.to_string())
            }
        }
    }

    fn begin_custom_stream(&mut self, brightness: u8) -> Result<(), String> {
        match self {
            Self::Daemon(c) => c.begin_custom_stream(brightness).map_err(|e| {
                let msg = e.to_string();
                if msg.contains("unknown variant") {
                    "running reaulad predates fast RGB streaming; restart it (`reaula daemon stop && reaula daemon start`) or pass --dry-run".into()
                } else {
                    msg
                }
            }),
            Self::Direct(d) => d.begin_custom_stream(brightness).map_err(|e| e.to_string()),
        }
    }

    fn stream_custom_rgb(
        &mut self,
        slot: u8,
        colors: Vec<[u8; 3]>,
        force: bool,
    ) -> Result<usize, String> {
        match self {
            Self::Daemon(c) => c
                .stream_custom_rgb(slot, colors, force)
                .map_err(|e| e.to_string()),
            Self::Direct(d) => {
                if colors.len() != protocol::MATRIX_SLOTS {
                    return Err(format!(
                        "expected {} RGB slots, got {}",
                        protocol::MATRIX_SLOTS,
                        colors.len()
                    ));
                }
                let mut arr = [[0u8; 3]; protocol::MATRIX_SLOTS];
                arr.copy_from_slice(&colors);
                d.stream_custom_light(slot, &arr, force, true)
                    .map_err(|e| e.to_string())
            }
        }
    }
}

fn direct_unit_mm(drv: &mut Win60Driver<HidApiTransport>) -> f64 {
    drv.read_init_info()
        .ok()
        .and_then(|info| drv.read_max_trigger_travel(info.is_high_precision).ok())
        .map_or(protocol::TRIGGER_UNIT_MM_DEFAULT, |m| m.unit_mm)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::ChildExit(code)) => code,
        Err(CliError::Message(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    let policy = ConnectionPolicy {
        socket: cli.socket.as_deref(),
        require_daemon: cli.daemon,
        force_direct: cli.direct,
        force_sudo: cli.sudo,
        no_sudo: cli.no_sudo,
    };

    match cli.command {
        Command::Scan => cmd_scan().map_err(CliError::from),
        Command::Daemon { action } => cmd_daemon(action, policy),
        Command::Tui => launch_sibling("reaula-tui", policy.socket),
        Command::Gui => launch_sibling("reaula-gui", policy.socket),
        Command::Probe => cmd_probe(policy),
        Command::Actuation { action } => cmd_actuation(action, policy),
        Command::Light { action } => cmd_light(action, policy),
        Command::Switch { action } => cmd_switch(action, policy),
        Command::Deadband { action } => cmd_deadband(action, policy),
        Command::PollRate { action } => cmd_poll_rate(action, policy),
        Command::Socd { action } => cmd_socd(action, policy),
        Command::WinLock {
            disable_win,
            disable_shift_tab,
            disable_alt_tab,
            disable_alt_f4,
            off,
        } => {
            let flags = if off {
                WinLockFlags::default()
            } else {
                WinLockFlags {
                    disable_win,
                    disable_shift_tab,
                    disable_alt_tab,
                    disable_alt_f4,
                    win_lock_on: disable_win
                        || disable_shift_tab
                        || disable_alt_tab
                        || disable_alt_f4,
                }
            };
            let mut backend = connect_backend(policy)?;
            backend.set_win_lock(flags)?;
            println!("Updated Win-lock flags: {flags:?}");
            Ok(())
        }
        Command::Travel { keys, duration } => cmd_travel(&keys, duration, policy),
        Command::Keymap {
            r#fn,
            set_fn,
            clear_fn,
            reset_fn,
        } => cmd_keymap(
            r#fn,
            set_fn.as_deref(),
            clear_fn.as_deref(),
            reset_fn,
            policy,
        ),
        Command::Raw { hex_bytes } => cmd_raw(&hex_bytes, policy),
        Command::Audio { action } => cmd_audio(action, policy),
        Command::Doctor => cmd_doctor(policy),
    }
}

fn is_root() -> bool {
    ipc::current_uid() == 0
}

fn reexec_with_sudo(path: &str, detail: &str, udev_rule: &'static str, ask: bool) -> CliError {
    if !io::stdin().is_terminal() {
        return CliError::Message(
            TransportError::PermissionDenied {
                path: path.to_string(),
                detail: detail.to_string(),
                udev_rule,
            }
            .to_string(),
        );
    }

    if ask {
        eprintln!("Permission denied opening {path} (process is not running as root).");
        eprint!("Re-run this command with sudo? [Y/n] ");
        let _ = io::stderr().flush();

        let mut answer = String::new();
        if io::stdin().read_line(&mut answer).is_err() {
            return CliError::Message("failed to read response from stdin".into());
        }
        let trimmed = answer.trim();
        if !(trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("y")
            || trimmed.eq_ignore_ascii_case("yes"))
        {
            return CliError::Message(
                TransportError::PermissionDenied {
                    path: path.to_string(),
                    detail: detail.to_string(),
                    udev_rule,
                }
                .to_string(),
            );
        }
    }

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return CliError::Message(format!("failed to resolve current executable: {e}")),
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();

    match ProcessCommand::new("sudo").arg(exe).args(args).status() {
        Ok(status) => {
            let code = status
                .code()
                .and_then(|c| u8::try_from(c).ok())
                .map_or(ExitCode::FAILURE, ExitCode::from);
            CliError::ChildExit(code)
        }
        Err(e) => CliError::Message(format!("failed to invoke sudo: {e}")),
    }
}

fn connect_backend(policy: ConnectionPolicy<'_>) -> Result<Backend, CliError> {
    if !policy.force_direct {
        match DaemonClient::connect(policy.socket) {
            Ok(client) => return Ok(Backend::Daemon(client)),
            Err(err) if policy.require_daemon => {
                return Err(CliError::Message(format!(
                    "could not connect to reaulad ({err}); start it with `reaula daemon start`"
                )));
            }
            Err(_) => {}
        }
    }

    if policy.force_sudo && !is_root() {
        return Err(reexec_with_sudo(
            "/dev/hidraw*",
            "--sudo requested",
            keyboards::win60he::SPEC.udev_rule,
            false,
        ));
    }

    match HidApiTransport::open_win60() {
        Ok(t) => Ok(Backend::Direct(Win60Driver::new(t))),
        Err(TransportError::PermissionDenied {
            path,
            detail,
            udev_rule,
        }) => {
            if !is_root() && !policy.no_sudo {
                Err(reexec_with_sudo(&path, &detail, udev_rule, true))
            } else {
                Err(CliError::Message(
                    TransportError::PermissionDenied {
                        path,
                        detail,
                        udev_rule,
                    }
                    .to_string(),
                ))
            }
        }
        Err(other) => Err(CliError::Message(other.to_string())),
    }
}

fn sibling_binary(name: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("failed to locate exe: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "executable has no parent directory".to_string())?;
    let candidate = dir.join(name);
    if candidate.exists() {
        Ok(candidate)
    } else {
        Ok(PathBuf::from(name))
    }
}

fn launch_sibling(bin_name: &str, socket: Option<&Path>) -> Result<(), CliError> {
    let bin = sibling_binary(bin_name)?;
    let mut cmd = ProcessCommand::new(&bin);
    if let Some(sock) = socket {
        cmd.arg("--socket").arg(sock);
    }
    let status = cmd.status().map_err(|e| {
        CliError::Message(format!(
            "failed to launch {bin_name} ({}): {e}\nBuild workspace binaries with `cargo build --workspace`.",
            bin.display()
        ))
    })?;
    let code = status
        .code()
        .and_then(|c| u8::try_from(c).ok())
        .map_or(ExitCode::FAILURE, ExitCode::from);
    if status.success() {
        Ok(())
    } else {
        Err(CliError::ChildExit(code))
    }
}

fn cmd_daemon(action: DaemonAction, policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    match action {
        DaemonAction::Status => {
            let mut client = DaemonClient::connect(policy.socket).map_err(|e| {
                CliError::Message(format!(
                    "reaulad is not reachable at {}: {e}",
                    ipc::discover_client_socket_path(policy.socket).display()
                ))
            })?;
            let st = client.ping().map_err(|e| e.to_string())?;
            println!("reaulad status  : running");
            println!("Socket          : {}", st.socket_path);
            println!(
                "Daemon PID/UID  : pid={} uid={}",
                st.daemon_pid, st.daemon_uid
            );
            println!("Allowed UIDs    : {:?}", st.allowed_uids);
            println!(
                "Keyboard        : {}",
                if st.device_connected {
                    st.device_description.as_deref().unwrap_or("connected")
                } else {
                    "disconnected (waiting for USB device)"
                }
            );
            println!("Travel clients  : {}", st.travel_subscribers);
            Ok(())
        }
        DaemonAction::Stop => {
            let mut client = DaemonClient::connect(policy.socket).map_err(|e| e.to_string())?;
            let sock = ipc::discover_client_socket_path(policy.socket);
            client.shutdown().map_err(|e| e.to_string())?;
            drop(client);
            for _ in 0..80 {
                if !sock.exists() && DaemonClient::connect(policy.socket).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            println!("Stopped reaulad cleanly.");
            Ok(())
        }
        DaemonAction::Start { foreground } => {
            if DaemonClient::connect(policy.socket).is_ok() {
                println!("reaulad is already running.");
                return Ok(());
            }

            let bin = sibling_binary("reaulad")?;
            let scan = scan_devices(VID, PID).map_err(|e| e.to_string())?;
            let vendor_accessible = scan.sysfs.iter().any(|d| {
                d.descriptor.as_ref().and_then(|desc| desc.usage_page)
                    == Some(protocol::VENDOR_USAGE_PAGE)
                    && d.readable_writable
            });

            let needs_root = !is_root() && (!vendor_accessible || policy.force_sudo);
            let my_uid = ipc::current_uid();

            let mut cmd = if needs_root {
                if policy.no_sudo {
                    return Err(CliError::Message(
                        "vendor hidraw requires root and --no-sudo was set".into(),
                    ));
                }
                let mut c = ProcessCommand::new("sudo");
                c.arg(&bin).arg("--allow-uid").arg(my_uid.to_string());
                if let Some(s) = policy.socket {
                    c.arg("--socket").arg(s);
                } else {
                    c.arg("--socket").arg(format!(
                        "{}/{}",
                        ipc::SYSTEM_SOCKET_DIR,
                        ipc::SOCKET_FILENAME
                    ));
                }
                c
            } else {
                let mut c = ProcessCommand::new(&bin);
                if let Some(s) = policy.socket {
                    c.arg("--socket").arg(s);
                }
                c
            };

            if !foreground {
                cmd.arg("--detach");
            }

            let status = cmd
                .status()
                .map_err(|e| CliError::Message(format!("failed to start reaulad: {e}")))?;
            if status.success() {
                Ok(())
            } else {
                Err(CliError::Message(format!(
                    "reaulad exited with status {status}"
                )))
            }
        }
    }
}

fn cmd_scan() -> Result<(), String> {
    println!("Indexed Aula keyboards ({} registered):", keyboards::all().len());
    for spec in keyboards::all() {
        let primary = spec.primary_usb_id();
        println!(
            "  - {:<10} {} ({}, {} keys, {:04X}:{:04X}, usage_page=0x{:04X})",
            spec.id,
            spec.display_name,
            spec.form_factor,
            spec.key_count(),
            primary.vid,
            primary.pid,
            spec.vendor_usage_page
        );
    }
    println!();

    let mut found_any = false;
    let mut any_denied = false;
    for spec in keyboards::all() {
        for usb in spec.usb_ids {
            let scan = scan_devices(usb.vid, usb.pid).map_err(|e| e.to_string())?;
            if scan.sysfs.is_empty() && scan.hidapi_interfaces.is_empty() {
                continue;
            }
            found_any = true;
            println!(
                "Matched {} [{}: {:04X}:{:04X}]:\n",
                spec.display_name, spec.id, usb.vid, usb.pid
            );

            if !scan.sysfs.is_empty() {
                println!("Linux sysfs hidraw interfaces:");
                for dev in &scan.sysfs {
                    let iface = dev
                        .interface_number
                        .map_or_else(|| "?".into(), |n| n.to_string());
                    let desc = dev
                        .descriptor
                        .as_ref()
                        .map_or_else(|| "no descriptor".into(), ToString::to_string);
                    let is_vendor = keyboards::is_vendor_sysfs_device(dev, spec);
                    let role = if is_vendor { " [VENDOR CONFIG]" } else { "" };
                    let access = if dev.readable_writable {
                        "rw OK".into()
                    } else {
                        if is_vendor {
                            any_denied = true;
                        }
                        format!(
                            "BLOCKED ({})",
                            dev.permission_error.as_deref().unwrap_or("no access")
                        )
                    };
                    println!(
                        "  {}  name={:?}  iface={}  {}{role}\n      {} | {}",
                        dev.dev_path, dev.name, iface, dev.phys, desc, access
                    );
                }
                println!();
            }

            if !scan.hidapi_interfaces.is_empty() {
                println!("hidapi enumeration:");
                for iface in &scan.hidapi_interfaces {
                    println!(
                        "  {}  product={:?}  iface={}  usage_page=0x{:04X}  usage=0x{:02X}",
                        iface.path,
                        iface.product,
                        iface.interface_number,
                        iface.usage_page,
                        iface.usage
                    );
                }
                println!();
            }
        }
    }

    if !found_any {
        println!("No matching indexed Aula USB HID devices found.");
        return Ok(());
    }

    if let Ok(mut client) = DaemonClient::connect(None)
        && let Ok(st) = client.ping()
    {
        println!(
            "reaulad is running on {} (allowed_uids={:?}, keyboard={}, model={}).",
            st.socket_path,
            st.allowed_uids,
            if st.device_connected {
                "connected"
            } else {
                "disconnected"
            },
            st.keyboard_name
                .as_deref()
                .or(st.keyboard_id.as_deref())
                .unwrap_or("auto")
        );
    } else if any_denied {
        println!(
            "Vendor interface access is restricted to root.\n\
             Start the secure daemon once (`reaula daemon start`), or answer 'y' when prompted for sudo."
        );
    }

    Ok(())
}

fn cmd_probe(policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    let mut backend = connect_backend(policy)?;
    println!("Connected via {}\n", backend.source_label());
    let p = backend.probe()?;

    if let (Some(id), Some(name)) = (p.keyboard_id.as_deref(), p.model_name.as_deref()) {
        println!("Indexed model   : {name} ({id})");
    }
    println!("Device          : {}", p.device_description);
    println!(
        "Firmware string : {}",
        p.firmware_string.as_deref().unwrap_or("<no reply>")
    );
    println!(
        "Build date      : {}",
        p.build_date.as_deref().unwrap_or("<no reply>")
    );

    if let Some(info) = &p.init_info {
        println!(
            "Connection      : {} (battery={}%, charging={})",
            info.connection_mode(),
            info.battery,
            info.charging
        );
        println!(
            "Capabilities    : flags1=0x{:02X} flags2=0x{:02X}",
            info.raw_flags1, info.raw_flags2
        );
        println!(
            "  PRCS/SOCD={}  CustomRGB={}  Switches={}  PollRate={}  Deadband={}  MusicRhythm={}  AnyKeyCal={}",
            info.is_prcs,
            info.is_custom_light,
            info.is_more_switch,
            info.is_poll_rate,
            info.is_dead_band,
            info.is_music_rhythm,
            info.is_any_key_calibration,
        );
        println!(
            "  HighPrecision={}  Gamepad={}  RS={}  RKRT={}  6KeyMode={}  MusicOn={}",
            info.is_high_precision,
            info.is_gamepad,
            info.is_rs,
            info.is_rkrt,
            info.is_6key_mode,
            info.is_music_rhythm_on,
        );
    }
    if let Some(mt) = p.max_travel {
        println!(
            "Travel limits   : {:.2}..={:.2} mm (step {:.3} mm, raw {}..={})",
            mt.min_mm, mt.max_mm, mt.unit_mm, mt.min_raw, mt.max_raw
        );
    }
    if let Some(khz) = p.poll_rate_khz {
        println!(
            "Polling rate    : {} kHz ({} Hz)",
            khz,
            u16::from(khz) * 1000
        );
    }
    if let Some(list) = &p.light_list {
        let names: Vec<String> = list
            .modes
            .iter()
            .map(|&m| format!("{m}:{}", protocol::light_mode_name(m)))
            .collect();
        println!(
            "Lighting modes  : [{}] (max_bri={}, max_spd={})",
            names.join(", "),
            list.max_brightness,
            list.max_speed
        );
    }
    if let Some(ls) = &p.light_state {
        println!(
            "Current light   : mode={} ({}) bri={} spd={} fg=#{:02X}{:02X}{:02X} bg=#{:02X}{:02X}{:02X} dir={} power={}",
            ls.mode,
            protocol::light_mode_name(ls.mode),
            ls.brightness,
            ls.speed,
            ls.fg[0],
            ls.fg[1],
            ls.fg[2],
            ls.bg[0],
            ls.bg[1],
            ls.bg[2],
            protocol::direction_name(ls.direction),
            if ls.power_on { "on" } else { "off" }
        );
    }
    if let Some(switches) = &p.switch_list {
        let desc: Vec<String> = switches
            .iter()
            .map(|&id| format!("{id}:{}", protocol::switch_profile_name(id)))
            .collect();
        println!("Switch profiles : [{}]", desc.join(", "));
    }
    if let Some(prcs) = p.prcs_enabled {
        println!(
            "PRCS / SOCD     : {}",
            if prcs { "enabled" } else { "disabled" }
        );
    }

    Ok(())
}

fn cmd_actuation(action: ActuationAction, policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    match action {
        ActuationAction::Get { keys } => {
            let indices = keymap::parse_key_selector(&keys)?;
            let mut backend = connect_backend(policy)?;
            let (unit_mm, entries) = backend.get_actuation(indices)?;
            for entry in entries {
                let label = Win60Driver::<HidApiTransport>::format_key(entry.index);
                if let Some(c) = entry.config {
                    println!(
                        "{:<16} mode={:<21} travel={:.2} mm  rt_press={:.2} mm  rt_release={:.2} mm",
                        label,
                        c.mode.label(),
                        f64::from(c.travel_raw) * unit_mm,
                        f64::from(c.interval1_raw) * unit_mm,
                        f64::from(c.interval2_raw) * unit_mm,
                    );
                } else {
                    println!("{:<16} <no reply>", label);
                }
            }
        }
        ActuationAction::Set {
            keys,
            travel,
            mode,
            rt_press,
            rt_release,
        } => {
            let indices = keymap::parse_key_selector(&keys)?;
            let trig_mode: TriggerMode = mode.into();
            let mut backend = connect_backend(policy)?;
            backend.set_actuation(indices.clone(), trig_mode, travel, rt_press, rt_release)?;
            println!(
                "Updated {} key(s): mode={} travel={:.2} mm",
                indices.len(),
                trig_mode.label(),
                travel
            );
        }
    }
    Ok(())
}

fn cmd_light(action: LightAction, policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    match action {
        LightAction::Get => {
            let mut backend = connect_backend(policy)?;
            let ls = backend.get_light()?;
            println!(
                "mode={} ({})  brightness={}  speed={}  fg=#{:02X}{:02X}{:02X}  bg=#{:02X}{:02X}{:02X}  dir={} ({})  full_color={}  power={}",
                ls.mode,
                protocol::light_mode_name(ls.mode),
                ls.brightness,
                ls.speed,
                ls.fg[0],
                ls.fg[1],
                ls.fg[2],
                ls.bg[0],
                ls.bg[1],
                ls.bg[2],
                ls.direction,
                protocol::direction_name(ls.direction),
                ls.full_color,
                if ls.power_on { "on" } else { "off" }
            );
        }
        LightAction::Set {
            mode,
            color,
            bg,
            brightness,
            speed,
            direction,
            full_color,
            off,
        } => {
            let mode_byte = protocol::parse_light_mode_name(&mode)
                .ok_or_else(|| format!("unknown lighting mode '{mode}'"))?;
            let state = LightState {
                mode: mode_byte,
                brightness: brightness.min(4),
                speed: speed.min(4),
                fg: parse_rgb(&color)?,
                bg: parse_rgb(&bg)?,
                direction,
                full_color,
                power_on: !off,
            };
            let mut backend = connect_backend(policy)?;
            backend.set_light(state)?;
            println!(
                "Applied lighting: mode={} ({}) bri={} spd={} fg=#{:02X}{:02X}{:02X}",
                state.mode,
                protocol::light_mode_name(state.mode),
                state.brightness,
                state.speed,
                state.fg[0],
                state.fg[1],
                state.fg[2]
            );
        }
    }
    Ok(())
}

fn cmd_switch(action: SwitchAction, policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    match action {
        SwitchAction::Get { keys } => {
            let indices = keymap::parse_key_selector(&keys)?;
            let mut backend = connect_backend(policy)?;
            for (idx, id) in backend.get_switches(indices)? {
                println!(
                    "{:<16} switch={} ({})",
                    Win60Driver::<HidApiTransport>::format_key(idx),
                    id,
                    protocol::switch_profile_name(id)
                );
            }
        }
        SwitchAction::Set { keys, profile } => {
            let id = protocol::parse_switch_id(&profile).ok_or_else(|| {
                format!("unknown switch profile '{profile}' (use hm1, hh1, cy1, tc1, or id)")
            })?;
            let indices = keymap::parse_key_selector(&keys)?;
            let mut backend = connect_backend(policy)?;
            backend.set_switches(indices.clone(), id)?;
            println!(
                "Updated switch profile on {} key(s) to {} ({})",
                indices.len(),
                id,
                protocol::switch_profile_name(id)
            );
        }
    }
    Ok(())
}

fn cmd_deadband(action: DeadbandAction, policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    match action {
        DeadbandAction::Get { keys } => {
            let indices = keymap::parse_key_selector(&keys)?;
            let mut backend = connect_backend(policy)?;
            for (idx, top_raw, bottom_raw) in backend.get_deadband(indices)? {
                println!(
                    "{:<16} top={:.2} mm  bottom={:.2} mm",
                    Win60Driver::<HidApiTransport>::format_key(idx),
                    f64::from(top_raw) * 0.01,
                    f64::from(bottom_raw) * 0.01
                );
            }
        }
        DeadbandAction::Set { keys, top, bottom } => {
            if !(0.0..=2.55).contains(&top) || !(0.0..=2.55).contains(&bottom) {
                return Err("deadband values must be in 0.00..=2.55 mm"
                    .to_string()
                    .into());
            }
            let top_raw = mm_to_raw(top, 0.01) as u8;
            let bottom_raw = mm_to_raw(bottom, 0.01) as u8;
            let indices = keymap::parse_key_selector(&keys)?;
            let mut backend = connect_backend(policy)?;
            backend.set_deadband(indices.clone(), top_raw, bottom_raw)?;
            println!(
                "Updated deadband on {} key(s): top={:.2} mm, bottom={:.2} mm",
                indices.len(),
                f64::from(top_raw) * 0.01,
                f64::from(bottom_raw) * 0.01
            );
        }
    }
    Ok(())
}

fn cmd_poll_rate(action: PollRateAction, policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    match action {
        PollRateAction::Get => {
            let mut backend = connect_backend(policy)?;
            let khz = backend.get_poll_rate()?;
            println!("Polling rate: {} kHz ({} Hz)", khz, u16::from(khz) * 1000);
        }
        PollRateAction::Set { khz } => {
            PollRate::from_khz(khz).map_err(|e| e.to_string())?;
            let mut backend = connect_backend(policy)?;
            backend.set_poll_rate(khz)?;
            println!(
                "Set polling rate to {} Hz (keyboard will restart)",
                u16::from(khz) * 1000
            );
        }
    }
    Ok(())
}

fn cmd_socd(action: SocdAction, policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    match action {
        SocdAction::Get => {
            let mut backend = connect_backend(policy)?;
            let (enabled, pairs) = backend.get_socd()?;
            println!(
                "PRCS / SOCD: {}",
                if enabled { "enabled" } else { "disabled" }
            );
            if pairs.is_empty() {
                println!("  (no pairs configured)");
            } else {
                for (i, p) in pairs.iter().enumerate() {
                    let k1 = keymap::WIN60HE_KEYS
                        .iter()
                        .find(|k| k.hid_code == p.key1_hid)
                        .map_or_else(|| format!("0x{:02X}", p.key1_hid), |k| k.name.to_string());
                    let k2 = keymap::WIN60HE_KEYS
                        .iter()
                        .find(|k| k.hid_code == p.key2_hid)
                        .map_or_else(|| format!("0x{:02X}", p.key2_hid), |k| k.name.to_string());
                    println!("  [{}] {} <-> {} (Model {})", i + 1, k1, k2, p.model + 1);
                }
            }
        }
        SocdAction::Set { on, off, pairs } => {
            let enabled = if off {
                false
            } else {
                on || !pairs.trim().is_empty()
            };
            let parsed = parse_socd_pairs(&pairs)?;
            let mut backend = connect_backend(policy)?;
            backend.set_socd(enabled, parsed.clone())?;
            println!(
                "Updated PRCS/SOCD: enabled={enabled}, {} pair(s)",
                parsed.len()
            );
        }
    }
    Ok(())
}

fn cmd_travel(
    keys: &str,
    duration: Option<u64>,
    policy: ConnectionPolicy<'_>,
) -> Result<(), CliError> {
    let indices = keymap::parse_key_selector(keys)?;
    let backend = connect_backend(policy)?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_handler = Arc::clone(&stop);
    ctrlc::set_handler(move || {
        stop_handler.store(true, Ordering::Relaxed);
    })
    .map_err(|e| format!("failed to install Ctrl+C handler: {e}"))?;

    println!(
        "Streaming live analog travel for {} key(s) via {} (press keys; Ctrl+C to stop)...",
        indices.len(),
        backend.source_label()
    );

    let dur = duration.map(Duration::from_secs);
    match backend {
        Backend::Daemon(client) => {
            let mut sub = client
                .subscribe_travel(indices)
                .map_err(|e| e.to_string())?;
            let start = std::time::Instant::now();
            while !stop.load(Ordering::Relaxed) {
                if let Some(limit) = dur
                    && start.elapsed() >= limit
                {
                    break;
                }
                match sub.next_sample() {
                    Ok(Some((sample, unit_mm))) => {
                        println!(
                            "{:<16} depth={:.2} mm (raw={})",
                            Win60Driver::<HidApiTransport>::format_key(sample.index),
                            sample.depth_mm(unit_mm),
                            sample.travel_raw
                        );
                    }
                    Ok(None) => {}
                    Err(e) => return Err(CliError::Message(e.to_string())),
                }
            }
            sub.close();
        }
        Backend::Direct(mut drv) => {
            let unit_mm = direct_unit_mm(&mut drv);
            drv.stream_travel(&indices, dur, stop, |sample| {
                println!(
                    "{:<16} depth={:.2} mm (raw={})",
                    Win60Driver::<HidApiTransport>::format_key(sample.index),
                    sample.depth_mm(unit_mm),
                    sample.travel_raw
                );
            })
            .map_err(|e| e.to_string())?;
        }
    }

    println!("Stopped travel stream.");
    Ok(())
}

fn cmd_keymap(
    fn_layer: bool,
    set_fn: Option<&str>,
    clear_fn: Option<&str>,
    reset_fn: bool,
    policy: ConnectionPolicy<'_>,
) -> Result<(), CliError> {
    let mut backend = connect_backend(policy)?;
    if reset_fn {
        let overrides = backend.reset_keyboard_fn_map()?;
        println!(
            "Reset keyboard Fn flash to factory defaults ({} override(s) remaining).",
            overrides.len()
        );
        return Ok(());
    }
    if set_fn.is_some() || clear_fn.is_some() {
        let set = set_fn
            .map(parse_fn_set_spec)
            .transpose()?
            .unwrap_or_default();
        let clear = clear_fn
            .map(keymap::parse_key_selector)
            .transpose()?
            .unwrap_or_default();
        let overrides = backend.apply_keyboard_fn_map(set, clear)?;
        println!(
            "Updated keyboard Fn flash ({} override(s) stored):",
            overrides.len()
        );
        for o in overrides {
            let src = keymap::by_index(o.index).map_or("Key", |k| k.name);
            let dst = keymap::usb_hid_name(o.target_hid);
            println!(
                "  Fn + {src} [slot {}] -> {dst} (HID 0x{:02X})",
                o.index, o.target_hid
            );
        }
        return Ok(());
    }

    let table = backend.get_keymap(fn_layer)?;
    println!(
        "{} layer keymap (non-zero entries; 00 00 00 00 = firmware default):",
        if fn_layer { "Fn" } else { "Base" }
    );
    for k in keymap::WIN60HE_KEYS {
        let off = (k.index as usize) * 4;
        let e = &table[off..off + 4];
        println!(
            "  {:<10} [slot {:>3}] -> {:02X} {:02X} {:02X} {:02X}",
            k.name, k.index, e[0], e[1], e[2], e[3]
        );
    }
    Ok(())
}

fn parse_fn_target_hid(raw: &str) -> Result<u8, String> {
    let token = raw.trim();
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        return u8::from_str_radix(hex, 16).map_err(|_| format!("invalid hex HID usage '{token}'"));
    }
    let norm = token.to_ascii_lowercase().replace([' ', '-', '_'], "");
    let vk = match norm.as_str() {
        "up" | "arrowup" => Some(VirtualKey::Up),
        "down" | "arrowdown" => Some(VirtualKey::Down),
        "left" | "arrowleft" => Some(VirtualKey::Left),
        "right" | "arrowright" => Some(VirtualKey::Right),
        "home" => Some(VirtualKey::Home),
        "end" => Some(VirtualKey::End),
        "pageup" | "pgup" => Some(VirtualKey::PageUp),
        "pagedown" | "pgdn" => Some(VirtualKey::PageDown),
        "insert" | "ins" => Some(VirtualKey::Insert),
        "delete" | "del" => Some(VirtualKey::Delete),
        "esc" | "escape" => Some(VirtualKey::Escape),
        "grave" | "`" | "~" => Some(VirtualKey::Grave),
        "prtsc" | "printscreen" | "sysrq" => Some(VirtualKey::PrintScreen),
        "f1" => Some(VirtualKey::F1),
        "f2" => Some(VirtualKey::F2),
        "f3" => Some(VirtualKey::F3),
        "f4" => Some(VirtualKey::F4),
        "f5" => Some(VirtualKey::F5),
        "f6" => Some(VirtualKey::F6),
        "f7" => Some(VirtualKey::F7),
        "f8" => Some(VirtualKey::F8),
        "f9" => Some(VirtualKey::F9),
        "f10" => Some(VirtualKey::F10),
        "f11" => Some(VirtualKey::F11),
        "f12" => Some(VirtualKey::F12),
        _ => None,
    };
    if let Some(v) = vk
        && let Some(hid) = keymap::virtual_key_to_hid_usage(v)
    {
        return Ok(hid);
    }
    if let Some(key) = keymap::resolve_key(token)
        && !key.is_fn()
    {
        return Ok(key.hid_code);
    }
    Err(format!(
        "unknown target key '{token}' (try Up, Down, Left, Right, Home, End, PgUp, PgDn, Del, F1..F12, or 0x52)"
    ))
}

fn parse_fn_set_spec(spec: &str) -> Result<Vec<FnOverride>, String> {
    let mut out = Vec::new();
    for item in spec.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (src, dst) = trimmed
            .split_once('=')
            .ok_or_else(|| format!("expected KEY=TARGET in '{trimmed}' (e.g. R=Up)"))?;
        let key = keymap::resolve_key(src.trim())
            .ok_or_else(|| format!("unknown source key '{}'", src.trim()))?;
        let target_hid = parse_fn_target_hid(dst)?;
        out.push(FnOverride {
            index: key.index,
            target_hid,
        });
    }
    if out.is_empty() {
        return Err("empty --set-fn specification".into());
    }
    Ok(out)
}

fn cmd_raw(parts: &[String], policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    let joined: String = parts.concat();
    let hex: String = joined.chars().filter(|c| !c.is_whitespace()).collect();
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return Err("provide an even number of hex characters"
            .to_string()
            .into());
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| format!("invalid hex byte '{}'", &hex[i..i + 2]))
        })
        .collect::<Result<Vec<u8>, String>>()?;
    if bytes.len() > BODY_LEN {
        return Err(format!("payload exceeds {BODY_LEN} bytes (got {})", bytes.len()).into());
    }

    let mut body = [0u8; BODY_LEN];
    body[..bytes.len()].copy_from_slice(&bytes);

    let mut backend = connect_backend(policy)?;
    let reply = backend.raw(body)?;
    println!("TX: {}", format_hex(&protocol::wrap_report(&body)));
    match reply {
        Some(rx) => println!("RX: {}", format_hex(&rx)),
        None => println!("RX: <no reply within 200 ms>"),
    }
    Ok(())
}

fn parse_socd_pairs(spec: &str) -> Result<Vec<PrcsPair>, String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for item in trimmed.split(',') {
        let parts: Vec<&str> = item.trim().split(':').collect();
        if parts.len() != 2 && parts.len() != 3 {
            return Err(format!(
                "invalid SOCD pair '{item}' (expected KEY1:KEY2 or KEY1:KEY2:MODEL1..4)"
            ));
        }
        let k1 =
            keymap::resolve_key(parts[0]).ok_or_else(|| format!("unknown key '{}'", parts[0]))?;
        let k2 =
            keymap::resolve_key(parts[1]).ok_or_else(|| format!("unknown key '{}'", parts[1]))?;
        let model_1based = if parts.len() == 3 {
            parts[2]
                .trim()
                .parse::<u8>()
                .map_err(|_| format!("invalid SOCD model '{}'", parts[2]))?
        } else {
            2
        };
        if !(1..=4).contains(&model_1based) {
            return Err(format!("SOCD model {model_1based} must be 1..=4"));
        }
        out.push(PrcsPair {
            model: model_1based - 1,
            key1_hid: k1.hid_code,
            key2_hid: k2.hid_code,
        });
    }
    Ok(out)
}

fn parse_rgb(s: &str) -> Result<[u8; 3], String> {
    let trimmed = s.trim();
    if let Some(hex) = trimmed
        .strip_prefix('#')
        .or_else(|| (trimmed.len() == 6 && !trimmed.contains(',')).then_some(trimmed))
    {
        if hex.len() != 6 {
            return Err(format!("invalid hex color '{s}'"));
        }
        let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| format!("bad hex '{s}'"))?;
        let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| format!("bad hex '{s}'"))?;
        let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| format!("bad hex '{s}'"))?;
        return Ok([r, g, b]);
    }
    let parts: Vec<&str> = trimmed.split(',').collect();
    if parts.len() != 3 {
        return Err(format!("invalid RGB color '{s}' (use R,G,B or #RRGGBB)"));
    }
    let r = parts[0]
        .trim()
        .parse::<u8>()
        .map_err(|_| format!("bad red '{s}'"))?;
    let g = parts[1]
        .trim()
        .parse::<u8>()
        .map_err(|_| format!("bad green '{s}'"))?;
    let b = parts[2]
        .trim()
        .parse::<u8>()
        .map_err(|_| format!("bad blue '{s}'"))?;
    Ok([r, g, b])
}

fn cmd_doctor(policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    let report = if !policy.force_direct {
        match DaemonClient::connect(policy.socket) {
            Ok(mut client) => match client.run_diagnostics() {
                Ok(rep) => rep,
                Err(err) => {
                    let mut checks = diagnostics::run_kernel_checks();
                    checks.push(DiagnosticCheck {
                        layer: DiagnosticLayer::Daemon,
                        name: "Daemon RPC diagnostics".into(),
                        status: CheckStatus::Fail,
                        latency_ms: None,
                        detail: err.to_string(),
                        remedy: Some(
                            "Restart reaulad (`reaula daemon stop && reaula daemon start`).".into(),
                        ),
                    });
                    checks.extend(diagnostics::run_software_checks());
                    DiagnosticReport {
                        generated_at_unix_ms: diagnostics::now_unix_ms(),
                        attribution: diagnostics::compute_attribution(&checks),
                        checks,
                    }
                }
            },
            Err(err) if policy.require_daemon => {
                let mut checks = diagnostics::run_kernel_checks();
                checks.push(DiagnosticCheck {
                    layer: DiagnosticLayer::Daemon,
                    name: "Daemon socket connection".into(),
                    status: CheckStatus::Fail,
                    latency_ms: None,
                    detail: err.to_string(),
                    remedy: Some("Start reaulad with `reaula daemon start`.".into()),
                });
                checks.extend(diagnostics::run_software_checks());
                DiagnosticReport {
                    generated_at_unix_ms: diagnostics::now_unix_ms(),
                    attribution: diagnostics::compute_attribution(&checks),
                    checks,
                }
            }
            Err(err) => build_local_diagnostic_report(&format!("reaulad offline ({err})")),
        }
    } else {
        build_local_diagnostic_report("--direct mode requested; bypassed daemon IPC")
    };

    print_diagnostic_report(&report);
    Ok(())
}

fn build_local_diagnostic_report(daemon_note: &str) -> DiagnosticReport {
    let mut checks = diagnostics::run_kernel_checks();
    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Daemon,
        name: "Daemon IPC".into(),
        status: CheckStatus::Warn,
        latency_ms: None,
        detail: daemon_note.to_string(),
        remedy: Some(
            "Start reaulad (`reaula daemon start`) for shared non-root GUI/CLI access.".into(),
        ),
    });

    match HidApiTransport::open_win60() {
        Ok(t) => {
            let mut drv = Win60Driver::new(t);
            checks.extend(diagnostics::run_keyboard_checks(&mut drv));
        }
        Err(e) => {
            checks.push(DiagnosticCheck {
                layer: DiagnosticLayer::Keyboard,
                name: "Direct HID transport".into(),
                status: CheckStatus::Fail,
                latency_ms: None,
                detail: e.to_string(),
                remedy: Some(
                    "Run via `reaula daemon start` (or install udev rules) to access /dev/hidraw*."
                        .into(),
                ),
            });
        }
    }

    checks.extend(diagnostics::run_software_checks());
    let attribution = diagnostics::compute_attribution(&checks);
    DiagnosticReport {
        generated_at_unix_ms: diagnostics::now_unix_ms(),
        checks,
        attribution,
    }
}

fn print_diagnostic_report(report: &DiagnosticReport) {
    println!("=== Reaula 4-Layer Diagnostics ===");
    for layer in [
        DiagnosticLayer::Kernel,
        DiagnosticLayer::Daemon,
        DiagnosticLayer::Keyboard,
        DiagnosticLayer::Software,
    ] {
        println!("\n[{}]", layer.label());
        let layer_checks: Vec<_> = report.checks.iter().filter(|c| c.layer == layer).collect();
        if layer_checks.is_empty() {
            println!("  (no checks reported)");
            continue;
        }
        for c in layer_checks {
            let lat = c
                .latency_ms
                .map_or_else(String::new, |ms| format!(" ({ms:.2} ms)"));
            println!(
                "  [{:<4}] {}{}: {}",
                c.status.label(),
                c.name,
                lat,
                c.detail
            );
            if let Some(rem) = &c.remedy {
                println!("         -> Remedy: {rem}");
            }
        }
    }
    println!("\nAttribution: {}", report.attribution.summary);
}

enum AudioInputStream {
    #[cfg(target_os = "linux")]
    Alsa(AlsaCapture),
    Stdin {
        bytes: Vec<u8>,
        eof: bool,
    },
    Demo(DemoAudioSynth),
}

impl AudioInputStream {
    fn label(&self) -> String {
        match self {
            #[cfg(target_os = "linux")]
            Self::Alsa(cap) => format!(
                "ALSA '{}' ({} Hz, {} ch)",
                cap.pcm_name(),
                cap.sample_rate(),
                cap.channels()
            ),
            Self::Stdin { .. } => "stdin raw s16le mono PCM".into(),
            Self::Demo(_) => "synthetic 120 BPM demo loop".into(),
        }
    }

    fn read_chunk(&mut self, buf: &mut [f32]) -> Result<Option<usize>, String> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Alsa(cap) => cap.read_mono_f32(buf).map(Some),
            Self::Stdin { bytes, eof } => {
                if *eof {
                    return Ok(None);
                }
                let want = buf.len() * 2;
                if bytes.len() != want {
                    bytes.resize(want, 0);
                }
                let mut total = 0usize;
                let mut stdin = io::stdin().lock();
                while total < want {
                    match stdin.read(&mut bytes[total..]) {
                        Ok(0) => {
                            *eof = true;
                            break;
                        }
                        Ok(n) => total += n,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(format!("stdin PCM read failed: {e}")),
                    }
                }
                let samples = total / 2;
                if samples == 0 {
                    return Ok(None);
                }
                for (i, dst) in buf[..samples].iter_mut().enumerate() {
                    let s = i16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
                    *dst = (f32::from(s) / 32768.0).clamp(-1.0, 1.0);
                }
                Ok(Some(samples))
            }
            Self::Demo(synth) => {
                synth.generate(buf);
                std::thread::sleep(Duration::from_millis(10));
                Ok(Some(buf.len()))
            }
        }
    }
}

fn open_audio_input(
    source: AudioSourceArg,
    device: Option<&str>,
    rate: u32,
) -> Result<AudioInputStream, String> {
    match source {
        AudioSourceArg::Stdin => Ok(AudioInputStream::Stdin {
            bytes: Vec::new(),
            eof: false,
        }),
        AudioSourceArg::Demo => Ok(AudioInputStream::Demo(DemoAudioSynth::new(rate))),
        AudioSourceArg::Monitor | AudioSourceArg::Mic | AudioSourceArg::Device => {
            #[cfg(target_os = "linux")]
            {
                let mode =
                    match source {
                        AudioSourceArg::Monitor => AlsaSourceMode::Monitor,
                        AudioSourceArg::Mic => AlsaSourceMode::DefaultInput,
                        AudioSourceArg::Device => {
                            let dev = device.map(str::trim).filter(|s| !s.is_empty()).ok_or_else(
                                || {
                                    "--device <PCM> is required when --source device is selected"
                                        .to_string()
                                },
                            )?;
                            AlsaSourceMode::Device(dev.to_string())
                        }
                        _ => unreachable!(),
                    };
                if matches!(mode, AlsaSourceMode::Monitor) {
                    // SAFETY: Called cleanly on the main thread before spawning signal/worker threads.
                    unsafe {
                        alsa::prepare_monitor_env();
                    }
                }
                let latency_us = 35_000;
                let channels = 1;
                let cap = AlsaCapture::open(&mode, rate, channels, latency_us)?;
                Ok(AudioInputStream::Alsa(cap))
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = device;
                Err("ALSA audio capture is only supported on Linux".into())
            }
        }
    }
}

fn build_audio_sync_config(cfg: &AudioSyncArgs) -> Result<AudioSyncConfig, CliError> {
    let source = match cfg.source {
        AudioSourceArg::Monitor => AudioSyncSource::Monitor,
        AudioSourceArg::Mic => AudioSyncSource::DefaultMic,
        AudioSourceArg::Demo => AudioSyncSource::Demo,
        AudioSourceArg::Device => {
            let dev = cfg
                .device
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    CliError::Message(
                        "--device <PCM> is required when --source device is selected".into(),
                    )
                })?;
            AudioSyncSource::Device {
                pcm: dev.to_string(),
                label: dev.to_string(),
            }
        }
        AudioSourceArg::Stdin => {
            return Err(CliError::Message(
                "--source stdin streams from this terminal process; use `reaula audio run --source stdin`".into(),
            ));
        }
    };

    Ok(AudioSyncConfig {
        source,
        style: cfg.effect.into(),
        palette: cfg.palette.into(),
        fg: parse_rgb(&cfg.fg)?,
        bg: parse_rgb(&cfg.bg)?,
        brightness: cfg.brightness.min(4),
        fps: cfg.fps.clamp(5, 60),
        gain: cfg.gain.clamp(0.1, 10.0),
    })
}

fn friendly_audio_ipc_err(err: ipc::IpcError) -> CliError {
    let msg = err.to_string();
    if msg.contains("unknown variant") {
        CliError::Message(
            "Running reaulad predates daemon-hosted Audio Sync; restart reaulad (`reaula daemon stop && reaula daemon start`).".into(),
        )
    } else {
        CliError::Message(msg)
    }
}

fn print_audio_status(st: &AudioSyncStatus) {
    println!(
        "Audio Equalizer : {}",
        if st.active { "ACTIVE (running in reaulad)" } else { "IDLE" }
    );
    println!("Input Source    : {}", st.source_label);
    println!(
        "Effect / Palette: {} / {}",
        st.config.style.label(),
        st.config.palette.label()
    );
    println!(
        "Tuning          : brightness={}/4 target_fps={} gain={:.2}x",
        st.config.brightness, st.config.fps, st.config.gain
    );
    if st.active || st.frames_rendered > 0 {
        println!(
            "Telemetry       : {:.1} fps | {} frames | {} HID packets (last={})",
            st.actual_fps, st.frames_rendered, st.packets_total, st.packets_last
        );
    }
    if st.active {
        println!("Spectrum Meter  : {}", audio_fx::render_ascii_meter(&st.metrics));
    }
    if let Some(err) = &st.last_error {
        println!("Last Error      : {err}");
    }
}

fn install_ctrlc_flag() -> Result<Arc<AtomicBool>, CliError> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_handler = Arc::clone(&stop);
    ctrlc::set_handler(move || {
        stop_handler.store(true, Ordering::Relaxed);
    })
    .map_err(|e| CliError::Message(format!("failed to install Ctrl+C handler: {e}")))?;
    Ok(stop)
}

fn watch_daemon_audio(
    client: &mut DaemonClient,
    duration: Option<u64>,
    stop_on_exit: bool,
    restore_on_stop: bool,
) -> Result<(), CliError> {
    let stop = install_ctrlc_flag()?;
    let start = Instant::now();
    let is_tty = io::stdout().is_terminal();
    let mut last_line_at = Instant::now() - Duration::from_secs(1);
    let mut exit_err: Option<String> = None;

    while !stop.load(Ordering::Relaxed) {
        if let Some(sec) = duration
            && start.elapsed() >= Duration::from_secs(sec)
        {
            break;
        }

        let st = client.get_audio_sync_state().map_err(friendly_audio_ipc_err)?;
        if !st.active {
            exit_err = st.last_error;
            if is_tty {
                println!();
            }
            println!("Daemon Audio Equalizer stopped.");
            break;
        }

        let now = Instant::now();
        let meter = audio_fx::render_ascii_meter(&st.metrics);
        if is_tty && last_line_at.elapsed() >= Duration::from_millis(50) {
            print!(
                "\r{meter} | {:.1} fps | frame={:>4} ({} HID pkts)   ",
                st.actual_fps, st.frames_rendered, st.packets_total
            );
            let _ = io::stdout().flush();
            last_line_at = now;
        } else if !is_tty && last_line_at.elapsed() >= Duration::from_secs(1) {
            println!(
                "{meter} | {:.1} fps | frames={} hid_pkts={}",
                st.actual_fps, st.frames_rendered, st.packets_total
            );
            last_line_at = now;
        }

        std::thread::sleep(Duration::from_millis(50));
    }

    if is_tty {
        println!();
    }

    if stop_on_exit {
        let _ = client.stop_audio_sync(restore_on_stop);
        if restore_on_stop {
            println!("Stopped daemon Audio Equalizer and restored previous keyboard lighting.");
        } else {
            println!("Stopped daemon Audio Equalizer.");
        }
    }

    if let Some(err) = exit_err {
        return Err(CliError::Message(err));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_audio_local(
    cfg: &AudioSyncArgs,
    rate: u32,
    duration: Option<u64>,
    dry_run: bool,
    no_restore: bool,
    policy: ConnectionPolicy<'_>,
) -> Result<(), CliError> {
    let fps_clamped = u64::from(cfg.fps.clamp(5, 60));
    let fg_rgb = parse_rgb(&cfg.fg)?;
    let bg_rgb = parse_rgb(&cfg.bg)?;
    let style: AudioEffectStyle = cfg.effect.into();
    let pal: AudioPalette = cfg.palette.into();

    // Open ALSA before spawning the Ctrl+C handler thread so any monitor env prep
    // happens strictly single-threaded.
    let mut input = open_audio_input(cfg.source, cfg.device.as_deref(), rate)?;
    let actual_rate = match &input {
        #[cfg(target_os = "linux")]
        AudioInputStream::Alsa(cap) => cap.sample_rate(),
        _ => rate.max(8000),
    };

    let mut backend = if dry_run {
        None
    } else {
        Some(connect_backend(policy)?)
    };

    let saved_light = if let (Some(b), false) = (backend.as_mut(), no_restore) {
        let ls = b.get_light().ok();
        let custom = b.get_custom_rgb(0).ok();
        Some((ls, custom))
    } else {
        None
    };

    if let Some(b) = backend.as_mut() {
        b.begin_custom_stream(cfg.brightness.min(4))?;
    }

    let stop = install_ctrlc_flag()?;

    let target_mode = if dry_run {
        "dry-run telemetry only".to_string()
    } else if let Some(b) = backend.as_ref() {
        format!("WIN 60 HE via {} (cmd 0x09 stream)", b.source_label())
    } else {
        unreachable!()
    };
    println!(
        "Audio-reactive experiment started (local process):\n  Input   : {}\n  Effect  : {} ({} Hz FFT -> {} fps target)\n  Output  : {}\nPress Ctrl+C to stop cleanly.",
        input.label(),
        style.label(),
        actual_rate,
        fps_clamped,
        target_mode
    );

    let mut vis = AudioVisualizer::new(actual_rate, style, pal, fg_rgb, bg_rgb, cfg.gain);
    let hop = (actual_rate as usize / 100).clamp(240, 1024);
    let mut pcm_buf = vec![0.0f32; hop];
    let frame_period = Duration::from_secs_f64(1.0 / (fps_clamped as f64));
    let start = Instant::now();
    let mut last_step = Instant::now();
    let mut next_frame_at = Instant::now();
    let mut last_status_print = Instant::now();
    let mut frames_rendered = 0u64;
    let mut packets_total = 0u64;
    let is_tty = io::stdout().is_terminal();

    let mut loop_err: Option<String> = None;
    while !stop.load(Ordering::Relaxed) {
        if let Some(sec) = duration
            && start.elapsed() >= Duration::from_secs(sec)
        {
            break;
        }

        match input.read_chunk(&mut pcm_buf) {
            Ok(Some(n)) => vis.push_samples(&pcm_buf[..n]),
            Ok(None) => break,
            Err(e) => {
                loop_err = Some(e);
                break;
            }
        }

        let now = Instant::now();
        if now >= next_frame_at {
            let dt = now.duration_since(last_step).as_secs_f32().max(0.001);
            last_step = now;
            next_frame_at = (next_frame_at + frame_period).max(now);

            let (metrics, grid) = vis.step(dt);
            let mut sent_this_frame = 0usize;
            if let Some(b) = backend.as_mut() {
                match b.stream_custom_rgb(0, grid.to_vec(), frames_rendered == 0) {
                    Ok(n) => {
                        sent_this_frame = n;
                        packets_total += n as u64;
                    }
                    Err(e) => {
                        loop_err = Some(e);
                        break;
                    }
                }
            }
            frames_rendered += 1;

            let meter = audio_fx::render_ascii_meter(&metrics);
            if is_tty && last_status_print.elapsed() >= Duration::from_millis(50) {
                print!(
                    "\r{meter} | frame={frames_rendered:>4} pkts+={sent_this_frame} ({packets_total} total)   "
                );
                let _ = io::stdout().flush();
                last_status_print = now;
            } else if !is_tty && last_status_print.elapsed() >= Duration::from_secs(1) {
                println!("{meter} | frames={frames_rendered} hid_pkts={packets_total}");
                last_status_print = now;
            }
        }
    }

    if is_tty {
        println!();
    }

    if let (Some(b), Some((prev_light, prev_custom))) = (backend.as_mut(), saved_light) {
        if let Some(colors) = prev_custom {
            let _ = b.set_custom_rgb(0, colors, cfg.brightness.min(4));
        }
        if let Some(ls) = prev_light {
            let _ = b.set_light(ls);
        }
        println!("Restored previous keyboard lighting state.");
    }

    if let Some(err) = loop_err {
        return Err(CliError::Message(err));
    }
    println!(
        "Audio session finished ({frames_rendered} frames, {packets_total} HID packets written in {:.2}s).",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn cmd_audio(action: AudioAction, policy: ConnectionPolicy<'_>) -> Result<(), CliError> {
    match action {
        AudioAction::Devices => {
            #[cfg(target_os = "linux")]
            {
                if let Ok(out) = ProcessCommand::new("pactl")
                    .arg("get-default-sink")
                    .output()
                    && out.status.success()
                {
                    let sink = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !sink.is_empty() {
                        println!(
                            "PipeWire/Pulse monitor : {sink}.monitor (used by --source monitor)"
                        );
                    }
                }
                let devices = alsa::list_capture_devices()?;
                println!("ALSA capture PCMs ({}):", devices.len());
                for dev in devices {
                    println!("  {:<28} [{}] {}", dev.name, dev.ioid, dev.description);
                }
                Ok(())
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err(CliError::Message(
                    "ALSA device enumeration is only supported on Linux".into(),
                ))
            }
        }
        AudioAction::Start { cfg } => {
            if policy.force_direct {
                return Err(CliError::Message(
                    "`audio start` runs in reaulad; remove --direct or use `audio run --direct`".into(),
                ));
            }
            let sync_cfg = build_audio_sync_config(&cfg)?;
            let mut client = DaemonClient::connect(policy.socket).map_err(|e| {
                CliError::Message(format!(
                    "could not connect to reaulad ({e}); start it with `reaula daemon start`"
                ))
            })?;
            let st = client.start_audio_sync(sync_cfg).map_err(friendly_audio_ipc_err)?;
            print_audio_status(&st);
            Ok(())
        }
        AudioAction::Stop { no_restore } => {
            let mut client = DaemonClient::connect(policy.socket).map_err(|e| e.to_string())?;
            let st = client
                .stop_audio_sync(!no_restore)
                .map_err(friendly_audio_ipc_err)?;
            println!(
                "Stopped daemon Audio Equalizer{}.",
                if no_restore {
                    ""
                } else {
                    " (restored previous keyboard lighting)"
                }
            );
            print_audio_status(&st);
            Ok(())
        }
        AudioAction::Status { watch } => {
            let mut client = DaemonClient::connect(policy.socket).map_err(|e| {
                CliError::Message(format!(
                    "could not connect to reaulad ({e}); start it with `reaula daemon start`"
                ))
            })?;
            let st = client.get_audio_sync_state().map_err(friendly_audio_ipc_err)?;
            print_audio_status(&st);
            if watch && st.active {
                println!("Watching daemon telemetry (Ctrl+C detaches without stopping keyboard RGB)...");
                watch_daemon_audio(&mut client, None, false, false)?;
            }
            Ok(())
        }
        AudioAction::Run {
            cfg,
            rate,
            duration,
            dry_run,
            no_restore,
            detach,
        } => {
            let use_local =
                dry_run || policy.force_direct || matches!(cfg.source, AudioSourceArg::Stdin);
            if !use_local {
                match DaemonClient::connect(policy.socket) {
                    Ok(mut client) => {
                        let sync_cfg = build_audio_sync_config(&cfg)?;
                        let st = client
                            .start_audio_sync(sync_cfg)
                            .map_err(friendly_audio_ipc_err)?;
                        print_audio_status(&st);
                        if detach {
                            println!("Detached: equalizer continues running inside reaulad.");
                            return Ok(());
                        }
                        println!("Streaming inside reaulad (Ctrl+C stops and restores lighting)...");
                        return watch_daemon_audio(&mut client, duration, true, !no_restore);
                    }
                    Err(err) if policy.require_daemon => {
                        return Err(CliError::Message(format!(
                            "could not connect to reaulad ({err}); start it with `reaula daemon start`"
                        )));
                    }
                    Err(_) => {}
                }
            } else if detach {
                return Err(CliError::Message(
                    "--detach requires running through reaulad (incompatible with --dry-run/--direct/--source stdin)".into(),
                ));
            }
            run_audio_local(&cfg, rate, duration, dry_run, no_restore, policy)
        }
    }
}

fn format_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rgb_and_socd_pairs() {
        assert_eq!(parse_rgb("#00ffaa").unwrap(), [0, 255, 170]);
        let pairs = parse_socd_pairs("A:D:2").unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].model, 1);
        assert_eq!(pairs[0].key1_hid, 0x04);
        assert_eq!(pairs[0].key2_hid, 0x07);
    }

    #[test]
    fn parses_daemon_and_sudo_flags() {
        let cli = Cli::try_parse_from(["reaula", "--daemon", "probe"]).unwrap();
        assert!(cli.daemon);
        assert!(!cli.direct);

        let conflict = Cli::try_parse_from(["reaula", "--daemon", "--direct", "probe"]);
        assert!(conflict.is_err());

        let doc = Cli::try_parse_from(["reaula", "doctor"]).unwrap();
        assert!(matches!(doc.command, Command::Doctor));
    }

    #[test]
    fn parses_fn_set_spec_arrows_and_hex() {
        let set = parse_fn_set_spec("R=Up,D=Left,F=Down,G=Right,Capslock=0x29").unwrap();
        assert_eq!(set.len(), 5);
        assert_eq!(
            set[0],
            FnOverride {
                index: keymap::idx::R,
                target_hid: 0x52
            }
        );
        assert_eq!(
            set[1],
            FnOverride {
                index: keymap::idx::D,
                target_hid: 0x50
            }
        );
        assert_eq!(
            set[2],
            FnOverride {
                index: keymap::idx::F,
                target_hid: 0x51
            }
        );
        assert_eq!(
            set[3],
            FnOverride {
                index: keymap::idx::G,
                target_hid: 0x4F
            }
        );
        assert_eq!(
            set[4],
            FnOverride {
                index: keymap::idx::CAPS,
                target_hid: 0x29
            }
        );
    }

    #[test]
    fn parses_audio_subcommands_and_flags() {
        let dev = Cli::try_parse_from(["reaula", "audio", "devices"]).unwrap();
        assert!(matches!(
            dev.command,
            Command::Audio {
                action: AudioAction::Devices
            }
        ));

        let start = Cli::try_parse_from([
            "reaula",
            "audio",
            "start",
            "--source",
            "demo",
            "--effect",
            "wave",
            "--palette",
            "neon",
        ])
        .unwrap();
        assert!(matches!(
            start.command,
            Command::Audio {
                action: AudioAction::Start { .. }
            }
        ));

        let stop = Cli::try_parse_from(["reaula", "audio", "stop", "--no-restore"]).unwrap();
        assert!(matches!(
            stop.command,
            Command::Audio {
                action: AudioAction::Stop { no_restore: true }
            }
        ));

        let status = Cli::try_parse_from(["reaula", "audio", "status", "--watch"]).unwrap();
        assert!(matches!(
            status.command,
            Command::Audio {
                action: AudioAction::Status { watch: true }
            }
        ));

        let run = Cli::try_parse_from([
            "reaula",
            "audio",
            "run",
            "--source",
            "demo",
            "--effect",
            "pulse",
            "--palette",
            "regal",
            "--fps",
            "45",
            "--dry-run",
        ])
        .unwrap();
        match run.command {
            Command::Audio {
                action:
                    AudioAction::Run {
                        cfg,
                        dry_run: true,
                        ..
                    },
            } => assert_eq!(cfg.fps, 45),
            _ => panic!("expected AudioAction::Run"),
        }
    }
}
