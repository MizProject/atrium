//! Non-destructive USB/HID discovery and capability probing for unsupported keyboards.
//!
//! Designed with strict privacy & safety boundaries:
//! - Never reads or exports serial numbers, usernames, hostnames, or local secrets.
//! - Runs passive sysfs/report-descriptor analysis by default (zero USB writes).
//! - Optional active probes send ONLY read-only Aula discovery queries (`0x01`, `0x0D`,
//!   `0x07/01`, `0x21/04`) on user-selected vendor-capable endpoints.

use std::collections::BTreeMap;
use std::fs;
use std::time::Duration;

use hidapi::HidApi;
use serde::{Deserialize, Serialize};

use crate::diagnostics::now_unix_ms;
use crate::hid_sysfs::{SysfsHidrawDevice, enumerate_sysfs_hidraw};
use crate::keyboards;
use crate::protocol::{BODY_LEN, REPORT_LEN, cmd, new_body, parse_init_info, parse_light_state, parse_max_trigger_travel, strip_report_id, wrap_report};

pub const RECOVERY_NOTICE: &str = "If the keyboard acted weird during probing, unplug and replug it, and then try resetting if possible by holding Fn + Esc for 10 seconds.";
pub const DEVELOPER_NOTICE: &str = "Once submitted via GitHub Issues, the developer may reply with follow-up questions; please check back and respond promptly so your AULA keyboard can be supported quickly.";
pub const PRIVACY_NOTICE: &str = "Sanitized hardware capability snapshot only. Serial numbers, usernames, hostnames, and secrets are omitted.";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HidCandidateSummary {
    pub hidraw_name: String,
    #[serde(skip_serializing)]
    pub dev_path: String,
    pub vid: u16,
    pub pid: u16,
    pub vid_hex: String,
    pub pid_hex: String,
    pub manufacturer: String,
    pub product: String,
    pub sysfs_name: String,
    pub interface_number: Option<i32>,
    pub kernel_driver: String,
    pub usage_page: Option<u16>,
    pub usage_page_hex: Option<String>,
    pub usage: Option<u16>,
    pub usage_hex: Option<String>,
    pub report_ids: Vec<u8>,
    pub input_bytes: Option<usize>,
    pub output_bytes: Option<usize>,
    pub feature_bytes: Option<usize>,
    pub report_descriptor_len: usize,
    pub readable_writable: bool,
    pub permission_error: Option<String>,
    pub known_supported_id: Option<String>,
    pub known_supported_name: Option<String>,
}

impl HidCandidateSummary {
    pub fn display_label(&self) -> String {
        let up = self
            .usage_page_hex
            .as_deref()
            .unwrap_or("UP?");
        let iface = self
            .interface_number
            .map_or_else(|| "if?".into(), |i| format!("if{i}"));
        let name = if !self.product.is_empty() {
            self.product.as_str()
        } else if !self.sysfs_name.is_empty() {
            self.sysfs_name.as_str()
        } else {
            "HID Device"
        };
        let access = if self.readable_writable { "R/W" } else { "NoPerm" };
        format!(
            "{} · {}:{} · {} · {} · {} ({})",
            self.hidraw_name, self.vid_hex, self.pid_hex, iface, up, name, access
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HidSiblingSummary {
    pub hidraw_name: String,
    pub interface_number: Option<i32>,
    pub usage_page_hex: Option<String>,
    pub usage_hex: Option<String>,
    pub input_bytes: Option<usize>,
    pub output_bytes: Option<usize>,
    pub feature_bytes: Option<usize>,
    pub readable_writable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeCheckStatus {
    Pass,
    Warn,
    Fail,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeCapabilityCheck {
    pub stage: String,
    pub name: String,
    pub status: ProbeCheckStatus,
    pub summary: String,
    pub raw_hex: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InferredUnknownCapabilities {
    pub has_vendor_usage_page: bool,
    pub has_64b_bidirectional_reports: bool,
    pub aula_win_protocol_responded: bool,
    pub detected_firmware: Option<String>,
    pub detected_build_date: Option<String>,
    pub detected_init_flags: Option<[u8; 2]>,
    pub max_travel_mm: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct UnknownProbeOptions {
    pub model_hint: String,
    pub allow_active_read_queries: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnknownKeyboardProbeReport {
    pub schema_version: u32,
    pub generated_at_unix_ms: u64,
    pub privacy_notice: String,
    pub developer_notice: String,
    pub recovery_notice: String,
    pub model_hint: String,
    pub selected_interface: HidCandidateSummary,
    pub report_descriptor_hex: Option<String>,
    pub sibling_interfaces: Vec<HidSiblingSummary>,
    pub inferred_capabilities: InferredUnknownCapabilities,
    pub checks: Vec<ProbeCapabilityCheck>,
}

impl UnknownKeyboardProbeReport {
    pub fn to_pretty_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Enumerate all connected HID interfaces without exposing user secrets or serial numbers.
pub fn enumerate_hid_candidates() -> Vec<HidCandidateSummary> {
    let sysfs = enumerate_sysfs_hidraw(None);
    let hidapi_meta = read_sanitized_hidapi_map();

    sysfs
        .into_iter()
        .map(|s| build_candidate_summary(&s, &hidapi_meta))
        .collect()
}

struct SanitizedHidApiMeta {
    manufacturer: String,
    product: String,
    usage_page: u16,
    usage: u16,
}

fn read_sanitized_hidapi_map() -> BTreeMap<(u16, u16, i32), SanitizedHidApiMeta> {
    let mut map = BTreeMap::new();
    let Ok(api) = HidApi::new() else {
        return map;
    };
    for info in api.device_list() {
        // Deliberately never read `info.serial_number()` to avoid exporting identifiers.
        map.entry((info.vendor_id(), info.product_id(), info.interface_number()))
            .or_insert_with(|| SanitizedHidApiMeta {
                manufacturer: info.manufacturer_string().unwrap_or("").trim().to_string(),
                product: info.product_string().unwrap_or("").trim().to_string(),
                usage_page: info.usage_page(),
                usage: info.usage(),
            });
    }
    map
}

fn build_candidate_summary(
    sys: &SysfsHidrawDevice,
    hidapi_meta: &BTreeMap<(u16, u16, i32), SanitizedHidApiMeta>,
) -> HidCandidateSummary {
    let api_match = sys
        .interface_number
        .and_then(|iface| hidapi_meta.get(&(sys.vid, sys.pid, iface)));
    let usage_page = sys
        .descriptor
        .as_ref()
        .and_then(|d| d.usage_page)
        .or_else(|| api_match.and_then(|m| (m.usage_page != 0).then_some(m.usage_page)));
    let usage = sys
        .descriptor
        .as_ref()
        .and_then(|d| d.usage)
        .or_else(|| api_match.and_then(|m| (m.usage != 0).then_some(m.usage)));
    let report_ids = sys
        .descriptor
        .as_ref()
        .map(|d| d.report_ids.clone())
        .unwrap_or_default();
    let report_descriptor_len = read_raw_report_descriptor(&sys.hidraw_name)
        .map(|b| b.len())
        .unwrap_or(0);
    let known = keyboards::match_usb_id(sys.vid, sys.pid);

    HidCandidateSummary {
        hidraw_name: sys.hidraw_name.clone(),
        dev_path: sys.dev_path.clone(),
        vid: sys.vid,
        pid: sys.pid,
        vid_hex: format!("{:04X}", sys.vid),
        pid_hex: format!("{:04X}", sys.pid),
        manufacturer: api_match.map_or_else(String::new, |m| m.manufacturer.clone()),
        product: api_match.map_or_else(String::new, |m| m.product.clone()),
        sysfs_name: sys.name.clone(),
        interface_number: sys.interface_number,
        kernel_driver: sys.driver_name.clone(),
        usage_page,
        usage_page_hex: usage_page.map(|up| format!("0x{up:04X}")),
        usage,
        usage_hex: usage.map(|u| format!("0x{u:02X}")),
        report_ids,
        input_bytes: sys.descriptor.as_ref().and_then(|d| d.input_bytes),
        output_bytes: sys.descriptor.as_ref().and_then(|d| d.output_bytes),
        feature_bytes: sys.descriptor.as_ref().and_then(|d| d.feature_bytes),
        report_descriptor_len,
        readable_writable: sys.readable_writable,
        permission_error: sys.permission_error.clone(),
        known_supported_id: known.map(|k| k.id.to_string()),
        known_supported_name: known.map(|k| k.display_name.to_string()),
    }
}

fn read_raw_report_descriptor(hidraw_name: &str) -> Option<Vec<u8>> {
    if hidraw_name.is_empty()
        || hidraw_name.contains('/')
        || hidraw_name.contains('\\')
        || hidraw_name.contains("..")
    {
        return None;
    }
    let path = format!("/sys/class/hidraw/{hidraw_name}/device/report_descriptor");
    fs::read(path).ok()
}

/// Probe a user-selected HID device path safely and produce a shareable JSON capability report.
pub fn probe_unknown_hid_device(
    dev_path: &str,
    opts: &UnknownProbeOptions,
) -> Result<UnknownKeyboardProbeReport, String> {
    let all = enumerate_hid_candidates();
    let selected = all
        .iter()
        .find(|d| d.dev_path == dev_path || d.hidraw_name == dev_path)
        .cloned()
        .ok_or_else(|| format!("Selected HID interface '{dev_path}' is no longer connected."))?;

    let sibling_interfaces: Vec<HidSiblingSummary> = all
        .iter()
        .filter(|d| d.vid == selected.vid && d.pid == selected.pid)
        .map(|d| HidSiblingSummary {
            hidraw_name: d.hidraw_name.clone(),
            interface_number: d.interface_number,
            usage_page_hex: d.usage_page_hex.clone(),
            usage_hex: d.usage_hex.clone(),
            input_bytes: d.input_bytes,
            output_bytes: d.output_bytes,
            feature_bytes: d.feature_bytes,
            readable_writable: d.readable_writable,
        })
        .collect();

    let raw_desc = read_raw_report_descriptor(&selected.hidraw_name);
    let report_descriptor_hex = raw_desc
        .as_ref()
        .map(|bytes| encode_hex_limited(bytes, 1024));

    let mut checks = Vec::new();
    let mut inferred = InferredUnknownCapabilities::default();

    let has_vendor_up = selected.usage_page.is_some_and(|up| up >= 0xFF00);
    let has_64b = selected.input_bytes == Some(REPORT_LEN) && selected.output_bytes == Some(REPORT_LEN);
    inferred.has_vendor_usage_page = has_vendor_up;
    inferred.has_64b_bidirectional_reports = has_64b;

    checks.push(ProbeCapabilityCheck {
        stage: "sysfs".into(),
        name: "HID Report Descriptor".into(),
        status: if raw_desc.is_some() {
            ProbeCheckStatus::Pass
        } else {
            ProbeCheckStatus::Warn
        },
        summary: format!(
            "UsagePage={} Usage={} IN={:?} OUT={:?} FEAT={:?} IDs={:?} ({} bytes)",
            selected.usage_page_hex.as_deref().unwrap_or("unknown"),
            selected.usage_hex.as_deref().unwrap_or("unknown"),
            selected.input_bytes,
            selected.output_bytes,
            selected.feature_bytes,
            selected.report_ids,
            selected.report_descriptor_len
        ),
        raw_hex: None,
    });

    checks.push(ProbeCapabilityCheck {
        stage: "sysfs".into(),
        name: "Userspace R/W Permission".into(),
        status: if selected.readable_writable {
            ProbeCheckStatus::Pass
        } else {
            ProbeCheckStatus::Warn
        },
        summary: if selected.readable_writable {
            format!("{} is readable and writable by this process.", selected.hidraw_name)
        } else {
            format!(
                "{} is blocked ({}). Install udev rule or grant temporary access for active probing.",
                selected.hidraw_name,
                selected.permission_error.as_deref().unwrap_or("permission denied")
            )
        },
        raw_hex: None,
    });

    // Attempt passive HID GET_FEATURE only when the descriptor declares feature bytes and R/W access exists.
    if selected.feature_bytes.unwrap_or(0) > 0 && selected.readable_writable {
        run_passive_feature_checks(&selected, &mut checks);
    }

    if opts.allow_active_read_queries {
        if !selected.readable_writable {
            checks.push(ProbeCapabilityCheck {
                stage: "active_vendor_read".into(),
                name: "Read-only vendor queries (0x01/0x0D/0x07/0x21)".into(),
                status: ProbeCheckStatus::Skipped,
                summary: "Skipped active queries because current user lacks read/write permission on this hidraw node.".into(),
                raw_hex: None,
            });
        } else if !has_vendor_up && !has_64b {
            checks.push(ProbeCapabilityCheck {
                stage: "active_vendor_read".into(),
                name: "Safety guard for non-vendor HID endpoint".into(),
                status: ProbeCheckStatus::Skipped,
                summary: format!(
                    "Skipped writing 64B vendor packets to non-vendor endpoint (UsagePage={}, IN={:?}, OUT={:?}) to avoid confusing boot keyboard/mouse interfaces.",
                    selected.usage_page_hex.as_deref().unwrap_or("?"),
                    selected.input_bytes,
                    selected.output_bytes
                ),
                raw_hex: None,
            });
        } else {
            run_active_readonly_vendor_probe(&selected, &mut inferred, &mut checks);
        }
    } else {
        checks.push(ProbeCapabilityCheck {
            stage: "active_vendor_read".into(),
            name: "Active vendor readback probe".into(),
            status: ProbeCheckStatus::Skipped,
            summary: "Passive mode only (enable read-only active queries to test WIN-series commands 0x01/0x0D/0x07/0x21).".into(),
            raw_hex: None,
        });
    }

    Ok(UnknownKeyboardProbeReport {
        schema_version: 1,
        generated_at_unix_ms: now_unix_ms(),
        privacy_notice: PRIVACY_NOTICE.into(),
        developer_notice: DEVELOPER_NOTICE.into(),
        recovery_notice: RECOVERY_NOTICE.into(),
        model_hint: opts.model_hint.trim().to_string(),
        selected_interface: selected,
        report_descriptor_hex,
        sibling_interfaces,
        inferred_capabilities: inferred,
        checks,
    })
}

fn open_selected_hidapi(selected: &HidCandidateSummary) -> Result<hidapi::HidDevice, String> {
    let api = HidApi::new().map_err(|e| format!("HidApi init failed: {e}"))?;
    let info = api
        .device_list()
        .find(|d| d.path().to_string_lossy() == selected.dev_path)
        .or_else(|| {
            selected.interface_number.and_then(|iface| {
                api.device_list().find(|d| {
                    d.vendor_id() == selected.vid
                        && d.product_id() == selected.pid
                        && d.interface_number() == iface
                })
            })
        })
        .ok_or_else(|| format!("HID interface {} not found in hidapi list", selected.hidraw_name))?;

    info.open_device(&api)
        .map_err(|e| format!("Failed opening {}: {e}", selected.hidraw_name))
}

fn run_passive_feature_checks(
    selected: &HidCandidateSummary,
    checks: &mut Vec<ProbeCapabilityCheck>,
) {
    let dev = match open_selected_hidapi(selected) {
        Ok(d) => d,
        Err(e) => {
            checks.push(ProbeCapabilityCheck {
                stage: "hid_feature".into(),
                name: "Passive HID Feature Report GET".into(),
                status: ProbeCheckStatus::Warn,
                summary: e,
                raw_hex: None,
            });
            return;
        }
    };

    let ids: Vec<u8> = if selected.report_ids.is_empty() {
        vec![0x00]
    } else {
        selected.report_ids.iter().copied().take(4).collect()
    };

    for id in ids {
        let mut buf = [0u8; 65];
        buf[0] = id;
        match dev.get_feature_report(&mut buf) {
            Ok(len) => checks.push(ProbeCapabilityCheck {
                stage: "hid_feature".into(),
                name: format!("GET_FEATURE Report ID 0x{id:02X}"),
                status: ProbeCheckStatus::Pass,
                summary: format!("Returned {len} bytes"),
                raw_hex: Some(encode_hex_limited(&buf[..len.min(buf.len())], 64)),
            }),
            Err(e) => checks.push(ProbeCapabilityCheck {
                stage: "hid_feature".into(),
                name: format!("GET_FEATURE Report ID 0x{id:02X}"),
                status: ProbeCheckStatus::Warn,
                summary: format!("No response / unsupported ({e})"),
                raw_hex: None,
            }),
        }
    }
}

fn run_active_readonly_vendor_probe(
    selected: &HidCandidateSummary,
    inferred: &mut InferredUnknownCapabilities,
    checks: &mut Vec<ProbeCapabilityCheck>,
) {
    let dev = match open_selected_hidapi(selected) {
        Ok(d) => d,
        Err(e) => {
            checks.push(ProbeCapabilityCheck {
                stage: "active_vendor_read".into(),
                name: "Open selected vendor HID interface".into(),
                status: ProbeCheckStatus::Fail,
                summary: e,
                raw_hex: None,
            });
            return;
        }
    };

    // Drain any stale buffered IN reports cleanly before querying.
    let mut drain_buf = [0u8; REPORT_LEN];
    for _ in 0..8 {
        if dev.read_timeout(&mut drain_buf, 0).unwrap_or(0) == 0 {
            break;
        }
    }

    // 1. Heartbeat / capabilities (0x01)
    let hb_body = new_body(cmd::HEARTBEAT);
    match query_readonly_frame(&dev, &hb_body, cmd::HEARTBEAT) {
        Ok((raw, body)) => {
            inferred.aula_win_protocol_responded = true;
            if let Some(info) = parse_init_info(&body) {
                inferred.detected_init_flags = Some([info.raw_flags1, info.raw_flags2]);
                checks.push(ProbeCapabilityCheck {
                    stage: "active_vendor_read".into(),
                    name: "Heartbeat / InitInfo (cmd 0x01)".into(),
                    status: ProbeCheckStatus::Pass,
                    summary: format!(
                        "mode={}, flags1=0x{:02X}, flags2=0x{:02X}, prcs={}, custom_rgb={}, hp={}",
                        info.connection_mode(),
                        info.raw_flags1,
                        info.raw_flags2,
                        info.is_prcs,
                        info.is_custom_light,
                        info.is_high_precision,
                    ),
                    raw_hex: Some(encode_hex_limited(&raw, 64)),
                });
            } else {
                checks.push(ProbeCapabilityCheck {
                    stage: "active_vendor_read".into(),
                    name: "Heartbeat / InitInfo (cmd 0x01)".into(),
                    status: ProbeCheckStatus::Warn,
                    summary: format!("Received {} byte reply, unparsed framing.", raw.len()),
                    raw_hex: Some(encode_hex_limited(&raw, 64)),
                });
            }
        }
        Err(e) => checks.push(ProbeCapabilityCheck {
            stage: "active_vendor_read".into(),
            name: "Heartbeat / InitInfo (cmd 0x01)".into(),
            status: ProbeCheckStatus::Warn,
            summary: e,
            raw_hex: None,
        }),
    }

    // 2. Device info / firmware string (0x0D)
    let mut dev_body = new_body(cmd::DEVICE_INFO);
    dev_body[1] = 0x01;
    match query_readonly_frame(&dev, &dev_body, cmd::DEVICE_INFO) {
        Ok((raw, body)) => {
            inferred.aula_win_protocol_responded = true;
            let ascii_preview = extract_ascii_runs(&body);
            if !ascii_preview.is_empty() {
                inferred.detected_firmware = Some(ascii_preview.clone());
            }
            checks.push(ProbeCapabilityCheck {
                stage: "active_vendor_read".into(),
                name: "Device Info / Firmware (cmd 0x0D)".into(),
                status: ProbeCheckStatus::Pass,
                summary: if ascii_preview.is_empty() {
                    format!("Received {} byte reply", raw.len())
                } else {
                    format!("Strings: {ascii_preview}")
                },
                raw_hex: Some(encode_hex_limited(&raw, 64)),
            });
        }
        Err(e) => checks.push(ProbeCapabilityCheck {
            stage: "active_vendor_read".into(),
            name: "Device Info / Firmware (cmd 0x0D)".into(),
            status: ProbeCheckStatus::Warn,
            summary: e,
            raw_hex: None,
        }),
    }

    // 3. Light state readback (0x07/01)
    let mut light_body = new_body(cmd::LIGHT);
    light_body[1] = 0x01;
    match query_readonly_frame(&dev, &light_body, cmd::LIGHT) {
        Ok((raw, body)) => {
            inferred.aula_win_protocol_responded = true;
            let parsed = parse_light_state(&body);
            checks.push(ProbeCapabilityCheck {
                stage: "active_vendor_read".into(),
                name: "Lighting Readback (cmd 0x07/01)".into(),
                status: ProbeCheckStatus::Pass,
                summary: parsed.map_or_else(
                    || format!("Received {} byte reply", raw.len()),
                    |ls| {
                        format!(
                            "mode={} bri={} spd={} fg=#{:02X}{:02X}{:02X}",
                            ls.mode, ls.brightness, ls.speed, ls.fg[0], ls.fg[1], ls.fg[2]
                        )
                    },
                ),
                raw_hex: Some(encode_hex_limited(&raw, 64)),
            });
        }
        Err(e) => checks.push(ProbeCapabilityCheck {
            stage: "active_vendor_read".into(),
            name: "Lighting Readback (cmd 0x07/01)".into(),
            status: ProbeCheckStatus::Warn,
            summary: e,
            raw_hex: None,
        }),
    }

    // 4. Max travel readback (0x21/04) for Hall-effect detection
    let mut trig_body = new_body(cmd::TRIGGER);
    trig_body[1] = 0x04;
    match query_readonly_frame(&dev, &trig_body, cmd::TRIGGER) {
        Ok((raw, body)) => {
            inferred.aula_win_protocol_responded = true;
            let hp = inferred
                .detected_init_flags
                .is_some_and(|[_, f2]| (f2 & 0x08) != 0);
            let max_travel = parse_max_trigger_travel(&body, hp);
            if let Some(m) = max_travel {
                inferred.max_travel_mm = Some(m.max_mm);
            }
            checks.push(ProbeCapabilityCheck {
                stage: "active_vendor_read".into(),
                name: "Travel Range Readback (cmd 0x21/04)".into(),
                status: ProbeCheckStatus::Pass,
                summary: max_travel.map_or_else(
                    || format!("Received {} byte reply", raw.len()),
                    |m| format!("{:.2}..={:.2} mm (step {:.3} mm)", m.min_mm, m.max_mm, m.unit_mm),
                ),
                raw_hex: Some(encode_hex_limited(&raw, 64)),
            });
        }
        Err(e) => checks.push(ProbeCapabilityCheck {
            stage: "active_vendor_read".into(),
            name: "Travel Range Readback (cmd 0x21/04)".into(),
            status: ProbeCheckStatus::Warn,
            summary: e,
            raw_hex: None,
        }),
    }
}

fn query_readonly_frame(
    dev: &hidapi::HidDevice,
    body: &[u8; BODY_LEN],
    expected_cmd: u8,
) -> Result<(Vec<u8>, [u8; BODY_LEN]), String> {
    let report = wrap_report(body);
    let wrote = dev.write(&report).map_err(|e| format!("HID write failed: {e}"))?;
    if wrote == 0 {
        return Err("HID write returned 0 bytes".into());
    }

    let deadline = std::time::Instant::now() + Duration::from_millis(220);
    let mut buf = [0u8; REPORT_LEN];
    loop {
        let rem_ms = deadline
            .saturating_duration_since(std::time::Instant::now())
            .as_millis()
            .min(180) as i32;
        if rem_ms <= 0 {
            return Err("Timed out waiting for HID response".into());
        }
        let n = dev
            .read_timeout(&mut buf, rem_ms)
            .map_err(|e| format!("HID read failed: {e}"))?;
        if n == 0 {
            return Err("Timed out waiting for HID response".into());
        }
        let raw = buf[..n].to_vec();
        if let Some(stripped) = strip_report_id(&raw) {
            if stripped.first().copied() == Some(expected_cmd) || n == REPORT_LEN {
                let mut out_body = [0u8; BODY_LEN];
                let copy_len = stripped.len().min(BODY_LEN);
                out_body[..copy_len].copy_from_slice(&stripped[..copy_len]);
                return Ok((raw, out_body));
            }
        } else {
            return Err(format!("Unexpected {n}-byte response framing"));
        }
    }
}

fn encode_hex_limited(bytes: &[u8], max_bytes: usize) -> String {
    bytes
        .iter()
        .take(max_bytes)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn extract_ascii_runs(bytes: &[u8]) -> String {
    let mut runs = Vec::new();
    let mut current = String::new();
    for &b in bytes {
        if (0x20..=0x7E).contains(&b) {
            current.push(b as char);
        } else {
            if current.len() >= 4 {
                runs.push(std::mem::take(&mut current));
            }
            current.clear();
        }
    }
    if current.len() >= 4 {
        runs.push(current);
    }
    runs.join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_report_json_omits_dev_path_and_keeps_recovery_notice() {
        let candidate = HidCandidateSummary {
            hidraw_name: "hidraw9".into(),
            dev_path: "/dev/hidraw9".into(),
            vid: 0x2E3C,
            pid: 0x9999,
            vid_hex: "2E3C".into(),
            pid_hex: "9999".into(),
            manufacturer: "AULA".into(),
            product: "AULA F75".into(),
            sysfs_name: "AULA F75".into(),
            interface_number: Some(2),
            kernel_driver: "hid-generic".into(),
            usage_page: Some(0xFF1B),
            usage_page_hex: Some("0xFF1B".into()),
            usage: Some(0x91),
            usage_hex: Some("0x91".into()),
            report_ids: vec![1],
            input_bytes: Some(64),
            output_bytes: Some(64),
            feature_bytes: None,
            report_descriptor_len: 32,
            readable_writable: false,
            permission_error: Some("Permission denied".into()),
            known_supported_id: None,
            known_supported_name: None,
        };

        let report = UnknownKeyboardProbeReport {
            schema_version: 1,
            generated_at_unix_ms: 123456,
            privacy_notice: PRIVACY_NOTICE.into(),
            developer_notice: DEVELOPER_NOTICE.into(),
            recovery_notice: RECOVERY_NOTICE.into(),
            model_hint: "AULA F75".into(),
            selected_interface: candidate,
            report_descriptor_hex: Some("061bff".into()),
            sibling_interfaces: vec![],
            inferred_capabilities: InferredUnknownCapabilities::default(),
            checks: vec![],
        };

        let json = report.to_pretty_json().expect("json serialization");
        assert!(json.contains("hidraw9"));
        assert!(json.contains("Fn + Esc"));
        assert!(json.contains("developer"));
        assert!(!json.contains("\"/dev/hidraw9\""));
        assert!(!json.contains("serial"));
    }

    #[test]
    fn enumerate_hid_candidates_runs_without_panicking() {
        let _ = enumerate_hid_candidates();
    }
}
