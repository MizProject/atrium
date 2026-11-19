# Contributing to reaula

Thank you for helping improve `reaula`! This workspace implements userspace drivers, a privilege-isolated HID daemon, CLI, TUI, and GPUI/`wgpu` desktop control panel for Aula Hall-effect and RGB keyboards on Linux.

## Workspace Layout

| Crate | Binary / Lib | Responsibility |
| --- | --- | --- |
| `crates/reaula-core` | library | Indexed keyboard catalog (`src/keyboards/`), wire protocol codecs, `hidapi` transport & sysfs enumeration, ALSA (`libasound`) audio capture, 14-band FFT/beat synthesizer & visualizer (`audio_fx`), 4-layer diagnostics, and `SO_PEERCRED` Unix socket IPC. |
| `crates/reaula-daemon` | `reaulad` | Single-owner HID hardware daemon (`/run/reaula/reaulad.sock`), indexed driver dispatcher (`src/keyboards/`), in-memory Hall-effect shortcut/Fn watcher (`evdev` + `uinput`), and background audio equalizer with privilege-dropped `--capture-pcm` helper. |
| `crates/reaula-cli` | `reaula` | Command-line client connecting via `reaulad` or direct HID (`--direct`), plus `doctor` diagnostics and `audio` subcommands. |
| `crates/reaula-tui` | `reaula-tui` | `ratatui` terminal dashboard for live key travel, actuation/switch profiles, firmware lighting, daemon Audio EQ controls, and SOCD/security status. |
| `crates/reaula-gui` | `reaula-gui` | GPUI desktop app with `wgpu` 3D keyboard & switch-cutaway visualizations and 2D fallbacks. |

## Development Setup & Verification

### Prerequisites

- Rust toolchain supporting **Edition 2024** (see `Cargo.toml` in each crate).
- Linux development headers/runtime libraries used by dependencies:
  - HID/USB: `libudev` / `libusb` (required by `hidapi`)
  - Live desktop/mic audio capture: `libasound` (`libasound.so.2`) and `pactl` (optional, for PipeWire/Pulse monitor lookup)
  - Desktop 3D previews (`reaula-gui`): Vulkan or OpenGL ES runtime for `wgpu` (the GUI automatically provides 2D fallbacks if a GPU adapter is unavailable)

### Required Checks Before Sending Changes

Run strict Clippy checks, all workspace tests, and full binary compilation:

```bash
cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace --bins
```

> **Note when developing daemon/IPC changes:** If an older `reaulad` instance is already running in the background, stop and restart it with your newly built binaries before testing frontends:
> ```bash
> ./target/debug/reaula daemon stop && ./target/debug/reaula daemon start
> ```

## Adding a New Aula Keyboard Model

The driver architecture indexes keyboards cleanly across core, daemon, and 3D GUI modules so adding another Aula board (such as `win68he`, `f75`, or `hero68he`) follows a repeatable four-step pattern:

1. **Core protocol, keymap, and driver (`crates/reaula-core/src/keyboards/<model>/`)**:
   - Create `<model>/mod.rs` exporting `pub const SPEC: KeyboardSpec` with a unique `id` slug, `aliases`, `KeyboardFamily`, supported `UsbId`s (`vid`, `pid`, `UsbTransportKind`), vendor `vendor_usage_page` / `vendor_usage` / fallback interface, matrix dimensions, physical `&KEYS: &[KeyDef]`, `udev_rule`, and accurate `KeyboardCapabilities` flags.
   - Add `<model>/protocol.rs`, `<model>/keymap.rs`, and `<model>/driver.rs`.
   - Declare `pub mod <model>;` and append `&<model>::SPEC` to `REGISTRY` in [`crates/reaula-core/src/keyboards/mod.rs`](crates/reaula-core/src/keyboards/mod.rs).
2. **Daemon driver adapter (`crates/reaula-daemon/src/keyboards/<model>.rs`)**:
   - Implement [`AulaDriver`](crates/reaula-daemon/src/keyboards/mod.rs) for your hardware driver and export `pub const ENTRY: DriverEntry`.
   - Declare `pub mod <model>;` and add `<model>::ENTRY` to `DRIVERS` in [`crates/reaula-daemon/src/keyboards/mod.rs`](crates/reaula-daemon/src/keyboards/mod.rs).
   - Verify CLI/daemon discovery lists it cleanly: `cargo run -p reaula-daemon -- --list-keyboards`.
3. **GUI 3D model (`crates/reaula-gui/src/kb_3d/<model>.rs`)**:
   - Export `pub const MODEL: KeyboardModel3d` with chassis, keycaps, LEDs, and Type-C wiring geometry and register it in `MODELS` inside [`crates/reaula-gui/src/kb_3d/mod.rs`](crates/reaula-gui/src/kb_3d/mod.rs).
4. **`udev` permission rule (`udev/`)**:
   - Add `udev/99-aula-<model>.rules` granting unprivileged `hidraw` access (`MODE="0660", TAG+="uaccess"`).

## Hardware & Security Guidelines

- **Single HID ownership**: `reaulad`'s `hardware_loop` owns all live USB HID traffic (`stream_custom_light`, travel polling, actuation writes, calibration). Do not open competing HID handles while `reaulad` is managing the device.
- **Read-modify-write safety**: Always read existing device tables before mutating subset bitfields (for example preserving stored Rapid Trigger intervals on fixed-actuation writes, preserving Fn/base layer bits in `cmd 0x18`, or clearing `MUSIC_RHYTHM` before lighting writes) and abort with `DriverError::ReadModifyWriteAborted` if readback fails.
- **Guard destructive commands**: Never weaken [`protocol::dangerous_command_reason`](crates/reaula-core/src/keyboards/win60he/protocol.rs), which blocks factory resets (`cmd 0x14 [5]=1`), trigger wipes (`cmd 0x21 sub 6`), and unguarded calibration-clearing commands (`cmd 0x21 sub 8/15`).
- **Fail-closed IPC & privilege separation**:
  - Keep the daemon socket directory `0700`, socket file `0600`, and UID allowlist enforcement via kernel `SO_PEERCRED`.
  - When `reaulad` runs as `root` and accesses desktop audio streams, capture must stay isolated inside the single-threaded privilege-dropped `--capture-pcm` child process (`uid`/`gid` dropped to the non-root user).
- **GUI `wgpu` constraints & fallbacks**:
  - Keep offscreen `RenderImage` buffer widths aligned so `width * 4` is divisible by `256` bytes (`wgpu` `COPY_BYTES_PER_ROW_ALIGNMENT`), targeting `Bgra8Unorm`.
  - Always maintain 2D fallbacks (`LightViewMode::TwoD`, `SwitchViewMode::TwoD`) so machines without Vulkan/GLES continue working cleanly.

## Testing Expectations

- Every protocol encoder/decoder, keymap geometry table, IPC message type, and daemon engine state machine should include unit tests that run **without requiring attached physical hardware** (for example using packet capture vectors or `StubDriver` mocks).
- Visual 3D mesh builders in `kb_3d` should assert non-empty, finite geometry and test offscreen `wgpu` frame readback cleanly when a GPU adapter is available (gracefully returning early without failing headless CI when no GPU adapter exists, while still catching WGSL/pipeline validation errors).
