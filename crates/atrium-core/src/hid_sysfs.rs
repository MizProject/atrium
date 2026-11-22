//! Linux sysfs inspection and HID Report Descriptor parser.
//!
//! Inspecting `/sys/class/hidraw/*` works without opening `/dev/hidraw*`
//! (so diagnostics work even before a udev rule grants read/write access),
//! and avoids the Linux `hidapi` quirk where `usage_page` can be reported as 0.

use std::fmt;

/// Parsed HID usage and report summary from a raw HID Report Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportDescriptorSummary {
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    pub report_ids: Vec<u8>,
    pub input_bytes: Option<usize>,
    pub output_bytes: Option<usize>,
    pub feature_bytes: Option<usize>,
}

/// Metadata discovered for a `/dev/hidrawN` node on Linux.
#[derive(Debug, Clone)]
pub struct SysfsHidrawDevice {
    pub hidraw_name: String,
    pub dev_path: String,
    pub vid: u16,
    pub pid: u16,
    pub name: String,
    pub phys: String,
    pub driver_name: String,
    pub interface_number: Option<i32>,
    pub descriptor: Option<ReportDescriptorSummary>,
    pub readable_writable: bool,
    pub permission_error: Option<String>,
}

impl fmt::Display for ReportDescriptorSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.usage_page, self.usage) {
            (Some(up), Some(u)) => write!(f, "UsagePage=0x{up:04X} Usage=0x{u:02X}")?,
            (Some(up), None) => write!(f, "UsagePage=0x{up:04X}")?,
            _ => write!(f, "UsagePage=unknown")?,
        }
        if !self.report_ids.is_empty() {
            write!(f, " ReportIDs={:?}", self.report_ids)?;
        }
        if let Some(n) = self.input_bytes {
            write!(f, " IN={n}B")?;
        }
        if let Some(n) = self.output_bytes {
            write!(f, " OUT={n}B")?;
        }
        if let Some(n) = self.feature_bytes {
            write!(f, " FEAT={n}B")?;
        }
        Ok(())
    }
}

/// Parse the primary top-level Application Collection and report sizes from a
/// USB HID Report Descriptor byte slice.
pub fn parse_report_descriptor(bytes: &[u8]) -> ReportDescriptorSummary {
    let mut i = 0;
    let mut depth: i32 = 0;
    let mut global_usage_page: Option<u16> = None;
    let mut local_usage: Option<u16> = None;
    let mut top_usage_page: Option<u16> = None;
    let mut top_usage: Option<u16> = None;
    let mut report_size: u32 = 0;
    let mut report_count: u32 = 0;
    let mut current_report_id: Option<u8> = None;
    let mut report_ids = Vec::new();
    let mut input_bits: u32 = 0;
    let mut output_bits: u32 = 0;
    let mut feature_bits: u32 = 0;

    while i < bytes.len() {
        let prefix = bytes[i];
        if prefix == 0xFE {
            // Long item: [0xFE, data_size, long_item_tag, ...]
            if i + 2 >= bytes.len() {
                break;
            }
            let len = bytes[i + 1] as usize;
            i += 3 + len;
            continue;
        }

        let size = match prefix & 0x03 {
            0 => 0usize,
            1 => 1,
            2 => 2,
            _ => 4,
        };
        let item_type = (prefix >> 2) & 0x03;
        let tag = (prefix >> 4) & 0x0F;

        if i + 1 + size > bytes.len() {
            break;
        }
        let val = read_le_uint(&bytes[i + 1..i + 1 + size]);
        i += 1 + size;

        match (item_type, tag) {
            // Global items
            (1, 0) => global_usage_page = Some(val as u16),
            (1, 7) => report_size = val,
            (1, 8) => {
                let id = val as u8;
                current_report_id = Some(id);
                if !report_ids.contains(&id) {
                    report_ids.push(id);
                }
            }
            (1, 9) => report_count = val,
            // Local items
            (2, 0) => {
                if local_usage.is_none() {
                    local_usage = Some(val as u16);
                }
            }
            // Main items
            (0, 10) => {
                // Collection
                if depth == 0 && val == 0x01 && top_usage_page.is_none() {
                    top_usage_page = global_usage_page;
                    top_usage = local_usage;
                }
                depth += 1;
                local_usage = None;
            }
            (0, 12) => {
                // End Collection
                depth = (depth - 1).max(0);
                local_usage = None;
            }
            (0, 8) => {
                // Input
                input_bits = input_bits.saturating_add(report_size.saturating_mul(report_count));
                local_usage = None;
            }
            (0, 9) => {
                // Output
                output_bits = output_bits.saturating_add(report_size.saturating_mul(report_count));
                local_usage = None;
            }
            (0, 11) => {
                // Feature
                feature_bits = feature_bits.saturating_add(report_size.saturating_mul(report_count));
                local_usage = None;
            }
            (0, _) => {
                local_usage = None;
            }
            _ => {}
        }
    }

    let id_overhead = usize::from(current_report_id.is_some());
    let bits_to_bytes = |bits: u32| -> Option<usize> {
        if bits == 0 {
            None
        } else {
            Some(((bits as usize).div_ceil(8)) + id_overhead)
        }
    };

    ReportDescriptorSummary {
        usage_page: top_usage_page.or(global_usage_page),
        usage: top_usage,
        report_ids,
        input_bytes: bits_to_bytes(input_bits),
        output_bytes: bits_to_bytes(output_bits),
        feature_bytes: bits_to_bytes(feature_bits),
    }
}

fn read_le_uint(slice: &[u8]) -> u32 {
    let mut out = 0u32;
    for (idx, &b) in slice.iter().enumerate() {
        out |= (b as u32) << (idx * 8);
    }
    out
}

/// Enumerate `/sys/class/hidraw/*` on Linux, returning all matching (or all if
/// `vid_pid_filter` is `None`) devices sorted by hidraw name.
#[cfg(target_os = "linux")]
pub fn enumerate_sysfs_hidraw(vid_pid_filter: Option<(u16, u16)>) -> Vec<SysfsHidrawDevice> {
    use std::fs::{self, OpenOptions};
    use std::path::Path;

    let Ok(entries) = fs::read_dir("/sys/class/hidraw") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let hidraw_name = entry.file_name().to_string_lossy().into_owned();
        let sys_path = entry.path();
        let uevent_path = sys_path.join("device/uevent");
        let Ok(uevent) = fs::read_to_string(&uevent_path) else {
            continue;
        };

        let Some((vid, pid, name, phys, driver_name)) = parse_uevent(&uevent) else {
            continue;
        };
        if let Some((want_vid, want_pid)) = vid_pid_filter
            && (vid != want_vid || pid != want_pid)
        {
            continue;
        }

        let interface_number = fs::canonicalize(&sys_path)
            .ok()
            .and_then(|p| extract_usb_interface(&p))
            .or_else(|| extract_phys_interface(&phys));

        let descriptor = fs::read(sys_path.join("device/report_descriptor"))
            .ok()
            .map(|bytes| parse_report_descriptor(&bytes));

        let dev_path = format!("/dev/{hidraw_name}");
        let (readable_writable, permission_error) = if Path::new(&dev_path).exists() {
            match OpenOptions::new().read(true).write(true).open(&dev_path) {
                Ok(_) => (true, None),
                Err(e) => (false, Some(e.to_string())),
            }
        } else {
            (false, Some("device node not present".into()))
        };

        out.push(SysfsHidrawDevice {
            hidraw_name,
            dev_path,
            vid,
            pid,
            name,
            phys,
            driver_name,
            interface_number,
            descriptor,
            readable_writable,
            permission_error,
        });
    }

    out.sort_by(|a, b| a.hidraw_name.cmp(&b.hidraw_name));
    out
}

#[cfg(not(target_os = "linux"))]
pub fn enumerate_sysfs_hidraw(_vid_pid_filter: Option<(u16, u16)>) -> Vec<SysfsHidrawDevice> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn parse_uevent(uevent: &str) -> Option<(u16, u16, String, String, String)> {
    let mut vid = None;
    let mut pid = None;
    let mut name = String::new();
    let mut phys = String::new();
    let mut driver_name = String::new();

    for line in uevent.lines() {
        if let Some(rest) = line.strip_prefix("HID_ID=") {
            // Format: BBBB:VVVVVVVV:PPPPPPPP
            let mut parts = rest.split(':');
            let _bus = parts.next();
            if let (Some(v), Some(p)) = (parts.next(), parts.next()) {
                vid = u16::from_str_radix(v.trim_start_matches('0'), 16)
                    .ok()
                    .or_else(|| u16::from_str_radix(v, 16).ok());
                pid = u16::from_str_radix(p.trim_start_matches('0'), 16)
                    .ok()
                    .or_else(|| u16::from_str_radix(p, 16).ok());
            }
        } else if let Some(rest) = line.strip_prefix("HID_NAME=") {
            name = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("HID_PHYS=") {
            phys = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("DRIVER=") {
            driver_name = rest.to_string();
        }
    }

    Some((vid?, pid?, name, phys, driver_name))
}

#[cfg(target_os = "linux")]
fn extract_usb_interface(path: &std::path::Path) -> Option<i32> {
    // Walk deepest-first so `1-7:1.2` wins over PCI `0000:00:14.0`, and ignore
    // segments whose right-hand side contains another `:` (PCI domain:bus).
    for comp in path.components().rev() {
        let s = comp.as_os_str().to_string_lossy();
        if let Some((_, iface_part)) = s.split_once(':')
            && !iface_part.contains(':')
            && let Some((_, iface_num)) = iface_part.split_once('.')
            && let Ok(n) = iface_num.parse::<i32>()
        {
            return Some(n);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn extract_phys_interface(phys: &str) -> Option<i32> {
    // e.g. `usb-0000:00:14.0-7/input2`
    phys.rsplit("input")
        .next()
        .and_then(|s| s.parse::<i32>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_win60_vendor_descriptor() {
        // Exact bytes from /sys/class/hidraw/hidraw3/device/report_descriptor
        let desc = [
            0x06, 0x1b, 0xff, 0x09, 0x91, 0xa1, 0x01, 0x85, 0x01, 0x19, 0x00, 0x2a, 0xff, 0x00,
            0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x3f, 0x81, 0x02, 0x19, 0x00, 0x29,
            0xff, 0x91, 0x02, 0xc0,
        ];
        let parsed = parse_report_descriptor(&desc);
        assert_eq!(parsed.usage_page, Some(0xFF1B));
        assert_eq!(parsed.usage, Some(0x91));
        assert_eq!(parsed.report_ids, vec![1]);
        assert_eq!(parsed.input_bytes, Some(64));
        assert_eq!(parsed.output_bytes, Some(64));
        assert_eq!(parsed.feature_bytes, None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn extracts_usb_interface_ignoring_pci_address() {
        let p = std::path::Path::new(
            "/sys/devices/pci0000:00/0000:00:14.0/usb1/1-7/1-7:1.2/0003:2E3C:C365.0010/hidraw/hidraw3",
        );
        assert_eq!(extract_usb_interface(p), Some(2));
    }
}
