//! Linux `/dev/uinput` virtual keyboard backend for `atriumd` custom Fn / gesture keybinds.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::mem::size_of;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

use atrium_core::ipc::VirtualKey;

const UINPUT_MAX_NAME_SIZE: usize = 80;
const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REP: i32 = 0x14;
const SYN_REPORT: u16 = 0;
const BUS_USB: u16 = 0x03;

const fn ioctl_io(ty: u8, nr: u8) -> libc::Ioctl {
    (((ty as u32) << 8) | (nr as u32)) as libc::Ioctl
}

const fn ioctl_iow(ty: u8, nr: u8, size: usize) -> libc::Ioctl {
    ((1u32 << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | (nr as u32)) as libc::Ioctl
}

const UI_DEV_CREATE: libc::Ioctl = ioctl_io(b'U', 1);
const UI_DEV_DESTROY: libc::Ioctl = ioctl_io(b'U', 2);
const UI_DEV_SETUP: libc::Ioctl = ioctl_iow(b'U', 3, size_of::<UinputSetup>());
const UI_SET_EVBIT: libc::Ioctl = ioctl_iow(b'U', 100, size_of::<libc::c_int>());
const UI_SET_KEYBIT: libc::Ioctl = ioctl_iow(b'U', 101, size_of::<libc::c_int>());

#[repr(C)]
struct UinputSetup {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; UINPUT_MAX_NAME_SIZE],
    ff_effects_max: u32,
}

#[repr(C)]
struct InputEvent {
    time: libc::timeval,
    type_: u16,
    code: u16,
    value: i32,
}

pub struct VirtualKeyboard {
    file: File,
}

impl VirtualKeyboard {
    pub fn open() -> io::Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uinput")?;

        let fd = file.as_raw_fd();
        ioctl_int(fd, UI_SET_EVBIT, i32::from(EV_KEY))?;
        ioctl_int(fd, UI_SET_EVBIT, EV_REP)?;
        for code in 1..=248i32 {
            ioctl_int(fd, UI_SET_KEYBIT, code)?;
        }
        for &vk in &VirtualKey::ALL {
            ioctl_int(fd, UI_SET_KEYBIT, i32::from(vk.linux_code()))?;
        }

        let mut setup = UinputSetup {
            bustype: BUS_USB,
            vendor: 0x2E3C,
            product: 0xC365,
            version: 1,
            name: [0u8; UINPUT_MAX_NAME_SIZE],
            ff_effects_max: 0,
        };
        let label = b"Atrium Virtual Fn Keyboard";
        setup.name[..label.len()].copy_from_slice(label);

        // SAFETY: `setup` is a valid initialized `UinputSetup` struct matching Linux kernel ABI.
        let rc = unsafe { libc::ioctl(fd, UI_DEV_SETUP, &setup as *const UinputSetup) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` is a valid configured uinput device descriptor.
        let rc = unsafe { libc::ioctl(fd, UI_DEV_CREATE) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self { file })
    }

    pub fn set_key(&mut self, key: VirtualKey, pressed: bool) -> io::Result<()> {
        self.send_key_code(key.linux_code(), pressed)
    }

    /// Emit a Linux `EV_KEY` transition followed by `SYN_REPORT` on the virtual keyboard.
    pub fn send_key_code(&mut self, code: u16, pressed: bool) -> io::Result<()> {
        let value = if pressed { 1 } else { 0 };
        self.write_event(EV_KEY, code, value)?;
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    /// Forward an `EV_KEY` transition without flushing `SYN_REPORT` yet.
    pub fn queue_key_code(&mut self, code: u16, pressed: bool) -> io::Result<()> {
        self.write_event(EV_KEY, code, i32::from(pressed))
    }

    /// Emit a `SYN_REPORT` synchronization frame on the virtual keyboard.
    pub fn sync(&mut self) -> io::Result<()> {
        self.write_event(EV_SYN, SYN_REPORT, 0)
    }

    pub fn tap_key(&mut self, key: VirtualKey) -> io::Result<()> {
        self.set_key(key, true)?;
        self.set_key(key, false)
    }

    fn write_event(&mut self, type_: u16, code: u16, value: i32) -> io::Result<()> {
        let ev = InputEvent {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        // SAFETY: `InputEvent` is `#[repr(C)]` plain data with no padding bytes containing uninit data after initialization.
        let bytes = unsafe {
            std::slice::from_raw_parts((&ev as *const InputEvent).cast::<u8>(), size_of::<InputEvent>())
        };
        self.file.write_all(bytes)
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        // SAFETY: Destroying our uinput virtual device before closing its descriptor.
        unsafe {
            let _ = libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

fn ioctl_int(fd: libc::c_int, req: libc::Ioctl, val: libc::c_int) -> io::Result<()> {
    // SAFETY: `fd` is an open `/dev/uinput` descriptor and `val` is a scalar `c_int`.
    let rc = unsafe { libc::ioctl(fd, req, val) };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_uinput_abi_sizes_match_kernel() {
        assert_eq!(size_of::<UinputSetup>(), 92);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(size_of::<InputEvent>(), 24);
        assert_eq!(UI_DEV_SETUP as u32, 0x405C_5503);
        assert_eq!(UI_SET_EVBIT as u32, 0x4004_5564);
        assert_eq!(UI_SET_KEYBIT as u32, 0x4004_5565);
    }
}

