//! Daemon-owned real-time ALSA/Demo audio capture, 1024-pt FFT visualizer, and status snapshot.
//!
//! When `reaulad` runs as `root`, ALSA capture for desktop PipeWire/PulseAudio monitors is
//! executed inside a privilege-dropped self child (`reaulad --capture-pcm`) running under the
//! authorized local UID with `XDG_RUNTIME_DIR` and `HOME` configured, streaming mono `i16le` PCM
//! frames into this engine over an anonymous pipe.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use reaula_core::alsa::{self, AlsaCapture, AlsaSourceMode};
use reaula_core::audio_fx::{
    AudioFrameMetrics, AudioVisualizer, DEFAULT_SAMPLE_RATE, DemoAudioSynth,
};
use reaula_core::ipc::{self, AudioSyncConfig, AudioSyncSource, AudioSyncStatus};
use reaula_core::keyboards::DriverError;
use reaula_core::protocol::{LIGHT_MAX_LEVEL, LightState, MATRIX_SLOTS};

use crate::keyboards::AulaDriver;

struct SharedTelemetry {
    source_label: String,
    metrics: AudioFrameMetrics,
    frame: [[u8; 3]; MATRIX_SLOTS],
    frame_seq: u64,
    actual_fps: f32,
    fatal_error: Option<String>,
}

impl Default for SharedTelemetry {
    fn default() -> Self {
        Self {
            source_label: "Idle".into(),
            metrics: AudioFrameMetrics::default(),
            frame: [[0u8; 3]; MATRIX_SLOTS],
            frame_seq: 0,
            actual_fps: 0.0,
            fatal_error: None,
        }
    }
}

struct AudioWorkerHandle {
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    join: Option<JoinHandle<()>>,
}

impl Drop for AudioWorkerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut guard) = self.child.lock()
            && let Some(mut ch) = guard.take()
        {
            let _ = ch.kill();
            let _ = ch.wait();
        }
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

pub(crate) struct DaemonAudioEngine {
    allowed_uids: Vec<u32>,
    active: bool,
    config: AudioSyncConfig,
    saved_light: Option<LightState>,
    saved_custom0: Option<[[u8; 3]; MATRIX_SLOTS]>,
    shared_cfg: Arc<Mutex<AudioSyncConfig>>,
    telemetry: Arc<Mutex<SharedTelemetry>>,
    frames_rendered: u64,
    packets_total: u64,
    packets_last: usize,
    last_sent_seq: u64,
    worker: Option<AudioWorkerHandle>,
}

impl DaemonAudioEngine {
    pub(crate) fn new(allowed_uids: Vec<u32>) -> Self {
        let initial = AudioSyncConfig::default();
        Self {
            allowed_uids,
            active: false,
            shared_cfg: Arc::new(Mutex::new(initial.clone())),
            config: initial,
            saved_light: None,
            saved_custom0: None,
            telemetry: Arc::new(Mutex::new(SharedTelemetry::default())),
            frames_rendered: 0,
            packets_total: 0,
            packets_last: 0,
            last_sent_seq: 0,
            worker: None,
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn status(&self) -> AudioSyncStatus {
        let telem = self.telemetry.lock().map(|g| g.clone());
        match telem {
            Ok(t) => AudioSyncStatus {
                active: self.active,
                config: self.config.clone(),
                source_label: t.source_label,
                metrics: t.metrics,
                frame: if self.active { t.frame.to_vec() } else { Vec::new() },
                frames_rendered: self.frames_rendered,
                packets_total: self.packets_total,
                packets_last: self.packets_last,
                actual_fps: t.actual_fps,
                last_error: t.fatal_error,
            },
            Err(_) => AudioSyncStatus::default(),
        }
    }

    pub(crate) fn start(
        &mut self,
        mut config: AudioSyncConfig,
        drv: &mut dyn AulaDriver,
    ) -> Result<AudioSyncStatus, String> {
        sanitize_config(&mut config);

        // Preserve original keyboard state across in-session restarts / source changes.
        if !self.active && self.saved_light.is_none() && self.saved_custom0.is_none() {
            self.saved_light = drv.read_light().ok();
            self.saved_custom0 = drv.read_custom_light(0).ok();
        }

        if let Err(e) = drv.begin_custom_stream(config.brightness) {
            self.saved_light = None;
            self.saved_custom0 = None;
            return Err(format!("failed to enter Custom RGB stream: {e}"));
        }

        self.spawn_worker(&config)?;
        self.config = config;
        self.active = true;
        self.frames_rendered = 0;
        self.packets_total = 0;
        self.packets_last = 0;
        self.last_sent_seq = 0;
        Ok(self.status())
    }

    pub(crate) fn update(
        &mut self,
        mut config: AudioSyncConfig,
        drv: Option<&mut (dyn AulaDriver + 'static)>,
    ) -> Result<AudioSyncStatus, String> {
        sanitize_config(&mut config);
        let source_changed = config.source != self.config.source;
        let bri_changed = config.brightness != self.config.brightness;

        if self.active && bri_changed {
            let Some(d) = drv else {
                return Err("keyboard not connected".into());
            };
            d.begin_custom_stream(config.brightness)
                .map_err(|e| format!("failed to update stream brightness: {e}"))?;
        }

        if let Ok(mut g) = self.shared_cfg.lock() {
            *g = config.clone();
        }
        self.config = config.clone();

        if self.active && source_changed {
            self.spawn_worker(&config)?;
            self.last_sent_seq = 0;
        }

        Ok(self.status())
    }

    pub(crate) fn stop(
        &mut self,
        restore_lighting: bool,
        drv: Option<&mut (dyn AulaDriver + 'static)>,
    ) -> AudioSyncStatus {
        self.worker.take();
        self.active = false;
        self.packets_last = 0;

        if let Ok(mut t) = self.telemetry.lock() {
            t.actual_fps = 0.0;
            if t.fatal_error.is_none() {
                t.source_label = "Stopped".into();
            }
        }

        if restore_lighting && let Some(d) = drv {
            if let Some(custom) = self.saved_custom0.take() {
                let _ = d.set_custom_light(0, &custom, self.config.brightness);
            }
            if let Some(ls) = self.saved_light.take() {
                let _ = d.set_light(&ls);
            }
        } else {
            self.saved_custom0 = None;
            self.saved_light = None;
        }

        self.status()
    }

    /// Push newly analyzed audio RGB frames onto the single-owner HID driver.
    ///
    /// Returns `Some(DriverError)` if a transport error occurred so `HardwareState` can drop HID cleanly.
    pub(crate) fn tick_hid(
        &mut self,
        drv: &mut dyn AulaDriver,
        travel_active: bool,
        indicator_overlay: Option<&[[u8; 3]; MATRIX_SLOTS]>,
    ) -> Result<(), DriverError> {
        if !self.active {
            return Ok(());
        }

        let maybe_frame = {
            let Ok(t) = self.telemetry.lock() else {
                return Ok(());
            };
            if t.fatal_error.is_some() {
                None
            } else if t.frame_seq > self.last_sent_seq || indicator_overlay.is_some() {
                Some((t.frame_seq, t.frame))
            } else {
                None
            }
        };

        let fatal = self
            .telemetry
            .lock()
            .ok()
            .and_then(|t| t.fatal_error.clone());
        if fatal.is_some() {
            self.worker.take();
            self.active = false;
            return Ok(());
        }

        let Some((seq, mut grid)) = maybe_frame else {
            return Ok(());
        };

        if let Some(overlay) = indicator_overlay {
            for (dst, &ov) in grid.iter_mut().zip(overlay.iter()) {
                if ov != [0, 0, 0] {
                    *dst = ov;
                }
            }
        }

        let new_audio_frame = seq > self.last_sent_seq;
        match drv.stream_custom_light(0, &grid, self.frames_rendered == 0, !travel_active) {
            Ok(sent) => {
                self.last_sent_seq = seq;
                if new_audio_frame {
                    self.frames_rendered += 1;
                }
                self.packets_total += sent as u64;
                self.packets_last = sent;
                Ok(())
            }
            Err(err @ DriverError::Transport(_)) => {
                self.worker.take();
                self.active = false;
                if let Ok(mut t) = self.telemetry.lock() {
                    t.fatal_error = Some(err.to_string());
                }
                Err(err)
            }
            Err(err) => {
                self.worker.take();
                self.active = false;
                if let Ok(mut t) = self.telemetry.lock() {
                    t.fatal_error = Some(err.to_string());
                }
                Ok(())
            }
        }
    }

    fn spawn_worker(&mut self, config: &AudioSyncConfig) -> Result<(), String> {
        self.worker.take();
        if let Ok(mut g) = self.shared_cfg.lock() {
            *g = config.clone();
        }
        if let Ok(mut t) = self.telemetry.lock() {
            t.fatal_error = None;
            t.source_label = format!("Starting {}...", config.source.label());
            t.actual_fps = 0.0;
        }

        let stop = Arc::new(AtomicBool::new(false));
        let child_arc: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<String, String>>();

        let stop_bg = Arc::clone(&stop);
        let child_bg = Arc::clone(&child_arc);
        let cfg_bg = Arc::clone(&self.shared_cfg);
        let telem_bg = Arc::clone(&self.telemetry);
        let allowed_uids = self.allowed_uids.clone();
        let source = config.source.clone();

        let join = thread::Builder::new()
            .name("reaulad-audio".into())
            .spawn(move || {
                run_audio_worker(
                    source,
                    allowed_uids,
                    cfg_bg,
                    telem_bg,
                    stop_bg,
                    child_bg,
                    ready_tx,
                );
            })
            .map_err(|e| format!("failed to spawn daemon audio thread: {e}"))?;

        match ready_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(_label)) => {
                self.worker = Some(AudioWorkerHandle {
                    stop,
                    child: child_arc,
                    join: Some(join),
                });
                Ok(())
            }
            Ok(Err(err)) => {
                stop.store(true, Ordering::Relaxed);
                let _ = join.join();
                if let Ok(mut t) = self.telemetry.lock() {
                    t.fatal_error = Some(err.clone());
                }
                Err(err)
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                if let Ok(mut g) = child_arc.lock()
                    && let Some(mut ch) = g.take()
                {
                    let _ = ch.kill();
                    let _ = ch.wait();
                }
                let _ = join.join();
                let err = "timed out opening audio capture stream".to_string();
                if let Ok(mut t) = self.telemetry.lock() {
                    t.fatal_error = Some(err.clone());
                }
                Err(err)
            }
        }
    }
}

impl Clone for SharedTelemetry {
    fn clone(&self) -> Self {
        Self {
            source_label: self.source_label.clone(),
            metrics: self.metrics.clone(),
            frame: self.frame,
            frame_seq: self.frame_seq,
            actual_fps: self.actual_fps,
            fatal_error: self.fatal_error.clone(),
        }
    }
}

fn sanitize_config(cfg: &mut AudioSyncConfig) {
    cfg.brightness = cfg.brightness.clamp(1, LIGHT_MAX_LEVEL);
    cfg.fps = cfg.fps.clamp(5, 60);
    cfg.gain = cfg.gain.clamp(0.1, 10.0);
}

enum DaemonAudioInput {
    Demo(DemoAudioSynth),
    DirectAlsa(AlsaCapture),
    ChildPipe {
        reader: io::BufReader<std::process::ChildStdout>,
        pcm_label: String,
        sample_rate: u32,
        byte_buf: Vec<u8>,
    },
}

impl DaemonAudioInput {
    fn open(
        source: &AudioSyncSource,
        allowed_uids: &[u32],
        child_slot: &Arc<Mutex<Option<Child>>>,
    ) -> Result<(Self, String, u32), String> {
        match source {
            AudioSyncSource::Demo => Ok((
                Self::Demo(DemoAudioSynth::new(DEFAULT_SAMPLE_RATE)),
                "120 BPM Demo Synth".into(),
                DEFAULT_SAMPLE_RATE,
            )),
            AudioSyncSource::Monitor | AudioSyncSource::DefaultMic | AudioSyncSource::Device { .. } => {
                if ipc::current_uid() == 0
                    && let Some(&target_uid) = allowed_uids.iter().find(|&&u| u != 0)
                {
                    Self::open_helper_child(source, target_uid, child_slot)
                } else {
                    let mode = match source {
                        AudioSyncSource::Monitor => AlsaSourceMode::Monitor,
                        AudioSyncSource::DefaultMic => AlsaSourceMode::DefaultInput,
                        AudioSyncSource::Device { pcm, .. } => AlsaSourceMode::Device(pcm.clone()),
                        AudioSyncSource::Demo => unreachable!(),
                    };
                    let cap = AlsaCapture::open(&mode, DEFAULT_SAMPLE_RATE, 1, 35_000)?;
                    let sr = cap.sample_rate();
                    let label = format!("ALSA '{}' ({} Hz)", cap.pcm_name(), sr);
                    Ok((Self::DirectAlsa(cap), label, sr))
                }
            }
        }
    }

    fn open_helper_child(
        source: &AudioSyncSource,
        target_uid: u32,
        child_slot: &Arc<Mutex<Option<Child>>>,
    ) -> Result<(Self, String, u32), String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("failed to resolve reaulad binary for audio helper: {e}"))?;

        let mut cmd = Command::new(exe);
        cmd.arg("--capture-pcm")
            .arg("--pcm-rate")
            .arg(DEFAULT_SAMPLE_RATE.to_string());
        match source {
            AudioSyncSource::Monitor => {
                cmd.arg("--pcm-source").arg("monitor");
            }
            AudioSyncSource::DefaultMic => {
                cmd.arg("--pcm-source").arg("mic");
            }
            AudioSyncSource::Device { pcm, .. } => {
                cmd.arg("--pcm-source")
                    .arg("device")
                    .arg("--pcm-device")
                    .arg(pcm);
            }
            AudioSyncSource::Demo => unreachable!(),
        }

        let user = ipc::user_account_for_uid(target_uid).ok_or_else(|| {
            format!("no local /etc/passwd account found for authorized UID {target_uid}")
        })?;
        let runtime_dir = format!("/run/user/{target_uid}");
        cmd.env("XDG_RUNTIME_DIR", &runtime_dir);
        cmd.env("PULSE_RUNTIME_PATH", format!("{runtime_dir}/pulse"));
        if !user.home_dir.as_os_str().is_empty() {
            cmd.env("HOME", &user.home_dir);
        }
        cmd.env("USER", &user.username);
        cmd.env("LOGNAME", &user.username);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::systemd::scrub_child_env(&mut cmd);

        ipc::configure_user_command_credentials(&mut cmd, &user, false)?;

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn UID {target_uid} audio capture helper: {e}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "audio helper missing stdout pipe".to_string())?;
        let stderr = child.stderr.take();

        let mut reader = BufReader::new(stdout);
        let mut header = String::new();
        let read_res = reader.read_line(&mut header);
        if read_res.unwrap_or(0) == 0 {
            let mut err_txt = String::new();
            if let Some(mut se) = stderr {
                let _ = se.read_to_string(&mut err_txt);
            }
            let _ = child.wait();
            let detail = err_txt.trim();
            return Err(if detail.is_empty() {
                "audio capture helper exited before opening ALSA stream".into()
            } else {
                detail.to_string()
            });
        }

        let trimmed = header.trim();
        let rest = trimmed
            .strip_prefix("READY ")
            .ok_or_else(|| format!("unexpected audio helper header: {trimmed}"))?;
        let (sr_str, pcm_name) = rest
            .split_once(' ')
            .ok_or_else(|| format!("malformed audio helper header: {trimmed}"))?;
        let sample_rate: u32 = sr_str.parse().unwrap_or(DEFAULT_SAMPLE_RATE);
        let pcm_label = format!("ALSA '{pcm_name}' ({sample_rate} Hz, uid={target_uid})");

        if let Ok(mut g) = child_slot.lock() {
            *g = Some(child);
        }

        Ok((
            Self::ChildPipe {
                reader,
                pcm_label: pcm_label.clone(),
                sample_rate,
                byte_buf: Vec::new(),
            },
            pcm_label,
            sample_rate,
        ))
    }

    fn read_chunk(&mut self, buf: &mut [f32]) -> Result<usize, String> {
        match self {
            Self::Demo(synth) => {
                synth.generate(buf);
                thread::sleep(Duration::from_millis(10));
                Ok(buf.len())
            }
            Self::DirectAlsa(cap) => cap.read_mono_f32(buf),
            Self::ChildPipe {
                reader,
                pcm_label,
                sample_rate,
                byte_buf,
            } => {
                let _ = (*sample_rate, pcm_label.as_str());
                let needed = buf.len() * 2;
                if byte_buf.len() < needed {
                    byte_buf.resize(needed, 0);
                }
                let mut total = 0usize;
                while total < needed {
                    match reader.read(&mut byte_buf[total..needed]) {
                        Ok(0) => {
                            return Err("audio capture helper closed stream".into());
                        }
                        Ok(n) => {
                            total += n;
                            if total >= 2 && total.is_multiple_of(2) {
                                break;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(format!("audio helper pipe read error: {e}")),
                    }
                }
                let samples = total / 2;
                for i in 0..samples {
                    let s = i16::from_le_bytes([byte_buf[i * 2], byte_buf[i * 2 + 1]]);
                    buf[i] = f32::from(s) / 32768.0;
                }
                Ok(samples)
            }
        }
    }
}

fn run_audio_worker(
    source: AudioSyncSource,
    allowed_uids: Vec<u32>,
    shared_cfg: Arc<Mutex<AudioSyncConfig>>,
    telemetry: Arc<Mutex<SharedTelemetry>>,
    stop: Arc<AtomicBool>,
    child_slot: Arc<Mutex<Option<Child>>>,
    ready_tx: mpsc::Sender<Result<String, String>>,
) {
    let initial = shared_cfg.lock().map(|g| g.clone()).unwrap_or_default();
    let (mut input, src_label, sample_rate) =
        match DaemonAudioInput::open(&source, &allowed_uids, &child_slot) {
            Ok(v) => v,
            Err(e) => {
                let _ = ready_tx.send(Err(e));
                return;
            }
        };

    if let Ok(mut t) = telemetry.lock() {
        t.source_label = src_label.clone();
        t.fatal_error = None;
    }
    let _ = ready_tx.send(Ok(src_label.clone()));

    let mut vis = AudioVisualizer::new(
        sample_rate,
        initial.style,
        initial.palette,
        initial.fg,
        initial.bg,
        initial.gain,
    );
    let mut cur_fps = initial.fps.clamp(5, 60);
    let hop = (sample_rate as usize / 100).clamp(240, 1024);
    let mut pcm_buf = vec![0.0f32; hop];

    let mut last_step = Instant::now();
    let mut next_frame_at = Instant::now();
    let mut fps_window_start = Instant::now();
    let mut fps_window_frames = 0u32;
    let mut actual_fps = f32::from(cur_fps);

    while !stop.load(Ordering::Relaxed) {
        match input.read_chunk(&mut pcm_buf) {
            Ok(n) if n > 0 => vis.push_samples(&pcm_buf[..n]),
            Ok(_) => {}
            Err(e) => {
                if !stop.load(Ordering::Relaxed)
                    && let Ok(mut t) = telemetry.lock()
                {
                    t.fatal_error = Some(e);
                }
                break;
            }
        }

        if let Ok(cfg) = shared_cfg.lock() {
            vis.update_render_params(cfg.style, cfg.palette, cfg.fg, cfg.bg, cfg.gain);
            cur_fps = cfg.fps.clamp(5, 60);
        }

        let frame_period = Duration::from_secs_f64(1.0 / f64::from(cur_fps));
        let now = Instant::now();
        if now >= next_frame_at {
            let dt = now.duration_since(last_step).as_secs_f32().max(0.001);
            last_step = now;
            next_frame_at = (next_frame_at + frame_period).max(now);

            let (metrics, grid) = vis.step(dt);
            fps_window_frames += 1;
            let elapsed = fps_window_start.elapsed().as_secs_f32();
            if elapsed >= 0.5 {
                actual_fps = (fps_window_frames as f32) / elapsed;
                fps_window_start = now;
                fps_window_frames = 0;
            }

            if let Ok(mut t) = telemetry.lock() {
                t.metrics = metrics;
                t.frame = grid;
                t.frame_seq = t.frame_seq.wrapping_add(1);
                t.actual_fps = actual_fps;
            }
        }
    }

    if let Ok(mut g) = child_slot.lock()
        && let Some(mut ch) = g.take()
    {
        let _ = ch.kill();
        let _ = ch.wait();
    }
}

/// Entrypoint for `reaulad --capture-pcm`: runs strictly single-threaded under user UID.
pub(crate) fn run_pcm_capture_helper(
    source: &str,
    device: Option<&str>,
    rate: u32,
) -> Result<(), String> {
    let mode = match source {
        "monitor" => {
            // SAFETY: single-threaded helper child process before spawning any worker threads.
            unsafe {
                alsa::prepare_monitor_env();
            }
            AlsaSourceMode::Monitor
        }
        "mic" => AlsaSourceMode::DefaultInput,
        "device" => {
            let pcm = device.ok_or_else(|| "--pcm-device required for --pcm-source device".to_string())?;
            AlsaSourceMode::Device(pcm.to_string())
        }
        other => return Err(format!("unknown --pcm-source '{other}'")),
    };

    let mut cap = AlsaCapture::open(&mode, rate, 1, 35_000)?;
    let actual_rate = cap.sample_rate();
    let hop = (actual_rate as usize / 100).clamp(240, 1024);
    let mut pcm_i16 = vec![0i16; hop];
    let mut raw_bytes = vec![0u8; hop * 2];

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "READY {actual_rate} {}", cap.pcm_name())
        .and_then(|()| stdout.flush())
        .map_err(|e| format!("failed to write capture header: {e}"))?;

    loop {
        let n = cap.read_mono_i16(&mut pcm_i16)?;
        if n == 0 {
            continue;
        }
        for (i, &s) in pcm_i16[..n].iter().enumerate() {
            let le = s.to_le_bytes();
            raw_bytes[i * 2] = le[0];
            raw_bytes[i * 2 + 1] = le[1];
        }
        if let Err(e) = stdout.write_all(&raw_bytes[..n * 2]) {
            if e.kind() == io::ErrorKind::BrokenPipe {
                return Ok(());
            }
            return Err(format!("stdout PCM write error: {e}"));
        }
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reaula_core::diagnostics::DiagnosticCheck;
    use reaula_core::keyboards::{self, DeviceProbe, KeyboardSpec};
    use reaula_core::protocol::LightState;

    struct StubDriver {
        begin_count: usize,
        stream_count: usize,
        restored_custom: bool,
        restored_light: bool,
    }

    impl StubDriver {
        fn new() -> Self {
            Self {
                begin_count: 0,
                stream_count: 0,
                restored_custom: false,
                restored_light: false,
            }
        }
    }

    impl AulaDriver for StubDriver {
        fn spec(&self) -> &'static KeyboardSpec {
            &keyboards::win60he::SPEC
        }

        fn device_description(&self) -> &str {
            "stub-keyboard"
        }

        fn probe(&mut self) -> Result<DeviceProbe, DriverError> {
            Err(DriverError::Timeout("stub"))
        }

        fn run_keyboard_checks(&mut self) -> Vec<DiagnosticCheck> {
            Vec::new()
        }

        fn read_light(&mut self) -> Result<LightState, DriverError> {
            Ok(LightState::default())
        }

        fn set_light(&mut self, _state: &LightState) -> Result<(), DriverError> {
            self.restored_light = true;
            Ok(())
        }

        fn read_custom_light(&mut self, _slot: u8) -> Result<[[u8; 3]; MATRIX_SLOTS], DriverError> {
            Ok([[0; 3]; MATRIX_SLOTS])
        }

        fn set_custom_light(
            &mut self,
            _slot: u8,
            _colors: &[[u8; 3]; MATRIX_SLOTS],
            _brightness: u8,
        ) -> Result<(), DriverError> {
            self.restored_custom = true;
            Ok(())
        }

        fn begin_custom_stream(&mut self, _brightness: u8) -> Result<(), DriverError> {
            self.begin_count += 1;
            Ok(())
        }

        fn stream_custom_light(
            &mut self,
            _slot: u8,
            _colors: &[[u8; 3]; MATRIX_SLOTS],
            _force: bool,
            _drain_first: bool,
        ) -> Result<usize, DriverError> {
            self.stream_count += 1;
            Ok(4)
        }
    }

    #[test]
    fn demo_engine_analyzes_frames_and_cleans_up_on_stop() {
        let mut drv = StubDriver::new();
        let mut eng = DaemonAudioEngine::new(vec![1000]);
        let cfg = AudioSyncConfig {
            source: AudioSyncSource::Demo,
            fps: 60,
            ..AudioSyncConfig::default()
        };

        let st0 = eng.start(cfg, &mut drv).expect("demo start succeeds");
        assert!(st0.active);
        assert_eq!(drv.begin_count, 1);

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            eng.tick_hid(&mut drv, false, None).expect("tick ok");
            if drv.stream_count >= 2 {
                break;
            }
            thread::sleep(Duration::from_millis(12));
        }

        let st = eng.status();
        assert!(st.active);
        assert!(st.frames_rendered >= 2, "expected rendered frames, got {}", st.frames_rendered);
        assert_eq!(st.frame.len(), MATRIX_SLOTS);
        assert_eq!(st.packets_total, st.frames_rendered * 4);

        let stopped = eng.stop(true, Some(&mut drv));
        assert!(!stopped.active);
        assert!(stopped.frame.is_empty());
        assert!(drv.restored_custom);
        assert!(drv.restored_light);
    }
}
