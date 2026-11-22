//! Single-owner hardware worker loop, RPC dispatcher, and live stream coordinator.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use reaula_core::diagnostics::{
    self, CheckStatus, DiagnosticCheck, DiagnosticLayer, DiagnosticReport,
};
use reaula_core::ipc::{
    self, DaemonRequest, DaemonResponse, DaemonStatus, FnOverride, KeyActuationEntry,
    KeyDeadbandEntry, KeySwitchEntry, ShortcutAction, ShortcutRule, ShortcutTriggerLog,
};
use reaula_core::keyboards::{DriverError, KeyboardSpec};
use reaula_core::protocol::{
    BODY_LEN, CalibrationProgress, CalibrationStatus, LightState, LiveTravelSample, MATRIX_SLOTS,
    PollRate,
};

use crate::audio::DaemonAudioEngine;
use crate::keyboards::{self as driver_index, AulaDriver};
use crate::shortcuts::{ShortcutEngine, spawn_user_command};

pub(crate) struct DaemonRuntimeHealth {
    started_at: Instant,
    hw_heartbeat_ms: AtomicU64,
    device_connected: AtomicBool,
    pub(crate) systemd_managed: bool,
}

impl DaemonRuntimeHealth {
    pub(crate) fn new(systemd_managed: bool) -> Self {
        Self {
            started_at: Instant::now(),
            hw_heartbeat_ms: AtomicU64::new(0),
            device_connected: AtomicBool::new(false),
            systemd_managed,
        }
    }

    pub(crate) fn record_heartbeat(&self, device_connected: bool) {
        let elapsed_ms = self.started_at.elapsed().as_millis() as u64;
        self.hw_heartbeat_ms.store(elapsed_ms, Ordering::Relaxed);
        self.device_connected
            .store(device_connected, Ordering::Relaxed);
    }

    pub(crate) fn heartbeat_age(&self) -> Duration {
        let now_ms = self.started_at.elapsed().as_millis() as u64;
        let last_ms = self.hw_heartbeat_ms.load(Ordering::Relaxed);
        Duration::from_millis(now_ms.saturating_sub(last_ms))
    }

    pub(crate) fn is_device_connected(&self) -> bool {
        self.device_connected.load(Ordering::Relaxed)
    }
}

pub(crate) type TravelSubscribeReply = Result<(f64, Receiver<(LiveTravelSample, f64)>), String>;
pub(crate) type CalibrationSubReply = Result<Receiver<CalibrationProgress>, String>;

pub(crate) enum CoordinatorMsg {
    Rpc {
        req: DaemonRequest,
        reply_tx: Sender<DaemonResponse>,
    },
    SubscribeTravel {
        client_id: u64,
        indices: Vec<u8>,
        reply_tx: Sender<TravelSubscribeReply>,
    },
    UnsubscribeTravel {
        client_id: u64,
    },
    StartCalibration {
        client_id: u64,
        any_key: bool,
        reply_tx: Sender<CalibrationSubReply>,
    },
    StopCalibration {
        client_id: u64,
    },
    RequestShutdown,
}

struct TravelSubscriber {
    client_id: u64,
    indices: Vec<u8>,
    tx: Sender<(LiveTravelSample, f64)>,
}

struct CalibrationSession {
    client_id: u64,
    any_key: bool,
    tx: Sender<CalibrationProgress>,
}

struct DaemonIndicatorOverlay {
    active: bool,
    brightness: u8,
    saved_light: Option<LightState>,
    saved_custom0: Option<[[u8; 3]; MATRIX_SLOTS]>,
    last_grid: Option<[[u8; 3]; MATRIX_SLOTS]>,
}

impl DaemonIndicatorOverlay {
    fn new() -> Self {
        Self {
            active: false,
            brightness: 3,
            saved_light: None,
            saved_custom0: None,
            last_grid: None,
        }
    }

    fn clear_without_restore(&mut self) {
        self.active = false;
        self.saved_light = None;
        self.saved_custom0 = None;
        self.last_grid = None;
    }

    fn restore(&mut self, drv: Option<&mut (dyn AulaDriver + 'static)>) {
        if !self.active {
            return;
        }
        self.active = false;
        self.last_grid = None;
        if let Some(d) = drv {
            if let Some(c0) = self.saved_custom0.take() {
                let _ = d.set_custom_light(0, &c0, self.brightness);
            }
            if let Some(ls) = self.saved_light.take() {
                let _ = d.set_light(&ls);
            }
        } else {
            self.saved_custom0 = None;
            self.saved_light = None;
        }
    }

    fn sync(
        &mut self,
        drv: &mut (dyn AulaDriver + 'static),
        desired: Option<&[[u8; 3]; MATRIX_SLOTS]>,
        travel_active: bool,
    ) -> Result<(), DriverError> {
        match desired {
            Some(grid) => {
                if !self.active {
                    self.saved_light = drv.read_light().ok();
                    self.saved_custom0 = drv.read_custom_light(0).ok();
                    self.brightness = self
                        .saved_light
                        .as_ref()
                        .map_or(3, |l| l.brightness.clamp(2, 4));
                    drv.begin_custom_stream(self.brightness)?;
                    drv.stream_custom_light(0, grid, true, !travel_active)?;
                    self.active = true;
                    self.last_grid = Some(*grid);
                } else if self.last_grid.as_ref() != Some(grid) {
                    drv.stream_custom_light(0, grid, false, !travel_active)?;
                    self.last_grid = Some(*grid);
                }
            }
            None if self.active => {
                self.restore(Some(drv));
            }
            None => {}
        }
        Ok(())
    }
}

struct HardwareState {
    driver: Option<Box<dyn AulaDriver>>,
    preferred_keyboard: Option<String>,
    subscribers: Vec<TravelSubscriber>,
    travel_active: bool,
    calibration: Option<CalibrationSession>,
    shortcuts: ShortcutEngine,
    indicator: DaemonIndicatorOverlay,
    audio: DaemonAudioEngine,
    last_reconnect_attempt: Instant,
}

impl HardwareState {
    fn new(
        shortcuts_path: Option<PathBuf>,
        preferred_keyboard: Option<String>,
        allowed_uids: Vec<u32>,
    ) -> Self {
        let initial_spec = driver_index::preferred_spec(preferred_keyboard.as_deref());
        let mut s = Self {
            driver: None,
            preferred_keyboard,
            subscribers: Vec::new(),
            travel_active: false,
            calibration: None,
            shortcuts: ShortcutEngine::new(shortcuts_path, initial_spec),
            indicator: DaemonIndicatorOverlay::new(),
            audio: DaemonAudioEngine::new(allowed_uids),
            last_reconnect_attempt: Instant::now() - Duration::from_secs(10),
        };
        let _ = s.ensure_connected(true);
        s.sync_travel_stream();
        s
    }

    fn active_spec(&self) -> &'static KeyboardSpec {
        self.driver
            .as_ref()
            .map_or_else(
                || driver_index::preferred_spec(self.preferred_keyboard.as_deref()),
                |drv| drv.spec(),
            )
    }

    fn unit_mm(&self) -> f64 {
        self.driver
            .as_ref()
            .map_or(reaula_core::protocol::TRIGGER_UNIT_MM_DEFAULT, |d| d.unit_mm())
    }

    fn ensure_connected(&mut self, force: bool) -> Result<&mut dyn AulaDriver, String> {
        if self.driver.is_none() {
            if !force && self.last_reconnect_attempt.elapsed() < Duration::from_millis(750) {
                return Err("keyboard not connected".into());
            }
            self.last_reconnect_attempt = Instant::now();
            match driver_index::connect(self.preferred_keyboard.as_deref()) {
                Ok(drv) => {
                    self.shortcuts.set_keyboard_spec(drv.spec());
                    self.driver = Some(drv);
                    self.travel_active = false;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(self.driver.as_deref_mut().expect("driver just initialized"))
    }

    fn drop_on_transport_err(&mut self, err: &DriverError) {
        if matches!(err, DriverError::Transport(_)) {
            self.audio.stop(false, None);
            self.indicator.clear_without_restore();
            self.driver = None;
            self.travel_active = false;
            self.calibration = None;
            self.shortcuts.reset_live_state();
        }
    }

    fn need_travel_stream(&self) -> bool {
        self.calibration.is_none()
            && (!self.subscribers.is_empty() || self.shortcuts.has_active_rules())
    }

    fn sync_travel_stream(&mut self) {
        if !self.need_travel_stream() {
            if self.travel_active {
                if let Some(drv) = self.driver.as_mut() {
                    let _ = drv.close_trigger_test();
                }
                self.travel_active = false;
            }
            return;
        }

        let mut union = BTreeSet::new();
        let mut need_all = false;
        for sub in &self.subscribers {
            if sub.indices.is_empty() {
                need_all = true;
            } else {
                for &idx in &sub.indices {
                    union.insert(idx);
                }
            }
        }
        if self.shortcuts.watcher_active {
            for rule in &self.shortcuts.rules {
                if rule.enabled {
                    union.insert(rule.trigger_key);
                    if let Some(m) = rule.modifier_key {
                        union.insert(m);
                    }
                }
            }
        }

        if let Ok(drv) = self.ensure_connected(false) {
            let keys: Vec<u8> = if need_all || union.is_empty() {
                drv.all_key_indices()
            } else {
                union.into_iter().collect()
            };
            if let Err(e) = drv.open_trigger_test(&keys) {
                self.drop_on_transport_err(&e);
            } else {
                self.travel_active = true;
            }
        }
    }

    fn stop_active_calibration(&mut self) {
        if let Some(cal) = self.calibration.take()
            && let Some(drv) = self.driver.as_mut()
        {
            let _ = drv.stop_calibration(cal.any_key);
        }
    }

    fn cleanup_on_exit(&mut self) {
        self.indicator.restore(self.driver.as_deref_mut());
        self.audio.stop(true, self.driver.as_deref_mut());
        self.shortcuts.release_all_virtual_keys();
        self.stop_active_calibration();
        if self.travel_active
            && let Some(drv) = self.driver.as_mut()
        {
            let _ = drv.close_trigger_test();
        }
    }
}

pub(crate) fn hardware_loop(
    rx: Receiver<CoordinatorMsg>,
    shutdown: Arc<AtomicBool>,
    allowed_uids: Vec<u32>,
    socket_path: String,
    shortcuts_path: Option<PathBuf>,
    preferred_keyboard: Option<String>,
    health: Arc<DaemonRuntimeHealth>,
) {
    let mut state = HardwareState::new(shortcuts_path, preferred_keyboard, allowed_uids.clone());
    health.record_heartbeat(state.driver.is_some());

    while !shutdown.load(Ordering::Relaxed) {
        health.record_heartbeat(state.driver.is_some());
        let streaming = state.travel_active || state.calibration.is_some();
        let msg = if streaming {
            match rx.try_recv() {
                Ok(m) => Some(m),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        } else {
            let timeout = if state.audio.is_active() {
                Duration::from_millis(8)
            } else if state.indicator.active || state.shortcuts.has_active_pulses() {
                Duration::from_millis(30)
            } else {
                Duration::from_millis(200)
            };
            match rx.recv_timeout(timeout) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };

        if let Some(msg) = msg {
            let should_quit = handle_coord_msg(
                msg,
                &mut state,
                &shutdown,
                &allowed_uids,
                &socket_path,
                &health,
            );
            if should_quit {
                break;
            }
            // Drain burst RPCs immediately before polling HID streams.
            while let Ok(next) = rx.try_recv() {
                if handle_coord_msg(
                    next,
                    &mut state,
                    &shutdown,
                    &allowed_uids,
                    &socket_path,
                    &health,
                ) {
                    state.cleanup_on_exit();
                    return;
                }
            }
        }

        if let Some(cal_any_key) = state.calibration.as_ref().map(|c| c.any_key) {
            let poll_res = match state.driver.as_mut() {
                Some(drv) => drv.poll_calibration_once(20),
                None => {
                    state.calibration = None;
                    continue;
                }
            };
            match poll_res {
                Ok(Some(prog)) => {
                    let terminal = !matches!(prog.status, CalibrationStatus::Progress);
                    let delivered = state
                        .calibration
                        .as_ref()
                        .is_some_and(|c| c.tx.send(prog).is_ok());
                    if terminal || !delivered {
                        if let Some(drv) = state.driver.as_mut() {
                            let _ = drv.stop_calibration(cal_any_key);
                        }
                        state.calibration = None;
                        state.sync_travel_stream();
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    state.drop_on_transport_err(&e);
                }
            }
        } else if state.travel_active {
            let unit_mm = state.unit_mm();
            let poll_res = match state.driver.as_mut() {
                Some(drv) => drv.poll_live_travel_batch(32, 5),
                None => continue,
            };
            match poll_res {
                Ok(samples) if !samples.is_empty() => {
                    let mut fired_rules = Vec::new();
                    for sample in samples {
                        state
                            .subscribers
                            .retain(|sub| sub.tx.send((sample, unit_mm)).is_ok());
                        fired_rules.extend(state.shortcuts.evaluate_sample(&sample, unit_mm));
                    }
                    let any_fired = !fired_rules.is_empty();
                    for rule in fired_rules {
                        let log = apply_shortcut_rule(&rule, &mut state, &allowed_uids);
                        state.shortcuts.push_log(log);
                    }
                    if any_fired || !state.need_travel_stream() {
                        state.sync_travel_stream();
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    state.drop_on_transport_err(&e);
                }
            }
        } else if state.need_travel_stream() {
            state.sync_travel_stream();
        }

        if state.calibration.is_none() {
            let desired_indicator = state.shortcuts.build_indicator_colors();
            if state.audio.is_active() {
                state.indicator.clear_without_restore();
                if let Some(drv) = state.driver.as_deref_mut()
                    && let Err(err) =
                        state
                            .audio
                            .tick_hid(drv, state.travel_active, desired_indicator.as_ref())
                {
                    state.drop_on_transport_err(&err);
                }
            } else if let Some(drv) = state.driver.as_deref_mut()
                && let Err(err) =
                    state
                        .indicator
                        .sync(drv, desired_indicator.as_ref(), state.travel_active)
            {
                state.drop_on_transport_err(&err);
            }
        }
    }

    state.cleanup_on_exit();
}

fn handle_coord_msg(
    msg: CoordinatorMsg,
    state: &mut HardwareState,
    shutdown: &Arc<AtomicBool>,
    allowed_uids: &[u32],
    socket_path: &str,
    health: &Arc<DaemonRuntimeHealth>,
) -> bool {
    match msg {
        CoordinatorMsg::RequestShutdown => {
            shutdown.store(true, Ordering::Relaxed);
            true
        }
        CoordinatorMsg::SubscribeTravel {
            client_id,
            indices,
            reply_tx,
        } => {
            if state.calibration.is_some() {
                let _ = reply_tx.send(Err("cannot stream travel while calibration is armed".into()));
                return false;
            }
            if let Err(e) = state.ensure_connected(true) {
                let _ = reply_tx.send(Err(e));
                return false;
            }
            let (sample_tx, sample_rx) = mpsc::channel();
            state.subscribers.push(TravelSubscriber {
                client_id,
                indices,
                tx: sample_tx,
            });
            state.sync_travel_stream();
            if state.travel_active {
                let _ = reply_tx.send(Ok((state.unit_mm(), sample_rx)));
            } else {
                state.subscribers.retain(|s| s.client_id != client_id);
                let _ = reply_tx.send(Err("failed to open live travel stream".into()));
            }
            false
        }
        CoordinatorMsg::UnsubscribeTravel { client_id } => {
            state.subscribers.retain(|s| s.client_id != client_id);
            state.sync_travel_stream();
            false
        }
        CoordinatorMsg::StartCalibration {
            client_id,
            any_key,
            reply_tx,
        } => {
            if state.calibration.is_some() {
                let _ = reply_tx.send(Err("calibration is already in progress".into()));
                return false;
            }
            if state.indicator.active {
                state.indicator.restore(state.driver.as_deref_mut());
            }
            if state.audio.is_active() {
                state.audio.stop(true, state.driver.as_deref_mut());
            }
            if let Err(e) = state.ensure_connected(true) {
                let _ = reply_tx.send(Err(e));
                return false;
            }
            if state.travel_active
                && let Some(drv) = state.driver.as_mut()
            {
                let _ = drv.close_trigger_test();
                state.travel_active = false;
            }
            let start_res = state
                .driver
                .as_mut()
                .expect("driver connected")
                .start_calibration(any_key);
            match start_res {
                Ok(()) => {
                    let (tx, rx) = mpsc::channel();
                    state.calibration = Some(CalibrationSession {
                        client_id,
                        any_key,
                        tx,
                    });
                    let _ = reply_tx.send(Ok(rx));
                }
                Err(e) => {
                    state.drop_on_transport_err(&e);
                    state.sync_travel_stream();
                    let _ = reply_tx.send(Err(e.to_string()));
                }
            }
            false
        }
        CoordinatorMsg::StopCalibration { client_id } => {
            if state
                .calibration
                .as_ref()
                .is_some_and(|c| c.client_id == client_id)
            {
                state.stop_active_calibration();
                state.sync_travel_stream();
            }
            false
        }
        CoordinatorMsg::Rpc { req, reply_tx } => {
            if matches!(req, DaemonRequest::Ping) {
                let _ = state.ensure_connected(false);
                let resp = DaemonResponse::Status(DaemonStatus {
                    daemon_pid: std::process::id(),
                    daemon_uid: ipc::current_uid(),
                    allowed_uids: allowed_uids.to_vec(),
                    device_connected: state.driver.is_some(),
                    device_description: state
                        .driver
                        .as_ref()
                        .map(|d| d.device_description().to_string()),
                    keyboard_id: state.driver.as_ref().map(|d| d.spec().id.to_string()),
                    keyboard_name: state
                        .driver
                        .as_ref()
                        .map(|d| d.spec().display_name.to_string()),
                    socket_path: socket_path.to_string(),
                    travel_subscribers: state.subscribers.len(),
                    systemd_managed: health.systemd_managed,
                });
                let _ = reply_tx.send(resp);
                return false;
            }

            if matches!(req, DaemonRequest::Shutdown) {
                shutdown.store(true, Ordering::Relaxed);
                let _ = reply_tx.send(DaemonResponse::Ack);
                return true;
            }

            let resp = match req {
                DaemonRequest::RunDiagnostics => {
                    run_daemon_diagnostics(state, allowed_uids, socket_path)
                }
                DaemonRequest::GetShortcuts => {
                    state.shortcuts.ensure_uinput();
                    state.shortcuts.snapshot_response()
                }
                DaemonRequest::SetShortcuts {
                    rules,
                    watcher_active,
                } => {
                    if let Err(message) = crate::shortcuts::validate_daemon_rules(&rules) {
                        DaemonResponse::Error { message }
                    } else {
                        state.shortcuts.ensure_uinput();
                        state.shortcuts.set_rules(rules, watcher_active);
                        state.sync_travel_stream();
                        state.shortcuts.snapshot_response()
                    }
                }
                DaemonRequest::TriggerShortcut { id } => {
                    if let Some(rule) = state
                        .shortcuts
                        .rules
                        .iter()
                        .find(|r| r.id == id)
                        .cloned()
                    {
                        let log = apply_shortcut_rule(&rule, state, allowed_uids);
                        state.shortcuts.record_rule_pulse(&rule);
                        state.shortcuts.push_log(log.clone());
                        state.sync_travel_stream();
                        DaemonResponse::ShortcutTriggered(log)
                    } else {
                        DaemonResponse::Error {
                            message: format!("shortcut rule #{id} not found in daemon memory"),
                        }
                    }
                }
                DaemonRequest::GetAudioSyncState => {
                    DaemonResponse::AudioSync(Box::new(state.audio.status()))
                }
                DaemonRequest::StartAudioSync { config } => {
                    if state.calibration.is_some() {
                        DaemonResponse::Error {
                            message: "calibration is currently active; wait or stop calibration first".into(),
                        }
                    } else if let Err(e) = state.ensure_connected(true) {
                        DaemonResponse::Error { message: e }
                    } else {
                        state.indicator.restore(state.driver.as_deref_mut());
                        let drv = state.driver.as_deref_mut().expect("driver connected");
                        match state.audio.start(config, drv) {
                            Ok(st) => DaemonResponse::AudioSync(Box::new(st)),
                            Err(message) => DaemonResponse::Error { message },
                        }
                    }
                }
                DaemonRequest::UpdateAudioSync { config } => {
                    match state.audio.update(config, state.driver.as_deref_mut()) {
                        Ok(st) => DaemonResponse::AudioSync(Box::new(st)),
                        Err(message) => DaemonResponse::Error { message },
                    }
                }
                DaemonRequest::StopAudioSync { restore_lighting } => {
                    if restore_lighting && state.travel_active && let Some(drv) = state.driver.as_mut() {
                        let _ = drv.close_trigger_test();
                        state.travel_active = false;
                    }
                    let st = state.audio.stop(restore_lighting, state.driver.as_deref_mut());
                    DaemonResponse::AudioSync(Box::new(st))
                }
                other => execute_rpc(other, state),
            };
            if state.need_travel_stream() {
                state.sync_travel_stream();
            }
            let _ = reply_tx.send(resp);
            false
        }
    }
}

fn fn_overrides_response(list: Vec<(u8, u8)>) -> DaemonResponse {
    DaemonResponse::KeyboardFnMap {
        overrides: list
            .into_iter()
            .map(|(index, target_hid)| FnOverride { index, target_hid })
            .collect(),
    }
}

fn execute_rpc(req: DaemonRequest, state: &mut HardwareState) -> DaemonResponse {
    if state.calibration.is_some() {
        return DaemonResponse::Error {
            message: "calibration is currently active; wait or stop calibration first".into(),
        };
    }
    if matches!(
        req,
        DaemonRequest::SetLight { .. }
            | DaemonRequest::SetCustomRgb { .. }
            | DaemonRequest::BeginCustomStream { .. }
            | DaemonRequest::StreamCustomRgb { .. }
    ) {
        state.indicator.clear_without_restore();
        if state.audio.is_active() {
            state.audio.stop(false, state.driver.as_deref_mut());
        }
    } else if matches!(
        req,
        DaemonRequest::GetLight
            | DaemonRequest::GetActiveCustomSlot
            | DaemonRequest::GetCustomRgb { .. }
    ) && state.indicator.active
    {
        state.indicator.restore(state.driver.as_deref_mut());
    }
    if matches!(
        req,
        DaemonRequest::GetKeymap { .. }
            | DaemonRequest::ApplyKeyboardFnMap { .. }
            | DaemonRequest::ResetKeyboardFnMap
            | DaemonRequest::GetLight
            | DaemonRequest::SetLight { .. }
            | DaemonRequest::GetLightList
            | DaemonRequest::GetActiveCustomSlot
            | DaemonRequest::GetCustomRgb { .. }
            | DaemonRequest::SetCustomRgb { .. }
    ) && state.travel_active
    {
        if let Some(drv) = state.driver.as_mut() {
            let _ = drv.close_trigger_test();
        }
        state.travel_active = false;
    }
    if let Err(e) = state.ensure_connected(true) {
        return DaemonResponse::Error { message: e };
    }
    let travel_active = state.travel_active;
    let drv = state.driver.as_deref_mut().expect("driver connected");
    let unit_mm = drv.unit_mm();

    let res: Result<DaemonResponse, DriverError> = match req {
        DaemonRequest::Probe => drv.probe().map(|p| DaemonResponse::Probe(Box::new(p))),
        DaemonRequest::GetActuation { indices } => drv.read_trigger_configs(&indices).map(|map| {
            let entries = indices
                .into_iter()
                .map(|index| KeyActuationEntry {
                    index,
                    config: map.get(&index).copied(),
                })
                .collect();
            DaemonResponse::Actuation { unit_mm, entries }
        }),
        DaemonRequest::SetActuation {
            indices,
            mode,
            travel_mm,
            rt_press_mm,
            rt_release_mm,
        } => drv
            .apply_actuation(&indices, mode, travel_mm, rt_press_mm, rt_release_mm)
            .map(|()| DaemonResponse::Ack),
        DaemonRequest::GetLight => drv.read_light().map(DaemonResponse::Light),
        DaemonRequest::SetLight { state: ls } => drv.set_light(&ls).map(|()| DaemonResponse::Ack),
        DaemonRequest::GetLightList => drv.read_light_list().map(DaemonResponse::LightList),
        DaemonRequest::GetActiveCustomSlot => drv
            .read_active_custom_slot()
            .map(|slot| DaemonResponse::ActiveCustomSlot { slot }),
        DaemonRequest::GetCustomRgb { slot } => drv
            .read_custom_light(slot)
            .map(|c| DaemonResponse::CustomRgb {
                slot,
                colors: c.to_vec(),
            }),
        DaemonRequest::SetCustomRgb {
            slot,
            colors,
            brightness,
        } => {
            if colors.len() != MATRIX_SLOTS {
                return DaemonResponse::Error {
                    message: format!("expected {MATRIX_SLOTS} RGB entries, got {}", colors.len()),
                };
            }
            let mut arr = [[0u8; 3]; MATRIX_SLOTS];
            arr.copy_from_slice(&colors);
            drv.set_custom_light(slot, &arr, brightness)
                .map(|()| DaemonResponse::Ack)
        }
        DaemonRequest::BeginCustomStream { brightness } => drv
            .begin_custom_stream(brightness)
            .map(|()| DaemonResponse::Ack),
        DaemonRequest::StreamCustomRgb {
            slot,
            colors,
            force,
        } => {
            if colors.len() != MATRIX_SLOTS {
                return DaemonResponse::Error {
                    message: format!("expected {MATRIX_SLOTS} RGB entries, got {}", colors.len()),
                };
            }
            let mut arr = [[0u8; 3]; MATRIX_SLOTS];
            arr.copy_from_slice(&colors);
            drv.stream_custom_light(slot, &arr, force, !travel_active)
                .map(|packets_sent| DaemonResponse::CustomStreamed { packets_sent })
        }
        DaemonRequest::GetSwitches { indices } => drv.read_switch_table().map(|table| {
            let entries = indices
                .into_iter()
                .map(|index| KeySwitchEntry {
                    index,
                    switch_id: table.get(index as usize).copied().unwrap_or(0),
                })
                .collect();
            DaemonResponse::Switches { entries }
        }),
        DaemonRequest::SetSwitches { indices, switch_id } => drv
            .set_switch_profile(&indices, switch_id)
            .map(|()| DaemonResponse::Ack),
        DaemonRequest::GetDeadband { indices } => drv.read_deadband_table().map(|table| {
            let entries = indices
                .into_iter()
                .map(|index| {
                    let off = (index as usize) * 2;
                    KeyDeadbandEntry {
                        index,
                        top_raw: table.get(off).copied().unwrap_or(0),
                        bottom_raw: table.get(off + 1).copied().unwrap_or(0),
                    }
                })
                .collect();
            DaemonResponse::Deadband { entries }
        }),
        DaemonRequest::SetDeadband {
            indices,
            top_raw,
            bottom_raw,
        } => drv
            .set_deadband(&indices, top_raw, bottom_raw)
            .map(|()| DaemonResponse::Ack),
        DaemonRequest::GetPollRate => drv
            .read_poll_rate()
            .map(|khz| DaemonResponse::PollRate { khz }),
        DaemonRequest::SetPollRate { khz } => match PollRate::from_khz(khz) {
            Ok(rate) => drv.set_poll_rate(rate).map(|()| DaemonResponse::Ack),
            Err(e) => Err(DriverError::Protocol(e)),
        },
        DaemonRequest::GetSocd => drv.read_prcs_power().and_then(|enabled| {
            drv.read_prcs_pairs()
                .map(|pairs| DaemonResponse::Socd { enabled, pairs })
        }),
        DaemonRequest::SetSocd { enabled, pairs } => drv
            .set_socd(enabled, &pairs)
            .map(|()| DaemonResponse::Ack),
        DaemonRequest::GetKeymap { fn_layer } => drv.read_keymap(fn_layer).map(|t| {
            DaemonResponse::Keymap {
                fn_layer,
                table: t.to_vec(),
            }
        }),
        DaemonRequest::ApplyKeyboardFnMap { set, clear } => {
            let pairs: Vec<(u8, u8)> = set.into_iter().map(|o| (o.index, o.target_hid)).collect();
            drv.apply_fn_keymap_overrides(&pairs, &clear)
                .map(fn_overrides_response)
        }
        DaemonRequest::ResetKeyboardFnMap => drv.reset_fn_keymap().map(fn_overrides_response),
        DaemonRequest::SetWinLock { flags } => drv
            .set_win_lock(flags)
            .map(|()| DaemonResponse::Ack),
        DaemonRequest::Raw { body } => {
            if body.len() != BODY_LEN {
                return DaemonResponse::Error {
                    message: format!("raw body must be {BODY_LEN} bytes"),
                };
            }
            let mut arr = [0u8; BODY_LEN];
            arr.copy_from_slice(&body);
            drv.send_raw_checked(&arr)
                .map(|reply| DaemonResponse::RawReply { reply })
        }
        DaemonRequest::Ping
        | DaemonRequest::Shutdown
        | DaemonRequest::SubscribeTravel { .. }
        | DaemonRequest::UnsubscribeTravel
        | DaemonRequest::GetShortcuts
        | DaemonRequest::SetShortcuts { .. }
        | DaemonRequest::TriggerShortcut { .. }
        | DaemonRequest::RunDiagnostics
        | DaemonRequest::StartCalibration { .. }
        | DaemonRequest::StopCalibration
        | DaemonRequest::StartAudioSync { .. }
        | DaemonRequest::UpdateAudioSync { .. }
        | DaemonRequest::StopAudioSync { .. }
        | DaemonRequest::GetAudioSyncState => unreachable!(),
    };

    match res {
        Ok(resp) => resp,
        Err(e) => {
            state.drop_on_transport_err(&e);
            DaemonResponse::Error {
                message: e.to_string(),
            }
        }
    }
}

fn run_daemon_diagnostics(
    state: &mut HardwareState,
    allowed_uids: &[u32],
    socket_path: &str,
) -> DaemonResponse {
    let mut checks = diagnostics::run_kernel_checks_for(state.active_spec());

    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Daemon,
        name: "Daemon IPC & SO_PEERCRED guard".into(),
        status: CheckStatus::Pass,
        latency_ms: Some(0.0),
        detail: format!(
            "reaulad pid={} uid={} socket={} model={} allowlist={:?} subs={} shortcuts={}",
            std::process::id(),
            ipc::current_uid(),
            socket_path,
            state.active_spec().id,
            allowed_uids,
            state.subscribers.len(),
            state.shortcuts.rules.len(),
        ),
        remedy: None,
    });

    if state.calibration.is_some() {
        checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Daemon,
            name: "HID interface link".into(),
            status: CheckStatus::Warn,
            latency_ms: None,
            detail: "Calibration mode currently armed; live keyboard checks paused until stopped."
                .into(),
            remedy: Some("Finish or abort calibration before running full HID diagnostics.".into()),
        });
    } else {
        if state.indicator.active {
            state.indicator.restore(state.driver.as_deref_mut());
        }
        let link_t = Instant::now();
        match state.ensure_connected(true) {
            Ok(_) => {
                let model_name = state.active_spec().display_name;
                let desc = state
                    .driver
                    .as_ref()
                    .map_or(model_name, |d| d.device_description());
                let unit_mm = state.unit_mm();
                checks.push(DiagnosticCheck {
                    layer: DiagnosticLayer::Daemon,
                    name: "HID interface link".into(),
                    status: CheckStatus::Pass,
                    latency_ms: Some(link_t.elapsed().as_secs_f64() * 1000.0),
                    detail: format!("Connected to {desc} (unit_mm={unit_mm:.3} mm)"),
                    remedy: None,
                });

                if state.travel_active
                    && let Some(drv) = state.driver.as_mut()
                {
                    let _ = drv.close_trigger_test();
                    state.travel_active = false;
                }
                if let Some(drv) = state.driver.as_mut() {
                    checks.extend(drv.run_keyboard_checks());
                }
                state.sync_travel_stream();
            }
            Err(err) => {
                checks.push(DiagnosticCheck {
                    layer: DiagnosticLayer::Daemon,
                    name: "HID interface link".into(),
                    status: CheckStatus::Fail,
                    latency_ms: Some(link_t.elapsed().as_secs_f64() * 1000.0),
                    detail: err,
                    remedy: Some(
                        "Ensure udev rules or root permissions allow opening /dev/hidraw*, and close competing web drivers."
                            .into(),
                    ),
                });
                checks.push(DiagnosticCheck {
                    layer: DiagnosticLayer::Keyboard,
                    name: "Keyboard HID probe".into(),
                    status: CheckStatus::Fail,
                    latency_ms: None,
                    detail: "Skipped: daemon could not open vendor HID interface.".into(),
                    remedy: Some("Resolve Kernel or Daemon link failure above first.".into()),
                });
            }
        }
    }

    checks.extend(diagnostics::run_software_checks());
    let attribution = diagnostics::compute_attribution(&checks);
    DaemonResponse::Diagnostics(Box::new(DiagnosticReport {
        generated_at_unix_ms: diagnostics::now_unix_ms(),
        checks,
        attribution,
    }))
}

fn apply_shortcut_rule(
    rule: &ShortcutRule,
    state: &mut HardwareState,
    allowed_uids: &[u32],
) -> ShortcutTriggerLog {
    let unix_ms = diagnostics::now_unix_ms();
    let action_summary = rule.action.summary();

    if let ShortcutAction::EmitKey { key } = rule.action {
        return state.shortcuts.tap_virtual_key(rule, key);
    }

    if let ShortcutAction::RunCommand { command } = &rule.action {
        let (success, message) = match spawn_user_command(command, allowed_uids) {
            Ok(msg) => (true, msg),
            Err(err) => (false, err),
        };
        return ShortcutTriggerLog {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            trigger_key: rule.trigger_key,
            action_summary,
            unix_ms,
            success,
            message,
        };
    }

    if state.calibration.is_some() {
        return ShortcutTriggerLog {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            trigger_key: rule.trigger_key,
            action_summary,
            unix_ms,
            success: false,
            message: "Skipped while calibration is armed".into(),
        };
    }

    if let Err(e) = state.ensure_connected(true) {
        return ShortcutTriggerLog {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            trigger_key: rule.trigger_key,
            action_summary,
            unix_ms,
            success: false,
            message: e,
        };
    }

    if matches!(
        rule.action,
        ShortcutAction::LightingPreset { .. } | ShortcutAction::ToggleLighting
    ) {
        state.indicator.clear_without_restore();
    }

    let drv = state.driver.as_deref_mut().expect("driver connected");
    let outcome: Result<String, DriverError> = match &rule.action {
        ShortcutAction::ActuationPreset {
            mode,
            indices,
            depth_mm,
            rt_press_mm,
            rt_release_mm,
            ..
        } => {
            let targets = if indices.is_empty() {
                drv.all_key_indices()
            } else {
                indices.clone()
            };
            drv.apply_actuation(
                &targets,
                *mode,
                *depth_mm,
                Some(*rt_press_mm),
                Some(*rt_release_mm),
            )
            .map(|()| format!("Applied actuation ({} keys)", targets.len()))
        }
        ShortcutAction::ToggleSocd => drv.read_prcs_power().and_then(|enabled| {
            let pairs = drv.read_prcs_pairs()?;
            let next = !enabled;
            drv.set_socd(next, &pairs).map(|()| {
                format!("SOCD {}", if next { "enabled" } else { "disabled" })
            })
        }),
        ShortcutAction::LightingPreset { state: ls, .. } => {
            drv.set_light(ls).map(|()| "Applied lighting preset".into())
        }
        ShortcutAction::ToggleLighting => drv.read_light().and_then(|mut ls| {
            ls.power_on = !ls.power_on;
            let on = ls.power_on;
            drv.set_light(&ls)
                .map(|()| format!("Keyboard RGB {}", if on { "ON" } else { "OFF" }))
        }),
        ShortcutAction::SetPollRate { khz } => match PollRate::from_khz(*khz) {
            Ok(rate) => drv
                .set_poll_rate(rate)
                .map(|()| format!("Polling rate -> {khz} kHz")),
            Err(e) => Err(DriverError::Protocol(e)),
        },
        ShortcutAction::RunCommand { .. } | ShortcutAction::EmitKey { .. } => unreachable!(),
    };

    match outcome {
        Ok(message) => ShortcutTriggerLog {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            trigger_key: rule.trigger_key,
            action_summary,
            unix_ms,
            success: true,
            message,
        },
        Err(e) => {
            state.drop_on_transport_err(&e);
            ShortcutTriggerLog {
                rule_id: rule.id,
                rule_name: rule.name.clone(),
                trigger_key: rule.trigger_key,
                action_summary,
                unix_ms,
                success: false,
                message: e.to_string(),
            }
        }
    }
}
