//! High-level Aula WIN 60 HE driver built on top of [`HidTransport`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

pub use crate::keyboards::{DeviceProbe, DriverError};
use super::keymap;
use super::protocol::{
    self, DEADBAND_TABLE_BYTES, InitInfo, KEYMAP_BYTES, KEYMAP_CHUNK, LightList, LightState,
    MATRIX_SLOTS, MaxTriggerTravel, PRCS_BYTES, PRCS_CHUNK, PagedChunk, PollRate, PrcsPair,
    ProtocolError, Report, SWITCH_TABLE_BYTES, TABLE_CHUNK_58, TriggerConfig, TriggerMode,
    WinLockFlags, assemble_paged_table, parse_paged_chunk, strip_report_id,
};
use crate::transport::HidTransport;

const DEFAULT_QUERY_TIMEOUT: Duration = Duration::from_millis(400);
const DEFAULT_TABLE_TIMEOUT: Duration = Duration::from_millis(1500);
const DEFAULT_TRIGGER_KEY_TIMEOUT: Duration = Duration::from_millis(60);
const INTER_PACKET_DELAY: Duration = Duration::from_millis(5);
const FACTORY_DEFAULT_RT_RAW: u16 = 1;

pub struct Win60Driver<T: HidTransport> {
    transport: T,
    last_stream_slot: Option<u8>,
    last_stream_packets: Option<Vec<Report>>,
}

impl<T: HidTransport> Win60Driver<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            last_stream_slot: None,
            last_stream_packets: None,
        }
    }

    pub fn device_description(&self) -> &str {
        self.transport.device_description()
    }

    pub fn into_inner(self) -> T {
        self.transport
    }

    /// Perform a non-destructive connect handshake & capability readout.
    pub fn probe(&mut self) -> Result<DeviceProbe, DriverError> {
        let init_info = self.read_init_info().ok();
        let high_precision = init_info.as_ref().is_some_and(|i| i.is_high_precision);
        let (firmware_string, build_date) = self.read_device_info().unwrap_or((None, None));
        let max_travel = self.read_max_trigger_travel(high_precision).ok();
        let poll_rate_khz = self.read_poll_rate().ok();
        let light_list = self.read_light_list().ok();
        let light_state = self.read_light().ok();
        let switch_list = self.read_switch_list().ok();
        let prcs_enabled = self.read_prcs_power().ok();

        Ok(DeviceProbe {
            keyboard_id: Some(super::SPEC.id.to_string()),
            model_name: Some(super::SPEC.display_name.to_string()),
            device_description: self.transport.device_description().to_string(),
            firmware_string,
            build_date,
            init_info,
            max_travel,
            poll_rate_khz,
            light_list,
            light_state,
            switch_list,
            prcs_enabled,
        })
    }

    /// Send heartbeat (`0x01`) and parse capability flags.
    pub fn read_init_info(&mut self) -> Result<InitInfo, DriverError> {
        self.query_single(
            &protocol::build_heartbeat(),
            DEFAULT_QUERY_TIMEOUT,
            "initInfo (cmd 0x01)",
            protocol::parse_init_info,
        )
    }

    /// Query firmware identifier & build date (`0x0d`).
    pub fn read_device_info(&mut self) -> Result<(Option<String>, Option<String>), DriverError> {
        self.transport.drain()?;
        self.transport
            .write_report(&protocol::build_device_info_query())?;

        let mut fw: Option<String> = None;
        let mut date: Option<String> = None;
        let deadline = Instant::now() + DEFAULT_QUERY_TIMEOUT;

        while Instant::now() < deadline && (fw.is_none() || date.is_none()) {
            let Some(raw) = self.transport.read_report(25)? else {
                continue;
            };
            let Some(body) = strip_report_id(&raw) else {
                continue;
            };
            if let Some(chunk) = protocol::parse_device_info_chunk(body) {
                match chunk.index {
                    0 => fw = Some(chunk.text),
                    1 => date = Some(chunk.text),
                    _ => {}
                }
            }
        }

        if fw.is_none() && date.is_none() {
            return Err(DriverError::Timeout("device info (cmd 0x0d)"));
        }
        Ok((fw, date))
    }

    /// Query device-reported travel unit and min/max travel (`0x21` sub `0x04`).
    pub fn read_max_trigger_travel(
        &mut self,
        high_precision: bool,
    ) -> Result<MaxTriggerTravel, DriverError> {
        self.query_single(
            &protocol::build_read_max_trigger_travel(),
            DEFAULT_QUERY_TIMEOUT,
            "max trigger travel (cmd 0x21 sub 0x04)",
            |body| protocol::parse_max_trigger_travel(body, high_precision),
        )
    }

    /// Query current USB polling rate in kHz (`0x21` sub `0x09`).
    pub fn read_poll_rate(&mut self) -> Result<u8, DriverError> {
        self.query_single(
            &protocol::build_read_poll_rate(),
            DEFAULT_QUERY_TIMEOUT,
            "poll rate (cmd 0x21 sub 0x09)",
            protocol::parse_poll_rate,
        )
    }

    /// Change USB polling rate (`0x21` sub `0x09`). Note: the keyboard restarts on change.
    pub fn set_poll_rate(&mut self, rate: PollRate) -> Result<(), DriverError> {
        self.transport
            .write_report(&protocol::build_set_poll_rate(rate))?;
        Ok(())
    }

    /// Query supported firmware lighting modes & max limits (`0x0a`).
    pub fn read_light_list(&mut self) -> Result<LightList, DriverError> {
        self.query_single(
            &protocol::build_read_light_list(),
            DEFAULT_QUERY_TIMEOUT,
            "lighting mode list (cmd 0x0a)",
            protocol::parse_light_list,
        )
    }

    /// Read current keyboard lighting state (`0x07` sub `0x01`).
    pub fn read_light(&mut self) -> Result<LightState, DriverError> {
        self.query_single(
            &protocol::build_read_light(),
            DEFAULT_QUERY_TIMEOUT,
            "lighting state (cmd 0x07 sub 0x01)",
            protocol::parse_light_state,
        )
    }

    /// Apply lighting mode/colors/brightness/speed (`0x17` + `0x07`).
    pub fn set_light(&mut self, state: &LightState) -> Result<(), DriverError> {
        self.invalidate_custom_stream();
        if state.mode != 100 {
            self.transport
                .write_report(&protocol::build_music_rhythm(false))?;
            thread::sleep(INTER_PACKET_DELAY);
        }
        self.transport.write_report(&protocol::build_set_light(state))?;
        if state.mode == 100 {
            thread::sleep(INTER_PACKET_DELAY);
            self.transport
                .write_report(&protocol::build_music_rhythm(true))?;
        }
        Ok(())
    }

    fn custom_mode_light_state(brightness: u8) -> LightState {
        LightState {
            mode: 10,
            brightness: brightness.min(protocol::LIGHT_MAX_LEVEL),
            speed: protocol::LIGHT_MAX_LEVEL,
            fg: [255, 255, 255],
            bg: [0, 0, 0],
            direction: 0,
            full_color: false,
            power_on: true,
        }
    }

    fn invalidate_custom_stream(&mut self) {
        self.last_stream_slot = None;
        self.last_stream_packets = None;
    }

    /// Query supported switch profile IDs (`0x25` sub `0x00`).
    pub fn read_switch_list(&mut self) -> Result<Vec<u8>, DriverError> {
        self.query_single(
            &protocol::build_read_switch_list(),
            DEFAULT_QUERY_TIMEOUT,
            "switch profile list (cmd 0x25 sub 0x00)",
            protocol::parse_switch_list,
        )
    }

    /// Read the 132-byte per-key switch profile table (`0x25` sub `0x02`).
    pub fn read_switch_table(&mut self) -> Result<[u8; SWITCH_TABLE_BYTES], DriverError> {
        let bytes = self.read_paged_table(
            &protocol::build_read_switch_table(),
            protocol::cmd::SWITCH_PROFILE,
            0x02,
            SWITCH_TABLE_BYTES,
            TABLE_CHUNK_58,
            DEFAULT_TABLE_TIMEOUT,
            "switch profile table (cmd 0x25 sub 0x02)",
        )?;
        Ok(bytes
            .try_into()
            .expect("assemble_paged_table matches SWITCH_TABLE_BYTES"))
    }

    /// Read-modify-write switch profiles for the given matrix indices.
    pub fn set_switch_profile(
        &mut self,
        indices: &[u8],
        switch_id: u8,
    ) -> Result<(), DriverError> {
        let mut table = self
            .read_switch_table()
            .map_err(|_| DriverError::ReadModifyWriteAborted("switch table read failed"))?;
        for &idx in indices {
            if let Some(slot) = table.get_mut(idx as usize) {
                *slot = switch_id;
            }
        }
        for pkt in protocol::build_switch_table(&table) {
            self.transport.write_report(&pkt)?;
            thread::sleep(INTER_PACKET_DELAY);
        }
        Ok(())
    }

    /// Read the 264-byte per-key deadband table `[top, bottom] * 132` (`0x26` sub `0x00`).
    pub fn read_deadband_table(&mut self) -> Result<[u8; DEADBAND_TABLE_BYTES], DriverError> {
        let bytes = self.read_paged_table(
            &protocol::build_read_deadband(),
            protocol::cmd::DEADBAND,
            0x00,
            DEADBAND_TABLE_BYTES,
            TABLE_CHUNK_58,
            DEFAULT_TABLE_TIMEOUT,
            "deadband table (cmd 0x26 sub 0x00)",
        )?;
        Ok(bytes
            .try_into()
            .expect("assemble_paged_table matches DEADBAND_TABLE_BYTES"))
    }

    /// Read-modify-write per-key top/bottom deadbands (in 0.01 mm units).
    pub fn set_deadband(
        &mut self,
        indices: &[u8],
        top_raw: u8,
        bottom_raw: u8,
    ) -> Result<(), DriverError> {
        let mut table = self
            .read_deadband_table()
            .map_err(|_| DriverError::ReadModifyWriteAborted("deadband table read failed"))?;
        for &idx in indices {
            let off = (idx as usize) * 2;
            if off + 1 < table.len() {
                table[off] = top_raw;
                table[off + 1] = bottom_raw;
            }
        }
        for pkt in protocol::build_deadband_table(&table) {
            self.transport.write_report(&pkt)?;
            thread::sleep(INTER_PACKET_DELAY);
        }
        Ok(())
    }

    /// Read stored trigger configuration for each requested matrix index (`0x21` sub `0x05`).
    pub fn read_trigger_configs(
        &mut self,
        indices: &[u8],
    ) -> Result<BTreeMap<u8, TriggerConfig>, DriverError> {
        self.transport.drain()?;
        let mut out = BTreeMap::new();

        for &idx in indices {
            self.transport
                .write_report(&protocol::build_read_trigger_config(idx))?;
            let deadline = Instant::now() + DEFAULT_TRIGGER_KEY_TIMEOUT;
            while Instant::now() < deadline {
                let Some(raw) = self.transport.read_report(10)? else {
                    continue;
                };
                let Some(body) = strip_report_id(&raw) else {
                    continue;
                };
                if let Some(cfg) = protocol::parse_trigger_config(body)
                    && cfg.index == idx
                {
                    out.insert(idx, cfg);
                    break;
                }
            }
        }

        Ok(out)
    }

    /// Configure actuation travel and optional Rapid Trigger mode for `indices`.
    ///
    /// When `mode == TriggerMode::Fixed` and both `rt_press_mm` and
    /// `rt_release_mm` are `None`, reads back each key's stored RT intervals
    /// first and preserves them (matching the vendor driver's behavior).
    pub fn set_actuation(
        &mut self,
        indices: &[u8],
        mode: TriggerMode,
        travel_mm: f64,
        rt_press_mm: Option<f64>,
        rt_release_mm: Option<f64>,
        limits: Option<MaxTriggerTravel>,
    ) -> Result<(), DriverError> {
        let (unit_mm, min_mm, max_mm) = match limits {
            Some(l) => (l.unit_mm, l.min_mm, l.max_mm),
            None => (protocol::TRIGGER_UNIT_MM_DEFAULT, 0.08, 3.40),
        };
        if !(min_mm..=max_mm).contains(&travel_mm) {
            return Err(DriverError::TravelOutOfRange {
                value: travel_mm,
                min: min_mm,
                max: max_mm,
            });
        }
        let travel_raw = protocol::mm_to_raw(travel_mm, unit_mm);

        match mode {
            TriggerMode::Fixed if rt_press_mm.is_none() && rt_release_mm.is_none() => {
                let existing = self.read_trigger_configs(indices)?;
                let mut groups: BTreeMap<(u16, u16), Vec<u8>> = BTreeMap::new();
                for &idx in indices {
                    let pair = existing.get(&idx).map_or(
                        (FACTORY_DEFAULT_RT_RAW, FACTORY_DEFAULT_RT_RAW),
                        |c| (c.interval1_raw, c.interval2_raw),
                    );
                    groups.entry(pair).or_default().push(idx);
                }
                let multiple = groups.len() > 1;
                for ((i1, i2), group_keys) in groups {
                    let pkt = protocol::build_trigger_raw(
                        TriggerMode::Fixed,
                        &group_keys,
                        travel_raw,
                        i1,
                        i2,
                    )?;
                    self.transport.write_report(&pkt)?;
                    if multiple {
                        thread::sleep(INTER_PACKET_DELAY);
                    }
                }
            }
            TriggerMode::RapidTrigger => {
                let rt = rt_press_mm.or(rt_release_mm).unwrap_or(0.30);
                let rt_raw = protocol::mm_to_raw(rt, unit_mm);
                let pkt = protocol::build_trigger_raw(mode, indices, travel_raw, rt_raw, rt_raw)?;
                self.transport.write_report(&pkt)?;
            }
            _ => {
                let i1_raw = protocol::mm_to_raw(rt_press_mm.unwrap_or(0.01), unit_mm);
                let i2_raw = protocol::mm_to_raw(rt_release_mm.unwrap_or(0.01), unit_mm);
                let pkt =
                    protocol::build_trigger_raw(mode, indices, travel_raw, i1_raw, i2_raw)?;
                self.transport.write_report(&pkt)?;
            }
        }

        Ok(())
    }

    /// Read a keymap layer (`false` = base, `true` = Fn) of 528 bytes (`0x18`).
    pub fn read_keymap(&mut self, fn_layer: bool) -> Result<[u8; KEYMAP_BYTES], DriverError> {
        let sub = if fn_layer { 0x82 } else { 0x80 };
        let bytes = self.read_paged_table(
            &protocol::build_read_keymap(fn_layer),
            protocol::cmd::KEYMAP,
            sub,
            KEYMAP_BYTES,
            KEYMAP_CHUNK,
            DEFAULT_TABLE_TIMEOUT,
            if fn_layer {
                "Fn keymap (cmd 0x18 sub 0x82)"
            } else {
                "base keymap (cmd 0x18 sub 0x80)"
            },
        )?;
        Ok(bytes
            .try_into()
            .expect("assemble_paged_table matches KEYMAP_BYTES"))
    }

    fn write_paged_reports(&mut self, pkts: Vec<Report>) -> Result<(), DriverError> {
        for pkt in pkts {
            self.transport.write_report(&pkt)?;
            thread::sleep(INTER_PACKET_DELAY);
        }
        Ok(())
    }

    /// Write both base (`0x00`) and Fn (`0x02`) 528-byte keymaps via `cmd 0x18`.
    ///
    /// Writing base followed immediately by Fn ensures the firmware exits keymap
    /// programming cleanly rather than remaining stuck in Fn edit mode.
    pub fn write_keymap_layers(
        &mut self,
        base: &[u8; KEYMAP_BYTES],
        fn_table: &[u8; KEYMAP_BYTES],
    ) -> Result<(), DriverError> {
        self.write_paged_reports(protocol::build_write_keymap_layer(false, base))?;
        self.write_paged_reports(protocol::build_write_keymap_layer(true, fn_table))
    }

    fn update_fn_keymap_with(
        &mut self,
        edit: impl FnOnce(&mut [u8; KEYMAP_BYTES]) -> Result<(), DriverError>,
    ) -> Result<Vec<(u8, u8)>, DriverError> {
        // Read BOTH base and Fn layers before sending any write report so
        // partial reads never risk overwriting active flash bindings.
        let base_existing = self.read_keymap(false).map_err(|_| {
            DriverError::ReadModifyWriteAborted("base keymap (cmd 0x18 sub 0x80)")
        })?;
        let mut fn_table = self.read_keymap(true).map_err(|_| {
            DriverError::ReadModifyWriteAborted("Fn keymap (cmd 0x18 sub 0x82)")
        })?;

        edit(&mut fn_table)?;
        let base_composed = protocol::compose_base_keymap(&base_existing);
        self.write_keymap_layers(&base_composed, &fn_table)?;
        Ok(protocol::decode_fn_keymap_overrides(&fn_table))
    }

    /// Safely apply `(slot_index, hid_usage)` additions and clear slots on the hardware Fn layer (`0x18`).
    pub fn apply_fn_keymap_overrides(
        &mut self,
        set: &[(u8, u8)],
        clear: &[u8],
    ) -> Result<Vec<(u8, u8)>, DriverError> {
        self.update_fn_keymap_with(|fn_table| {
            protocol::apply_fn_table_edits(fn_table, set, clear)?;
            Ok(())
        })
    }

    /// Clear all remapped bindings on the hardware Fn layer (`0x18`) while preserving base keys.
    pub fn reset_fn_keymap(&mut self) -> Result<Vec<(u8, u8)>, DriverError> {
        self.update_fn_keymap_with(|fn_table| {
            fn_table.fill(0);
            Ok(())
        })
    }

    /// Read PRCS/SOCD enabled state (`0x24` sub `0x02`).
    pub fn read_prcs_power(&mut self) -> Result<bool, DriverError> {
        self.query_single(
            &protocol::build_read_prcs_power(),
            DEFAULT_QUERY_TIMEOUT,
            "PRCS/SOCD power state (cmd 0x24 sub 0x02)",
            protocol::parse_prcs_power,
        )
    }

    /// Read configured PRCS/SOCD key pairs (`0x24` sub `0x01`).
    pub fn read_prcs_pairs(&mut self) -> Result<Vec<PrcsPair>, DriverError> {
        let raw = self.read_paged_table(
            &protocol::build_read_prcs_data(),
            protocol::cmd::PRCS,
            0x01,
            PRCS_BYTES,
            PRCS_CHUNK,
            DEFAULT_TABLE_TIMEOUT,
            "PRCS/SOCD table (cmd 0x24 sub 0x01)",
        )?;
        Ok(protocol::parse_prcs_table(&raw))
    }

    /// Configure PRCS/SOCD power and key pairs (`0x24`).
    pub fn set_socd(&mut self, enabled: bool, pairs: &[PrcsPair]) -> Result<(), DriverError> {
        let pkts = protocol::build_prcs_packets(pairs)?;
        self.transport
            .write_report(&protocol::build_prcs_power(enabled))?;
        thread::sleep(INTER_PACKET_DELAY);
        for pkt in pkts {
            self.transport.write_report(&pkt)?;
            thread::sleep(INTER_PACKET_DELAY);
        }
        Ok(())
    }

    /// Write a custom per-key RGB palette (`slot` 0 or 1) and activate mode 10.
    pub fn set_custom_light(
        &mut self,
        slot: u8,
        colors: &[[u8; 3]; MATRIX_SLOTS],
        brightness: u8,
    ) -> Result<(), DriverError> {
        let pkts = protocol::build_custom_light_packets(colors, slot)?;
        let light = Self::custom_mode_light_state(brightness);
        self.set_light(&light)?;
        thread::sleep(INTER_PACKET_DELAY);
        for pkt in pkts {
            self.transport.write_report(&pkt)?;
            thread::sleep(INTER_PACKET_DELAY);
        }
        Ok(())
    }

    /// Enter Custom mode (`mode = 10`) once so subsequent frames can stream without mode resets.
    pub fn begin_custom_stream(&mut self, brightness: u8) -> Result<(), DriverError> {
        let light = Self::custom_mode_light_state(brightness);
        self.set_light(&light)?;
        thread::sleep(INTER_PACKET_DELAY);
        self.transport.drain()?;
        Ok(())
    }

    /// Stream one per-key RGB frame (`cmd 0x09`), sending only changed 54-byte pages unless `force`.
    ///
    /// Returns the number of HID output packets written to the device.
    pub fn stream_custom_light(
        &mut self,
        slot: u8,
        colors: &[[u8; 3]; MATRIX_SLOTS],
        force: bool,
        drain_acks: bool,
    ) -> Result<usize, DriverError> {
        let pkts = protocol::build_custom_light_packets(colors, slot)?;
        let prev = if !force && self.last_stream_slot == Some(slot) {
            self.last_stream_packets.as_deref()
        } else {
            None
        };

        let mut sent = 0usize;
        for (idx, pkt) in pkts.iter().enumerate() {
            if prev.and_then(|p| p.get(idx)) != Some(pkt) {
                self.transport.write_report(pkt)?;
                sent += 1;
            }
        }
        self.last_stream_slot = Some(slot);
        self.last_stream_packets = Some(pkts);

        if drain_acks && sent > 0 {
            for _ in 0..32 {
                if self.transport.read_report(0)?.is_none() {
                    break;
                }
            }
        }
        Ok(sent)
    }

    /// Open live analog travel streaming for `indices` (`0x21` sub `0x02`).
    pub fn open_trigger_test(&mut self, indices: &[u8]) -> Result<(), DriverError> {
        self.transport.drain()?;
        self.transport
            .write_report(&protocol::build_open_trigger_test(indices)?)?;
        Ok(())
    }

    /// Close live analog travel streaming (`0x21` sub `0x03`).
    pub fn close_trigger_test(&mut self) -> Result<(), DriverError> {
        self.transport
            .write_report(&protocol::build_close_trigger_test())?;
        Ok(())
    }

    /// Poll a single live analog travel frame with the given timeout.
    pub fn poll_live_travel_once(
        &mut self,
        timeout_ms: i32,
    ) -> Result<Option<protocol::LiveTravelSample>, DriverError> {
        let Some(raw) = self.transport.read_report(timeout_ms)? else {
            return Ok(None);
        };
        let Some(body) = strip_report_id(&raw) else {
            return Ok(None);
        };
        Ok(protocol::parse_live_travel(body))
    }

    /// Poll up to `max_frames` queued HID input reports, extracting all live travel samples while
    /// safely skipping intervening command ACKs (e.g. `cmd 0x09` write echoes).
    pub fn poll_live_travel_batch(
        &mut self,
        max_frames: usize,
        first_timeout_ms: i32,
    ) -> Result<Vec<protocol::LiveTravelSample>, DriverError> {
        let mut samples = Vec::new();
        for i in 0..max_frames {
            let timeout = if i == 0 { first_timeout_ms } else { 0 };
            let Some(raw) = self.transport.read_report(timeout)? else {
                break;
            };
            if let Some(body) = strip_report_id(&raw)
                && let Some(sample) = protocol::parse_live_travel(body)
            {
                samples.push(sample);
            }
        }
        Ok(samples)
    }

    /// Read which custom RGB palette slot (0 or 1) is currently active (`0x09` sub `0x20`).
    pub fn read_active_custom_slot(&mut self) -> Result<u8, DriverError> {
        self.query_single(
            &protocol::build_read_custom_number(),
            DEFAULT_QUERY_TIMEOUT,
            "active custom slot (cmd 0x09 sub 0x20)",
            protocol::parse_custom_number,
        )
    }

    /// Arm Hall-effect key calibration (`0x21` sub `8` or `15`).
    pub fn start_calibration(&mut self, any_key: bool) -> Result<(), DriverError> {
        self.transport.drain()?;
        self.transport
            .write_report(&protocol::build_calibration(true, any_key))?;
        Ok(())
    }

    /// Disarm Hall-effect key calibration (`0x21` sub `8/1` or `16/0`).
    pub fn stop_calibration(&mut self, any_key: bool) -> Result<(), DriverError> {
        self.transport
            .write_report(&protocol::build_calibration(false, any_key))?;
        Ok(())
    }

    /// Poll a single calibration status/progress packet (`0x21`, sub `8 | 15`).
    pub fn poll_calibration_once(
        &mut self,
        timeout_ms: i32,
    ) -> Result<Option<protocol::CalibrationProgress>, DriverError> {
        let Some(raw) = self.transport.read_report(timeout_ms)? else {
            return Ok(None);
        };
        let Some(body) = strip_report_id(&raw) else {
            return Ok(None);
        };
        Ok(protocol::parse_calibration_reply(body))
    }

    /// Configure Win-lock / shortcut lock flags (`0x14`).
    pub fn set_win_lock(&mut self, flags: WinLockFlags) -> Result<(), DriverError> {
        self.transport
            .write_report(&protocol::build_win_lock(flags))?;
        Ok(())
    }

    /// Send a raw 63-byte body (wrapped with Report ID `0x01`), after verifying
    /// it does not trigger destructive commands.
    pub fn send_raw_checked(&mut self, body: &[u8; 63]) -> Result<Option<Vec<u8>>, DriverError> {
        if let Some(reason) = protocol::dangerous_command_reason(body) {
            return Err(DriverError::Protocol(ProtocolError::DangerousCommand(
                reason,
            )));
        }
        self.transport.drain()?;
        self.transport.write_report(&protocol::wrap_report(body))?;
        Ok(self.transport.read_report(200)?)
    }

    /// Stream live Hall-effect travel depth samples (`0x21` sub `0x02`..`0x03`).
    pub fn stream_travel<F>(
        &mut self,
        indices: &[u8],
        duration: Option<Duration>,
        stop_flag: Arc<AtomicBool>,
        mut on_sample: F,
    ) -> Result<(), DriverError>
    where
        F: FnMut(protocol::LiveTravelSample),
    {
        self.transport.drain()?;
        self.transport
            .write_report(&protocol::build_open_trigger_test(indices)?)?;

        let start = Instant::now();
        let mut loop_res = Ok(());

        while !stop_flag.load(Ordering::Relaxed) {
            if let Some(limit) = duration
                && start.elapsed() >= limit
            {
                break;
            }
            match self.transport.read_report(25) {
                Ok(Some(raw)) => {
                    if let Some(body) = strip_report_id(&raw)
                        && let Some(sample) = protocol::parse_live_travel(body)
                    {
                        on_sample(sample);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    loop_res = Err(DriverError::Transport(e));
                    break;
                }
            }
        }

        // Always send closeTriggerTest so the firmware does not keep streaming.
        let close_res = self
            .transport
            .write_report(&protocol::build_close_trigger_test());
        loop_res?;
        close_res?;
        Ok(())
    }

    /// Format a matrix index into a friendly label like `"W (46)"`.
    pub fn format_key(index: u8) -> String {
        match keymap::by_index(index) {
            Some(k) => format!("{} ({})", k.name, k.index),
            None => format!("slot {index}"),
        }
    }

    /// Read custom per-key RGB slot (`0` or `1`) back (`0x09`).
    pub fn read_custom_light(&mut self, slot: u8) -> Result<[[u8; 3]; MATRIX_SLOTS], DriverError> {
        let req = protocol::build_read_custom_light(slot)?;
        let raw = self.read_paged_table(
            &req,
            protocol::cmd::CUSTOM_LIGHT,
            0x80 | slot,
            protocol::CUSTOM_LIGHT_BYTES,
            protocol::CUSTOM_LIGHT_CHUNK,
            DEFAULT_TABLE_TIMEOUT,
            "custom light table (cmd 0x09)",
        )?;
        let mut out = [[0u8; 3]; MATRIX_SLOTS];
        for (i, rgb) in raw.chunks_exact(3).enumerate() {
            out[i] = [rgb[0], rgb[1], rgb[2]];
        }
        Ok(out)
    }

    fn query_single<R, P>(
        &mut self,
        request: &Report,
        timeout: Duration,
        label: &'static str,
        mut parser: P,
    ) -> Result<R, DriverError>
    where
        P: FnMut(&[u8]) -> Option<R>,
    {
        self.transport.drain()?;
        self.transport.write_report(request)?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let Some(raw) = self.transport.read_report(20)? else {
                continue;
            };
            let Some(body) = strip_report_id(&raw) else {
                continue;
            };
            if let Some(parsed) = parser(body) {
                return Ok(parsed);
            }
        }
        Err(DriverError::Timeout(label))
    }

    #[allow(clippy::too_many_arguments)]
    fn read_paged_table(
        &mut self,
        request: &Report,
        expected_cmd: u8,
        expected_sub: u8,
        total_bytes: usize,
        stride: usize,
        timeout: Duration,
        label: &'static str,
    ) -> Result<Vec<u8>, DriverError> {
        self.transport.drain()?;
        self.transport.write_report(request)?;

        let n_pages = total_bytes.div_ceil(stride);
        let mut chunks: Vec<PagedChunk> = Vec::with_capacity(n_pages);
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            if let Some(table) = assemble_paged_table(&chunks, total_bytes, stride) {
                return Ok(table);
            }
            let Some(raw) = self.transport.read_report(20)? else {
                continue;
            };
            let Some(body) = strip_report_id(&raw) else {
                continue;
            };
            if let Some(ch) = parse_paged_chunk(body, expected_cmd, expected_sub)
                && !chunks.iter().any(|existing| existing.page == ch.page)
            {
                chunks.push(ch);
                if let Some(table) = assemble_paged_table(&chunks, total_bytes, stride) {
                    return Ok(table);
                }
            }
        }

        Err(DriverError::Timeout(label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use crate::transport::TransportError;

    struct MockTransport {
        tx: Vec<Report>,
        rx: VecDeque<Vec<u8>>,
        fail_reads: bool,
    }

    impl MockTransport {
        fn new() -> Self {
            Self {
                tx: Vec::new(),
                rx: VecDeque::new(),
                fail_reads: false,
            }
        }

        fn push_body(&mut self, body: &[u8]) {
            let mut full = [0u8; 64];
            full[0] = protocol::REPORT_ID;
            let len = body.len().min(63);
            full[1..1 + len].copy_from_slice(&body[..len]);
            self.rx.push_back(full.to_vec());
        }
    }

    impl HidTransport for MockTransport {
        fn write_report(&mut self, report: &Report) -> Result<(), TransportError> {
            self.tx.push(*report);
            Ok(())
        }

        fn read_report(&mut self, _timeout_ms: i32) -> Result<Option<Vec<u8>>, TransportError> {
            if self.fail_reads {
                return Ok(None);
            }
            Ok(self.rx.pop_front())
        }

        fn drain(&mut self) -> Result<(), TransportError> {
            Ok(())
        }

        fn device_description(&self) -> &str {
            "Mock WIN 60 HE"
        }
    }

    #[test]
    fn fixed_actuation_preserves_stored_rt_intervals() {
        let mut mock = MockTransport::new();
        // Reply for key 46 (W, row=2, col=2) with stored intervals 0x32 / 0x23
        mock.push_body(&[
            0x21, 0x00, 0x00, 0x00, 0x0c, 0x05, 0x0c, 0xaa, 0xaa, 0x32, 0x23, 0x00, 0x00, 0x00,
            0x00, 2, 2,
        ]);

        let mut drv = Win60Driver::new(mock);
        drv.set_actuation(&[46], TriggerMode::Fixed, 1.50, None, None, None)
            .unwrap();

        let mock = drv.into_inner();
        assert_eq!(mock.tx.len(), 2);
        // First packet: readTriggerData for key 46
        assert_eq!(mock.tx[0][1], 0x21);
        assert_eq!(mock.tx[0][6], 0x05);
        // Second packet: trigger write preserving interval 0x32/0x23 and travel 150 (0x96)
        let write = mock.tx[1];
        assert_eq!(write[6], 0x00); // Fixed mode
        assert_eq!(write[29], 150);
        assert_eq!(write[30], 150);
        assert_eq!(write[31], 0x32);
        assert_eq!(write[32], 0x23);
    }

    #[test]
    fn deadband_rmw_aborts_when_read_fails() {
        let mut mock = MockTransport::new();
        mock.fail_reads = true;
        let mut drv = Win60Driver::new(mock);
        let err = drv.set_deadband(&[46], 2, 2).unwrap_err();
        assert!(matches!(err, DriverError::ReadModifyWriteAborted(_)));
    }

    #[test]
    fn set_light_clears_music_rhythm_before_cmd_7() {
        let mock = MockTransport::new();
        let mut drv = Win60Driver::new(mock);
        drv.set_light(&LightState::default()).unwrap();
        let mock = drv.into_inner();
        assert_eq!(mock.tx.len(), 2);
        assert_eq!(mock.tx[0][1], protocol::cmd::MUSIC_RHYTHM);
        assert_eq!(mock.tx[0][6], 0);
        assert_eq!(mock.tx[1][1], protocol::cmd::LIGHT);
    }

    #[test]
    fn fn_keymap_rmw_aborts_when_read_fails() {
        let mut mock = MockTransport::new();
        mock.fail_reads = true;
        let mut drv = Win60Driver::new(mock);
        let err = drv.apply_fn_keymap_overrides(&[(48, 0x52)], &[]).unwrap_err();
        assert!(matches!(err, DriverError::ReadModifyWriteAborted(_)));
        let mock = drv.into_inner();
        assert_eq!(mock.tx.len(), 1);
    }

    #[test]
    fn fn_keymap_rmw_preserves_fn_bit_and_writes_both_layers() {
        let mut mock = MockTransport::new();
        let zeros = [0u8; KEYMAP_BYTES];
        for pkt in protocol::paged_write_packets(protocol::cmd::KEYMAP, 0x80, &zeros, KEYMAP_CHUNK)
        {
            mock.push_body(&pkt[1..]);
        }
        for pkt in protocol::paged_write_packets(protocol::cmd::KEYMAP, 0x82, &zeros, KEYMAP_CHUNK)
        {
            mock.push_body(&pkt[1..]);
        }

        let mut drv = Win60Driver::new(mock);
        let overrides = drv.apply_fn_keymap_overrides(&[(48, 0x52)], &[]).unwrap();
        assert_eq!(overrides, vec![(48, 0x52)]);

        let mock = drv.into_inner();
        let base_writes: Vec<&Report> = mock
            .tx
            .iter()
            .filter(|r| r[1] == protocol::cmd::KEYMAP && r[2] == 0x00)
            .collect();
        let fn_writes: Vec<&Report> = mock
            .tx
            .iter()
            .filter(|r| r[1] == protocol::cmd::KEYMAP && r[2] == 0x02)
            .collect();
        assert_eq!(base_writes.len(), 10);
        assert_eq!(fn_writes.len(), 10);

        let mut base_assembled = [0u8; KEYMAP_BYTES];
        for (i, r) in base_writes.iter().enumerate() {
            let len = usize::from(r[5]);
            let off = i * KEYMAP_CHUNK;
            base_assembled[off..off + len].copy_from_slice(&r[6..6 + len]);
        }
        let fn_slot = usize::from(crate::keymap::idx::FN) * 4;
        assert_eq!(base_assembled[fn_slot..fn_slot + 4], [1, 0xFA, 0, 0]);
        let r_slot = usize::from(crate::keymap::idx::R) * 4;
        assert_eq!(base_assembled[r_slot..r_slot + 4], [0, 0x15, 0, 0]);
    }

    #[test]
    fn stream_custom_light_sends_only_dirty_chunks() {
        let mock = MockTransport::new();
        let mut drv = Win60Driver::new(mock);
        let mut grid = [[0u8; 3]; MATRIX_SLOTS];
        grid[22] = [10, 20, 30];

        let sent1 = drv.stream_custom_light(0, &grid, false, false).unwrap();
        assert_eq!(sent1, 8);

        let sent2 = drv.stream_custom_light(0, &grid, false, false).unwrap();
        assert_eq!(sent2, 0);

        // Slot 46 * 3 = byte 138 -> chunk index 138 / 54 = 2.
        grid[46] = [200, 100, 50];
        let sent3 = drv.stream_custom_light(0, &grid, false, false).unwrap();
        assert_eq!(sent3, 1);

        let mock = drv.into_inner();
        assert_eq!(mock.tx.len(), 9);
        assert_eq!(mock.tx[8][1], protocol::cmd::CUSTOM_LIGHT);
        assert_eq!(mock.tx[8][4], 2); // chunk page 2
    }

    #[test]
    fn poll_live_travel_batch_skips_non_travel_acks() {
        let mut mock = MockTransport::new();
        // Precede live travel frame with two cmd-0x09 ACK echoes.
        mock.push_body(&[protocol::cmd::CUSTOM_LIGHT, 0, 0, 0, 0]);
        mock.push_body(&[protocol::cmd::CUSTOM_LIGHT, 0, 0, 0, 0]);
        mock.push_body(&[
            protocol::cmd::TRIGGER,
            0,
            0,
            0,
            5,
            1,
            2,
            2,
            150,
            0,
        ]);

        let mut drv = Win60Driver::new(mock);
        let samples = drv.poll_live_travel_batch(16, 0).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].index, 46); // row 2 * 22 + col 2 = W
        assert_eq!(samples[0].travel_raw, 150);
    }
}

