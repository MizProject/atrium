// Copyright (C) 2026 MizProject Philippines
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

#[cfg(target_os = "linux")]
pub mod alsa;
pub mod audio_fx;
pub mod diagnostics;
pub mod hid_probe;
pub mod hid_sysfs;
pub mod ipc;
pub mod keyboards;
pub mod legal;
pub mod transport;

// Backward-compatible module re-exports for WIN 60 HE (primary indexed keyboard).
pub use keyboards::win60he::{driver, keymap, protocol};
