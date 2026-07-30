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

    /// What player 2 gets its own binding for. Pause stays global — one
    /// key stops the match for both boards, so it needs no second copy.
    pub const PLAYER2: [Action; 8] = [
        Action::MoveLeft,
        Action::MoveRight,
        Action::SoftDrop,
        Action::HardDrop,
        Action::RotateCw,
        Action::RotateCcw,
        Action::Hold,
        Action::Zone,
    ];
}

/// CPU playstyle selector for custom matches; maps onto the AI archetypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CpuStyle {
    /// Use the archetype the ladder assigns to the chosen level.
    #[default]
    Auto,
    Balanced,
    Rusher,
    Thinker,
    Spinner,
}

impl CpuStyle {
    const ORDER: [CpuStyle; 5] = [
        CpuStyle::Auto,
        CpuStyle::Balanced,
        CpuStyle::Rusher,
        CpuStyle::Thinker,
        CpuStyle::Spinner,
    ];

    pub fn archetype(self) -> Option<crate::core::ai::Archetype> {
        use crate::core::ai::Archetype;
        match self {
            CpuStyle::Auto => None,
            CpuStyle::Balanced => Some(Archetype::Balanced),
            CpuStyle::Rusher => Some(Archetype::Rusher),
            CpuStyle::Thinker => Some(Archetype::Thinker),
            CpuStyle::Spinner => Some(Archetype::Spinner),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CpuStyle::Auto => "AUTO",
            CpuStyle::Balanced => "BALANCED",
            CpuStyle::Rusher => "RUSHER",
            CpuStyle::Thinker => "THINKER",
            CpuStyle::Spinner => "SPINNER",
        }
    }

    pub fn cycled(self, dir: i32) -> Self {
        let i = Self::ORDER.iter().position(|c| *c == self).unwrap_or(0) as i32;
        Self::ORDER[(i + dir).rem_euclid(Self::ORDER.len() as i32) as usize]
    }
}

/// Who sits on the right-hand board in a custom match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Opponent {
    #[default]
    Cpu,
    /// Local two-player: a second human on the player 2 key set (or the
    /// second connected gamepad).
    Human,
}

impl Opponent {
    pub fn toggled(self) -> Self {
        match self {
            Opponent::Cpu => Opponent::Human,
            Opponent::Human => Opponent::Cpu,
        }
    }
}

/// Rule sheet for a custom match, persisted with the settings so the
/// last-used setup comes back next launch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomMatchConfig {
    /// Who plays the right-hand board.
    pub opponent: Opponent,
    /// CPU skill on the 1..=30 ladder.
    pub cpu_level: u32,
    pub cpu_style: CpuStyle,
    /// Rounds needed to take the match (1..=5).
    pub wins_needed: u32,
    /// Zone gauges for both players.
    pub zone: bool,
    /// Seconds before the attack ramp starts; 0 disables margin time.
    pub margin_secs: u32,
    /// 0 = timed gravity ramp (like VS); 1..=20 = fixed gravity level.
    pub speed_level: u32,
    /// Attack handicaps, percent of normal garbage sent (50..=200).
    pub player_attack_pct: u32,
    pub cpu_attack_pct: u32,
    /// Cheese rows pre-stacked on both boards (0..=8).
    pub start_garbage: u32,
    /// Percent chance each row of an incoming attack re-rolls its hole
    /// column (0 = one clean well per attack, 100 = pure cheese).
    pub messiness: u32,
    /// False bans the hold slot for both boards.
    pub hold: bool,
    /// Previews shown to both boards (0..=5).
    pub previews: u32,
}

impl Default for CustomMatchConfig {
    fn default() -> Self {
        Self {
            opponent: Opponent::Cpu,
            cpu_level: 15,
            cpu_style: CpuStyle::Auto,
            wins_needed: 2,
            zone: false,
            margin_secs: 90,
            speed_level: 0,
            player_attack_pct: 100,
            cpu_attack_pct: 100,
            start_garbage: 0,
            messiness: 0,
            hold: true,
            previews: 5,
        }
    }
}

impl CustomMatchConfig {
    /// Clamp every field into its legal range (files can say anything).
    fn sanitized(mut self) -> Self {
        self.cpu_level = self.cpu_level.clamp(1, crate::core::ai::MAX_STAGE);
        self.wins_needed = self.wins_needed.clamp(1, 5);
        self.margin_secs = self.margin_secs.min(300);
        self.speed_level = self.speed_level.min(20);
        self.player_attack_pct = self.player_attack_pct.clamp(50, 200);
        self.cpu_attack_pct = self.cpu_attack_pct.clamp(50, 200);
        self.start_garbage = self.start_garbage.min(8);
        self.messiness = (self.messiness.min(100) / 25) * 25;
        self.previews = self.previews.min(5);
        self
    }
}

/// All key codes offered for rebinding; also the parse table for the
/// serialized Debug-string form.
pub fn bindable_keys() -> Vec<KeyCode> {
    use KeyCode::*;
    vec![
        KeyA,
        KeyB,
        KeyC,
        KeyD,
        KeyE,
        KeyF,
        KeyG,
        KeyH,
        KeyI,
        KeyJ,
        KeyK,
        KeyL,
        KeyM,
        KeyN,
        KeyO,
        KeyP,
        KeyQ,
        KeyR,
        KeyS,
        KeyT,
        KeyU,
        KeyV,
        KeyW,
        KeyX,
        KeyY,
        KeyZ,
        Digit0,
        Digit1,
        Digit2,
        Digit3,
        Digit4,
        Digit5,
        Digit6,
        Digit7,
        Digit8,
        Digit9,
        ArrowLeft,
        ArrowRight,
        ArrowUp,
        ArrowDown,
        Space,
        Enter,
        Escape,
        Tab,
        Backspace,
        ShiftLeft,
        ShiftRight,
        ControlLeft,
        ControlRight,
        AltLeft,
        AltRight,
        Minus,
        Equal,
        BracketLeft,
        BracketRight,
        Semicolon,
        Quote,
        Comma,
        Period,
        Slash,
        Backslash,
        Numpad0,
        Numpad1,
        Numpad2,
        Numpad3,
        Numpad4,
        Numpad5,
        Numpad6,
        Numpad7,
        Numpad8,
        Numpad9,
        NumpadAdd,
        NumpadSubtract,
        NumpadMultiply,
        NumpadDivide,
        NumpadEnter,
        F1,
        F2,
        F3,
        F4,
        F5,
        F6,
        F7,
        F8,
        F9,
        F10,
        F11,
        F12,
    ]
}

pub fn key_name(key: KeyCode) -> String {
    format!("{key:?}")
}

/// Gamepad buttons offered for rebinding; also the parse table for the
/// serialized Debug-string form. Anything a pad reports outside this
/// list stays unbindable (menus own the fixed navigation controls).
pub fn bindable_pad_buttons() -> Vec<GamepadButton> {
    use GamepadButton::*;
    vec![
        South,
        East,
        West,
        North,
        LeftTrigger,
        RightTrigger,
        LeftTrigger2,
        RightTrigger2,
        Select,
        Start,
        LeftThumb,
        RightThumb,
        DPadUp,
        DPadDown,
        DPadLeft,
        DPadRight,
    ]
}

pub fn pad_button_name(button: GamepadButton) -> String {
    format!("{button:?}")
}

/// Short, Xbox-style button label for the UI.
pub fn pad_button_label(button: GamepadButton) -> &'static str {
    match button {
        GamepadButton::South => "A",
        GamepadButton::East => "B",
        GamepadButton::West => "X",
        GamepadButton::North => "Y",
        GamepadButton::LeftTrigger => "LB",
        GamepadButton::RightTrigger => "RB",
        GamepadButton::LeftTrigger2 => "LT",
        GamepadButton::RightTrigger2 => "RT",
        GamepadButton::Select => "SELECT",
        GamepadButton::Start => "START",
        GamepadButton::LeftThumb => "L3",
        GamepadButton::RightThumb => "R3",
        GamepadButton::DPadUp => "D-UP",
        GamepadButton::DPadDown => "D-DOWN",
        GamepadButton::DPadLeft => "D-LEFT",
        GamepadButton::DPadRight => "D-RIGHT",
        _ => "?",
    }
}

fn parse_pad_button(name: &str) -> Option<GamepadButton> {
    bindable_pad_buttons()
        .into_iter()
        .find(|b| pad_button_name(*b) == name)
}

fn default_pad_bindings() -> HashMap<Action, GamepadButton> {
    let mut bindings = HashMap::new();
    bindings.insert(Action::MoveLeft, GamepadButton::DPadLeft);
    bindings.insert(Action::MoveRight, GamepadButton::DPadRight);
    bindings.insert(Action::SoftDrop, GamepadButton::DPadDown);
    bindings.insert(Action::HardDrop, GamepadButton::DPadUp);
    bindings.insert(Action::RotateCw, GamepadButton::South);
    bindings.insert(Action::RotateCcw, GamepadButton::East);
    bindings.insert(Action::Hold, GamepadButton::LeftTrigger);
    bindings.insert(Action::Zone, GamepadButton::RightTrigger2);
    bindings.insert(Action::Pause, GamepadButton::Start);
    bindings
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
    #[serde(default)]
    bindings2: HashMap<Action, String>,
    #[serde(default)]
    pad_bindings: HashMap<Action, String>,
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
    #[serde(default)]
    custom: CustomMatchConfig,
    /// Folder name of the character the player picked last. Only a
    /// starting point for the picker's cursor — the match itself always
    /// takes what the picker set.
    #[serde(default)]
    character: String,
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
    /// Player 2's keyboard set for local versus. Defaults to the left
    /// hand of the keyboard so both players fit on one board without
    /// reaching across each other.
    pub bindings2: HashMap<Action, KeyCode>,
    /// Gamepad button per action. Menu navigation (D-pad/stick, A
    /// confirm, B back) and stick movement stay fixed; these only drive
    /// in-game actions.
    pub pad_bindings: HashMap<Action, GamepadButton>,
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
    /// Last-used custom match rule sheet.
    pub custom: CustomMatchConfig,
    /// Folder name of the last character the player chose, so the picker
    /// opens on them. Empty until they pick one, and not authoritative:
    /// a character that has since been deleted just means the picker
    /// opens on the first one instead.
    pub character: String,
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
            bindings2: default_bindings2(),
            pad_bindings: default_pad_bindings(),
            das_ms: 160,
            arr_ms: 40,
            master_volume: 8,
            bgm_volume: 6,
            sfx_volume: 8,
            vsync: false,
            sdf: 20,
            fullscreen: false,
            language: crate::i18n::LangChoice::Auto,
            custom: CustomMatchConfig::default(),
            character: String::new(),
        }
    }
}

/// Candidate layouts for player 2, in preference order: the left hand of
/// the keyboard first (it exists on every keyboard), the numpad second
/// for players whose own bindings already sit on WASD.
const PLAYER2_LAYOUTS: [[(Action, KeyCode); 8]; 2] = [
    [
        (Action::MoveLeft, KeyCode::KeyA),
        (Action::MoveRight, KeyCode::KeyD),
        (Action::SoftDrop, KeyCode::KeyS),
        (Action::RotateCw, KeyCode::KeyW),
        (Action::RotateCcw, KeyCode::KeyQ),
        (Action::HardDrop, KeyCode::ShiftLeft),
        (Action::Hold, KeyCode::KeyE),
        (Action::Zone, KeyCode::KeyR),
    ],
    [
        (Action::MoveLeft, KeyCode::Numpad4),
        (Action::MoveRight, KeyCode::Numpad6),
        (Action::SoftDrop, KeyCode::Numpad5),
        (Action::RotateCw, KeyCode::Numpad8),
        (Action::RotateCcw, KeyCode::Numpad7),
        (Action::HardDrop, KeyCode::Numpad0),
        (Action::Hold, KeyCode::Numpad9),
        (Action::Zone, KeyCode::NumpadAdd),
    ],
];

fn default_bindings2() -> HashMap<Action, KeyCode> {
    PLAYER2_LAYOUTS[0].iter().copied().collect()
}

/// Player 2's starting layout given what player 1 already uses. Two
/// people on one keyboard must not share a key — a stray press would
/// move both boards — so pick the first layout that stays clear of
/// player 1, and fall back to the left hand if none does (the per-action
/// fixup in [`GameSettings::from_file`] then moves what still collides).
fn fit_bindings2(p1: &HashMap<Action, KeyCode>) -> HashMap<Action, KeyCode> {
    PLAYER2_LAYOUTS
        .iter()
        .find(|layout| {
            layout
                .iter()
                .all(|(_, key)| !p1.values().any(|taken| taken == key))
        })
        .unwrap_or(&PLAYER2_LAYOUTS[0])
        .iter()
        .copied()
        .collect()
}

impl GameSettings {
    pub fn key_for(&self, action: Action) -> KeyCode {
        *self
            .bindings
            .get(&action)
            .expect("every action always has a binding")
    }

    /// Player 2's key for `action`. Falls back to player 1's binding for
    /// anything player 2 has no separate copy of (only Pause, today).
    pub fn key2_for(&self, action: Action) -> KeyCode {
        self.bindings2
            .get(&action)
            .copied()
            .unwrap_or_else(|| self.key_for(action))
    }

    /// Rebind one of player 2's keys, with the same swap-steal semantics
    /// as [`GameSettings::bind`] — within player 2's own set only, so the
    /// two players are free to overlap if someone really wants that.
    pub fn bind2(&mut self, action: Action, key: KeyCode) {
        let previous = self.key2_for(action);
        if let Some((&other, _)) = self
            .bindings2
            .iter()
            .find(|(a, k)| **k == key && **a != action)
        {
            self.bindings2.insert(other, previous);
        }
        self.bindings2.insert(action, key);
    }

    pub fn bind(&mut self, action: Action, key: KeyCode) {
        // Steal the key from any action that currently uses it by swapping
        // bindings, so no action is ever left unbound or duplicated.
        let previous = self.key_for(action);
        if let Some((&other, _)) = self
            .bindings
            .iter()
            .find(|(a, k)| **k == key && **a != action)
        {
            self.bindings.insert(other, previous);
        }
        self.bindings.insert(action, key);
    }

    pub fn pad_for(&self, action: Action) -> GamepadButton {
        *self
            .pad_bindings
            .get(&action)
            .expect("every action always has a pad binding")
    }

    /// Same swap-steal semantics as [`GameSettings::bind`], for the pad.
    pub fn bind_pad(&mut self, action: Action, button: GamepadButton) {
        let previous = self.pad_for(action);
        if let Some((&other, _)) = self
            .pad_bindings
            .iter()
            .find(|(a, b)| **b == button && **a != action)
        {
            self.pad_bindings.insert(other, previous);
        }
        self.pad_bindings.insert(action, button);
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
            bindings2: self
                .bindings2
                .iter()
                .map(|(a, k)| (*a, key_name(*k)))
                .collect(),
            pad_bindings: self
                .pad_bindings
                .iter()
                .map(|(a, b)| (*a, pad_button_name(*b)))
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
            custom: self.custom,
            character: self.character.clone(),
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
            if taken_by_other
                && !file.bindings.contains_key(&action)
                && let Some(free) = bindable_keys()
                    .into_iter()
                    .find(|k| Action::ALL.iter().all(|a| settings.key_for(*a) != *k))
            {
                settings.bindings.insert(action, free);
            }
        }
        // Player 2's set. A file written before local versus existed has
        // none, so choose a layout that clears whatever player 1 rebound
        // themselves to rather than handing both players the same keys.
        if file.bindings2.is_empty() {
            settings.bindings2 = fit_bindings2(&settings.bindings);
        }
        for (action, name) in &file.bindings2 {
            if let Some(key) = parse_key(name) {
                settings.bindings2.insert(*action, key);
            }
        }
        // Anything the file left unspecified that still collides — with
        // player 1 or within player 2's own set — moves to a free key.
        // Keys the file DID specify stay put: an explicit choice, even a
        // clashing one, is the player's to make.
        for action in Action::PLAYER2 {
            if file.bindings2.contains_key(&action) {
                continue;
            }
            let key = settings.key2_for(action);
            let clashes = Action::PLAYER2
                .iter()
                .any(|a| *a != action && settings.key2_for(*a) == key)
                || Action::ALL.iter().any(|a| settings.key_for(*a) == key);
            if clashes
                && let Some(free) = bindable_keys().into_iter().find(|k| {
                    Action::ALL.iter().all(|a| settings.key_for(*a) != *k)
                        && Action::PLAYER2.iter().all(|a| settings.key2_for(*a) != *k)
                })
            {
                settings.bindings2.insert(action, free);
            }
        }
        // Pad bindings: same restore + collision fixup as the keyboard.
        for (action, name) in &file.pad_bindings {
            if let Some(button) = parse_pad_button(name) {
                settings.pad_bindings.insert(*action, button);
            }
        }
        for action in Action::ALL {
            let button = settings.pad_for(action);
            let taken_by_other = Action::ALL
                .iter()
                .any(|a| *a != action && settings.pad_for(*a) == button);
            if taken_by_other
                && !file.pad_bindings.contains_key(&action)
                && let Some(free) = bindable_pad_buttons()
                    .into_iter()
                    .find(|b| Action::ALL.iter().all(|a| settings.pad_for(*a) != *b))
            {
                settings.pad_bindings.insert(action, free);
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
        settings.custom = file.custom.sanitized();
        // A folder name from disk is untrusted the same way the character
        // packs themselves are: keep it only if it still looks like an id.
        if crate::character::valid_id(&file.character) {
            settings.character = file.character;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The stock layouts: player 1 on the arrows, player 2 on the left
    /// hand. Nothing should be shared.
    #[test]
    fn default_layouts_do_not_share_keys() {
        let s = GameSettings::default();
        for a in Action::PLAYER2 {
            let key = s.key2_for(a);
            assert!(
                Action::ALL.iter().all(|p1| s.key_for(*p1) != key),
                "{a:?} on {key:?} collides with player 1"
            );
        }
    }

    #[test]
    fn player2_moves_to_the_numpad_when_player_1_took_wasd() {
        let mut p1 = GameSettings::default().bindings;
        p1.insert(Action::MoveLeft, KeyCode::KeyA);
        p1.insert(Action::MoveRight, KeyCode::KeyD);
        p1.insert(Action::SoftDrop, KeyCode::KeyS);
        p1.insert(Action::RotateCw, KeyCode::KeyW);
        let p2 = fit_bindings2(&p1);
        assert_eq!(p2[&Action::MoveLeft], KeyCode::Numpad4);
        assert_eq!(p2[&Action::HardDrop], KeyCode::Numpad0);
    }

    /// Upgrading a settings file written before local versus existed must
    /// not hand player 2 keys the player already rebound themselves to.
    #[test]
    fn upgrading_an_old_file_avoids_the_existing_bindings() {
        let mut old = GameSettings::default();
        // A left-hand-free layout: J/L move, K soft, I / A rotate, S hold.
        for (action, key) in [
            (Action::MoveLeft, KeyCode::KeyJ),
            (Action::MoveRight, KeyCode::KeyL),
            (Action::SoftDrop, KeyCode::KeyK),
            (Action::RotateCw, KeyCode::KeyI),
            (Action::RotateCcw, KeyCode::KeyA),
            (Action::Hold, KeyCode::KeyS),
        ] {
            old.bindings.insert(action, key);
        }
        let mut file = old.to_file();
        file.bindings2.clear(); // as if written by an older build
        let loaded = GameSettings::from_file(file);
        for a in Action::PLAYER2 {
            let key = loaded.key2_for(a);
            assert!(
                Action::ALL.iter().all(|p1| loaded.key_for(*p1) != key),
                "{a:?} on {key:?} collides with player 1"
            );
        }
    }

    #[test]
    fn custom_rules_survive_a_save_load_round_trip() {
        let mut s = GameSettings::default();
        s.custom.opponent = Opponent::Human;
        s.custom.messiness = 75;
        s.custom.hold = false;
        s.custom.previews = 2;
        let loaded = GameSettings::from_file(s.to_file());
        assert_eq!(loaded.custom.opponent, Opponent::Human);
        assert_eq!(loaded.custom.messiness, 75);
        assert!(!loaded.custom.hold);
        assert_eq!(loaded.custom.previews, 2);
    }

    #[test]
    fn out_of_range_rules_from_a_hand_edited_file_are_clamped() {
        let mut s = GameSettings::default();
        s.custom.messiness = 999;
        s.custom.previews = 40;
        let loaded = GameSettings::from_file(s.to_file());
        assert_eq!(loaded.custom.messiness, 100);
        assert_eq!(loaded.custom.previews, 5);
    }
}
