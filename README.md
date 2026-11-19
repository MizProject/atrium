# reaula

Userspace Linux driver suite in Rust for indexed **Aula** Hall-effect keyboards (currently **Aula WIN 60 HE** — USB `2E3C:C365`, Usage Page `0xFF1B`).

## Workspace Components

- **`crates/reaula-core`**: Indexed keyboard catalog (`keyboards/`), WIN 60 HE HID wire codec & `hidapi` driver, sysfs HID inspector, ALSA audio capture & 14-band FFT/beat engine (`audio_fx`), 4-layer diagnostics, and `SO_PEERCRED`-secured Unix socket IPC.
- **`crates/reaula-daemon` (`reaulad`)**: Single-owner USB/HID background daemon exposing a UID-restricted Unix socket (`/run/reaula/reaulad.sock`), indexed driver registry, in-memory Hall-effect shortcut/Fn watcher (`evdev` + `uinput`), and persistent daemon-hosted Audio Equalizer RGB engine.
- **`crates/reaula-cli` (`reaula`)**: CLI connecting through `reaulad` (or directly via `--direct`, prompting for `sudo` when `hidraw` permissions require it).
- **`crates/reaula-tui` (`reaula-tui`)**: Interactive `ratatui` terminal dashboard for keymap travel, actuation/switches, lighting + live Audio EQ controls, and SOCD/security inspection.
- **`crates/reaula-gui` (`reaula-gui`)**: Native GPUI + `wgpu` desktop suite featuring:
  - **Main**: 61-key actuation & Rapid Trigger (`Fixed` / `RT` / `Split`), magnetic switch profiles (`HM1`, `HH1`, `CY1`, `TC1`), top/bottom deadbands, PRCS/SOCD pairs, USB polling rate (`1/2/4/8 kHz`), Win-Lock, and live travel monitor.
  - **Calibration**: Guarded all-key and any-key Hall-effect sensor calibration.
  - **Color & Light**: Interactive `wgpu` 3D WIN 60 HE studio (beveled caps, braided rear-left Type-C cable, drag-orbit & camera presets) + 2D top-down view, per-key Custom RGB slots (`cmd 0x09`), and daemon-hosted **Audio Sync** reactive RGB (persists after closing the GUI).
  - **Hotkeys**: In-memory daemon Hall-effect chord shortcuts (`evdev`/`uinput` with key-leak guard) and onboard keyboard `cmd 0x18` Fn-layer overrides.
  - **Diagnostics**: 4-layer fault isolation (`Kernel`, `Daemon`, `Keyboard`, `Software`), 5x HID echo RTT benchmark, all-61 keycap live/peak depth verification map with `0.00 mm` return checks, and an interactive **Hall-effect switch mechanism cutaway** (`wgpu` 3D + `2D` schematic fallback).

## Quick Start

Build all binaries and run the test suite:

```bash
cargo build --workspace --bins
```

Start the background daemon (creates a `0700`/`0600` socket authenticated with kernel `SO_PEERCRED`):

```bash
cargo run -- daemon start
cargo run -- daemon status
```

Run any frontend without `sudo` while `reaulad` is active:

```bash
cargo run -- probe
cargo run -- tui    # or cargo run -p reaula-tui
cargo run -- gui    # or cargo run -p reaula-gui
```

Inspect indexed daemon keyboard drivers:

```bash
cargo run -p reaula-daemon -- --list-keyboards
```

### Experimental Audio Equalizer (Daemon-Hosted)

`reaula-daemon` captures desktop/mic audio via a privilege-dropped child helper (`--capture-pcm`) and streams analyzed 14-band FFT/beat frames to the keyboard so effects continue running after closing the GUI, TUI, or detached CLI:

```bash
reaula audio devices
reaula audio start --source monitor --effect spectrum --palette aurora
reaula audio status --watch   # Ctrl+C detaches without stopping keyboard RGB
reaula audio stop             # stops and restores previous keyboard lighting
```

> **Hardware notice**: AULA states WIN 60 HE does not officially support audio rhythm lighting. Continuous per-key `cmd 0x09` streaming is an experimental feature and may reduce LED lifespan on some units; use at your own risk (`reaula-gui` requires reading and accepting a 10-second liability confirmation first).

Stop `reaulad` cleanly when finished:

```bash
cargo run -- daemon stop
```

## Optional: Unprivileged `udev` Access

To run `--direct` HID commands or run `reaulad` without `root` prompts, install the provided `udev` rule:

```bash
sudo cp udev/99-aula-win60he.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=hidraw
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for workspace build/test checks, hardware protocol safety rules, and the guide for adding new indexed Aula keyboard models.
