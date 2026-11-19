use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read};
use std::mem;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::uinput::VirtualKeyboard;

const VENDOR_HEX: &str = "2e3c";
const PRODUCT_HEX: &str = "c365";
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const SYN_REPORT: u16 = 0;
const KEY_A_CODE: u16 = 30;

/// `_IOW('E', 0x90, int)` from `<linux/input.h>`.
pub(crate) const EVIOCGRAB: libc::Ioctl =
    ((1u32 << 30) | (4u32 << 16) | ((b'E' as u32) << 8) | 0x90) as libc::Ioctl;

#[repr(C)]
#[derive(Clone, Copy)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

/// Shared armed-trigger table updated by `ShortcutEngine` and queried per grabbed keyboard node.
#[derive(Debug, Default)]
pub struct SuppressionState {
    armed_codes: HashSet<u16>,
}

impl SuppressionState {
    /// Update the set of armed trigger key codes (typically `Fn`-chorded keys).
    pub fn set_armed(&mut self, armed: HashSet<u16>) {
        self.armed_codes = armed;
    }
}

/// Per-evdev-node key transition tracker ensuring independent release symmetry
/// across WIN 60 HE's companion USB keyboard interfaces (`event4` and `event8`).
#[derive(Debug, Default)]
pub struct NodeKeyFilter {
    suppressed_down: HashSet<u16>,
    forwarded_down: HashSet<u16>,
}

impl NodeKeyFilter {
    /// Decide how to handle a physical `EV_KEY` event from one grabbed keyboard node.
    ///
    /// Returns `Some(true)` to forward a press, `Some(false)` to forward a release,
    /// or `None` to swallow the event (either armed trigger key or kernel auto-repeat).
    pub fn decide_key_event(
        &mut self,
        armed_codes: &HashSet<u16>,
        code: u16,
        value: i32,
    ) -> Option<bool> {
        match value {
            // VirtualKeyboard enables EV_REP so uinput generates smooth kernel repeats itself.
            2 => None,
            1 => {
                if armed_codes.contains(&code) && !self.forwarded_down.contains(&code) {
                    self.suppressed_down.insert(code);
                    None
                } else {
                    self.forwarded_down.insert(code);
                    Some(true)
                }
            }
            0 => {
                if self.suppressed_down.remove(&code) {
                    None
                } else {
                    self.forwarded_down.remove(&code);
                    Some(false)
                }
            }
            _ => None,
        }
    }

    /// Drain all currently forwarded pressed keys so none remain stuck down on ungrab.
    pub fn take_forwarded_down(&mut self) -> Vec<u16> {
        self.suppressed_down.clear();
        self.forwarded_down.drain().collect()
    }
}

/// Manages exclusive `EVIOCGRAB` passthrough across physical WIN 60 HE `/dev/input/event*`
/// keyboard nodes whenever daemon hotkey suppression is active, ensuring matched shortcuts
/// never leak raw characters while forwarding all other keystrokes sub-ms via `uinput`.
pub struct EvdevInterceptor {
    desired_grab: Arc<AtomicBool>,
    active_grabs: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    manager: Option<JoinHandle<()>>,
}

impl EvdevInterceptor {
    pub fn spawn(
        uinput: Arc<Mutex<Option<VirtualKeyboard>>>,
        suppression: Arc<Mutex<SuppressionState>>,
    ) -> Self {
        let desired_grab = Arc::new(AtomicBool::new(false));
        let active_grabs = Arc::new(AtomicUsize::new(0));
        let shutdown = Arc::new(AtomicBool::new(false));

        let m_desired = Arc::clone(&desired_grab);
        let m_active = Arc::clone(&active_grabs);
        let m_shutdown = Arc::clone(&shutdown);

        let handle = thread::Builder::new()
            .name("reaulad-evdev-mgr".into())
            .spawn(move || {
                manager_loop(&m_desired, &m_active, &m_shutdown, &uinput, &suppression);
            })
            .expect("spawn reaulad-evdev-mgr thread");

        Self {
            desired_grab,
            active_grabs,
            shutdown,
            manager: Some(handle),
        }
    }

    /// Return a no-op interceptor (used in unit tests without spawning worker threads).
    #[cfg(test)]
    pub fn idle() -> Self {
        Self {
            desired_grab: Arc::new(AtomicBool::new(false)),
            active_grabs: Arc::new(AtomicUsize::new(0)),
            shutdown: Arc::new(AtomicBool::new(true)),
            manager: None,
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.desired_grab.store(enabled, Ordering::Release);
    }

    pub fn is_grab_active(&self) -> bool {
        self.active_grabs.load(Ordering::Acquire) > 0
    }
}

impl Drop for EvdevInterceptor {
    fn drop(&mut self) {
        self.desired_grab.store(false, Ordering::Release);
        self.shutdown.store(true, Ordering::Release);
        if let Some(handle) = self.manager.take() {
            let _ = handle.join();
        }
    }
}

fn manager_loop(
    desired_grab: &Arc<AtomicBool>,
    active_grabs: &Arc<AtomicUsize>,
    shutdown: &Arc<AtomicBool>,
    uinput: &Arc<Mutex<Option<VirtualKeyboard>>>,
    suppression: &Arc<Mutex<SuppressionState>>,
) {
    let mut workers: HashMap<PathBuf, JoinHandle<()>> = HashMap::new();

    while !shutdown.load(Ordering::Acquire) {
        workers.retain(|_, h| !h.is_finished());

        if desired_grab.load(Ordering::Acquire) && uinput_available(uinput) {
            for path in find_win60_keyboard_event_nodes() {
                if workers.contains_key(&path) {
                    continue;
                }
                let p_clone = path.clone();
                let w_desired = Arc::clone(desired_grab);
                let w_active = Arc::clone(active_grabs);
                let w_shutdown = Arc::clone(shutdown);
                let w_uinput = Arc::clone(uinput);
                let w_sup = Arc::clone(suppression);
                if let Ok(h) = thread::Builder::new()
                    .name(format!(
                        "reaulad-evdev-{}",
                        path.file_name().and_then(|s| s.to_str()).unwrap_or("kb")
                    ))
                    .spawn(move || {
                        node_worker_loop(
                            &p_clone,
                            &w_desired,
                            &w_active,
                            &w_shutdown,
                            &w_uinput,
                            &w_sup,
                        );
                    })
                {
                    workers.insert(path, h);
                }
            }
        } else {
            for (_, h) in workers.drain() {
                let _ = h.join();
            }
        }

        thread::sleep(Duration::from_millis(60));
    }

    for (_, h) in workers.drain() {
        let _ = h.join();
    }
}

fn node_worker_loop(
    dev_path: &Path,
    desired_grab: &AtomicBool,
    active_grabs: &Arc<AtomicUsize>,
    shutdown: &AtomicBool,
    uinput: &Mutex<Option<VirtualKeyboard>>,
    suppression: &Mutex<SuppressionState>,
) {
    while desired_grab.load(Ordering::Acquire)
        && !shutdown.load(Ordering::Acquire)
        && uinput_available(uinput)
        && dev_path.exists()
    {
        if let Err(err) = run_grab_session(
            dev_path,
            desired_grab,
            active_grabs,
            shutdown,
            uinput,
            suppression,
        ) {
            eprintln!(
                "reaulad evdev grab ended on {}: {err}",
                dev_path.display()
            );
            if err.raw_os_error() == Some(libc::EACCES)
                || err.raw_os_error() == Some(libc::ENODEV)
            {
                thread::sleep(Duration::from_millis(400));
                break;
            }
            thread::sleep(Duration::from_millis(150));
        }
    }
}

fn uinput_available(uinput: &Mutex<Option<VirtualKeyboard>>) -> bool {
    uinput.lock().map(|g| g.is_some()).unwrap_or(false)
}

fn run_grab_session(
    dev_path: &Path,
    desired_grab: &AtomicBool,
    active_grabs: &Arc<AtomicUsize>,
    shutdown: &AtomicBool,
    uinput: &Mutex<Option<VirtualKeyboard>>,
    suppression: &Mutex<SuppressionState>,
) -> io::Result<()> {
    let mut file = open_nonblocking(dev_path)?;
    let _grab = EvdevGrabGuard::acquire(file.as_raw_fd(), active_grabs)?;

    let mut filter = NodeKeyFilter::default();
    let ev_size = mem::size_of::<InputEvent>();
    let mut buf = vec![0u8; ev_size * 64];

    let session_res: io::Result<()> = (|| {
        while desired_grab.load(Ordering::Acquire)
            && !shutdown.load(Ordering::Acquire)
            && uinput_available(uinput)
        {
            if !poll_readable(file.as_raw_fd(), 40)? {
                continue;
            }

            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let count = n / ev_size;
                    let mut need_sync = false;
                    for i in 0..count {
                        let ev = unsafe {
                            std::ptr::read_unaligned(
                                buf.as_ptr().add(i * ev_size) as *const InputEvent
                            )
                        };
                        match ev.type_ {
                            EV_KEY => {
                                let decision = suppression
                                    .lock()
                                    .map(|sup| {
                                        filter.decide_key_event(&sup.armed_codes, ev.code, ev.value)
                                    })
                                    .unwrap_or(None);
                                if let Some(pressed) = decision {
                                    forward_key_only(uinput, ev.code, pressed)?;
                                    need_sync = true;
                                }
                            }
                            EV_SYN if ev.code == SYN_REPORT && need_sync => {
                                sync_uinput(uinput)?;
                                need_sync = false;
                            }
                            _ => {}
                        }
                    }
                    if need_sync {
                        sync_uinput(uinput)?;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    })();

    release_stuck_forwarded(uinput, &mut filter);
    session_res
}

struct EvdevGrabGuard {
    fd: libc::c_int,
    active_grabs: Arc<AtomicUsize>,
}

impl EvdevGrabGuard {
    fn acquire(fd: libc::c_int, active_grabs: &Arc<AtomicUsize>) -> io::Result<Self> {
        let rc = unsafe { libc::ioctl(fd, EVIOCGRAB, 1 as libc::c_int) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        active_grabs.fetch_add(1, Ordering::AcqRel);
        Ok(Self {
            fd,
            active_grabs: Arc::clone(active_grabs),
        })
    }
}

impl Drop for EvdevGrabGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::ioctl(self.fd, EVIOCGRAB, 0 as libc::c_int);
        }
        self.active_grabs.fetch_sub(1, Ordering::AcqRel);
    }
}

fn open_nonblocking(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
}

fn poll_readable(fd: libc::c_int, timeout_ms: libc::c_int) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let rc = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(err);
    }
    if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
        return Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "evdev input device disconnected",
        ));
    }
    Ok(rc > 0 && (pfd.revents & libc::POLLIN) != 0)
}

fn forward_key_only(
    uinput: &Mutex<Option<VirtualKeyboard>>,
    code: u16,
    pressed: bool,
) -> io::Result<()> {
    let mut guard = uinput
        .lock()
        .map_err(|_| io::Error::other("uinput lock poisoned"))?;
    let Some(vk) = guard.as_mut() else {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "virtual keyboard unavailable",
        ));
    };
    vk.queue_key_code(code, pressed)
}

fn sync_uinput(uinput: &Mutex<Option<VirtualKeyboard>>) -> io::Result<()> {
    let mut guard = uinput
        .lock()
        .map_err(|_| io::Error::other("uinput lock poisoned"))?;
    if let Some(vk) = guard.as_mut() {
        vk.sync()?;
    }
    Ok(())
}

fn release_stuck_forwarded(
    uinput: &Mutex<Option<VirtualKeyboard>>,
    filter: &mut NodeKeyFilter,
) {
    let stuck = filter.take_forwarded_down();
    if stuck.is_empty() {
        return;
    }
    if let Ok(mut guard) = uinput.lock()
        && let Some(vk) = guard.as_mut()
    {
        for code in stuck {
            let _ = vk.queue_key_code(code, false);
        }
        let _ = vk.sync();
    }
}

/// Filter out virtual input devices (`uinput`) so `reaulad` never grabs its own output device.
pub(crate) fn is_physical_keyboard_sysfs(
    canonical_dev_path: &Path,
    name: &str,
    phys: &str,
    key_cap: &str,
) -> bool {
    let canonical_str = canonical_dev_path.to_string_lossy();
    if canonical_str.starts_with("/sys/devices/virtual") {
        return false;
    }
    if name.trim().starts_with("Reaula") || phys.trim().is_empty() {
        return false;
    }
    sysfs_cap_has_bit(key_cap, KEY_A_CODE)
}

/// Locate physical WIN 60 HE USB keyboard `/dev/input/event*` nodes (`event4`, `event8`, ...),
/// excluding virtual `uinput` devices, Mouse, Consumer Control, and System Control interfaces.
pub fn find_win60_keyboard_event_nodes() -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir("/sys/class/input") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.starts_with("event") {
            continue;
        }
        let dev_dir = entry.path().join("device");
        let Ok(vendor) = fs::read_to_string(dev_dir.join("id/vendor")) else {
            continue;
        };
        let Ok(product) = fs::read_to_string(dev_dir.join("id/product")) else {
            continue;
        };
        if !vendor.trim().eq_ignore_ascii_case(VENDOR_HEX)
            || !product.trim().eq_ignore_ascii_case(PRODUCT_HEX)
        {
            continue;
        }
        let Ok(canonical) = fs::canonicalize(&dev_dir) else {
            continue;
        };
        let dev_name = fs::read_to_string(dev_dir.join("name")).unwrap_or_default();
        let phys = fs::read_to_string(dev_dir.join("phys")).unwrap_or_default();
        let Ok(key_cap) = fs::read_to_string(dev_dir.join("capabilities/key")) else {
            continue;
        };
        if is_physical_keyboard_sysfs(&canonical, &dev_name, &phys, &key_cap) {
            out.push(PathBuf::from("/dev/input").join(name));
        }
    }
    out.sort();
    out
}

/// Check whether a Linux sysfs space-separated hex capability mask includes `bit`.
pub(crate) fn sysfs_cap_has_bit(cap_line: &str, bit: u16) -> bool {
    let word_bits = usize::BITS as usize;
    let words: Vec<&str> = cap_line.split_whitespace().collect();
    // Sysfs prints most-significant word first; word index from the right is bit / word_bits.
    let word_idx_from_right = usize::from(bit) / word_bits;
    let bit_in_word = usize::from(bit) % word_bits;
    let Some(&hex_word) = words.iter().rev().nth(word_idx_from_right) else {
        return false;
    };
    let Ok(val) = usize::from_str_radix(hex_word, 16) else {
        return false;
    };
    (val & (1usize << bit_in_word)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eviocgrab_ioctl_matches_linux_abi() {
        assert_eq!(EVIOCGRAB as u32, 0x4004_4590);
    }

    #[test]
    fn suppression_swallows_armed_key_press_and_late_release() {
        let mut st = SuppressionState::default();
        let mut filter = NodeKeyFilter::default();
        const KEY_T: u16 = 20;
        const KEY_A: u16 = 30;

        // Normal key passes press and release; repeat is dropped.
        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_A, 1), Some(true));
        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_A, 2), None);
        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_A, 0), Some(false));

        // Arm KEY_T when Fn is held -> press swallowed.
        st.set_armed(HashSet::from([KEY_T]));
        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_T, 1), None);

        // Even if user releases Fn BEFORE releasing T (disarming armed_codes),
        // T's matching release is still swallowed so no stray KEY_T up occurs.
        st.set_armed(HashSet::new());
        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_T, 0), None);

        // Next press of T without Fn passes normally.
        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_T, 1), Some(true));
        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_T, 0), Some(false));
    }

    #[test]
    fn normal_pressed_key_still_releases_if_fn_engaged_mid_hold() {
        let mut st = SuppressionState::default();
        let mut filter = NodeKeyFilter::default();
        const KEY_T: u16 = 20;

        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_T, 1), Some(true));
        st.set_armed(HashSet::from([KEY_T]));
        assert_eq!(filter.decide_key_event(&st.armed_codes, KEY_T, 0), Some(false));
    }

    #[test]
    fn excludes_virtual_uinput_node_while_matching_physical_keyboards() {
        let kb_cap = "1000000000007 ff800000000007ff febeffdfffefffff fffffffffffffffe";
        assert!(is_physical_keyboard_sysfs(
            Path::new("/sys/devices/pci0000:00/0000:00:14.0/usb1/1-7/1-7:1.0/0003:2E3C:C365.0002/input/input4"),
            "WIN 60 HE",
            "usb-0000:00:14.0-7/input0",
            kb_cap,
        ));
        assert!(is_physical_keyboard_sysfs(
            Path::new("/sys/devices/pci0000:00/0000:00:14.0/usb1/1-7/1-7:1.1/0003:2E3C:C365.0003/input/input8"),
            "WIN 60 HE Keyboard",
            "usb-0000:00:14.0-7/input1",
            kb_cap,
        ));
        assert!(!is_physical_keyboard_sysfs(
            Path::new("/sys/devices/virtual/input/input20"),
            "Reaula WIN 60 HE Virtual Fn",
            "",
            kb_cap,
        ));
    }

    #[test]
    fn sysfs_key_mask_parses_least_significant_word() {
        assert!(sysfs_cap_has_bit("10000 0 0 40000000", KEY_A_CODE));
        assert!(!sysfs_cap_has_bit("10000 0 0 1", KEY_A_CODE));
    }
}
