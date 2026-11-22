//! Wire protocol codec for the Aula WIN 60 HE (`2E3C:C365`, Usage Page `0xFF1B`).
//!
//! All vendor config frames use Report ID `0x01` followed by a 63-byte body
//! (64 bytes total on the wire). Framing envelope:
//! - `body[0]`: command byte
//! - `body[1]`: sub-selector / slot / layer / read-flag
//! - `body[2..4]`: chunk index (big-endian u16)
//! - `body[4]`: payload length for this chunk (except `0x21` and `0x14` families)
//! - `body[5..]`: payload bytes

use std::collections::BTreeSet;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const VID: u16 = 0x2E3C;
pub const PID: u16 = 0xC365;
pub const VENDOR_USAGE_PAGE: u16 = 0xFF1B;
pub const VENDOR_USAGE: u16 = 0x91;
pub const VENDOR_INTERFACE_FALLBACK: i32 = 2;

pub const REPORT_ID: u8 = 0x01;
pub const BODY_LEN: usize = 63;
pub const REPORT_LEN: usize = 64;
pub const MATRIX_SLOTS: usize = 132; // 6 rows * 22 cols
pub const KEYMASK_LEN: usize = 22;

pub const CUSTOM_LIGHT_BYTES: usize = MATRIX_SLOTS * 3; // 396
pub const CUSTOM_LIGHT_CHUNK: usize = 54;
pub const CUSTOM_LIGHT_SLOTS: u8 = 2;

pub const KEYMAP_BYTES: usize = MATRIX_SLOTS * 4; // 528
pub const KEYMAP_CHUNK: usize = 56;

pub const SWITCH_TABLE_BYTES: usize = MATRIX_SLOTS; // 132
pub const DEADBAND_TABLE_BYTES: usize = MATRIX_SLOTS * 2; // 264
pub const TABLE_CHUNK_58: usize = 58;

pub const PRCS_BYTES: usize = 80; // 2 * 40 bytes = 20 entries * 4 bytes
pub const PRCS_CHUNK: usize = 40;
pub const PRCS_MAX_ENTRIES: usize = 20;
pub const PRCS_MODEL_MAX: u8 = 3; // Wire values 0..=3 correspond to UI Model 1..=4

pub const TRIGGER_UNIT_MM_DEFAULT: f64 = 0.01;
pub const LIGHT_MAX_LEVEL: u8 = 4;

pub type Report = [u8; REPORT_LEN];
pub type Body = [u8; BODY_LEN];

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("key index {0} is outside the 22x8 keymask range (0..176)")]
    KeyIndexOutOfRange(u8),
    #[error("custom light slot {0} out of range (expected 0..{CUSTOM_LIGHT_SLOTS})")]
    InvalidCustomSlot(u8),
    #[error("invalid poll rate {0} kHz (supported: 1, 2, 4, 8)")]
    InvalidPollRate(u8),
    #[error("SOCD/PRCS pair count {0} exceeds maximum {PRCS_MAX_ENTRIES}")]
    TooManyPrcsPairs(usize),
    #[error("SOCD/PRCS model {0} out of wire range 0..={PRCS_MODEL_MAX}")]
    InvalidPrcsModel(u8),
    #[error("hex payload error: {0}")]
    InvalidHex(String),
    #[error("refusing dangerous command: {0}")]
    DangerousCommand(&'static str),
}

pub mod cmd {
    pub const HEARTBEAT: u8 = 0x01;
    pub const SIDE_LIGHT_LIST: u8 = 0x02;
    pub const LIGHT: u8 = 0x07;
    pub const SIDE_LIGHT: u8 = 0x08;
    pub const CUSTOM_LIGHT: u8 = 0x09;
    pub const LIGHT_LIST: u8 = 0x0A;
    pub const DEVICE_INFO: u8 = 0x0D;
    pub const FN_STATE: u8 = 0x0E;
    pub const WIN_LOCK: u8 = 0x14;
    pub const MUSIC_RHYTHM: u8 = 0x17;
    pub const KEYMAP: u8 = 0x18;
    pub const MACRO: u8 = 0x19;
    pub const TRIGGER: u8 = 0x21;
    pub const ADVANCED_KEYS: u8 = 0x22;
    pub const PRCS: u8 = 0x24;
    pub const SWITCH_PROFILE: u8 = 0x25;
    pub const DEADBAND: u8 = 0x26;
}

/// Create a zeroed 63-byte body with `body[0] = command`.
pub fn new_body(command: u8) -> Body {
    let mut body = [0u8; BODY_LEN];
    body[0] = command;
    body
}

/// Prepend Report ID `0x01` to a 63-byte command body.
pub fn wrap_report(body: &Body) -> Report {
    let mut report = [0u8; REPORT_LEN];
    report[0] = REPORT_ID;
    report[1..].copy_from_slice(body);
    report
}

/// Strip the leading Report ID (`0x01`) from a 64-byte HID read buffer, or
/// accept an already-unprefixed 63-byte body.
pub fn strip_report_id(raw: &[u8]) -> Option<&[u8]> {
    if raw.len() >= REPORT_LEN && raw[0] == REPORT_ID {
        Some(&raw[1..REPORT_LEN])
    } else if raw.len() == BODY_LEN {
        Some(raw)
    } else if raw.len() > 1 && raw[0] == REPORT_ID {
        Some(&raw[1..])
    } else {
        None
    }
}

/// Build the 22-byte transposed bitmask: `mask[idx % 22] |= 1 << (idx / 22)`.
pub fn keymask(indices: &[u8]) -> Result<[u8; KEYMASK_LEN], ProtocolError> {
    let mut mask = [0u8; KEYMASK_LEN];
    for &idx in indices {
        let col = (idx as usize) % KEYMASK_LEN;
        let bit = (idx as usize) / KEYMASK_LEN;
        if bit >= 8 {
            return Err(ProtocolError::KeyIndexOutOfRange(idx));
        }
        mask[col] |= 1u8 << bit;
    }
    Ok(mask)
}

/// Decode a 22-byte keymask into sorted matrix indices.
pub fn decode_keymask(mask: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (col, &byte) in mask.iter().take(KEYMASK_LEN).enumerate() {
        for bit in 0..8 {
            if (byte & (1 << bit)) != 0 {
                out.push((bit * KEYMASK_LEN + col) as u8);
            }
        }
    }
    out.sort_unstable();
    out
}

/// Shared chunk representation for paged table transfers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedChunk {
    pub page: usize,
    pub data: Vec<u8>,
}

/// Build paged write packets `[cmd, sub, page_hi, page_lo, len, data...]`.
pub fn paged_write_packets(cmd_byte: u8, sub: u8, payload: &[u8], page_size: usize) -> Vec<Report> {
    payload
        .chunks(page_size)
        .enumerate()
        .map(|(page, chunk)| {
            let mut body = new_body(cmd_byte);
            body[1] = sub;
            body[2] = ((page >> 8) & 0xFF) as u8;
            body[3] = (page & 0xFF) as u8;
            body[4] = chunk.len() as u8;
            body[5..5 + chunk.len()].copy_from_slice(chunk);
            wrap_report(&body)
        })
        .collect()
}

/// Parse one paged read-reply chunk body `[cmd, sub, page_hi, page_lo, len, data...]`.
pub fn parse_paged_chunk(body: &[u8], expected_cmd: u8, expected_sub: u8) -> Option<PagedChunk> {
    if body.len() < 6 || body[0] != expected_cmd || body[1] != expected_sub {
        return None;
    }
    let page = u16::from_be_bytes([body[2], body[3]]) as usize;
    let len = body[4] as usize;
    if len == 0 || 5 + len > body.len() {
        return None;
    }
    Some(PagedChunk {
        page,
        data: body[5..5 + len].to_vec(),
    })
}

/// Reassemble a flat byte table from paged chunks. Returns `None` if any page
/// in `0..expected_pages` is missing.
pub fn assemble_paged_table(
    chunks: &[PagedChunk],
    total_bytes: usize,
    stride: usize,
) -> Option<Vec<u8>> {
    let mut table = vec![0u8; total_bytes];
    let mut seen = BTreeSet::new();
    let n_pages = total_bytes.div_ceil(stride);

    for ch in chunks {
        if ch.page >= n_pages {
            continue;
        }
        let start = ch.page * stride;
        let max_copy = total_bytes.saturating_sub(start).min(ch.data.len());
        if max_copy == 0 {
            continue;
        }
        table[start..start + max_copy].copy_from_slice(&ch.data[..max_copy]);
        seen.insert(ch.page);
    }

    if seen.len() >= n_pages {
        Some(table)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// 1. Heartbeat / initInfo (cmd 0x01) & Device Info (cmd 0x0D)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitInfo {
    pub charging: bool,
    pub battery: u8,
    pub tri_mode: u8,
    pub is_prcs: bool,
    pub is_any_key_calibration: bool,
    pub is_custom_light: bool,
    pub is_more_switch: bool,
    pub is_poll_rate: bool,
    pub is_dead_band: bool,
    pub is_music_rhythm: bool,
    pub is_all_and_6key_switch: bool,
    pub is_high_precision: bool,
    pub is_gamepad: bool,
    pub is_rs: bool,
    pub is_rkrt: bool,
    pub is_music_rhythm_on: bool,
    pub is_6key_mode: bool,
    pub raw_flags1: u8,
    pub raw_flags2: u8,
}

impl InitInfo {
    pub fn connection_mode(&self) -> &'static str {
        match self.tri_mode {
            0 => "USB",
            1 => "2.4G",
            2 => "Bluetooth",
            _ => "Unknown",
        }
    }
}

pub fn build_heartbeat() -> Report {
    wrap_report(&new_body(cmd::HEARTBEAT))
}

pub fn parse_init_info(body: &[u8]) -> Option<InitInfo> {
    if body.len() < 23 || body[0] != cmd::HEARTBEAT || body[4] != 0x11 {
        return None;
    }
    let flags1 = body[17];
    let flags2 = body[18];
    Some(InitInfo {
        charging: body[8] != 0,
        battery: body[9],
        tri_mode: body[10],
        is_prcs: body[14] == 1,
        is_any_key_calibration: (flags1 & 0x01) != 0,
        is_custom_light: (flags1 & 0x02) != 0,
        is_more_switch: (flags1 & 0x04) != 0,
        is_poll_rate: (flags1 & 0x08) != 0,
        is_dead_band: (flags1 & 0x10) != 0,
        is_music_rhythm: (flags1 & 0x20) != 0,
        is_all_and_6key_switch: (flags1 & 0x80) != 0,
        is_high_precision: (flags2 & 0x01) != 0,
        is_gamepad: (flags2 & 0x04) != 0,
        is_rs: (flags2 & 0x08) != 0,
        is_rkrt: (flags2 & 0x10) != 0,
        is_music_rhythm_on: body[21] != 0,
        is_6key_mode: (body[22] & 0x01) != 0,
        raw_flags1: flags1,
        raw_flags2: flags2,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfoChunk {
    pub index: u8,
    pub text: String,
}

pub fn build_device_info_query() -> Report {
    wrap_report(&new_body(cmd::DEVICE_INFO))
}

pub fn parse_device_info_chunk(body: &[u8]) -> Option<DeviceInfoChunk> {
    if body.len() < 6 || body[0] != cmd::DEVICE_INFO {
        return None;
    }
    let len = (body[4] as usize).min(body.len() - 5);
    let slice = &body[5..5 + len];
    let nul_end = slice.iter().position(|&b| b == 0).unwrap_or(slice.len());
    let text = String::from_utf8_lossy(&slice[..nul_end]).into_owned();
    Some(DeviceInfoChunk {
        index: body[3],
        text,
    })
}

// ---------------------------------------------------------------------------
// 2. Trigger / Actuation / Travel / Poll Rate (cmd 0x21)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MaxTriggerTravel {
    pub unit_numerator: u8,
    pub divisor: u16,
    pub unit_mm: f64,
    pub max_raw: u16,
    pub min_raw: u8,
    pub max_mm: f64,
    pub min_mm: f64,
}

pub fn build_read_max_trigger_travel() -> Report {
    let mut body = new_body(cmd::TRIGGER);
    body[4] = 0x18;
    body[5] = 0x04;
    wrap_report(&body)
}

pub fn parse_max_trigger_travel(body: &[u8], high_precision: bool) -> Option<MaxTriggerTravel> {
    if body.len() < 11 || body[0] != cmd::TRIGGER || body[5] != 0x04 {
        return None;
    }
    let numer = if body[7] == 0 { 10 } else { body[7] };
    let divisor: u16 = if high_precision { 1000 } else { 100 };
    let unit_mm = f64::from(numer) / f64::from(divisor);
    let max_raw = ((body[9] as u16) << 8) | (body[6] as u16);
    let min_raw = body[10];
    Some(MaxTriggerTravel {
        unit_numerator: numer,
        divisor,
        unit_mm,
        max_raw,
        min_raw,
        max_mm: f64::from(max_raw) * unit_mm,
        min_mm: f64::from(min_raw) * unit_mm,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TriggerMode {
    Fixed,
    RapidTrigger,
    RapidTriggerSplit,
    Unknown(u8),
}

impl TriggerMode {
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => Self::Fixed,
            0x0C => Self::RapidTrigger,
            0x0D => Self::RapidTriggerSplit,
            other => Self::Unknown(other),
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            Self::Fixed => 0x00,
            Self::RapidTrigger => 0x0C,
            Self::RapidTriggerSplit => 0x0D,
            Self::Unknown(b) => b,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "Fixed",
            Self::RapidTrigger => "Rapid Trigger",
            Self::RapidTriggerSplit => "Rapid Trigger (Split)",
            Self::Unknown(_) => "Unknown",
        }
    }
}

pub fn mm_to_raw(mm: f64, unit_mm: f64) -> u16 {
    let unit = if unit_mm > 0.0 {
        unit_mm
    } else {
        TRIGGER_UNIT_MM_DEFAULT
    };
    (mm / unit).round().clamp(0.0, f64::from(u16::MAX)) as u16
}

pub fn build_trigger_raw(
    mode: TriggerMode,
    indices: &[u8],
    travel_raw: u16,
    interval1_raw: u16,
    interval2_raw: u16,
) -> Result<Report, ProtocolError> {
    let mask = keymask(indices)?;
    let mut body = new_body(cmd::TRIGGER);
    body[4] = 0x18;
    body[5] = mode.to_byte();
    body[6..6 + KEYMASK_LEN].copy_from_slice(&mask);

    let t_lo = (travel_raw & 0xFF) as u8;
    let t_hi = ((travel_raw >> 8) & 0xFF) as u8;
    body[28] = t_lo;
    body[29] = t_lo;
    body[30] = (interval1_raw & 0xFF) as u8;
    body[31] = (interval2_raw & 0xFF) as u8;
    body[32] = t_hi;
    body[33] = t_hi;
    body[34] = ((interval1_raw >> 8) & 0xFF) as u8;
    body[35] = ((interval2_raw >> 8) & 0xFF) as u8;

    Ok(wrap_report(&body))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerConfig {
    pub index: u8,
    pub row: u8,
    pub col: u8,
    pub mode: TriggerMode,
    pub travel_raw: u16,
    pub interval1_raw: u16,
    pub interval2_raw: u16,
}

pub fn build_read_trigger_config(index: u8) -> Report {
    let mut body = new_body(cmd::TRIGGER);
    body[4] = 0x18;
    body[5] = 0x05;
    body[6] = index / 22;
    body[7] = index % 22;
    wrap_report(&body)
}

pub fn parse_trigger_config(body: &[u8]) -> Option<TriggerConfig> {
    if body.len() < 17 || body[0] != cmd::TRIGGER || body[5] != 0x05 {
        return None;
    }
    let row = body[15];
    let col = body[16];
    let index = row.saturating_mul(22).saturating_add(col);
    Some(TriggerConfig {
        index,
        row,
        col,
        mode: TriggerMode::from_byte(body[6]),
        travel_raw: ((body[11] as u16) << 8) | (body[7] as u16),
        interval1_raw: ((body[13] as u16) << 8) | (body[9] as u16),
        interval2_raw: ((body[14] as u16) << 8) | (body[10] as u16),
    })
}

pub fn build_open_trigger_test(indices: &[u8]) -> Result<Report, ProtocolError> {
    let mask = keymask(indices)?;
    let mut body = new_body(cmd::TRIGGER);
    body[4] = 0x18;
    body[5] = 0x02;
    body[6..6 + KEYMASK_LEN].copy_from_slice(&mask);
    Ok(wrap_report(&body))
}

pub fn build_close_trigger_test() -> Report {
    let mut body = new_body(cmd::TRIGGER);
    body[4] = 0x18;
    body[5] = 0x03;
    wrap_report(&body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveTravelSample {
    pub row: u8,
    pub col: u8,
    pub index: u8,
    pub travel_raw: u16,
}

impl LiveTravelSample {
    pub fn depth_mm(self, unit_mm: f64) -> f64 {
        f64::from(self.travel_raw) * unit_mm
    }
}

/// Decode live travel stream frame (`cmd 0x21`, `body[5] == 0x01`).
pub fn parse_live_travel(body: &[u8]) -> Option<LiveTravelSample> {
    if body.len() < 10 || body[0] != cmd::TRIGGER || body[5] != 0x01 {
        return None;
    }
    let row = body[6];
    let col = body[7];
    Some(LiveTravelSample {
        row,
        col,
        index: row.saturating_mul(22).saturating_add(col),
        travel_raw: u16::from_le_bytes([body[8], body[9]]),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PollRate {
    Hz1000 = 1,
    Hz2000 = 2,
    Hz4000 = 4,
    Hz8000 = 8,
}

impl PollRate {
    pub fn from_khz(khz: u8) -> Result<Self, ProtocolError> {
        match khz {
            1 => Ok(Self::Hz1000),
            2 => Ok(Self::Hz2000),
            4 => Ok(Self::Hz4000),
            8 => Ok(Self::Hz8000),
            other => Err(ProtocolError::InvalidPollRate(other)),
        }
    }

    pub fn hz(self) -> u16 {
        (self as u16) * 1000
    }
}

pub fn build_read_poll_rate() -> Report {
    let mut body = new_body(cmd::TRIGGER);
    body[4] = 0x01;
    body[5] = 0x09;
    wrap_report(&body)
}

pub fn parse_poll_rate(body: &[u8]) -> Option<u8> {
    if body.len() < 7 || body[0] != cmd::TRIGGER || body[5] != 0x09 || body[4] < 2 {
        return None;
    }
    Some(body[6])
}

pub fn build_set_poll_rate(rate: PollRate) -> Report {
    let mut body = new_body(cmd::TRIGGER);
    body[4] = 0x00;
    body[5] = 0x09;
    body[6] = 0x01;
    body[7] = rate as u8;
    wrap_report(&body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalibrationStatus {
    Finished,
    Progress,
    Failed(u8),
    Unknown(u8),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalibrationProgress {
    pub status: CalibrationStatus,
    pub calibrated_indices: Vec<u8>,
}

/// Start or stop Hall-effect key calibration (`cmd 0x21`, sub `8` all-key / `15..16` any-key).
pub fn build_calibration(start: bool, any_key: bool) -> Report {
    let mut body = new_body(cmd::TRIGGER);
    body[4] = 0x18;
    if any_key {
        body[5] = if start { 15 } else { 16 };
        body[6] = 0;
    } else {
        body[5] = 8;
        body[6] = u8::from(!start);
    }
    wrap_report(&body)
}

/// Decode a calibration progress/completion frame (`body[0] == 0x21`, `body[5]` in `8 | 15`).
pub fn parse_calibration_reply(body: &[u8]) -> Option<CalibrationProgress> {
    if body.len() < 29 || body[0] != cmd::TRIGGER || (body[5] != 8 && body[5] != 15) {
        return None;
    }
    let status = match body[6] {
        0 => CalibrationStatus::Finished,
        1 => CalibrationStatus::Progress,
        c @ (2 | 4) => CalibrationStatus::Failed(c),
        other => CalibrationStatus::Unknown(other),
    };
    let calibrated_indices = if status == CalibrationStatus::Progress {
        decode_keymask(&body[7..29])
            .into_iter()
            .filter(|&idx| (idx as usize) < MATRIX_SLOTS)
            .collect()
    } else {
        Vec::new()
    };
    Some(CalibrationProgress {
        status,
        calibrated_indices,
    })
}

// ---------------------------------------------------------------------------
// 3. Lighting (cmds 0x07, 0x09, 0x0A, 0x17)
// ---------------------------------------------------------------------------

pub fn light_mode_name(mode: u8) -> &'static str {
    match mode {
        0 => "Static",
        1 => "Breath",
        2 => "Wave",
        3 => "Neon",
        4 => "Radar",
        6 => "Reactive",
        7 => "Aurora",
        8 => "Ripple",
        9 => "Twinkle",
        10 => "Custom",
        11 => "Cross",
        12 => "SpeedRespond",
        14 => "AutoRipple",
        15 => "Striation",
        16 => "Fireworks",
        100 => "MusicRhythm",
        _ => "Unknown",
    }
}

pub fn parse_light_mode_name(input: &str) -> Option<u8> {
    let trimmed = input.trim();
    if let Ok(num) = trimmed.parse::<u8>() {
        return Some(num);
    }
    match trimmed.to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
        "static" => Some(0),
        "breath" | "breathing" => Some(1),
        "wave" => Some(2),
        "neon" => Some(3),
        "radar" => Some(4),
        "reactive" => Some(6),
        "aurora" => Some(7),
        "ripple" => Some(8),
        "twinkle" => Some(9),
        "custom" => Some(10),
        "cross" => Some(11),
        "speedrespond" | "speed" => Some(12),
        "autoripple" => Some(14),
        "striation" => Some(15),
        "fireworks" | "frenzy" => Some(16),
        "music" | "musicrhythm" | "rhythm" => Some(100),
        _ => None,
    }
}

pub fn direction_name(dir: u8) -> &'static str {
    match dir {
        0 => "Right",
        1 => "Left",
        2 => "Up",
        3 => "Down",
        4 => "Spread",
        5 => "Gather",
        _ => "Other",
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightList {
    pub modes: Vec<u8>,
    pub max_speed: u8,
    pub max_brightness: u8,
}

pub fn build_read_light_list() -> Report {
    wrap_report(&new_body(cmd::LIGHT_LIST))
}

pub fn parse_light_list(body: &[u8]) -> Option<LightList> {
    if body.len() < 6 || (body[0] != cmd::LIGHT_LIST && body[0] != cmd::SIDE_LIGHT_LIST) {
        return None;
    }
    let len = body[4] as usize;
    if len < 4 || 5 + len > body.len() {
        return None;
    }
    let payload = &body[5..5 + len];
    Some(LightList {
        modes: payload[..len - 4].to_vec(),
        max_speed: payload[len - 2],
        max_brightness: payload[len - 1],
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightState {
    pub mode: u8,
    pub brightness: u8,
    pub speed: u8,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub direction: u8,
    pub full_color: bool,
    pub power_on: bool,
}

impl Default for LightState {
    fn default() -> Self {
        Self {
            mode: 0,
            brightness: LIGHT_MAX_LEVEL,
            speed: LIGHT_MAX_LEVEL,
            fg: [255, 255, 255],
            bg: [0, 0, 0],
            direction: 0,
            full_color: false,
            power_on: true,
        }
    }
}

pub fn build_read_light() -> Report {
    let mut body = new_body(cmd::LIGHT);
    body[1] = 0x01;
    wrap_report(&body)
}

pub fn parse_light_state(body: &[u8]) -> Option<LightState> {
    if body.len() < 14 || body[0] != cmd::LIGHT || body[1] != 0x00 {
        return None;
    }
    let len = body[4] as usize;
    if len < 9 || 5 + len > body.len() {
        return None;
    }
    Some(LightState {
        mode: body[5],
        brightness: body[6],
        speed: body[7],
        fg: [body[8], body[9], body[10]],
        bg: [body[11], body[12], body[13]],
        direction: if len >= 10 { body[14] } else { 0 },
        full_color: len >= 11 && body[15] != 0,
        power_on: len < 12 || body[16] == 0,
    })
}

pub fn build_set_light(state: &LightState) -> Report {
    let mut body = new_body(cmd::LIGHT);
    body[4] = 0x0E;
    body[5] = state.mode;
    body[6] = state.brightness.min(LIGHT_MAX_LEVEL);
    body[7] = state.speed.min(LIGHT_MAX_LEVEL);
    body[8..11].copy_from_slice(&state.fg);
    body[11..14].copy_from_slice(&state.bg);
    body[14] = state.direction;
    body[15] = u8::from(state.full_color);
    body[16] = u8::from(!state.power_on);
    wrap_report(&body)
}

pub fn build_music_rhythm(on: bool) -> Report {
    let mut body = new_body(cmd::MUSIC_RHYTHM);
    body[4] = 0x02;
    let flag = u8::from(on);
    body[5] = flag;
    body[6] = flag;
    wrap_report(&body)
}

pub fn build_read_custom_number() -> Report {
    let mut body = new_body(cmd::CUSTOM_LIGHT);
    body[1] = 0x20;
    wrap_report(&body)
}

pub fn parse_custom_number(body: &[u8]) -> Option<u8> {
    if body.len() < 5 || body[0] != cmd::CUSTOM_LIGHT || (body[1] != 0 && body[1] != 1) || body[4] != 0 {
        return None;
    }
    Some(body[1])
}

pub fn build_read_custom_light(slot: u8) -> Result<Report, ProtocolError> {
    if slot >= CUSTOM_LIGHT_SLOTS {
        return Err(ProtocolError::InvalidCustomSlot(slot));
    }
    let mut body = new_body(cmd::CUSTOM_LIGHT);
    body[1] = 0x80 | slot;
    Ok(wrap_report(&body))
}

pub fn build_custom_light_packets(
    colors: &[[u8; 3]; MATRIX_SLOTS],
    slot: u8,
) -> Result<Vec<Report>, ProtocolError> {
    if slot >= CUSTOM_LIGHT_SLOTS {
        return Err(ProtocolError::InvalidCustomSlot(slot));
    }
    let mut flat = [0u8; CUSTOM_LIGHT_BYTES];
    for (idx, rgb) in colors.iter().enumerate() {
        let off = idx * 3;
        flat[off..off + 3].copy_from_slice(rgb);
    }
    Ok(paged_write_packets(
        cmd::CUSTOM_LIGHT,
        slot,
        &flat,
        CUSTOM_LIGHT_CHUNK,
    ))
}

// ---------------------------------------------------------------------------
// 4. Switch Profiles (cmd 0x25) & Deadband (cmd 0x26)
// ---------------------------------------------------------------------------

pub fn switch_profile_name(id: u8) -> &'static str {
    match id {
        0 => "Unset",
        1 => "HM1 (Gateron)",
        2 => "HH1 (Gateron)",
        3 => "CY1 (KZZI)",
        5 => "TC1 (TTC Kom)",
        _ => "Unknown",
    }
}

pub fn parse_switch_id(token: &str) -> Option<u8> {
    let trimmed = token.trim();
    if let Ok(id) = trimmed.parse::<u8>() {
        return Some(id);
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "hm1" => Some(1),
        "hh1" => Some(2),
        "cy1" => Some(3),
        "tc1" => Some(5),
        _ => None,
    }
}

pub fn build_read_switch_list() -> Report {
    wrap_report(&new_body(cmd::SWITCH_PROFILE))
}

pub fn parse_switch_list(body: &[u8]) -> Option<Vec<u8>> {
    if body.len() < 7 || body[0] != cmd::SWITCH_PROFILE || body[1] != 0x00 {
        return None;
    }
    let len = body[4] as usize;
    if len < 2 || 5 + len > body.len() {
        return None;
    }
    let payload = &body[5..5 + len];
    let count = u16::from_be_bytes([payload[0], payload[1]]) as usize;
    if 2 + count > payload.len() {
        return None;
    }
    Some(payload[2..2 + count].to_vec())
}

pub fn build_read_switch_table() -> Report {
    let mut body = new_body(cmd::SWITCH_PROFILE);
    body[1] = 0x02;
    wrap_report(&body)
}

pub fn build_switch_table(table: &[u8; SWITCH_TABLE_BYTES]) -> Vec<Report> {
    paged_write_packets(cmd::SWITCH_PROFILE, 0x01, table, TABLE_CHUNK_58)
}

pub fn build_read_deadband() -> Report {
    wrap_report(&new_body(cmd::DEADBAND))
}

pub fn build_deadband_table(table: &[u8; DEADBAND_TABLE_BYTES]) -> Vec<Report> {
    paged_write_packets(cmd::DEADBAND, 0x01, table, TABLE_CHUNK_58)
}

// ---------------------------------------------------------------------------
// 5. Keymap (cmd 0x18), PRCS/SOCD (cmd 0x24), Win-Lock (cmd 0x14)
// ---------------------------------------------------------------------------

pub fn build_read_keymap(fn_layer: bool) -> Report {
    let mut body = new_body(cmd::KEYMAP);
    body[1] = if fn_layer { 0x82 } else { 0x80 };
    wrap_report(&body)
}

pub fn build_write_keymap_layer(fn_layer: bool, table: &[u8; KEYMAP_BYTES]) -> Vec<Report> {
    paged_write_packets(
        cmd::KEYMAP,
        if fn_layer { 0x02 } else { 0x00 },
        table,
        KEYMAP_CHUNK,
    )
}

/// Populate factory `[0, 0, 0, 0]` base-layer keymap slots so the WIN 60 HE firmware
/// keeps normal typing and the `Fn` switch (`code1 = 1`) intact after writing `cmd 0x18`.
///
/// Any already-programmed non-zero 4-byte entry is preserved verbatim.
pub fn compose_base_keymap(existing_base: &[u8; KEYMAP_BYTES]) -> [u8; KEYMAP_BYTES] {
    let mut out = *existing_base;
    for key in crate::keymap::WIN60HE_KEYS {
        let off = usize::from(key.index) * 4;
        if off + 4 <= KEYMAP_BYTES && out[off..off + 4] == [0, 0, 0, 0] {
            out[off] = u8::from(key.is_fn());
            out[off + 1] = key.hid_code;
        }
    }
    out
}

/// Decode plain `(slot_index, hid_usage)` bindings programmed in the Fn keymap layer.
pub fn decode_fn_keymap_overrides(fn_table: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    for (idx, slot) in fn_table.chunks_exact(4).enumerate() {
        let Ok(index) = u8::try_from(idx) else {
            break;
        };
        if crate::keymap::by_index(index).is_none() {
            continue;
        }
        if slot[0] == 0 && slot[1] != 0 {
            out.push((index, slot[1]));
        }
    }
    out
}

/// Set or clear single-key HID bindings inside a 528-byte Fn keymap table.
pub fn apply_fn_table_edits(
    fn_table: &mut [u8; KEYMAP_BYTES],
    set: &[(u8, u8)],
    clear: &[u8],
) -> Result<(), ProtocolError> {
    let slot_offset = |idx: u8| -> Result<usize, ProtocolError> {
        if crate::keymap::by_index(idx).is_none() {
            return Err(ProtocolError::KeyIndexOutOfRange(idx));
        }
        Ok(usize::from(idx) * 4)
    };
    for &idx in clear {
        let off = slot_offset(idx)?;
        fn_table[off..off + 4].fill(0);
    }
    for &(idx, target_hid) in set {
        let off = slot_offset(idx)?;
        fn_table[off..off + 4].copy_from_slice(&[0, target_hid, 0, 0]);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrcsPair {
    /// Wire model index `0..=3` (corresponding to vendor UI Model 1..=4).
    pub model: u8,
    pub key1_hid: u8,
    pub key2_hid: u8,
}

pub fn build_read_prcs_power() -> Report {
    let mut body = new_body(cmd::PRCS);
    body[1] = 0x02;
    wrap_report(&body)
}

pub fn parse_prcs_power(body: &[u8]) -> Option<bool> {
    if body.len() < 6 || body[0] != cmd::PRCS || body[1] != 0x02 {
        return None;
    }
    Some(body[5] != 0)
}

pub fn build_read_prcs_data() -> Report {
    let mut body = new_body(cmd::PRCS);
    body[1] = 0x01;
    wrap_report(&body)
}

pub fn parse_prcs_table(raw_80: &[u8]) -> Vec<PrcsPair> {
    let mut out = Vec::new();
    for entry in raw_80.chunks_exact(4) {
        if entry[2] != 0 || entry[3] != 0 {
            out.push(PrcsPair {
                model: entry[1],
                key1_hid: entry[2],
                key2_hid: entry[3],
            });
        }
    }
    out
}

pub fn build_prcs_power(on: bool) -> Report {
    let mut body = new_body(cmd::PRCS);
    body[1] = 0x03;
    body[4] = 0x01;
    body[5] = u8::from(on);
    wrap_report(&body)
}

pub fn build_prcs_packets(pairs: &[PrcsPair]) -> Result<Vec<Report>, ProtocolError> {
    if pairs.len() > PRCS_MAX_ENTRIES {
        return Err(ProtocolError::TooManyPrcsPairs(pairs.len()));
    }
    for p in pairs {
        if p.model > PRCS_MODEL_MAX {
            return Err(ProtocolError::InvalidPrcsModel(p.model));
        }
    }
    let mut table = [0u8; PRCS_BYTES];
    for (idx, pair) in pairs.iter().enumerate() {
        let off = idx * 4;
        table[off] = 1;
        table[off + 1] = pair.model;
        table[off + 2] = pair.key1_hid;
        table[off + 3] = pair.key2_hid;
    }
    Ok(paged_write_packets(cmd::PRCS, 0x00, &table, PRCS_CHUNK))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WinLockFlags {
    pub disable_win: bool,
    pub disable_shift_tab: bool,
    pub disable_alt_tab: bool,
    pub disable_alt_f4: bool,
    pub win_lock_on: bool,
}

pub fn build_win_lock(flags: WinLockFlags) -> Report {
    let mut body = new_body(cmd::WIN_LOCK);
    body[4] = 0x01;
    body[12] = u8::from(flags.disable_win);
    body[13] = u8::from(flags.disable_shift_tab);
    body[14] = u8::from(flags.disable_alt_tab);
    body[15] = u8::from(flags.disable_alt_f4);
    body[18] = u8::from(flags.win_lock_on);
    wrap_report(&body)
}

// ---------------------------------------------------------------------------
// 6. Safety Guard
// ---------------------------------------------------------------------------

/// Return a human-readable refusal reason if `body` (63 bytes, without Report
/// ID) represents a destructive factory-reset, trigger-wipe, or unguarded
/// calibration-start frame.
pub fn dangerous_command_reason(body: &[u8]) -> Option<&'static str> {
    if body.len() < 7 {
        return None;
    }
    if body[0] == cmd::WIN_LOCK && body[1] == 0 && body[5] == 1 {
        return Some("cmd 0x14 [5]=1 is resetKeyboard (factory reset)");
    }
    if body[0] == cmd::TRIGGER && body[4] == 0x18 {
        if body[5] == 6 {
            return Some("cmd 0x21 sub 6 is resetTrigger (wipes every key's actuation)");
        }
        if (body[5] == 8 || body[5] == 15) && body[6] == 0 {
            return Some("cmd 0x21 sub 8/15 arms calibration mode and wipes stored calibration");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::all_indices;

    #[test]
    fn all_keys_mask_matches_capture() {
        let indices = all_indices();
        let mask = keymask(&indices).unwrap();
        let expected: [u8; 22] = [
            0x3e, 0x26, 0x3e, 0x1e, 0x1e, 0x1e, 0x3e, 0x1e, 0x1e, 0x3e, 0x3e, 0x3e, 0x3e, 0x00,
            0x0e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(mask, expected);
        assert_eq!(decode_keymask(&mask).len(), 61);
    }

    #[test]
    fn init_info_matches_win60_capture() {
        let mut body = [0u8; BODY_LEN];
        let sample = [
            0x01, 0x00, 0x00, 0x00, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x00, 0x7f, 0x00, 0x01, 0x01, 0x00, 0x00,
        ];
        body[..sample.len()].copy_from_slice(&sample);
        let info = parse_init_info(&body).unwrap();
        assert!(info.is_prcs);
        assert!(info.is_any_key_calibration);
        assert!(info.is_custom_light);
        assert!(info.is_more_switch);
        assert!(info.is_poll_rate);
        assert!(info.is_dead_band);
        assert!(info.is_music_rhythm);
        assert!(!info.is_high_precision);
        assert!(!info.is_gamepad);
        assert!(!info.is_rs);
        assert!(!info.is_rkrt);
        assert_eq!(info.connection_mode(), "USB");
    }

    #[test]
    fn max_travel_matches_capture() {
        let mut body = [0u8; BODY_LEN];
        let sample = [0x21, 0x00, 0x00, 0x00, 0x06, 0x04, 0x54, 0x01, 0x01, 0x01, 0x08];
        body[..sample.len()].copy_from_slice(&sample);
        let m = parse_max_trigger_travel(&body, false).unwrap();
        assert_eq!(m.unit_numerator, 1);
        assert_eq!(m.divisor, 100);
        assert!((m.unit_mm - 0.01).abs() < 1e-9);
        assert_eq!(m.max_raw, 340);
        assert_eq!(m.min_raw, 8);
        assert!((m.max_mm - 3.40).abs() < 1e-9);
        assert!((m.min_mm - 0.08).abs() < 1e-9);
    }

    #[test]
    fn trigger_write_3mm_matches_capture() {
        let pkt = build_trigger_raw(TriggerMode::Fixed, &all_indices(), 300, 1, 1).unwrap();
        assert_eq!(pkt[0], 0x01);
        assert_eq!(pkt[1], 0x21);
        assert_eq!(pkt[5], 0x18);
        assert_eq!(pkt[6], 0x00);
        assert_eq!(pkt[29], 0x2c);
        assert_eq!(pkt[30], 0x2c);
        assert_eq!(pkt[31], 0x01);
        assert_eq!(pkt[32], 0x01);
        assert_eq!(pkt[33], 0x01);
        assert_eq!(pkt[34], 0x01);
    }

    #[test]
    fn trigger_config_readback_matches_capture() {
        let mut body = [0u8; BODY_LEN];
        let sample = [
            0x21, 0x00, 0x00, 0x00, 0x0c, 0x05, 0x00, 0xaa, 0xaa, 0x01, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x00,
        ];
        body[..sample.len()].copy_from_slice(&sample);
        let cfg = parse_trigger_config(&body).unwrap();
        assert_eq!(cfg.index, 22);
        assert_eq!(cfg.mode, TriggerMode::Fixed);
        assert_eq!(cfg.travel_raw, 170);
        assert_eq!(cfg.interval1_raw, 1);
        assert_eq!(cfg.interval2_raw, 1);
    }

    #[test]
    fn light_list_and_state_match_capture() {
        let mut list_body = [0u8; BODY_LEN];
        let sample = [
            0x0a, 0x00, 0x00, 0x00, 0x12, 0x02, 0x01, 0x03, 0x09, 0x04, 0x00, 0x0e, 0x0f, 0x06,
            0x07, 0x0b, 0x0c, 0x10, 0x08, 0xa5, 0x5a, 0x04, 0x04,
        ];
        list_body[..sample.len()].copy_from_slice(&sample);
        let list = parse_light_list(&list_body).unwrap();
        assert_eq!(list.modes.len(), 14);
        assert_eq!(list.max_speed, 4);
        assert_eq!(list.max_brightness, 4);

        let mut state_body = [0u8; BODY_LEN];
        let s = [
            0x07, 0x00, 0x00, 0x00, 0x0c, 0x0a, 0x04, 0x04, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00,
        ];
        state_body[..s.len()].copy_from_slice(&s);
        let st = parse_light_state(&state_body).unwrap();
        assert_eq!(st.mode, 10);
        assert_eq!(st.brightness, 4);
        assert_eq!(st.speed, 4);
        assert_eq!(st.fg, [255, 255, 255]);
        assert_eq!(st.bg, [0, 0, 0]);
    }

    #[test]
    fn blocks_dangerous_commands() {
        let mut reset_kb = [0u8; BODY_LEN];
        reset_kb[0] = cmd::WIN_LOCK;
        reset_kb[4] = 1;
        reset_kb[5] = 1;
        assert!(dangerous_command_reason(&reset_kb).is_some());

        let mut reset_trig = [0u8; BODY_LEN];
        reset_trig[0] = cmd::TRIGGER;
        reset_trig[4] = 0x18;
        reset_trig[5] = 6;
        assert!(dangerous_command_reason(&reset_trig).is_some());
    }

    #[test]
    fn builds_and_parses_calibration_frames() {
        let start_all = build_calibration(true, false);
        assert_eq!(start_all[1], cmd::TRIGGER);
        assert_eq!(start_all[5], 0x18);
        assert_eq!(start_all[6], 8);
        assert_eq!(start_all[7], 0);

        let stop_any = build_calibration(false, true);
        assert_eq!(stop_any[6], 16);
        assert_eq!(stop_any[7], 0);

        let mut body = [0u8; BODY_LEN];
        body[0] = cmd::TRIGGER;
        body[5] = 8;
        body[6] = 1;
        let mask = keymask(&[46, 68]).unwrap();
        body[7..29].copy_from_slice(&mask);
        let progress = parse_calibration_reply(&body).unwrap();
        assert_eq!(progress.status, CalibrationStatus::Progress);
        assert_eq!(progress.calibrated_indices, vec![46, 68]);
    }

    #[test]
    fn composes_base_and_fn_keymap_safely() {
        let existing = [0u8; KEYMAP_BYTES];
        let base = compose_base_keymap(&existing);
        let w_off = usize::from(crate::keymap::idx::W) * 4;
        assert_eq!(base[w_off..w_off + 4], [0, 0x1A, 0, 0]);
        let fn_off = usize::from(crate::keymap::idx::FN) * 4;
        assert_eq!(base[fn_off..fn_off + 4], [1, 0xFA, 0, 0]);

        let mut custom = [0u8; KEYMAP_BYTES];
        custom[w_off..w_off + 4].copy_from_slice(&[0x10, 0x01, 0, 0]);
        let preserved = compose_base_keymap(&custom);
        assert_eq!(preserved[w_off..w_off + 4], [0x10, 0x01, 0, 0]);

        let mut fn_table = [0u8; KEYMAP_BYTES];
        apply_fn_table_edits(
            &mut fn_table,
            &[(crate::keymap::idx::R, 0x52), (crate::keymap::idx::F, 0x51)],
            &[],
        )
        .unwrap();
        assert_eq!(
            decode_fn_keymap_overrides(&fn_table),
            vec![(crate::keymap::idx::R, 0x52), (crate::keymap::idx::F, 0x51)]
        );
        apply_fn_table_edits(&mut fn_table, &[], &[crate::keymap::idx::R]).unwrap();
        assert_eq!(
            decode_fn_keymap_overrides(&fn_table),
            vec![(crate::keymap::idx::F, 0x51)]
        );
    }
}
