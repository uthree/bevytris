//! Persistent settings: key bindings, handling (DAS/ARR) and volumes.
//! Stored as RON in the platform config directory.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    MoveLeft,
    MoveRight,
    SoftDrop,
    HardDrop,
    RotateCw,
    RotateCcw,
    Hold,
    /// Fire the zone super move (zone battle mode).
    Zone,
    Pause,
}

impl Action {
    pub const ALL: [Action; 9] = [
        Action::MoveLeft,
        Action::MoveRight,
        Action::SoftDrop,
        Action::HardDrop,
        Action::RotateCw,
        Action::RotateCcw,
        Action::Hold,
        Action::Zone,
        Action::Pause,
    ];

}

/// All key codes offered for rebinding; also the parse table for the
/// serialized Debug-string form.
pub fn bindable_keys() -> Vec<KeyCode> {
    use KeyCode::*;
    vec![
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN,
        KeyO, KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ, Digit0, Digit1,
        Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9, ArrowLeft, ArrowRight,
        ArrowUp, ArrowDown, Space, Enter, Escape, Tab, Backspace, ShiftLeft, ShiftRight,
        ControlLeft, ControlRight, AltLeft, AltRight, Minus, Equal, BracketLeft, BracketRight,
        Semicolon, Quote, Comma, Period, Slash, Backslash, Numpad0, Numpad1, Numpad2, Numpad3,
        Numpad4, Numpad5, Numpad6, Numpad7, Numpad8, Numpad9, NumpadAdd, NumpadSubtract,
        NumpadMultiply, NumpadDivide, NumpadEnter, F1, F2, F3, F4, F5, F6, F7, F8, F9, F10,
        F11, F12,
    ]
}

pub fn key_name(key: KeyCode) -> String {
    format!("{key:?}")
}

/// Short, human-friendly key label for the UI.
pub fn key_label(key: KeyCode) -> String {
    let name = key_name(key);
    let pretty = name
        .strip_prefix("Key")
        .or_else(|| name.strip_prefix("Digit"))
        .unwrap_or(&name);
    // NB: the default Bevy font is a FiraMono subset without arrow glyphs,
    // so arrow keys get ASCII names.
    match key {
        KeyCode::ArrowLeft => "Left".to_string(),
        KeyCode::ArrowRight => "Right".to_string(),
        KeyCode::ArrowUp => "Up".to_string(),
        KeyCode::ArrowDown => "Down".to_string(),
        _ => pretty.to_string(),
    }
}

fn parse_key(name: &str) -> Option<KeyCode> {
    bindable_keys().into_iter().find(|k| key_name(*k) == name)
}

/// Serializable settings snapshot (what actually goes into the RON file).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsFile {
    bindings: HashMap<Action, String>,
    das_ms: u32,
    arr_ms: u32,
    master_volume: u32,
    bgm_volume: u32,
    sfx_volume: u32,
    #[serde(default)]
    vsync: bool,
    #[serde(default = "default_sdf")]
    sdf: u32,
    #[serde(default)]
    fullscreen: bool,
    #[serde(default)]
    language: crate::i18n::LangChoice,
}

fn default_sdf() -> u32 {
    20
}

/// Sentinel meaning "instant" soft drop.
pub const SDF_MAX: u32 = 999;

/// Live settings resource.
#[derive(Resource, Debug, Clone)]
pub struct GameSettings {
    pub bindings: HashMap<Action, KeyCode>,
    /// Delayed auto shift: hold time before auto-repeat starts (ms).
    pub das_ms: u32,
    /// Auto repeat rate: interval between auto-shifts (ms).
    pub arr_ms: u32,
    /// Volumes in steps 0..=10.
    pub master_volume: u32,
    pub bgm_volume: u32,
    pub sfx_volume: u32,
    /// Off by default: vsync adds a frame or two of input latency.
    pub vsync: bool,
    /// Soft drop factor: gravity multiplier while soft-dropping.
    /// 5..=40 in steps of 5, or SDF_MAX for instant.
    pub sdf: u32,
    /// Borderless fullscreen (also toggled with F11).
    pub fullscreen: bool,
    /// UI language: follow the OS or force one.
    pub language: crate::i18n::LangChoice,
}

impl Default for GameSettings {
    fn default() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert(Action::MoveLeft, KeyCode::ArrowLeft);
        bindings.insert(Action::MoveRight, KeyCode::ArrowRight);
        bindings.insert(Action::SoftDrop, KeyCode::ArrowDown);
        bindings.insert(Action::HardDrop, KeyCode::Space);
        bindings.insert(Action::RotateCw, KeyCode::ArrowUp);
        bindings.insert(Action::RotateCcw, KeyCode::KeyZ);
        bindings.insert(Action::Hold, KeyCode::KeyC);
        bindings.insert(Action::Zone, KeyCode::KeyV);
        bindings.insert(Action::Pause, KeyCode::Escape);
        Self {
            bindings,
            das_ms: 160,
            arr_ms: 40,
            master_volume: 8,
            bgm_volume: 6,
            sfx_volume: 8,
            vsync: false,
            sdf: 20,
            fullscreen: false,
            language: crate::i18n::LangChoice::Auto,
        }
    }
}

impl GameSettings {
    pub fn key_for(&self, action: Action) -> KeyCode {
        *self
            .bindings
            .get(&action)
            .expect("every action always has a binding")
    }

    pub fn bind(&mut self, action: Action, key: KeyCode) {
        // Steal the key from any action that currently uses it by swapping
        // bindings, so no action is ever left unbound or duplicated.
        let previous = self.key_for(action);
        if let Some((&other, _)) = self.bindings.iter().find(|(a, k)| **k == key && **a != action) {
            self.bindings.insert(other, previous);
        }
        self.bindings.insert(action, key);
    }

    /// Gravity multiplier the core should use while soft-dropping.
    pub fn sdf_factor(&self) -> f32 {
        if self.sdf >= SDF_MAX {
            1_000_000.0
        } else {
            self.sdf as f32
        }
    }

    /// Step the SDF setting through 5,10,...,40,MAX.
    pub fn adjust_sdf(&mut self, dir: i32) {
        self.sdf = match (self.sdf, dir.signum()) {
            (SDF_MAX, 1) => SDF_MAX,
            (SDF_MAX, _) => 40,
            (40, 1) => SDF_MAX,
            (v, 1) => (v + 5).min(40),
            (v, _) => v.saturating_sub(5).max(5),
        };
    }

    pub fn sdf_label(&self) -> String {
        if self.sdf >= SDF_MAX {
            "MAX".to_string()
        } else {
            format!("{}x", self.sdf)
        }
    }

    pub fn bgm_linear(&self) -> f32 {
        (self.master_volume as f32 / 10.0) * (self.bgm_volume as f32 / 10.0)
    }

    pub fn sfx_linear(&self) -> f32 {
        (self.master_volume as f32 / 10.0) * (self.sfx_volume as f32 / 10.0)
    }

    fn to_file(&self) -> SettingsFile {
        SettingsFile {
            bindings: self
                .bindings
                .iter()
                .map(|(a, k)| (*a, key_name(*k)))
                .collect(),
            das_ms: self.das_ms,
            arr_ms: self.arr_ms,
            master_volume: self.master_volume,
            bgm_volume: self.bgm_volume,
            sfx_volume: self.sfx_volume,
            vsync: self.vsync,
            sdf: self.sdf,
            fullscreen: self.fullscreen,
            language: self.language,
        }
    }

    fn from_file(file: SettingsFile) -> Self {
        let mut settings = Self::default();
        for (action, name) in &file.bindings {
            if let Some(key) = parse_key(name) {
                settings.bindings.insert(*action, key);
            }
        }
        // Older files may predate an action (e.g. Zone); its default key
        // could then collide with a user rebinding. Give any action missing
        // from the file a key nothing else uses.
        for action in Action::ALL {
            let key = settings.key_for(action);
            let taken_by_other = Action::ALL
                .iter()
                .any(|a| *a != action && settings.key_for(*a) == key);
            if taken_by_other && !file.bindings.contains_key(&action) {
                if let Some(free) = bindable_keys()
                    .into_iter()
                    .find(|k| Action::ALL.iter().all(|a| settings.key_for(*a) != *k))
                {
                    settings.bindings.insert(action, free);
                }
            }
        }
        settings.das_ms = file.das_ms.clamp(0, 500);
        settings.arr_ms = file.arr_ms.clamp(0, 200);
        settings.master_volume = file.master_volume.min(10);
        settings.bgm_volume = file.bgm_volume.min(10);
        settings.sfx_volume = file.sfx_volume.min(10);
        settings.vsync = file.vsync;
        settings.sdf = if file.sdf >= SDF_MAX {
            SDF_MAX
        } else {
            (file.sdf.clamp(5, 40) / 5) * 5
        };
        settings.fullscreen = file.fullscreen;
        settings.language = file.language;
        settings
    }
}

fn settings_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("bevytris").join("settings.ron"))
}

pub fn load_settings() -> GameSettings {
    let Some(path) = settings_path() else {
        return GameSettings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => match ron::from_str::<SettingsFile>(&text) {
            Ok(file) => GameSettings::from_file(file),
            Err(err) => {
                warn!("failed to parse {path:?}: {err}; using defaults");
                GameSettings::default()
            }
        },
        Err(_) => GameSettings::default(),
    }
}

pub fn save_settings(settings: &GameSettings) {
    let Some(path) = settings_path() else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    if let Err(err) = std::fs::create_dir_all(dir) {
        warn!("could not create config dir {dir:?}: {err}");
        return;
    }
    match ron::ser::to_string_pretty(&settings.to_file(), Default::default()) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                warn!("could not write settings to {path:?}: {err}");
            }
        }
        Err(err) => warn!("could not serialize settings: {err}"),
    }
}
