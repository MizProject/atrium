//! Persistent user preferences, language/theme setup state, and multi-keyboard hub profiles.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppLanguage {
    #[default]
    En,
    Id,
    Es,
    Ja,
    ZhCn,
}

impl AppLanguage {
    pub const ALL: [Self; 5] = [Self::En, Self::Id, Self::Es, Self::Ja, Self::ZhCn];

    pub const fn gpui_locale(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Id => "id",
            Self::Es => "es",
            Self::Ja => "ja",
            Self::ZhCn => "zh-CN",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::En => "English",
            Self::Id => "Bahasa Indonesia",
            Self::Es => "Español",
            Self::Ja => "日本語 (Japanese)",
            Self::ZhCn => "简体中文 (Chinese)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfiguredKeyboardKind {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyboardProfileEntry {
    pub id: String,
    pub label: String,
    pub kind: ConfiguredKeyboardKind,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_id: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hid_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub added_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_config_version")]
    pub version: u32,
    #[serde(default)]
    pub setup_completed: bool,
    #[serde(default)]
    pub disclaimer_accepted: bool,
    #[serde(default)]
    pub language: AppLanguage,
    #[serde(default = "default_theme_dark")]
    pub theme_dark: bool,
    #[serde(default)]
    pub keyboards: Vec<KeyboardProfileEntry>,
    #[serde(default)]
    pub active_keyboard_id: Option<String>,
}

const fn default_config_version() -> u32 {
    1
}

const fn default_theme_dark() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: default_config_version(),
            setup_completed: false,
            disclaimer_accepted: false,
            language: AppLanguage::En,
            theme_dark: true,
            keyboards: Vec::new(),
            active_keyboard_id: None,
        }
    }
}

impl AppConfig {
    pub fn active_keyboard(&self) -> Option<&KeyboardProfileEntry> {
        let active_id = self.active_keyboard_id.as_deref();
        active_id
            .and_then(|id| self.keyboards.iter().find(|k| k.id == id))
            .or_else(|| self.keyboards.first())
    }

    pub fn active_3d_model_id(&self) -> Option<&str> {
        self.active_keyboard().map(|k| k.model_id.as_str())
    }

    pub fn upsert_keyboard(&mut self, entry: KeyboardProfileEntry) {
        let id = entry.id.clone();
        if let Some(slot) = self.keyboards.iter_mut().find(|k| k.id == id) {
            *slot = entry;
        } else {
            self.keyboards.push(entry);
        }
        self.active_keyboard_id = Some(id);
    }

    pub fn activate_keyboard(&mut self, id: &str) -> bool {
        if self.keyboards.iter().any(|k| k.id == id) {
            self.active_keyboard_id = Some(id.to_string());
            true
        } else {
            false
        }
    }

    pub fn remove_keyboard(&mut self, id: &str) -> bool {
        let prev_len = self.keyboards.len();
        self.keyboards.retain(|k| k.id != id);
        if self.keyboards.len() == prev_len {
            return false;
        }
        if self.active_keyboard_id.as_deref() == Some(id) {
            self.active_keyboard_id = self.keyboards.first().map(|k| k.id.clone());
        }
        true
    }
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn config_file_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir).join("reaula").join("gui.json");
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home)
            .join(".config")
            .join("reaula")
            .join("gui.json");
    }
    PathBuf::from("/tmp/reaula-gui.json")
}

pub fn probe_export_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir).join("reaula").join("probes");
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("reaula")
            .join("probes");
    }
    PathBuf::from("/tmp/reaula-probes")
}

pub fn load_app_config() -> AppConfig {
    load_app_config_from(&config_file_path())
}

pub fn load_app_config_from(path: &Path) -> AppConfig {
    let Ok(bytes) = fs::read(path) else {
        return AppConfig::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

pub fn save_app_config(cfg: &AppConfig) -> Result<PathBuf, String> {
    let path = config_file_path();
    save_app_config_to(&path, cfg)?;
    Ok(path)
}

pub fn save_app_config_to(path: &Path, cfg: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let json =
        serde_json::to_string_pretty(cfg).map_err(|e| format!("serialize gui config: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    set_private_file_mode(&tmp);
    fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))?;
    Ok(())
}

pub fn save_probe_json_file(model_hint: &str, json: &str) -> Result<PathBuf, String> {
    save_probe_json_file_in(&probe_export_dir(), model_hint, json)
}

pub fn save_probe_json_file_in(dir: &Path, model_hint: &str, json: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let slug: String = model_hint
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let stem = if slug.is_empty() {
        "unsupported-keyboard"
    } else {
        slug
    };
    let path = dir.join(format!("reaula-probe-{stem}-{}.json", now_unix_ms()));
    fs::write(&path, json).map_err(|e| format!("write {}: {e}", path.display()))?;
    set_private_file_mode(&path);
    Ok(path)
}

#[cfg(unix)]
fn set_private_file_mode(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_private_file_mode(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_and_multi_keyboard_crud() {
        let dir =
            std::env::temp_dir().join(format!("reaula-gui-cfg-{}", std::process::id()));
        let path = dir.join("gui.json");

        let mut cfg = AppConfig {
            setup_completed: true,
            disclaimer_accepted: true,
            language: AppLanguage::Ja,
            theme_dark: false,
            ..Default::default()
        };

        cfg.upsert_keyboard(KeyboardProfileEntry {
            id: "win60he-primary".into(),
            label: "AULA WIN 60 HE".into(),
            kind: ConfiguredKeyboardKind::Supported,
            model_id: "win60he".into(),
            vendor_id: Some(0x2E3C),
            product_id: Some(0xC365),
            hid_summary: Some("USB 2E3C:C365".into()),
            notes: None,
            added_unix_ms: 100,
        });
        cfg.upsert_keyboard(KeyboardProfileEntry {
            id: "unsupported-f75".into(),
            label: "AULA F75".into(),
            kind: ConfiguredKeyboardKind::Unsupported,
            model_id: "generic-unsupported".into(),
            vendor_id: Some(0x258A),
            product_id: Some(0x010C),
            hid_summary: Some("258A:010C iface 1".into()),
            notes: Some("Tri-mode USB probe".into()),
            added_unix_ms: 200,
        });
        assert_eq!(cfg.active_3d_model_id(), Some("generic-unsupported"));
        assert!(cfg.activate_keyboard("win60he-primary"));
        assert_eq!(cfg.active_3d_model_id(), Some("win60he"));

        save_app_config_to(&path, &cfg).unwrap();
        let reloaded = load_app_config_from(&path);
        assert_eq!(reloaded, cfg);

        let mut pruned = reloaded;
        assert!(pruned.remove_keyboard("win60he-primary"));
        assert_eq!(pruned.active_3d_model_id(), Some("generic-unsupported"));
        assert_eq!(pruned.keyboards.len(), 1);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir_all(&dir);
    }
}
