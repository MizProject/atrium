//! Aula WIN 60 HE keyboard support module (`2E3C:C365`, Usage Page `0xFF1B`).

pub mod driver;
pub mod keymap;
pub mod protocol;

use crate::keyboards::{
    KeyboardCapabilities, KeyboardFamily, KeyboardSpec, UsbId, UsbTransportKind,
};

pub const USB_IDS: [UsbId; 1] = [UsbId {
    vid: protocol::VID,
    pid: protocol::PID,
    transport: UsbTransportKind::WiredUsb,
}];

pub const SPEC: KeyboardSpec = KeyboardSpec {
    id: "win60he",
    aliases: &["win60", "win-60-he", "win60he-pro", "win60he-max"],
    display_name: "Aula WIN 60 HE",
    family: KeyboardFamily::WinHe,
    usb_ids: &USB_IDS,
    vendor_usage_page: protocol::VENDOR_USAGE_PAGE,
    vendor_usage: protocol::VENDOR_USAGE,
    vendor_interface_fallback: Some(protocol::VENDOR_INTERFACE_FALLBACK),
    form_factor: "60% ANSI",
    matrix_rows: 6,
    matrix_cols: 22,
    keys: &keymap::WIN60HE_KEYS,
    udev_rule: "udev/99-aula-win60he.rules",
    capabilities: KeyboardCapabilities {
        hall_effect: true,
        rapid_trigger: true,
        socd_prcs: true,
        per_key_rgb: true,
        custom_rgb_stream: true,
        fn_keymap: true,
        switch_profiles: true,
        deadband: true,
        calibration: true,
        high_poll_rate: true,
        rotary_knob: false,
        wireless_24g: false,
    },
};
