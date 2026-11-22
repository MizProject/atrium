#[cfg(target_os = "linux")]
pub mod alsa;
pub mod audio_fx;
pub mod diagnostics;
pub mod hid_probe;
pub mod hid_sysfs;
pub mod ipc;
pub mod keyboards;
pub mod transport;

// Backward-compatible module re-exports for WIN 60 HE (primary indexed keyboard).
pub use keyboards::win60he::{driver, keymap, protocol};
