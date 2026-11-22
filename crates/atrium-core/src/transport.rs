//! HID transport abstraction and `hidapi` backend with Linux sysfs vendor matching.

use std::time::{Duration, Instant};
use hidapi::{DeviceInfo, HidApi, HidDevice};
use thiserror::Error;

use crate::hid_sysfs::{SysfsHidrawDevice, enumerate_sysfs_hidraw};
use crate::keyboards::{self, KeyboardSpec};
use crate::protocol::{
    PID, REPORT_LEN, Report, VENDOR_INTERFACE_FALLBACK, VENDOR_USAGE, VENDOR_USAGE_PAGE, VID,
};

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("{model} ({vid:04x}:{pid:04x}) not found. Is it plugged in?")]
    DeviceNotFound {
        model: &'static str,
        vid: u16,
        pid: u16,
    },

    #[error("found {model} ({vid:04x}:{pid:04x}) but could not uniquely identify the 0x{usage_page:04X} vendor HID interface")]
    VendorInterfaceUnresolved {
        model: &'static str,
        vid: u16,
        pid: u16,
        usage_page: u16,
    },

    #[error("permission denied opening {path}: {detail}\n\nInstall the {udev_rule} rule and re-plug the keyboard:\n  sudo cp {udev_rule} /etc/udev/rules.d/\n  sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw")]
    PermissionDenied {
        path: String,
        detail: String,
        udev_rule: &'static str,
    },

    #[error("HID I/O error: {0}")]
    Hid(#[from] hidapi::HidError),

    #[error("short HID write: wrote {wrote} of {expected} bytes")]
    ShortWrite { wrote: usize, expected: usize },
}

pub trait HidTransport {
    fn write_report(&mut self, report: &Report) -> Result<(), TransportError>;
    fn read_report(&mut self, timeout_ms: i32) -> Result<Option<Vec<u8>>, TransportError>;
    fn drain(&mut self) -> Result<(), TransportError>;
    fn device_description(&self) -> &str;
}

/// Discovery summary returned by `scan_devices`.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub sysfs: Vec<SysfsHidrawDevice>,
    pub hidapi_interfaces: Vec<HidApiInterfaceSummary>,
}

#[derive(Debug, Clone)]
pub struct HidApiInterfaceSummary {
    pub path: String,
    pub product: String,
    pub interface_number: i32,
    pub usage_page: u16,
    pub usage: u16,
}

/// Enumerate matching `(VID, PID)` interfaces via both Linux sysfs and `hidapi`.
pub fn scan_devices(vid: u16, pid: u16) -> Result<ScanResult, TransportError> {
    let sysfs = enumerate_sysfs_hidraw(Some((vid, pid)));
    let api = HidApi::new()?;
    let hidapi_interfaces = api
        .device_list()
        .filter(|d| d.vendor_id() == vid && d.product_id() == pid)
        .map(|d| HidApiInterfaceSummary {
            path: d.path().to_string_lossy().into_owned(),
            product: d.product_string().unwrap_or("").to_string(),
            interface_number: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
        })
        .collect();
    Ok(ScanResult {
        sysfs,
        hidapi_interfaces,
    })
}

pub struct HidApiTransport {
    device: HidDevice,
    description: String,
}

impl HidApiTransport {
    /// Locate and open the vendor HID interface for the Aula WIN 60 HE.
    pub fn open_win60() -> Result<Self, TransportError> {
        Self::open_spec(&keyboards::win60he::SPEC)
    }

    /// Locate and open the vendor HID interface for any indexed [`KeyboardSpec`].
    pub fn open_spec(spec: &'static KeyboardSpec) -> Result<Self, TransportError> {
        let mut last_err = None;
        for usb in spec.usb_ids {
            match Self::open_matching(
                usb.vid,
                usb.pid,
                spec.vendor_usage_page,
                spec.vendor_usage,
                spec.vendor_interface_fallback,
                spec.display_name,
                spec.udev_rule,
            ) {
                Ok(t) => return Ok(t),
                Err(e @ TransportError::DeviceNotFound { .. }) => last_err = Some(e),
                Err(e) => return Err(e),
            }
        }
        let primary = spec.primary_usb_id();
        Err(last_err.unwrap_or(TransportError::DeviceNotFound {
            model: spec.display_name,
            vid: primary.vid,
            pid: primary.pid,
        }))
    }

    pub fn open(vid: u16, pid: u16) -> Result<Self, TransportError> {
        if let Some(spec) = keyboards::match_usb_id(vid, pid) {
            return Self::open_matching(
                vid,
                pid,
                spec.vendor_usage_page,
                spec.vendor_usage,
                spec.vendor_interface_fallback,
                spec.display_name,
                spec.udev_rule,
            );
        }
        Self::open_matching(
            vid,
            pid,
            VENDOR_USAGE_PAGE,
            VENDOR_USAGE,
            Some(VENDOR_INTERFACE_FALLBACK),
            "Aula Keyboard",
            "udev/99-aula-win60he.rules",
        )
    }

    pub fn open_matching(
        vid: u16,
        pid: u16,
        usage_page: u16,
        usage: u16,
        fallback_iface: Option<i32>,
        model: &'static str,
        udev_rule: &'static str,
    ) -> Result<Self, TransportError> {
        let api = HidApi::new()?;
        let candidates: Vec<&DeviceInfo> = api
            .device_list()
            .filter(|d| d.vendor_id() == vid && d.product_id() == pid)
            .collect();

        if candidates.is_empty() {
            return Err(TransportError::DeviceNotFound { model, vid, pid });
        }

        let sysfs = enumerate_sysfs_hidraw(Some((vid, pid)));
        let chosen = select_vendor_interface(&candidates, &sysfs, usage_page, usage, fallback_iface)
            .ok_or(TransportError::VendorInterfaceUnresolved {
                model,
                vid,
                pid,
                usage_page,
            })?;

        let path_str = chosen.path().to_string_lossy().into_owned();
        let product = chosen.product_string().unwrap_or(model);
        let description = format!(
            "{product} ({vid:04X}:{pid:04X}, iface={}, path={path_str})",
            chosen.interface_number()
        );

        match chosen.open_device(&api) {
            Ok(device) => Ok(Self {
                device,
                description,
            }),
            Err(err) => {
                let msg = err.to_string();
                let sysfs_denied = sysfs
                    .iter()
                    .find(|s| s.dev_path == path_str)
                    .is_some_and(|s| !s.readable_writable);
                if sysfs_denied
                    || msg.to_ascii_lowercase().contains("permission")
                    || msg.to_ascii_lowercase().contains("access")
                    || msg.contains("EACCES")
                {
                    Err(TransportError::PermissionDenied {
                        path: path_str,
                        detail: msg,
                        udev_rule,
                    })
                } else {
                    Err(TransportError::Hid(err))
                }
            }
        }
    }
}

fn select_vendor_interface<'a>(
    candidates: &[&'a DeviceInfo],
    sysfs: &[SysfsHidrawDevice],
    usage_page: u16,
    usage: u16,
    fallback_iface: Option<i32>,
) -> Option<&'a DeviceInfo> {
    // 1. Direct hidapi usage_page/usage match (works on macOS, Windows, and modern hidraw).
    let exact_usage: Vec<&DeviceInfo> = candidates
        .iter()
        .copied()
        .filter(|d| d.usage_page() == usage_page && d.usage() == usage)
        .collect();
    if exact_usage.len() == 1 {
        return Some(exact_usage[0]);
    }

    let page_only: Vec<&DeviceInfo> = candidates
        .iter()
        .copied()
        .filter(|d| d.usage_page() == usage_page)
        .collect();
    if page_only.len() == 1 {
        return Some(page_only[0]);
    }

    // 2. Linux sysfs report-descriptor match (immune to hidapi usage_page=0 quirk).
    let sysfs_vendor: Vec<&SysfsHidrawDevice> = sysfs
        .iter()
        .filter(|s| {
            s.descriptor
                .as_ref()
                .is_some_and(|desc| desc.usage_page == Some(usage_page))
        })
        .collect();
    if sysfs_vendor.len() == 1 {
        let target = sysfs_vendor[0];
        if let Some(matched) = candidates.iter().copied().find(|d| {
            d.path().to_string_lossy() == target.dev_path
                || (target.interface_number.is_some()
                    && Some(d.interface_number()) == target.interface_number)
        }) {
            return Some(matched);
        }
    }

    // 3. Fallback to model's USB vendor interface number if unique.
    if let Some(iface_num) = fallback_iface {
        let iface_matches: Vec<&DeviceInfo> = candidates
            .iter()
            .copied()
            .filter(|d| d.interface_number() == iface_num)
            .collect();
        if iface_matches.len() == 1 {
            return Some(iface_matches[0]);
        }
    }

    let _ = (VID, PID); // retained in signature compatibility imports
    None
}

impl HidTransport for HidApiTransport {
    fn write_report(&mut self, report: &Report) -> Result<(), TransportError> {
        let wrote = self.device.write(report)?;
        if wrote < REPORT_LEN {
            return Err(TransportError::ShortWrite {
                wrote,
                expected: REPORT_LEN,
            });
        }
        Ok(())
    }

    fn read_report(&mut self, timeout_ms: i32) -> Result<Option<Vec<u8>>, TransportError> {
        let mut buf = [0u8; REPORT_LEN];
        let n = self.device.read_timeout(&mut buf, timeout_ms)?;
        if n == 0 {
            Ok(None)
        } else {
            Ok(Some(buf[..n].to_vec()))
        }
    }

    fn drain(&mut self) -> Result<(), TransportError> {
        let deadline = Instant::now() + Duration::from_millis(20);
        let mut buf = [0u8; REPORT_LEN];
        for _ in 0..64 {
            let n = self.device.read_timeout(&mut buf, 0)?;
            if n == 0 || Instant::now() >= deadline {
                break;
            }
        }
        Ok(())
    }

    fn device_description(&self) -> &str {
        &self.description
    }
}

