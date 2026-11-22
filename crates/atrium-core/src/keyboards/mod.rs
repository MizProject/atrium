//! Indexed catalog of Aula keyboard models, capabilities, USB IDs, and shared driver types.
//!
//! To integrate a new Aula keyboard model into `atrium`:
//! 1. Create `crates/atrium-core/src/keyboards/<model>/` with `mod.rs` (`pub const SPEC: KeyboardSpec`),
//!    `protocol.rs`, `keymap.rs`, and `driver.rs`.
//! 2. Register `&<model>::SPEC` in [`REGISTRY`] below.
//! 3. Add a matching `atrium-daemon/src/keyboards/<model>.rs` adapter implementing `AulaDriver` and
//!    register it in the daemon's `DRIVERS` index.

pub mod win60he;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hid_sysfs::{SysfsHidrawDevice, enumerate_sysfs_hidraw};
use crate::protocol::{InitInfo, LightList, LightState, MaxTriggerTravel, ProtocolError};
use crate::transport::TransportError;

/// Shared physical key metadata and vendor matrix coordinates across Aula layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyDef {
    /// Matrix slot index (`row * cols + col`).
    pub index: u8,
    /// Human-readable label shown in the UI/CLI.
    pub name: &'static str,
    /// Web `KeyboardEvent.code` identifier from the vendor layout table.
    pub code: &'static str,
    /// Standard USB HID Keyboard/Keypad usage ID (`0xFA` for Fn).
    pub hid_code: u8,
    /// Physical X coordinate in vendor layout units.
    pub x: u16,
    /// Physical Y coordinate in vendor layout units.
    pub y: u16,
    /// Physical width in vendor layout units (`35` = 1u).
    pub width: u16,
    /// Physical height in vendor layout units (`35` = 1u).
    pub height: u16,
}

impl KeyDef {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        index: u8,
        name: &'static str,
        code: &'static str,
        hid_code: u8,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    ) -> Self {
        Self {
            index,
            name,
            code,
            hid_code,
            x,
            y,
            width,
            height,
        }
    }

    pub const fn row(self) -> u8 {
        self.index / 22
    }

    pub const fn col(self) -> u8 {
        self.index % 22
    }

    pub const fn is_fn(self) -> bool {
        self.hid_code == 0xFA
    }
}

/// Product family grouping within the Aula keyboard lineup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardFamily {
    WinHe,
    FSeries,
    HeroHe,
    SSeries,
    Other,
}

impl KeyboardFamily {
    pub const fn label(self) -> &'static str {
        match self {
            Self::WinHe => "WIN HE Series",
            Self::FSeries => "F Series",
            Self::HeroHe => "HERO HE Series",
            Self::SSeries => "S Series",
            Self::Other => "Aula Custom",
        }
    }
}

/// Physical USB link mode represented by a [`UsbId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsbTransportKind {
    WiredUsb,
    Wireless24G,
}

/// Supported `(VID, PID)` pair and transport mode for a registered keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsbId {
    pub vid: u16,
    pub pid: u16,
    pub transport: UsbTransportKind,
}

/// Feature matrix exposed by a registered Aula keyboard driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardCapabilities {
    pub hall_effect: bool,
    pub rapid_trigger: bool,
    pub socd_prcs: bool,
    pub per_key_rgb: bool,
    pub custom_rgb_stream: bool,
    pub fn_keymap: bool,
    pub switch_profiles: bool,
    pub deadband: bool,
    pub calibration: bool,
    pub high_poll_rate: bool,
    pub rotary_knob: bool,
    pub wireless_24g: bool,
}

/// Static catalog descriptor for a supported Aula keyboard model.
#[derive(Debug, Clone, Copy)]
pub struct KeyboardSpec {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub display_name: &'static str,
    pub family: KeyboardFamily,
    pub usb_ids: &'static [UsbId],
    pub vendor_usage_page: u16,
    pub vendor_usage: u16,
    pub vendor_interface_fallback: Option<i32>,
    pub form_factor: &'static str,
    pub matrix_rows: u8,
    pub matrix_cols: u8,
    pub keys: &'static [KeyDef],
    pub udev_rule: &'static str,
    pub capabilities: KeyboardCapabilities,
}

impl KeyboardSpec {
    pub const fn primary_usb_id(&self) -> UsbId {
        self.usb_ids[0]
    }

    pub fn matches_usb(&self, vid: u16, pid: u16) -> bool {
        self.usb_ids.iter().any(|u| u.vid == vid && u.pid == pid)
    }

    pub fn matches_id(&self, token: &str) -> bool {
        let norm = normalize_id_token(token);
        if norm.is_empty() {
            return false;
        }
        normalize_id_token(self.id) == norm
            || normalize_id_token(self.display_name) == norm
            || self
                .aliases
                .iter()
                .any(|alias| normalize_id_token(alias) == norm)
    }

    pub const fn key_count(&self) -> usize {
        self.keys.len()
    }
}

/// Unified driver error type across registered Aula keyboard implementations.
#[derive(Debug, Error)]
pub enum DriverError {
    #[error(transparent)]
    Transport(#[from] TransportError),

    #[error(transparent)]
    Protocol(#[from] ProtocolError),

    #[error("timed out waiting for {0}")]
    Timeout(&'static str),

    #[error("value {value:.2} mm is outside board range {min:.2}..={max:.2} mm")]
    TravelOutOfRange { value: f64, min: f64, max: f64 },

    #[error("refusing write because pre-write readback failed ({0})")]
    ReadModifyWriteAborted(&'static str),

    #[error("{feature} is not supported by {model}")]
    Unsupported {
        model: &'static str,
        feature: &'static str,
    },
}

/// Device snapshot gathered during a non-destructive `probe()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProbe {
    #[serde(default)]
    pub keyboard_id: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    pub device_description: String,
    pub firmware_string: Option<String>,
    pub build_date: Option<String>,
    pub init_info: Option<InitInfo>,
    pub max_travel: Option<MaxTriggerTravel>,
    pub poll_rate_khz: Option<u8>,
    pub light_list: Option<LightList>,
    pub light_state: Option<LightState>,
    pub switch_list: Option<Vec<u8>>,
    pub prcs_enabled: Option<bool>,
}

/// Live sysfs match connecting an attached USB/HID device to a [`KeyboardSpec`].
#[derive(Debug, Clone)]
pub struct DiscoveredKeyboard {
    pub spec: &'static KeyboardSpec,
    pub usb_id: UsbId,
    pub sysfs_interfaces: Vec<SysfsHidrawDevice>,
    pub vendor_hidraw_path: Option<String>,
    pub vendor_readable_writable: bool,
}

/// Indexed catalog of all supported Aula keyboards compiled into `atrium`.
pub const REGISTRY: &[&KeyboardSpec] = &[&win60he::SPEC];

/// Return all registered Aula keyboard specifications in discovery priority order.
pub fn all() -> &'static [&'static KeyboardSpec] {
    REGISTRY
}

/// Look up a registered keyboard specification by slug, alias, or display name.
pub fn find_by_id(token: &str) -> Option<&'static KeyboardSpec> {
    REGISTRY.iter().copied().find(|spec| spec.matches_id(token))
}

/// Look up the registered keyboard specification matching a USB `(VID, PID)`.
pub fn match_usb_id(vid: u16, pid: u16) -> Option<&'static KeyboardSpec> {
    REGISTRY
        .iter()
        .copied()
        .find(|spec| spec.matches_usb(vid, pid))
}

/// Check whether a sysfs hidraw node exposes the vendor config collection for `spec`.
pub fn is_vendor_sysfs_device(dev: &SysfsHidrawDevice, spec: &KeyboardSpec) -> bool {
    let by_usage = dev
        .descriptor
        .as_ref()
        .and_then(|d| d.usage_page)
        .is_some_and(|up| up == spec.vendor_usage_page);
    if by_usage {
        return true;
    }
    spec.vendor_interface_fallback
        .is_some_and(|iface| dev.interface_number == Some(iface))
}

/// Scan Linux sysfs hidraw nodes and group attached devices by registered [`KeyboardSpec`].
pub fn discover_connected() -> Vec<DiscoveredKeyboard> {
    let all_devs = enumerate_sysfs_hidraw(None);
    let mut grouped: BTreeMap<(&'static str, u16, u16), Vec<SysfsHidrawDevice>> = BTreeMap::new();

    for dev in all_devs {
        let Some(spec) = match_usb_id(dev.vid, dev.pid) else {
            continue;
        };
        grouped
            .entry((spec.id, dev.vid, dev.pid))
            .or_default()
            .push(dev);
    }

    let mut out = Vec::new();
    for spec in REGISTRY {
        for usb_id in spec.usb_ids {
            if let Some(ifaces) = grouped.remove(&(spec.id, usb_id.vid, usb_id.pid)) {
                let vendor = ifaces.iter().find(|d| is_vendor_sysfs_device(d, spec));
                let vendor_hidraw_path = vendor.map(|v| v.dev_path.clone());
                let vendor_readable_writable = vendor.is_some_and(|v| v.readable_writable);
                out.push(DiscoveredKeyboard {
                    spec,
                    usb_id: *usb_id,
                    sysfs_interfaces: ifaces,
                    vendor_hidraw_path,
                    vendor_readable_writable,
                });
            }
        }
    }
    out
}

fn normalize_id_token(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_and_resolves_win60he() {
        assert!(!all().is_empty());
        let by_slug = find_by_id("win60he").expect("win60he should be indexed");
        assert_eq!(by_slug.id, "win60he");
        assert_eq!(by_slug.key_count(), 61);
        assert!(by_slug.capabilities.hall_effect);

        let by_alias = find_by_id("WIN-60-HE").expect("alias lookup should normalize hyphens");
        assert_eq!(by_alias.id, by_slug.id);

        let by_usb = match_usb_id(win60he::protocol::VID, win60he::protocol::PID)
            .expect("VID/PID should match win60he");
        assert_eq!(by_usb.id, "win60he");
        assert!(match_usb_id(0x0000, 0x0000).is_none());
    }
}
