//! Daemon keyboard driver index and polymorphic [`AulaDriver`] trait.
//!
//! `atriumd` resolves attached Aula keyboards through [`DRIVERS`] so adding
//! another keyboard family (e.g. `f75`, `win68he`, `hero68he`) only requires
//! adding a sibling `<model>.rs` adapter here and appending its [`DriverEntry`].

pub mod win60he;

use std::collections::BTreeMap;

use atrium_core::diagnostics::DiagnosticCheck;
use atrium_core::keyboards::{
    self, DeviceProbe, DriverError, KeyboardSpec, discover_connected,
};
use atrium_core::protocol::{
    BODY_LEN, CalibrationProgress, KEYMAP_BYTES, LightList, LightState, LiveTravelSample,
    MATRIX_SLOTS, PollRate, PrcsPair, SWITCH_TABLE_BYTES, DEADBAND_TABLE_BYTES, TriggerConfig,
    TriggerMode, WinLockFlags, TRIGGER_UNIT_MM_DEFAULT,
};

fn unsupported(spec: &KeyboardSpec, feature: &'static str) -> DriverError {
    DriverError::Unsupported {
        model: spec.display_name,
        feature,
    }
}

/// Unified hardware backend interface owned by `atriumd`.
#[allow(dead_code)]
pub trait AulaDriver {
    fn spec(&self) -> &'static KeyboardSpec;
    fn device_description(&self) -> &str;

    fn unit_mm(&self) -> f64 {
        TRIGGER_UNIT_MM_DEFAULT
    }

    fn all_key_indices(&self) -> Vec<u8> {
        self.spec().keys.iter().map(|k| k.index).collect()
    }

    fn probe(&mut self) -> Result<DeviceProbe, DriverError>;
    fn run_keyboard_checks(&mut self) -> Vec<DiagnosticCheck>;

    fn read_light(&mut self) -> Result<LightState, DriverError> {
        Err(unsupported(self.spec(), "RGB lighting read"))
    }

    fn set_light(&mut self, _state: &LightState) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "RGB lighting control"))
    }

    fn read_light_list(&mut self) -> Result<LightList, DriverError> {
        Err(unsupported(self.spec(), "lighting mode enumeration"))
    }

    fn read_active_custom_slot(&mut self) -> Result<u8, DriverError> {
        Err(unsupported(self.spec(), "custom RGB slots"))
    }

    fn read_custom_light(&mut self, _slot: u8) -> Result<[[u8; 3]; MATRIX_SLOTS], DriverError> {
        Err(unsupported(self.spec(), "per-key custom RGB read"))
    }

    fn set_custom_light(
        &mut self,
        _slot: u8,
        _colors: &[[u8; 3]; MATRIX_SLOTS],
        _brightness: u8,
    ) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "per-key custom RGB write"))
    }

    fn begin_custom_stream(&mut self, _brightness: u8) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "custom RGB streaming"))
    }

    fn stream_custom_light(
        &mut self,
        _slot: u8,
        _colors: &[[u8; 3]; MATRIX_SLOTS],
        _force: bool,
        _drain_first: bool,
    ) -> Result<usize, DriverError> {
        Err(unsupported(self.spec(), "custom RGB streaming"))
    }

    fn read_poll_rate(&mut self) -> Result<u8, DriverError> {
        Err(unsupported(self.spec(), "USB polling rate"))
    }

    fn set_poll_rate(&mut self, _rate: PollRate) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "USB polling rate"))
    }

    fn read_keymap(&mut self, _fn_layer: bool) -> Result<[u8; KEYMAP_BYTES], DriverError> {
        Err(unsupported(self.spec(), "onboard keymap"))
    }

    fn apply_fn_keymap_overrides(
        &mut self,
        _pairs: &[(u8, u8)],
        _clear: &[u8],
    ) -> Result<Vec<(u8, u8)>, DriverError> {
        Err(unsupported(self.spec(), "onboard Fn keymap"))
    }

    fn reset_fn_keymap(&mut self) -> Result<Vec<(u8, u8)>, DriverError> {
        Err(unsupported(self.spec(), "onboard Fn keymap"))
    }

    fn set_win_lock(&mut self, _flags: WinLockFlags) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "Win-lock flags"))
    }

    fn send_raw_checked(&mut self, _body: &[u8; BODY_LEN]) -> Result<Option<Vec<u8>>, DriverError> {
        Err(unsupported(self.spec(), "raw HID pass-through"))
    }

    fn read_trigger_configs(
        &mut self,
        _indices: &[u8],
    ) -> Result<BTreeMap<u8, TriggerConfig>, DriverError> {
        Err(unsupported(self.spec(), "Hall-effect actuation read"))
    }

    fn apply_actuation(
        &mut self,
        _indices: &[u8],
        _mode: TriggerMode,
        _travel_mm: f64,
        _rt_press_mm: Option<f64>,
        _rt_release_mm: Option<f64>,
    ) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "Hall-effect actuation write"))
    }

    fn open_trigger_test(&mut self, _indices: &[u8]) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "live analog key travel"))
    }

    fn close_trigger_test(&mut self) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "live analog key travel"))
    }

    fn poll_live_travel_batch(
        &mut self,
        _max_samples: usize,
        _timeout_ms: i32,
    ) -> Result<Vec<LiveTravelSample>, DriverError> {
        Err(unsupported(self.spec(), "live analog key travel"))
    }

    fn start_calibration(&mut self, _any_key: bool) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "sensor calibration"))
    }

    fn stop_calibration(&mut self, _any_key: bool) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "sensor calibration"))
    }

    fn poll_calibration_once(
        &mut self,
        _timeout_ms: i32,
    ) -> Result<Option<CalibrationProgress>, DriverError> {
        Err(unsupported(self.spec(), "sensor calibration"))
    }

    fn read_switch_table(&mut self) -> Result<[u8; SWITCH_TABLE_BYTES], DriverError> {
        Err(unsupported(self.spec(), "magnetic switch profiles"))
    }

    fn set_switch_profile(&mut self, _indices: &[u8], _switch_id: u8) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "magnetic switch profiles"))
    }

    fn read_deadband_table(&mut self) -> Result<[u8; DEADBAND_TABLE_BYTES], DriverError> {
        Err(unsupported(self.spec(), "Hall-effect deadband"))
    }

    fn set_deadband(
        &mut self,
        _indices: &[u8],
        _top_raw: u8,
        _bottom_raw: u8,
    ) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "Hall-effect deadband"))
    }

    fn read_prcs_power(&mut self) -> Result<bool, DriverError> {
        Err(unsupported(self.spec(), "PRCS / SOCD"))
    }

    fn read_prcs_pairs(&mut self) -> Result<Vec<PrcsPair>, DriverError> {
        Err(unsupported(self.spec(), "PRCS / SOCD"))
    }

    fn set_socd(&mut self, _enabled: bool, _pairs: &[PrcsPair]) -> Result<(), DriverError> {
        Err(unsupported(self.spec(), "PRCS / SOCD"))
    }
}

/// Daemon registration entry linking a core [`KeyboardSpec`] to its live HID backend constructor.
#[derive(Clone, Copy)]
pub struct DriverEntry {
    pub spec: &'static KeyboardSpec,
    pub open: fn() -> Result<Box<dyn AulaDriver>, String>,
}

/// Ordered index of keyboard drivers compiled into `atriumd`.
pub const DRIVERS: &[DriverEntry] = &[win60he::ENTRY];

/// Look up an indexed daemon driver by model slug, alias, or display name.
pub fn find_entry(token: &str) -> Option<&'static DriverEntry> {
    DRIVERS.iter().find(|entry| entry.spec.matches_id(token))
}

/// Resolve the [`KeyboardSpec`] for an optional `--keyboard` override or the default indexed model.
pub fn preferred_spec(preferred_id: Option<&str>) -> &'static KeyboardSpec {
    preferred_id
        .and_then(keyboards::find_by_id)
        .unwrap_or(&keyboards::win60he::SPEC)
}

/// Connect to the explicitly requested keyboard model or auto-discover any attached indexed model.
pub fn connect(preferred_id: Option<&str>) -> Result<Box<dyn AulaDriver>, String> {
    if let Some(id) = preferred_id {
        let entry = find_entry(id).ok_or_else(|| {
            let available: Vec<&str> = DRIVERS.iter().map(|e| e.spec.id).collect();
            format!(
                "unknown Aula keyboard driver '{id}' (indexed: {})",
                available.join(", ")
            )
        })?;
        return (entry.open)();
    }

    for found in discover_connected() {
        if let Some(entry) = DRIVERS.iter().find(|e| e.spec.id == found.spec.id) {
            return (entry.open)();
        }
    }

    let mut last_err = None;
    for entry in DRIVERS {
        match (entry.open)() {
            Ok(drv) => return Ok(drv),
            Err(err) => last_err = Some(err),
        }
    }

    Err(last_err.unwrap_or_else(|| "no indexed Aula keyboard drivers configured".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_driver_index_resolves_win60he() {
        let entry = find_entry("win60he").expect("win60he must be indexed in atriumd");
        assert_eq!(entry.spec.id, "win60he");
        assert!(find_entry("nonexistent_model_xyz").is_none());
    }
}
