//! Daemon driver adapter for the Aula WIN 60 HE (`2E3C:C365`).

use std::collections::BTreeMap;

use atrium_core::diagnostics::{self, DiagnosticCheck};
use atrium_core::keyboards::win60he::{self, driver::Win60Driver, protocol};
use atrium_core::keyboards::{DeviceProbe, DriverError, KeyboardSpec};
use atrium_core::protocol::{
    BODY_LEN, CalibrationProgress, DEADBAND_TABLE_BYTES, KEYMAP_BYTES, LightList, LightState,
    LiveTravelSample, MATRIX_SLOTS, PollRate, PrcsPair, SWITCH_TABLE_BYTES,
    TRIGGER_UNIT_MM_DEFAULT, TriggerConfig, TriggerMode, WinLockFlags,
};
use atrium_core::transport::HidApiTransport;

use super::{AulaDriver, DriverEntry};

pub const ENTRY: DriverEntry = DriverEntry {
    spec: &win60he::SPEC,
    open: open_win60he,
};

fn open_win60he() -> Result<Box<dyn AulaDriver>, String> {
    let transport = HidApiTransport::open_spec(&win60he::SPEC).map_err(|e| e.to_string())?;
    let mut inner = Win60Driver::new(transport);
    let unit_mm = inner
        .read_init_info()
        .ok()
        .and_then(|info| inner.read_max_trigger_travel(info.is_high_precision).ok())
        .map_or(TRIGGER_UNIT_MM_DEFAULT, |m| m.unit_mm);
    Ok(Box::new(Win60Backend { inner, unit_mm }))
}

struct Win60Backend {
    inner: Win60Driver<HidApiTransport>,
    unit_mm: f64,
}

impl AulaDriver for Win60Backend {
    fn spec(&self) -> &'static KeyboardSpec {
        &win60he::SPEC
    }

    fn device_description(&self) -> &str {
        self.inner.device_description()
    }

    fn unit_mm(&self) -> f64 {
        self.unit_mm
    }

    fn probe(&mut self) -> Result<DeviceProbe, DriverError> {
        self.inner.probe()
    }

    fn run_keyboard_checks(&mut self) -> Vec<DiagnosticCheck> {
        diagnostics::run_keyboard_checks(&mut self.inner)
    }

    fn read_light(&mut self) -> Result<LightState, DriverError> {
        self.inner.read_light()
    }

    fn set_light(&mut self, state: &LightState) -> Result<(), DriverError> {
        self.inner.set_light(state)
    }

    fn read_light_list(&mut self) -> Result<LightList, DriverError> {
        self.inner.read_light_list()
    }

    fn read_active_custom_slot(&mut self) -> Result<u8, DriverError> {
        self.inner.read_active_custom_slot()
    }

    fn read_custom_light(&mut self, slot: u8) -> Result<[[u8; 3]; MATRIX_SLOTS], DriverError> {
        self.inner.read_custom_light(slot)
    }

    fn set_custom_light(
        &mut self,
        slot: u8,
        colors: &[[u8; 3]; MATRIX_SLOTS],
        brightness: u8,
    ) -> Result<(), DriverError> {
        self.inner.set_custom_light(slot, colors, brightness)
    }

    fn begin_custom_stream(&mut self, brightness: u8) -> Result<(), DriverError> {
        self.inner.begin_custom_stream(brightness)
    }

    fn stream_custom_light(
        &mut self,
        slot: u8,
        colors: &[[u8; 3]; MATRIX_SLOTS],
        force: bool,
        drain_first: bool,
    ) -> Result<usize, DriverError> {
        self.inner
            .stream_custom_light(slot, colors, force, drain_first)
    }

    fn read_poll_rate(&mut self) -> Result<u8, DriverError> {
        self.inner.read_poll_rate()
    }

    fn set_poll_rate(&mut self, rate: PollRate) -> Result<(), DriverError> {
        self.inner.set_poll_rate(rate)
    }

    fn read_keymap(&mut self, fn_layer: bool) -> Result<[u8; KEYMAP_BYTES], DriverError> {
        self.inner.read_keymap(fn_layer)
    }

    fn apply_fn_keymap_overrides(
        &mut self,
        pairs: &[(u8, u8)],
        clear: &[u8],
    ) -> Result<Vec<(u8, u8)>, DriverError> {
        self.inner.apply_fn_keymap_overrides(pairs, clear)
    }

    fn reset_fn_keymap(&mut self) -> Result<Vec<(u8, u8)>, DriverError> {
        self.inner.reset_fn_keymap()
    }

    fn set_win_lock(&mut self, flags: WinLockFlags) -> Result<(), DriverError> {
        self.inner.set_win_lock(flags)
    }

    fn send_raw_checked(&mut self, body: &[u8; BODY_LEN]) -> Result<Option<Vec<u8>>, DriverError> {
        self.inner.send_raw_checked(body)
    }

    fn read_trigger_configs(
        &mut self,
        indices: &[u8],
    ) -> Result<BTreeMap<u8, TriggerConfig>, DriverError> {
        self.inner.read_trigger_configs(indices)
    }

    fn apply_actuation(
        &mut self,
        indices: &[u8],
        mode: TriggerMode,
        travel_mm: f64,
        rt_press_mm: Option<f64>,
        rt_release_mm: Option<f64>,
    ) -> Result<(), DriverError> {
        let hp = self
            .inner
            .read_init_info()
            .map(|i| i.is_high_precision)
            .unwrap_or(false);
        let limits = self.inner.read_max_trigger_travel(hp).ok();
        if let Some(ref l) = limits {
            self.unit_mm = l.unit_mm;
        }
        let _ = protocol::cmd::TRIGGER;
        self.inner
            .set_actuation(indices, mode, travel_mm, rt_press_mm, rt_release_mm, limits)
    }

    fn open_trigger_test(&mut self, indices: &[u8]) -> Result<(), DriverError> {
        self.inner.open_trigger_test(indices)
    }

    fn close_trigger_test(&mut self) -> Result<(), DriverError> {
        self.inner.close_trigger_test()
    }

    fn poll_live_travel_batch(
        &mut self,
        max_samples: usize,
        timeout_ms: i32,
    ) -> Result<Vec<LiveTravelSample>, DriverError> {
        self.inner.poll_live_travel_batch(max_samples, timeout_ms)
    }

    fn start_calibration(&mut self, any_key: bool) -> Result<(), DriverError> {
        self.inner.start_calibration(any_key)
    }

    fn stop_calibration(&mut self, any_key: bool) -> Result<(), DriverError> {
        self.inner.stop_calibration(any_key)
    }

    fn poll_calibration_once(
        &mut self,
        timeout_ms: i32,
    ) -> Result<Option<CalibrationProgress>, DriverError> {
        self.inner.poll_calibration_once(timeout_ms)
    }

    fn read_switch_table(&mut self) -> Result<[u8; SWITCH_TABLE_BYTES], DriverError> {
        self.inner.read_switch_table()
    }

    fn set_switch_profile(&mut self, indices: &[u8], switch_id: u8) -> Result<(), DriverError> {
        self.inner.set_switch_profile(indices, switch_id)
    }

    fn read_deadband_table(&mut self) -> Result<[u8; DEADBAND_TABLE_BYTES], DriverError> {
        self.inner.read_deadband_table()
    }

    fn set_deadband(
        &mut self,
        indices: &[u8],
        top_raw: u8,
        bottom_raw: u8,
    ) -> Result<(), DriverError> {
        self.inner.set_deadband(indices, top_raw, bottom_raw)
    }

    fn read_prcs_power(&mut self) -> Result<bool, DriverError> {
        self.inner.read_prcs_power()
    }

    fn read_prcs_pairs(&mut self) -> Result<Vec<PrcsPair>, DriverError> {
        self.inner.read_prcs_pairs()
    }

    fn set_socd(&mut self, enabled: bool, pairs: &[PrcsPair]) -> Result<(), DriverError> {
        self.inner.set_socd(enabled, pairs)
    }
}
