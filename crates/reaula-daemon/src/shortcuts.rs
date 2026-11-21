//! Daemon-owned shortcut watcher, evdev key-leak suppressor, and `/dev/uinput` injector.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fs::{self, Permissions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use reaula_core::diagnostics;
use reaula_core::ipc::{
    self, DAEMON_INDICATOR_AQUA_RGB, DAEMON_INDICATOR_PULSE_RGB, DaemonResponse, ShortcutRule,
    ShortcutTriggerLog, ShortcutsSnapshot, VirtualKey,
};
use reaula_core::keyboards::{KeyDef, KeyboardSpec};
use reaula_core::keymap;
use reaula_core::protocol::{LiveTravelSample, MATRIX_SLOTS};

use crate::{evdev, uinput};

const MAX_SHORTCUT_LOG: usize = 40;
const INDICATOR_PULSE_WINDOW: Duration = Duration::from_millis(650);
const SYSTEM_SHORTCUTS_FILE: &str = "/var/lib/reaula/shortcuts.json";

pub(crate) struct ShortcutEngine {
    pub(crate) rules: Vec<ShortcutRule>,
    pub(crate) watcher_active: bool,
    active_keys: &'static [KeyDef],
    held_keys: BTreeSet<u8>,
    modifier_engaged: BTreeSet<u8>,
    latched_rules: BTreeSet<u32>,
    active_virtual_keys: BTreeMap<u32, VirtualKey>,
    indicator_pulses: BTreeMap<u8, Instant>,
    uinput: Arc<Mutex<Option<uinput::VirtualKeyboard>>>,
    uinput_error: Option<String>,
    suppression: Arc<Mutex<evdev::SuppressionState>>,
    evdev: evdev::EvdevInterceptor,
    persist_path: Option<PathBuf>,
    recent_triggers: VecDeque<ShortcutTriggerLog>,
}

#[cfg(test)]
impl Default for ShortcutEngine {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            watcher_active: false,
            active_keys: reaula_core::keyboards::win60he::SPEC.keys,
            held_keys: BTreeSet::new(),
            modifier_engaged: BTreeSet::new(),
            latched_rules: BTreeSet::new(),
            active_virtual_keys: BTreeMap::new(),
            indicator_pulses: BTreeMap::new(),
            uinput: Arc::new(Mutex::new(None)),
            uinput_error: None,
            suppression: Arc::new(Mutex::new(evdev::SuppressionState::default())),
            evdev: evdev::EvdevInterceptor::idle(),
            persist_path: None,
            recent_triggers: VecDeque::new(),
        }
    }
}

impl ShortcutEngine {
    pub(crate) fn new(persist_path: Option<PathBuf>, spec: &'static KeyboardSpec) -> Self {
        let uinput = Arc::new(Mutex::new(None));
        let suppression = Arc::new(Mutex::new(evdev::SuppressionState::default()));
        let evdev = evdev::EvdevInterceptor::spawn(Arc::clone(&uinput), Arc::clone(&suppression));
        let mut engine = Self {
            rules: Vec::new(),
            watcher_active: false,
            active_keys: spec.keys,
            held_keys: BTreeSet::new(),
            modifier_engaged: BTreeSet::new(),
            latched_rules: BTreeSet::new(),
            active_virtual_keys: BTreeMap::new(),
            indicator_pulses: BTreeMap::new(),
            uinput,
            uinput_error: None,
            suppression,
            evdev,
            persist_path,
            recent_triggers: VecDeque::new(),
        };
        engine.ensure_uinput();
        engine.load_from_disk();
        engine.sync_evdev();
        engine
    }

    pub(crate) fn set_keyboard_spec(&mut self, spec: &'static KeyboardSpec) {
        self.active_keys = spec.keys;
        self.sync_evdev();
    }

    fn uinput_ready(&self) -> bool {
        self.uinput.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    fn with_uinput<T>(
        &self,
        f: impl FnOnce(&mut uinput::VirtualKeyboard) -> io::Result<T>,
    ) -> Result<T, String> {
        let mut guard = self
            .uinput
            .lock()
            .map_err(|_| "uinput mutex poisoned".to_string())?;
        let Some(kbd) = guard.as_mut() else {
            return Err(self
                .uinput_error
                .clone()
                .unwrap_or_else(|| "/dev/uinput unavailable".into()));
        };
        f(kbd).map_err(|e| e.to_string())
    }

    fn sync_evdev(&self) {
        let mut armed = HashSet::new();
        let any_enabled = self.watcher_active && self.rules.iter().any(|r| r.enabled);
        if any_enabled {
            for rule in &self.rules {
                if !rule.enabled {
                    continue;
                }
                let mod_ok = rule
                    .modifier_key
                    .is_none_or(|m| self.modifier_engaged.contains(&m) || self.held_keys.contains(&m));
                if !mod_ok {
                    continue;
                }
                if let Some(key) = self.active_keys.iter().find(|k| k.index == rule.trigger_key)
                    && let Some(ev_code) = keymap::hid_usage_to_linux_ev_key(key.hid_code)
                {
                    armed.insert(ev_code);
                }
            }
        }
        if let Ok(mut sup) = self.suppression.lock() {
            sup.set_armed(armed);
        }
        self.evdev.set_enabled(any_enabled && self.uinput_ready());
    }

    pub(crate) fn reset_live_state(&mut self) {
        self.held_keys.clear();
        self.modifier_engaged.clear();
        self.latched_rules.clear();
        self.indicator_pulses.clear();
        self.release_all_virtual_keys();
        self.sync_evdev();
    }

    pub(crate) fn ensure_uinput(&mut self) {
        if self.uinput_ready() {
            self.sync_evdev();
            return;
        }
        match uinput::VirtualKeyboard::open() {
            Ok(kbd) => {
                eprintln!("uinput virtual Fn keyboard active (/dev/uinput)");
                if let Ok(mut guard) = self.uinput.lock() {
                    *guard = Some(kbd);
                }
                self.uinput_error = None;
                self.sync_evdev();
            }
            Err(e) => {
                let (detail, remedy) = diagnostics::diagnose_uinput_open_error(&e);
                let combined = format!("{detail} — {remedy}");
                if self.uinput_error.as_deref() != Some(combined.as_str()) {
                    eprintln!("note: /dev/uinput unavailable: {combined}");
                }
                self.uinput_error = Some(combined);
                self.sync_evdev();
            }
        }
    }

    pub(crate) fn has_active_rules(&self) -> bool {
        self.watcher_active && self.rules.iter().any(|r| r.enabled)
    }

    pub(crate) fn release_all_virtual_keys(&mut self) {
        let keys: Vec<VirtualKey> = self.active_virtual_keys.values().copied().collect();
        self.active_virtual_keys.clear();
        if keys.is_empty() {
            return;
        }
        let _ = self.with_uinput(|kbd| {
            for vk in keys {
                let _ = kbd.set_key(vk, false);
            }
            Ok(())
        });
    }

    pub(crate) fn set_rules(&mut self, rules: Vec<ShortcutRule>, watcher_active: bool) {
        self.release_all_virtual_keys();
        let valid_ids: BTreeSet<u32> = rules
            .iter()
            .filter(|r| watcher_active && r.enabled)
            .map(|r| r.id)
            .collect();
        self.latched_rules.retain(|id| valid_ids.contains(id));
        self.rules = rules;
        self.watcher_active = watcher_active;
        self.save_to_disk();
        self.sync_evdev();
    }

    fn load_from_disk(&mut self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        if let Ok(meta) = fs::symlink_metadata(path) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                eprintln!("refusing non-regular shortcut config path {}", path.display());
                return;
            }
            match fs::read(path) {
                Ok(bytes) => match ipc::decode_persisted_shortcuts(&bytes) {
                    Ok(saved) => {
                        eprintln!(
                            "loaded {} memorized shortcut(s) (watcher={}) from {}",
                            saved.rules.len(),
                            saved.watcher_active,
                            path.display()
                        );
                        self.rules = saved.rules;
                        self.watcher_active = saved.watcher_active;
                        self.sync_evdev();
                    }
                    Err(e) => {
                        eprintln!(
                            "failed to parse memorized shortcuts from {}: {e}",
                            path.display()
                        );
                    }
                },
                Err(e) => {
                    eprintln!("failed to read memorized shortcuts from {}: {e}", path.display());
                }
            }
        }
    }

    fn save_to_disk(&self) {
        let Some(path) = &self.persist_path else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Ok(meta) = fs::symlink_metadata(parent)
                && meta.file_type().is_symlink()
            {
                eprintln!("refusing symlink shortcut dir {}", parent.display());
                return;
            }
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    eprintln!("failed to create {}: {e}", parent.display());
                    return;
                }
                let _ = fs::set_permissions(parent, Permissions::from_mode(0o700));
            }
        }
        if let Ok(meta) = fs::symlink_metadata(path)
            && meta.file_type().is_symlink()
        {
            eprintln!("refusing symlink shortcut file {}", path.display());
            return;
        }
        let Ok(json) = ipc::encode_persisted_shortcuts(&self.rules, self.watcher_active) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        let write_res = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .and_then(|mut f| {
                f.write_all(&json)?;
                f.sync_all()
            });
        match write_res {
            Ok(()) => {
                if let Err(e) = fs::rename(&tmp, path) {
                    eprintln!("failed to rename {}: {e}", path.display());
                }
            }
            Err(e) => eprintln!("failed to persist shortcuts to {}: {e}", path.display()),
        }
    }

    pub(crate) fn evaluate_sample(
        &mut self,
        sample: &LiveTravelSample,
        unit_mm: f64,
    ) -> Vec<ShortcutRule> {
        let depth_mm = sample.depth_mm(unit_mm);
        if depth_mm >= 0.35 {
            self.modifier_engaged.insert(sample.index);
        } else if depth_mm <= 0.15 {
            self.modifier_engaged.remove(&sample.index);
        }
        if depth_mm >= 0.85 {
            self.held_keys.insert(sample.index);
        } else if depth_mm <= 0.45 {
            self.held_keys.remove(&sample.index);
        }

        // Release latched rules when either the trigger key OR its chord modifier lifts.
        let mut to_release = Vec::new();
        for rule in &self.rules {
            if !self.latched_rules.contains(&rule.id) {
                continue;
            }
            let trig_released = sample.index == rule.trigger_key
                && depth_mm <= (rule.trigger_depth_mm * 0.65).max(0.05);
            let mod_released = rule.modifier_key == Some(sample.index) && depth_mm <= 0.45;
            if trig_released || mod_released {
                to_release.push(rule.id);
            }
        }
        for rule_id in to_release {
            self.latched_rules.remove(&rule_id);
            if let Some(vk) = self.active_virtual_keys.remove(&rule_id) {
                let _ = self.with_uinput(|kbd| kbd.set_key(vk, false));
            }
        }

        if !self.watcher_active {
            return Vec::new();
        }

        // Evaluate chorded rules before bare single-key rules on the same trigger key.
        let chord_matched = self.rules.iter().any(|rule| {
            rule.enabled
                && rule.trigger_key == sample.index
                && rule.modifier_key.is_some_and(|m| self.held_keys.contains(&m))
                && depth_mm >= rule.trigger_depth_mm
        });

        let mut fired_external = Vec::new();
        let mut virtual_presses: Vec<(ShortcutRule, VirtualKey)> = Vec::new();
        for rule in &self.rules {
            if !rule.enabled || rule.trigger_key != sample.index {
                continue;
            }
            if chord_matched && rule.modifier_key.is_none() {
                continue;
            }
            if let Some(m) = rule.modifier_key
                && !self.held_keys.contains(&m)
            {
                continue;
            }
            if !self.latched_rules.contains(&rule.id) && depth_mm >= rule.trigger_depth_mm {
                self.latched_rules.insert(rule.id);
                if let ipc::ShortcutAction::EmitKey { key } = rule.action {
                    virtual_presses.push((rule.clone(), key));
                } else {
                    fired_external.push(rule.clone());
                }
            }
        }

        for (rule, vk) in virtual_presses {
            self.ensure_uinput();
            let (success, message) = match self.with_uinput(|kbd| kbd.set_key(vk, true)) {
                Ok(()) => {
                    self.active_virtual_keys.insert(rule.id, vk);
                    self.record_rule_pulse(&rule);
                    (true, format!("Held {} ({})", vk.label(), rule.gesture_label()))
                }
                Err(msg) => (false, msg),
            };
            self.push_log(ShortcutTriggerLog {
                rule_id: rule.id,
                rule_name: rule.name,
                trigger_key: rule.trigger_key,
                action_summary: rule.action.summary(),
                unix_ms: diagnostics::now_unix_ms(),
                success,
                message,
            });
        }

        for rule in &fired_external {
            self.record_rule_pulse(rule);
        }

        self.sync_evdev();
        fired_external
    }

    pub(crate) fn record_rule_pulse(&mut self, rule: &ShortcutRule) {
        let now = Instant::now();
        self.prune_expired_pulses(now);
        self.indicator_pulses.insert(rule.trigger_key, now);
        if let Some(m) = rule.modifier_key {
            self.indicator_pulses.entry(m).or_insert(now);
        }
    }

    fn prune_expired_pulses(&mut self, now: Instant) {
        self.indicator_pulses
            .retain(|_, ts| now.saturating_duration_since(*ts) < INDICATOR_PULSE_WINDOW);
    }

    pub(crate) fn has_active_pulses(&self) -> bool {
        let now = Instant::now();
        self.indicator_pulses
            .values()
            .any(|ts| now.saturating_duration_since(*ts) < INDICATOR_PULSE_WINDOW)
    }

    /// Compute active daemon-memory indicator keys and actively pulsing keys.
    ///
    /// Mirrors firmware Fn-layer indicators cleanly when a daemon chord modifier
    /// is held or a daemon-level shortcut fires, separated visually in Aqua Blue.
    pub(crate) fn active_indicator_keys(&self) -> (Vec<u8>, Vec<u8>) {
        if !self.watcher_active {
            return (Vec::new(), Vec::new());
        }
        let now = Instant::now();
        let mut active = BTreeSet::new();
        let mut pulsing = BTreeSet::new();

        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }
            if let Some(m) = rule.modifier_key
                && (self.modifier_engaged.contains(&m) || self.held_keys.contains(&m))
            {
                active.insert(m);
                active.insert(rule.trigger_key);
            }
            if self.latched_rules.contains(&rule.id) {
                active.insert(rule.trigger_key);
                pulsing.insert(rule.trigger_key);
                if let Some(m) = rule.modifier_key {
                    active.insert(m);
                }
            }
        }

        for (&idx, &ts) in &self.indicator_pulses {
            if now.saturating_duration_since(ts) < INDICATOR_PULSE_WINDOW {
                active.insert(idx);
                pulsing.insert(idx);
            }
        }

        (active.into_iter().collect(), pulsing.into_iter().collect())
    }

    /// Build an Aqua Blue hardware LED frame when daemon-layer indicators are active.
    pub(crate) fn build_indicator_colors(&self) -> Option<[[u8; 3]; MATRIX_SLOTS]> {
        let (active, pulsing) = self.active_indicator_keys();
        if active.is_empty() && pulsing.is_empty() {
            return None;
        }
        let mut grid = [[0u8; 3]; MATRIX_SLOTS];
        for idx in active {
            if let Some(cell) = grid.get_mut(usize::from(idx)) {
                *cell = DAEMON_INDICATOR_AQUA_RGB;
            }
        }
        for idx in pulsing {
            if let Some(cell) = grid.get_mut(usize::from(idx)) {
                *cell = DAEMON_INDICATOR_PULSE_RGB;
            }
        }
        Some(grid)
    }

    pub(crate) fn tap_virtual_key(
        &mut self,
        rule: &ShortcutRule,
        vk: VirtualKey,
    ) -> ShortcutTriggerLog {
        self.ensure_uinput();
        let (success, message) = match self.with_uinput(|kbd| kbd.tap_key(vk)) {
            Ok(()) => (true, format!("Injected {} tap via /dev/uinput", vk.label())),
            Err(msg) => (false, msg),
        };
        ShortcutTriggerLog {
            rule_id: rule.id,
            rule_name: rule.name.clone(),
            trigger_key: rule.trigger_key,
            action_summary: rule.action.summary(),
            unix_ms: diagnostics::now_unix_ms(),
            success,
            message,
        }
    }

    pub(crate) fn push_log(&mut self, log: ShortcutTriggerLog) {
        self.recent_triggers.push_front(log);
        while self.recent_triggers.len() > MAX_SHORTCUT_LOG {
            self.recent_triggers.pop_back();
        }
    }

    pub(crate) fn snapshot_response(&self) -> DaemonResponse {
        let (active_indicator_keys, pulsing_indicator_keys) = self.active_indicator_keys();
        DaemonResponse::Shortcuts(ShortcutsSnapshot {
            rules: self.rules.clone(),
            watcher_active: self.watcher_active,
            recent_triggers: self.recent_triggers.iter().cloned().collect(),
            uinput_ready: self.uinput_ready(),
            uinput_error: self.uinput_error.clone(),
            evdev_grab_active: self.evdev.is_grab_active(),
            persist_path: self.persist_path.as_ref().map(|p| p.display().to_string()),
            active_indicator_keys,
            pulsing_indicator_keys,
        })
    }
}

pub(crate) fn resolve_shortcuts_path(args_path: Option<&Path>, socket_path: &Path) -> Option<PathBuf> {
    if let Some(p) = args_path {
        return Some(p.to_path_buf());
    }
    if ipc::current_uid() == 0 && socket_path.starts_with(ipc::SYSTEM_SOCKET_DIR) {
        return Some(PathBuf::from(SYSTEM_SHORTCUTS_FILE));
    }
    socket_path.parent().map(|d| d.join("shortcuts.json"))
}

pub(crate) fn spawn_user_command(command: &str, allowed_uids: &[u32]) -> Result<String, String> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return Err("shortcut shell command cannot be empty".into());
    }
    let Some(target_uid) = allowed_uids.iter().copied().find(|&u| u != 0) else {
        return Err(
            "refusing to run shortcut command without an authorized non-root UID".into(),
        );
    };
    let me = ipc::current_uid();
    if me != 0 && me != target_uid {
        return Err(format!(
            "daemon UID {me} cannot switch privileges to UID {target_uid}"
        ));
    }

    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(trimmed)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if me == 0 {
        let gid = ipc::primary_gid_for_uid(target_uid).unwrap_or(target_uid);
        cmd.uid(target_uid).gid(gid);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn user command: {e}"))?;
    let pid = child.id();
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(format!("Spawned pid={pid} as uid={target_uid}"))
}

#[cfg(test)]
mod tests {
    use reaula_core::ipc::ShortcutAction;
    use reaula_core::protocol::TriggerMode;

    use super::*;

    #[test]
    fn shortcut_engine_rising_edge_hysteresis_and_fn_chord() {
        let mut engine = ShortcutEngine::default();
        engine.set_rules(
            vec![
                ShortcutRule {
                    id: 7,
                    name: "Toggle LED".into(),
                    enabled: true,
                    modifier_key: None,
                    trigger_key: keymap::idx::ESC,
                    trigger_depth_mm: 2.0,
                    action: ShortcutAction::ToggleLighting,
                },
                ShortcutRule {
                    id: 8,
                    name: "Fn+T Up".into(),
                    enabled: true,
                    modifier_key: Some(keymap::idx::FN),
                    trigger_key: keymap::idx::T,
                    trigger_depth_mm: 1.5,
                    action: ShortcutAction::ActuationPreset {
                        name: "Test".into(),
                        mode: TriggerMode::Fixed,
                        indices: vec![keymap::idx::T],
                        depth_mm: 1.5,
                        rt_press_mm: 0.2,
                        rt_release_mm: 0.2,
                    },
                },
            ],
            true,
        );

        let shallow_esc = LiveTravelSample {
            row: 1,
            col: 0,
            index: keymap::idx::ESC,
            travel_raw: 100,
        };
        assert!(engine.evaluate_sample(&shallow_esc, 0.01).is_empty());

        let deep_esc = LiveTravelSample {
            row: 1,
            col: 0,
            index: keymap::idx::ESC,
            travel_raw: 220,
        };
        assert_eq!(engine.evaluate_sample(&deep_esc, 0.01).len(), 1);
        assert!(engine.evaluate_sample(&deep_esc, 0.01).is_empty());

        let released_esc = LiveTravelSample {
            row: 1,
            col: 0,
            index: keymap::idx::ESC,
            travel_raw: 0,
        };
        assert!(engine.evaluate_sample(&released_esc, 0.01).is_empty());
        assert_eq!(engine.evaluate_sample(&deep_esc, 0.01).len(), 1);

        // Pressing T without Fn held must NOT fire the Fn+T chord.
        let deep_t = LiveTravelSample {
            row: 2,
            col: 5,
            index: keymap::idx::T,
            travel_raw: 200,
        };
        assert!(engine.evaluate_sample(&deep_t, 0.01).is_empty());

        // Hold Fn, release T, then press T -> fires Fn+T chord cleanly.
        let released_t = LiveTravelSample {
            row: 2,
            col: 5,
            index: keymap::idx::T,
            travel_raw: 0,
        };
        let hold_fn = LiveTravelSample {
            row: 5,
            col: 12,
            index: keymap::idx::FN,
            travel_raw: 200,
        };
        let release_fn = LiveTravelSample {
            row: 5,
            col: 12,
            index: keymap::idx::FN,
            travel_raw: 0,
        };
        let _ = engine.evaluate_sample(&released_t, 0.01);
        let _ = engine.evaluate_sample(&hold_fn, 0.01);
        let fired = engine.evaluate_sample(&deep_t, 0.01);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, 8);

        // Lifting Fn unlatches the chord even while T stays held.
        let _ = engine.evaluate_sample(&release_fn, 0.01);
        assert!(!engine.latched_rules.contains(&8));
    }

    #[test]
    fn indicator_overlay_glows_aqua_blue_for_daemon_layer_and_triggers() {
        let mut engine = ShortcutEngine::default();
        engine.set_rules(
            vec![
                ShortcutRule {
                    id: 1,
                    name: "Fn+R Up".into(),
                    enabled: true,
                    modifier_key: Some(keymap::idx::FN),
                    trigger_key: keymap::idx::R,
                    trigger_depth_mm: 1.5,
                    action: ShortcutAction::EmitKey {
                        key: VirtualKey::Up,
                    },
                },
                ShortcutRule {
                    id: 2,
                    name: "Fn+F Down".into(),
                    enabled: true,
                    modifier_key: Some(keymap::idx::FN),
                    trigger_key: keymap::idx::F,
                    trigger_depth_mm: 1.5,
                    action: ShortcutAction::EmitKey {
                        key: VirtualKey::Down,
                    },
                },
            ],
            true,
        );

        assert!(engine.build_indicator_colors().is_none());

        // Holding Fn illuminates Fn + all enabled daemon Fn+... mapped keys in Aqua Blue.
        let hold_fn = LiveTravelSample {
            row: 5,
            col: 12,
            index: keymap::idx::FN,
            travel_raw: 180,
        };
        let _ = engine.evaluate_sample(&hold_fn, 0.01);
        let grid = engine
            .build_indicator_colors()
            .expect("holding Fn should illuminate daemon indicator");
        assert_eq!(grid[usize::from(keymap::idx::FN)], DAEMON_INDICATOR_AQUA_RGB);
        assert_eq!(grid[usize::from(keymap::idx::R)], DAEMON_INDICATOR_AQUA_RGB);
        assert_eq!(grid[usize::from(keymap::idx::F)], DAEMON_INDICATOR_AQUA_RGB);
        assert_eq!(grid[usize::from(keymap::idx::ESC)], [0, 0, 0]);

        // Latching Fn+R elevates R to the brighter electric Aqua flare.
        let press_r = LiveTravelSample {
            row: 2,
            col: 4,
            index: keymap::idx::R,
            travel_raw: 180,
        };
        let _ = engine.evaluate_sample(&press_r, 0.01);
        let (active, pulsing) = engine.active_indicator_keys();
        assert!(active.contains(&keymap::idx::R));
        assert!(pulsing.contains(&keymap::idx::R));
        let flared = engine.build_indicator_colors().unwrap();
        assert_eq!(flared[usize::from(keymap::idx::R)], DAEMON_INDICATOR_PULSE_RGB);

        // Lifting both clears indicator cleanly once pulse window expires.
        engine.reset_live_state();
        assert!(engine.build_indicator_colors().is_none());
    }

    #[test]
    fn resolves_default_shortcuts_file_path() {
        let sock = PathBuf::from("/tmp/reaula-test/reaulad.sock");
        assert_eq!(
            resolve_shortcuts_path(None, &sock),
            Some(PathBuf::from("/tmp/reaula-test/shortcuts.json"))
        );
    }

    #[test]
    fn spawn_user_command_refuses_root_only_allowlist() {
        let err = spawn_user_command("true", &[0]).unwrap_err();
        assert!(err.contains("non-root"));
    }
}
