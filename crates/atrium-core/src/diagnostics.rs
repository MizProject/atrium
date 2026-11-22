//! 4-layer diagnostic suite separating Kernel/USB, Daemon/IPC, Keyboard, and Software faults.

use std::ffi::CStr;
use std::fs;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::driver::Win60Driver;
use crate::hid_sysfs::enumerate_sysfs_hidraw;
use crate::keyboards::{self, KeyboardSpec};
use crate::keymap;
use crate::protocol::{self, BODY_LEN, TriggerMode};
use crate::transport::HidTransport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticLayer {
    Kernel,
    Daemon,
    Keyboard,
    Software,
}

impl DiagnosticLayer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Kernel => "Kernel / USB",
            Self::Daemon => "Daemon / IPC",
            Self::Keyboard => "Keyboard Hardware",
            Self::Software => "Software / Codec",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl CheckStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticCheck {
    pub layer: DiagnosticLayer,
    pub name: String,
    pub status: CheckStatus,
    pub latency_ms: Option<f64>,
    pub detail: String,
    pub remedy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaultAttribution {
    pub primary_fault: Option<DiagnosticLayer>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub generated_at_unix_ms: u64,
    pub checks: Vec<DiagnosticCheck>,
    pub attribution: FaultAttribution,
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Inspect Linux kernel USB/HID enumeration, sysfs driver binding, and descriptor health
/// for the default indexed keyboard (Aula WIN 60 HE).
pub fn run_kernel_checks() -> Vec<DiagnosticCheck> {
    run_kernel_checks_for(&keyboards::win60he::SPEC)
}

/// Inspect Linux kernel USB/HID enumeration, sysfs driver binding, and descriptor health
/// for a specific indexed [`KeyboardSpec`].
pub fn run_kernel_checks_for(spec: &KeyboardSpec) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();
    let t0 = Instant::now();
    let mut devs = Vec::new();
    for usb in spec.usb_ids {
        devs.extend(enumerate_sysfs_hidraw(Some((usb.vid, usb.pid))));
    }
    let scan_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let primary = spec.primary_usb_id();

    if devs.is_empty() {
        checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Kernel,
            name: "USB HID sysfs enumeration".into(),
            status: CheckStatus::Fail,
            latency_ms: Some(scan_ms),
            detail: format!(
                "No /sys/class/hidraw devices found for {} ({:04X}:{:04X})",
                spec.display_name, primary.vid, primary.pid
            ),
            remedy: Some(format!(
                "Check USB cable/port, verify `lsusb` sees {:04x}:{:04x}, and ensure `usbhid`/`hid-generic` kernel modules are loaded.",
                primary.vid, primary.pid
            )),
        });
        return checks;
    }

    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Kernel,
        name: "USB HID sysfs enumeration".into(),
        status: CheckStatus::Pass,
        latency_ms: Some(scan_ms),
        detail: format!(
            "Found {} hidraw interface(s) for {} ({:04X}:{:04X})",
            devs.len(),
            spec.display_name,
            primary.vid,
            primary.pid
        ),
        remedy: None,
    });

    let vendor = devs
        .iter()
        .find(|d| keyboards::is_vendor_sysfs_device(d, spec));

    let Some(v) = vendor else {
        checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Kernel,
            name: "Vendor HID report descriptor".into(),
            status: CheckStatus::Fail,
            latency_ms: None,
            detail: format!(
                "Found {} USB interfaces, but none exposed UsagePage 0x{:04X}",
                spec.display_name, spec.vendor_usage_page
            ),
            remedy: Some("Replug keyboard or inspect `dmesg` for USB HID descriptor parse errors.".into()),
        });
        return checks;
    };

    let desc = v.descriptor.as_ref();
    let desc_ok = desc.is_some_and(|d| {
        d.usage_page == Some(spec.vendor_usage_page)
            && d.usage == Some(spec.vendor_usage)
            && d.input_bytes == Some(64)
            && d.output_bytes == Some(64)
    });
    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Kernel,
        name: format!("Vendor HID descriptor (0x{:04X})", spec.vendor_usage_page),
        status: if desc_ok { CheckStatus::Pass } else { CheckStatus::Warn },
        latency_ms: None,
        detail: format!(
            "{} (iface {:?}): {}",
            v.dev_path,
            v.interface_number,
            desc.map_or_else(|| "missing descriptor".into(), ToString::to_string)
        ),
        remedy: (!desc_ok).then(|| {
            format!(
                "Expected UsagePage=0x{:04X} Usage=0x{:02X} IN=64B OUT=64B.",
                spec.vendor_usage_page, spec.vendor_usage
            )
        }),
    });

    let driver_ok = v.driver_name == "hid-generic" || !v.driver_name.is_empty();
    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Kernel,
        name: "Kernel HID driver binding".into(),
        status: if driver_ok { CheckStatus::Pass } else { CheckStatus::Fail },
        latency_ms: None,
        detail: if v.driver_name.is_empty() {
            format!("No kernel DRIVER= reported for {}", v.dev_path)
        } else {
            format!("{} bound to kernel driver `{}`", v.dev_path, v.driver_name)
        },
        remedy: (!driver_ok).then(|| "Check `dmesg` and ensure `hid_generic` is loaded (`modprobe hid_generic`).".into()),
    });

    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Kernel,
        name: "Device node permissions".into(),
        status: if v.readable_writable { CheckStatus::Pass } else { CheckStatus::Warn },
        latency_ms: None,
        detail: if v.readable_writable {
            format!("{} is readable & writable by current process", v.dev_path)
        } else {
            format!(
                "{} blocked for current process ({}); root daemon or udev rule required",
                v.dev_path,
                v.permission_error.as_deref().unwrap_or("permission denied")
            )
        },
        remedy: (!v.readable_writable).then(|| {
            format!(
                "This is normal when unprivileged clients connect via root `atriumd`, or install {}.",
                spec.udev_rule
            )
        }),
    });

    checks.push(probe_uinput_check());

    checks
}

/// Read the booted Linux kernel release via `uname(2)`.
pub fn kernel_release() -> Option<String> {
    let mut uts = std::mem::MaybeUninit::<libc::utsname>::uninit();
    // SAFETY: `uts` is valid storage for `libc::uname`.
    if unsafe { libc::uname(uts.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `uname` initialized `uts` when returning 0.
    let uts = unsafe { uts.assume_init() };
    // SAFETY: `uts.release` is a NUL-terminated kernel release string.
    let s = unsafe { CStr::from_ptr(uts.release.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    (!s.is_empty()).then_some(s)
}

/// Produce a specific, actionable diagnosis for `/dev/uinput` open errors.
pub fn diagnose_uinput_open_error(err: &io::Error) -> (String, String) {
    match err.raw_os_error() {
        Some(libc::ENODEV) => {
            if let Some(rel) = kernel_release() {
                let mod_dir = format!("/lib/modules/{rel}");
                if !Path::new(&mod_dir).exists() {
                    let installed: Vec<String> = fs::read_dir("/lib/modules")
                        .ok()
                        .into_iter()
                        .flatten()
                        .filter_map(|e| e.ok().map(|entry| entry.file_name().to_string_lossy().into_owned()))
                        .collect();
                    return (
                        format!(
                            "/dev/uinput returned ENODEV: booted kernel '{rel}' has no {mod_dir} (installed modules: {installed:?})"
                        ),
                        "A kernel upgrade replaced /lib/modules while running the older kernel. Reboot Linux so uinput.ko loads and custom Fn -> Arrow/OS keybinds work.".into(),
                    );
                }
            }
            (
                format!("/dev/uinput returned ENODEV ({err}): kernel `uinput` module is not loaded"),
                "Load the kernel module with `sudo modprobe uinput` (then save/trigger any hotkey rule).".into(),
            )
        }
        Some(libc::EACCES) | Some(libc::EPERM) => (
            format!("Permission denied opening /dev/uinput ({err})"),
            "Run `atriumd` as root (`atrium daemon stop && atrium daemon start`) or grant /dev/uinput ACL access.".into(),
        ),
        Some(libc::ENOENT) => (
            "/dev/uinput device node does not exist".into(),
            "Ensure kernel CONFIG_INPUT_UINPUT is enabled and run `sudo modprobe uinput`.".into(),
        ),
        _ => (
            format!("Failed to open /dev/uinput: {err}"),
            "Check kernel logs (`dmesg | grep -i uinput`) and permissions on /dev/uinput.".into(),
        ),
    }
}

fn probe_uinput_check() -> DiagnosticCheck {
    match fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open("/dev/uinput")
    {
        Ok(_) => DiagnosticCheck {
            layer: DiagnosticLayer::Kernel,
            name: "Virtual keyboard (/dev/uinput)".into(),
            status: CheckStatus::Pass,
            latency_ms: None,
            detail: "/dev/uinput opened cleanly (`uinput` kernel module active for Fn virtual keys)".into(),
            remedy: None,
        },
        Err(e) => {
            let is_perm = matches!(e.raw_os_error(), Some(libc::EACCES | libc::EPERM));
            let (detail, remedy) = diagnose_uinput_open_error(&e);
            DiagnosticCheck {
                layer: DiagnosticLayer::Kernel,
                name: "Virtual keyboard (/dev/uinput)".into(),
                status: if is_perm { CheckStatus::Warn } else { CheckStatus::Fail },
                latency_ms: None,
                detail,
                remedy: Some(remedy),
            }
        }
    }
}

/// Verify in-process codec builders, parsers, keymaps, and destructive-command guards.
pub fn run_software_checks() -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();
    let t0 = Instant::now();

    let indices = keymap::all_indices();
    let mask_ok = match protocol::keymask(&indices) {
        Ok(m) => {
            let expected: [u8; 22] = [
                0x3e, 0x26, 0x3e, 0x1e, 0x1e, 0x1e, 0x3e, 0x1e, 0x1e, 0x3e, 0x3e, 0x3e, 0x3e,
                0x00, 0x0e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ];
            indices.len() == 61 && m == expected
        }
        Err(_) => false,
    };

    let trig_ok = protocol::build_trigger_raw(TriggerMode::Fixed, &[46], 170, 1, 1)
        .map(|pkt| pkt[0] == 0x01 && pkt[1] == 0x21 && pkt[29] == 170)
        .unwrap_or(false);

    let mut reset_body = [0u8; BODY_LEN];
    reset_body[0] = protocol::cmd::WIN_LOCK;
    reset_body[4] = 1;
    reset_body[5] = 1;
    let guard_ok = protocol::dangerous_command_reason(&reset_body).is_some();

    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    let all_ok = mask_ok && trig_ok && guard_ok;

    checks.push(DiagnosticCheck {
        layer: DiagnosticLayer::Software,
        name: "Codec, 61-key mask & safety guard".into(),
        status: if all_ok { CheckStatus::Pass } else { CheckStatus::Fail },
        latency_ms: Some(ms),
        detail: format!(
            "keymask_61={mask_ok}, trigger_frame={trig_ok}, safety_guard={guard_ok}"
        ),
        remedy: (!all_ok).then(|| "Reinstall or rebuild atrium-core; internal protocol invariant failed.".into()),
    });

    checks
}

/// Time live non-destructive HID round-trips against the connected WIN 60 HE.
pub fn run_keyboard_checks<T: HidTransport>(drv: &mut Win60Driver<T>) -> Vec<DiagnosticCheck> {
    let mut checks = Vec::new();

    let t = Instant::now();
    let hp = match drv.read_init_info() {
        Ok(info) => {
            checks.push(DiagnosticCheck {
                layer: DiagnosticLayer::Keyboard,
                name: "Heartbeat / capabilities (0x01)".into(),
                status: CheckStatus::Pass,
                latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
                detail: format!(
                    "mode={}, flags1=0x{:02X} flags2=0x{:02X}",
                    info.connection_mode(),
                    info.raw_flags1,
                    info.raw_flags2
                ),
                remedy: None,
            });
            info.is_high_precision
        }
        Err(e) => {
            checks.push(DiagnosticCheck {
                layer: DiagnosticLayer::Keyboard,
                name: "Heartbeat / capabilities (0x01)".into(),
                status: CheckStatus::Fail,
                latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
                detail: format!("No valid heartbeat response: {e}"),
                remedy: Some("Keyboard may be asleep, busy in another configurator (close web driver tabs), or unresponsive.".into()),
            });
            false
        }
    };

    let t = Instant::now();
    match drv.read_device_info() {
        Ok((fw, dt)) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Firmware & build date (0x0D)".into(),
            status: CheckStatus::Pass,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: format!(
                "{} ({})",
                fw.as_deref().unwrap_or("?"),
                dt.as_deref().unwrap_or("?")
            ),
            remedy: None,
        }),
        Err(e) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Firmware & build date (0x0D)".into(),
            status: CheckStatus::Fail,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: e.to_string(),
            remedy: Some("Try unplugging and reconnecting USB.".into()),
        }),
    }

    let t = Instant::now();
    match drv.read_max_trigger_travel(hp) {
        Ok(m) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Travel range & unit (0x21/04)".into(),
            status: CheckStatus::Pass,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: format!("{:.2}..={:.2} mm (step {:.3} mm)", m.min_mm, m.max_mm, m.unit_mm),
            remedy: None,
        }),
        Err(e) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Travel range & unit (0x21/04)".into(),
            status: CheckStatus::Fail,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: e.to_string(),
            remedy: Some("Firmware failed to answer travel scale query.".into()),
        }),
    }

    let t = Instant::now();
    match drv.read_poll_rate() {
        Ok(khz) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Polling rate readback (0x21/09)".into(),
            status: CheckStatus::Pass,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: format!("{khz} kHz ({} Hz)", u16::from(khz) * 1000),
            remedy: None,
        }),
        Err(e) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Polling rate readback (0x21/09)".into(),
            status: CheckStatus::Fail,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: e.to_string(),
            remedy: None,
        }),
    }

    let t = Instant::now();
    match drv.read_light() {
        Ok(ls) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Lighting state readback (0x07/01)".into(),
            status: CheckStatus::Pass,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: format!(
                "{} (bri={}, spd={}, #{:02X}{:02X}{:02X})",
                protocol::light_mode_name(ls.mode),
                ls.brightness,
                ls.speed,
                ls.fg[0],
                ls.fg[1],
                ls.fg[2]
            ),
            remedy: None,
        }),
        Err(e) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Lighting state readback (0x07/01)".into(),
            status: CheckStatus::Fail,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: e.to_string(),
            remedy: None,
        }),
    }

    let t = Instant::now();
    match drv.read_switch_table() {
        Ok(table) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Switch table multi-chunk (0x25/02)".into(),
            status: CheckStatus::Pass,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: format!("Reassembled 3 chunks ({} bytes), W={}", table.len(), protocol::switch_profile_name(table[46])),
            remedy: None,
        }),
        Err(e) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Switch table multi-chunk (0x25/02)".into(),
            status: CheckStatus::Fail,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: e.to_string(),
            remedy: Some("Multi-page HID transfer dropped chunks; try a direct USB port instead of a hub.".into()),
        }),
    }

    let t = Instant::now();
    match drv.read_deadband_table() {
        Ok(table) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Deadband table multi-chunk (0x26/00)".into(),
            status: CheckStatus::Pass,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: format!(
                "Reassembled 5 chunks ({} bytes), W={:.2}/{:.2} mm",
                table.len(),
                f64::from(table[46 * 2]) * 0.01,
                f64::from(table[46 * 2 + 1]) * 0.01
            ),
            remedy: None,
        }),
        Err(e) => checks.push(DiagnosticCheck {
            layer: DiagnosticLayer::Keyboard,
            name: "Deadband table multi-chunk (0x26/00)".into(),
            status: CheckStatus::Fail,
            latency_ms: Some(t.elapsed().as_secs_f64() * 1000.0),
            detail: e.to_string(),
            remedy: Some("Multi-page HID transfer dropped chunks; check USB connection stability.".into()),
        }),
    }

    checks
}

/// Attribute the root cause across Kernel, Daemon, Keyboard, and Software layers.
pub fn compute_attribution(checks: &[DiagnosticCheck]) -> FaultAttribution {
    for layer in [
        DiagnosticLayer::Kernel,
        DiagnosticLayer::Daemon,
        DiagnosticLayer::Keyboard,
        DiagnosticLayer::Software,
    ] {
        if let Some(fail) = checks
            .iter()
            .find(|c| c.layer == layer && c.status == CheckStatus::Fail)
        {
            return FaultAttribution {
                primary_fault: Some(layer),
                summary: format!("Fault isolated to {} — {} ({})", layer.label(), fail.name, fail.detail),
            };
        }
    }

    let warns = checks
        .iter()
        .filter(|c| c.status == CheckStatus::Warn)
        .count();
    if warns > 0 {
        FaultAttribution {
            primary_fault: None,
            summary: format!("All 4 layers operational ({warns} advisory warning(s))."),
        }
    } else {
        FaultAttribution {
            primary_fault: None,
            summary: "All 4 layers (Kernel, Daemon, Keyboard, Software) passed with 0 faults.".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_checks_pass() {
        let checks = run_software_checks();
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, CheckStatus::Pass);
    }

    #[test]
    fn attribution_prioritizes_kernel_then_daemon_then_keyboard() {
        let checks = vec![
            DiagnosticCheck {
                layer: DiagnosticLayer::Kernel,
                name: "usb".into(),
                status: CheckStatus::Fail,
                latency_ms: None,
                detail: "missing".into(),
                remedy: None,
            },
            DiagnosticCheck {
                layer: DiagnosticLayer::Daemon,
                name: "sock".into(),
                status: CheckStatus::Fail,
                latency_ms: None,
                detail: "down".into(),
                remedy: None,
            },
        ];
        let attr = compute_attribution(&checks);
        assert_eq!(attr.primary_fault, Some(DiagnosticLayer::Kernel));
    }
}

