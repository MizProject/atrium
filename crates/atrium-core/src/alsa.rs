//! Minimal Linux ALSA PCM capture wrapper (`libasound.so.2`) for live audio analysis.

use std::ffi::{CStr, CString, c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::ptr;

#[repr(C)]
struct snd_pcm_t {
    _private: [u8; 0],
}

const SND_PCM_STREAM_CAPTURE: c_int = 1;
const SND_PCM_ACCESS_RW_INTERLEAVED: c_int = 3;
const SND_PCM_FORMAT_S16_LE: c_int = 2;

#[link(name = "asound")]
unsafe extern "C" {
    fn snd_pcm_open(
        pcm: *mut *mut snd_pcm_t,
        name: *const c_char,
        stream: c_int,
        mode: c_int,
    ) -> c_int;
    fn snd_pcm_close(pcm: *mut snd_pcm_t) -> c_int;
    fn snd_pcm_set_params(
        pcm: *mut snd_pcm_t,
        format: c_int,
        access: c_int,
        channels: c_uint,
        rate: c_uint,
        soft_resample: c_int,
        latency: c_uint,
    ) -> c_int;
    fn snd_pcm_readi(pcm: *mut snd_pcm_t, buffer: *mut c_void, size: c_ulong) -> c_long;
    fn snd_pcm_recover(pcm: *mut snd_pcm_t, err: c_int, silent: c_int) -> c_int;
    fn snd_strerror(errnum: c_int) -> *const c_char;
    fn snd_device_name_hint(card: c_int, iface: *const c_char, hints: *mut *mut *mut c_void)
    -> c_int;
    fn snd_device_name_free_hint(hints: *mut *mut c_void) -> c_int;
    fn snd_device_name_get_hint(hint: *const c_void, id: *const c_char) -> *mut c_char;
}

/// Summary of an ALSA PCM device suitable for capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlsaCaptureDevice {
    pub name: String,
    pub description: String,
    pub ioid: String,
}

/// Strategy used to open an ALSA capture stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlsaSourceMode {
    /// Capture desktop playback output via PipeWire's ALSA monitor or PulseAudio sink monitor.
    Monitor,
    /// Capture the system default ALSA recording source (microphone/line-in).
    DefaultInput,
    /// Capture a caller-specified ALSA PCM identifier (`hw:0,0`, `plughw:0,0`, `pulse`, etc.).
    Device(String),
}

/// Active ALSA capture stream producing normalized mono `f32` audio samples.
pub struct AlsaCapture {
    handle: *mut snd_pcm_t,
    pcm_name: String,
    sample_rate: u32,
    channels: u16,
    scratch_i16: Vec<i16>,
}

// SAFETY: `snd_pcm_t` ownership is transferred exclusively to the consumer thread without sharing.
unsafe impl Send for AlsaCapture {}

impl Drop for AlsaCapture {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: `self.handle` was returned by `snd_pcm_open` and has not yet been closed.
            unsafe {
                snd_pcm_close(self.handle);
            }
            self.handle = ptr::null_mut();
        }
    }
}

fn alsa_err_msg(code: c_int) -> String {
    // SAFETY: `snd_strerror` returns a static NUL-terminated error description for any error code.
    let ptr = unsafe { snd_strerror(code) };
    if ptr.is_null() {
        format!("ALSA error {code}")
    } else {
        // SAFETY: `ptr` is valid and NUL-terminated per libasound contract.
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    }
}

fn read_hint_string(hint: *const c_void, key: &CStr) -> Option<String> {
    // SAFETY: `hint` is a valid element from `snd_device_name_hint` and `key` is a static C string.
    let raw = unsafe { snd_device_name_get_hint(hint, key.as_ptr()) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: `raw` is a malloc-allocated NUL-terminated string from libasound; freed below.
    let s = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `snd_device_name_get_hint` allocations must be released with `libc::free`.
    unsafe {
        libc::free(raw.cast::<c_void>());
    }
    Some(s)
}

/// Enumerate ALSA PCM hints that support capture (`IOID` absent or `"Input"`).
pub fn list_capture_devices() -> Result<Vec<AlsaCaptureDevice>, String> {
    let mut hints: *mut *mut c_void = ptr::null_mut();
    // SAFETY: `"pcm"` is NUL-terminated and `hints` receives an allocated NULL-terminated array.
    let rc = unsafe { snd_device_name_hint(-1, c"pcm".as_ptr(), &mut hints) };
    if rc < 0 {
        return Err(format!("snd_device_name_hint failed: {}", alsa_err_msg(rc)));
    }
    if hints.is_null() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let mut idx = 0usize;
    loop {
        // SAFETY: `hints` is NULL-terminated and stays valid until `snd_device_name_free_hint`.
        let hint = unsafe { *hints.add(idx) };
        if hint.is_null() {
            break;
        }
        idx += 1;

        let ioid = read_hint_string(hint, c"IOID").unwrap_or_else(|| "Bidirectional".into());
        if !ioid.eq_ignore_ascii_case("Input") && ioid != "Bidirectional" {
            continue;
        }
        let Some(name) = read_hint_string(hint, c"NAME") else {
            continue;
        };
        if name == "null" {
            continue;
        }
        let description = read_hint_string(hint, c"DESC")
            .unwrap_or_default()
            .lines()
            .next()
            .unwrap_or("")
            .to_string();
        out.push(AlsaCaptureDevice {
            name,
            description,
            ioid,
        });
    }

    // SAFETY: `hints` was allocated by `snd_device_name_hint` above.
    unsafe {
        snd_device_name_free_hint(hints);
    }
    Ok(out)
}

/// Resolve the active PipeWire/PulseAudio default output monitor name (`<sink>.monitor`), if available.
pub fn default_monitor_source_name() -> Option<String> {
    let out = std::process::Command::new("pactl")
        .arg("get-default-sink")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sink = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!sink.is_empty()).then(|| format!("{sink}.monitor"))
}

/// Configure `PIPEWIRE_PROPS` for desktop sink-monitor capture before spawning worker threads.
///
/// # Safety
/// Must be called on the main thread before any background threads exist that may read/write the
/// process environment (Rust 2024 `std::env::set_var` precondition).
pub unsafe fn prepare_monitor_env() {
    if std::env::var_os("PIPEWIRE_PROPS").is_none() {
        // SAFETY: Precondition enforced by caller before spawning worker threads.
        unsafe {
            std::env::set_var("PIPEWIRE_PROPS", "{ stream.capture.sink=true }");
        }
    }
    if std::env::var_os("PULSE_SOURCE").is_none()
        && let Some(mon) = default_monitor_source_name()
    {
        // SAFETY: Precondition enforced by caller before spawning worker threads.
        unsafe {
            std::env::set_var("PULSE_SOURCE", mon);
        }
    }
}

impl AlsaCapture {
    /// Open an ALSA capture stream matching `mode` with `sample_rate`, `channels`, and `latency_us`.
    pub fn open(
        mode: &AlsaSourceMode,
        sample_rate: u32,
        channels: u16,
        latency_us: u32,
    ) -> Result<Self, String> {
        let candidates: Vec<String> = match mode {
            AlsaSourceMode::Monitor => {
                let mut list = Vec::with_capacity(4);
                // Prefer explicit ALSA `pulse:DEVICE=<sink>.monitor` so multi-threaded callers
                // (like atrium-gui) never need to mutate process environment variables.
                if let Some(mon) = default_monitor_source_name() {
                    list.push(format!("pulse:DEVICE={mon}"));
                }
                list.extend(["pipewire".into(), "pulse".into(), "default".into()]);
                list
            }
            AlsaSourceMode::DefaultInput => vec!["default".into(), "pipewire".into()],
            AlsaSourceMode::Device(name) => vec![name.clone()],
        };

        let mut last_err = String::new();
        for name in &candidates {
            match Self::open_pcm(name, sample_rate, channels, latency_us) {
                Ok(cap) => return Ok(cap),
                Err(err) => {
                    last_err = format!("{name}: {err}");
                }
            }
        }

        Err(format!("failed to open ALSA capture stream ({last_err})"))
    }

    fn open_pcm(
        pcm_name: &str,
        sample_rate: u32,
        channels: u16,
        latency_us: u32,
    ) -> Result<Self, String> {
        let c_name = CString::new(pcm_name)
            .map_err(|_| format!("invalid ALSA device name '{pcm_name}' (contains NUL)"))?;
        let mut handle: *mut snd_pcm_t = ptr::null_mut();

        // SAFETY: `handle` and `c_name` are valid; opens blocking capture PCM stream.
        let rc = unsafe { snd_pcm_open(&mut handle, c_name.as_ptr(), SND_PCM_STREAM_CAPTURE, 0) };
        if rc < 0 || handle.is_null() {
            return Err(alsa_err_msg(rc));
        }

        // SAFETY: `handle` was just opened by `snd_pcm_open`.
        let params_rc = unsafe {
            snd_pcm_set_params(
                handle,
                SND_PCM_FORMAT_S16_LE,
                SND_PCM_ACCESS_RW_INTERLEAVED,
                c_uint::from(channels.max(1)),
                c_uint::from(sample_rate.max(8000)),
                1, // allow libasound software resampling when hardware rate differs
                c_uint::from(latency_us.max(5000)),
            )
        };
        if params_rc < 0 {
            let err = alsa_err_msg(params_rc);
            // SAFETY: Close invalid handle on setup failure.
            unsafe {
                snd_pcm_close(handle);
            }
            return Err(format!("snd_pcm_set_params failed: {err}"));
        }

        Ok(Self {
            handle,
            pcm_name: pcm_name.to_string(),
            sample_rate,
            channels: channels.max(1),
            scratch_i16: Vec::new(),
        })
    }

    pub fn pcm_name(&self) -> &str {
        &self.pcm_name
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    fn read_interleaved_frames(&mut self, max_frames: usize) -> Result<usize, String> {
        let ch = usize::from(self.channels);
        let needed_samples = max_frames * ch;
        if self.scratch_i16.len() < needed_samples {
            self.scratch_i16.resize(needed_samples, 0);
        }

        loop {
            // SAFETY: `self.handle` is open and `scratch_i16` holds `max_frames * ch` i16 slots.
            let frames = unsafe {
                snd_pcm_readi(
                    self.handle,
                    self.scratch_i16.as_mut_ptr().cast::<c_void>(),
                    max_frames as c_ulong,
                )
            };
            if frames >= 0 {
                return Ok((frames as usize).min(max_frames));
            }

            let err = frames as c_int;
            // SAFETY: `snd_pcm_recover` attempts automatic recovery from EPIPE/ESTRPIPE/EINTR.
            let rec = unsafe { snd_pcm_recover(self.handle, err, 1) };
            if rec < 0 {
                return Err(format!("ALSA capture read failed: {}", alsa_err_msg(rec)));
            }
        }
    }

    /// Read up to `out.len()` mono signed 16-bit audio frames, recovering xruns cleanly.
    pub fn read_mono_i16(&mut self, out: &mut [i16]) -> Result<usize, String> {
        if out.is_empty() {
            return Ok(0);
        }
        let ch = usize::from(self.channels);
        let count = self.read_interleaved_frames(out.len())?;
        for (i, dst) in out[..count].iter_mut().enumerate() {
            let base = i * ch;
            let mut acc = 0i32;
            for &s in &self.scratch_i16[base..base + ch] {
                acc += i32::from(s);
            }
            *dst = (acc / ch as i32).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
        }
        Ok(count)
    }

    /// Read up to `out.len()` mono normalized audio frames (`-1.0..=1.0`), recovering xruns cleanly.
    pub fn read_mono_f32(&mut self, out: &mut [f32]) -> Result<usize, String> {
        if out.is_empty() {
            return Ok(0);
        }
        let ch = usize::from(self.channels);
        let count = self.read_interleaved_frames(out.len())?;
        let inv_scale = 1.0 / (32768.0 * ch as f32);
        for (i, dst) in out[..count].iter_mut().enumerate() {
            let base = i * ch;
            let mut acc = 0.0f32;
            for s in &self.scratch_i16[base..base + ch] {
                acc += f32::from(*s);
            }
            *dst = (acc * inv_scale).clamp(-1.0, 1.0);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_enumeration_succeeds_without_crashing() {
        let res = list_capture_devices();
        assert!(res.is_ok(), "ALSA hint query failed: {res:?}");
    }
}
