//! Persistent settings: key bindings, handling (DAS/ARR) and volumes.
//! Stored as RON in the platform config directory.
//!
//! An action holds a *list* of inputs rather than one, so hold can sit on
//! both shoulder buttons and the player who wants space and enter to both
//! hard-drop can have that. Everything here maintains one invariant: a
//! list is never empty. An action with nothing bound to it is an action
//! the player cannot perform and, worse, cannot find again — so the
//! functions that take an input away from one action give it something
//! back, and the ones that remove refuse to remove the last.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::Hash;
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

/// Navigating the menus, as opposed to playing.
///
/// Kept apart from [`Action`] rather than folded into it, because the two
/// answer different questions. An `Action` has a player-2 counterpart and
/// feeds the game simulation; a `UiAction` has neither and never reaches a
/// board. Merging them would have given every menu direction a phantom
/// second-player binding and put "confirm" in the middle of the list of
/// things a piece can do.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum UiAction {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
}

impl UiAction {
    pub const ALL: [UiAction; 6] = [
        UiAction::Up,
        UiAction::Down,
        UiAction::Left,
        UiAction::Right,
        UiAction::Confirm,
        UiAction::Back,
    ];

    fn default_key(self) -> KeyCode {
        match self {
            UiAction::Up => KeyCode::ArrowUp,
            UiAction::Down => KeyCode::ArrowDown,
            UiAction::Left => KeyCode::ArrowLeft,
            UiAction::Right => KeyCode::ArrowRight,
            UiAction::Confirm => KeyCode::Enter,
            UiAction::Back => KeyCode::Escape,
        }
    }

    fn default_button(self) -> GamepadButton {
        match self {
            UiAction::Up => GamepadButton::DPadUp,
            UiAction::Down => GamepadButton::DPadDown,
            UiAction::Left => GamepadButton::DPadLeft,
            UiAction::Right => GamepadButton::DPadRight,
            UiAction::Confirm => GamepadButton::South,
            UiAction::Back => GamepadButton::East,
        }
    }
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

fn default_pad_bindings() -> HashMap<Action, Vec<GamepadButton>> {
    let mut bindings = HashMap::new();
    bindings.insert(Action::MoveLeft, vec![GamepadButton::DPadLeft]);
    bindings.insert(Action::MoveRight, vec![GamepadButton::DPadRight]);
    bindings.insert(Action::SoftDrop, vec![GamepadButton::DPadDown]);
    bindings.insert(Action::HardDrop, vec![GamepadButton::DPadUp]);
    bindings.insert(Action::RotateCw, vec![GamepadButton::South]);
    bindings.insert(Action::RotateCcw, vec![GamepadButton::East]);
    bindings.insert(Action::Hold, vec![GamepadButton::LeftTrigger]);
    bindings.insert(Action::Zone, vec![GamepadButton::RightTrigger2]);
    bindings.insert(Action::Pause, vec![GamepadButton::Start]);
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

/// One binding as it appears in the file: either a bare name, as every
/// version before multiple bindings wrote it, or a list.
///
/// Untagged, so a settings.ron from an older build still loads — which is
/// the whole reason this type exists rather than a plain `Vec<String>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Bound {
    One(String),
    Many(Vec<String>),
}

impl Bound {
    fn names(&self) -> &[String] {
        match self {
            Bound::One(name) => std::slice::from_ref(name),
            Bound::Many(names) => names,
        }
    }

    /// Written back as a bare name while there is only one, so a file that
    /// nobody has added a second binding to reads the way it always did.
    fn from(names: &[impl AsRef<str>]) -> Self {
        match names {
            [only] => Bound::One(only.as_ref().to_string()),
            many => Bound::Many(many.iter().map(|n| n.as_ref().to_string()).collect()),
        }
    }
}

/// Serializable settings snapshot (what actually goes into the RON file).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingsFile {
    bindings: HashMap<Action, Bound>,
    #[serde(default)]
    bindings2: HashMap<Action, Bound>,
    #[serde(default)]
    pad_bindings: HashMap<Action, Bound>,
    #[serde(default)]
    ui_bindings: HashMap<UiAction, Bound>,
    #[serde(default)]
    ui_pad_bindings: HashMap<UiAction, Bound>,
    das_ms: u32,
    arr_ms: u32,
    master_volume: u32,
    bgm_volume: u32,
    sfx_volume: u32,
    #[serde(default = "default_voice_volume")]
    voice_volume: u32,
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

fn names<B: Copy>(inputs: &[B], name: fn(B) -> String) -> Vec<String> {
    inputs.iter().map(|b| name(*b)).collect()
}

/// Read one action's list back out of the file, dropping anything that no
/// longer parses. Returns None when nothing survived, so the caller can
/// leave the default in place rather than storing an empty list.
fn parse_bound<B>(bound: &Bound, parse: fn(&str) -> Option<B>) -> Option<Vec<B>> {
    let parsed: Vec<B> = bound.names().iter().filter_map(|n| parse(n)).collect();
    (!parsed.is_empty()).then_some(parsed)
}

fn default_voice_volume() -> u32 {
    8
}

fn default_sdf() -> u32 {
    20
}

/// Sentinel meaning "instant" soft drop.
pub const SDF_MAX: u32 = 999;

/// Live settings resource.
#[derive(Resource, Debug, Clone)]
pub struct GameSettings {
    pub bindings: HashMap<Action, Vec<KeyCode>>,
    /// Menu navigation, keyboard and pad. Escape and pad B keep working as
    /// a way out *while nothing else claims them* — see
    /// [`GameSettings::ui_keys`].
    pub ui_bindings: HashMap<UiAction, Vec<KeyCode>>,
    pub ui_pad_bindings: HashMap<UiAction, Vec<GamepadButton>>,
    /// Player 2's keyboard set for local versus. Defaults to the left
    /// hand of the keyboard so both players fit on one board without
    /// reaching across each other.
    pub bindings2: HashMap<Action, Vec<KeyCode>>,
    /// Gamepad buttons per action. The left stick always moves the piece
    /// whatever these say; everything else in-game comes from here.
    pub pad_bindings: HashMap<Action, Vec<GamepadButton>>,
    /// Delayed auto shift: hold time before auto-repeat starts (ms).
    pub das_ms: u32,
    /// Auto repeat rate: interval between auto-shifts (ms).
    pub arr_ms: u32,
    /// Volumes in steps 0..=10.
    pub master_volume: u32,
    pub bgm_volume: u32,
    pub sfx_volume: u32,
    /// Character voices, which are mixed separately from the effects even
    /// though both are "sound": a voice pack is somebody else's taste, and
    /// the player who wants the game loud and the character quiet has no
    /// other way to say so.
    pub voice_volume: u32,
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
        bindings.insert(Action::MoveLeft, vec![KeyCode::ArrowLeft]);
        bindings.insert(Action::MoveRight, vec![KeyCode::ArrowRight]);
        bindings.insert(Action::SoftDrop, vec![KeyCode::ArrowDown]);
        bindings.insert(Action::HardDrop, vec![KeyCode::Space]);
        bindings.insert(Action::RotateCw, vec![KeyCode::ArrowUp]);
        bindings.insert(Action::RotateCcw, vec![KeyCode::KeyZ]);
        bindings.insert(Action::Hold, vec![KeyCode::KeyC]);
        bindings.insert(Action::Zone, vec![KeyCode::KeyV]);
        bindings.insert(Action::Pause, vec![KeyCode::Escape]);
        Self {
            bindings,
            bindings2: default_bindings2(),
            pad_bindings: default_pad_bindings(),
            das_ms: 160,
            arr_ms: 40,
            ui_bindings: UiAction::ALL
                .iter()
                .map(|a| (*a, vec![a.default_key()]))
                .collect(),
            ui_pad_bindings: UiAction::ALL
                .iter()
                .map(|a| (*a, vec![a.default_button()]))
                .collect(),
            master_volume: 8,
            bgm_volume: 6,
            sfx_volume: 8,
            voice_volume: 8,
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

fn default_bindings2() -> HashMap<Action, Vec<KeyCode>> {
    PLAYER2_LAYOUTS[0]
        .iter()
        .map(|(a, k)| (*a, vec![*k]))
        .collect()
}

/// Player 2's starting layout given what player 1 already uses. Two
/// people on one keyboard must not share a key — a stray press would
/// move both boards — so pick the first layout that stays clear of
/// player 1, and fall back to the left hand if none does (the per-action
/// fixup in [`GameSettings::from_file`] then moves what still collides).
fn fit_bindings2(p1: &HashMap<Action, Vec<KeyCode>>) -> HashMap<Action, Vec<KeyCode>> {
    PLAYER2_LAYOUTS
        .iter()
        .find(|layout| {
            layout
                .iter()
                .all(|(_, key)| !p1.values().any(|taken| taken.contains(key)))
        })
        .unwrap_or(&PLAYER2_LAYOUTS[0])
        .iter()
        .map(|(a, k)| (*a, vec![*k]))
        .collect()
}

/// Give `input` to `action`, taking it off whoever else had it.
///
/// `replace` is the difference between "rebind this to X" and "X does this
/// too", and the difference matters for more than this row. Either way the
/// input ends up exclusively this action's, because one input doing two
/// things at once is a bug the player cannot see — it was exactly that, a
/// pad B that both confirmed and cancelled, that made this worth
/// generalising.
///
/// Replacing can always take: whoever loses their last input inherits the
/// one this action just gave up, which is the swap the single-binding
/// version did and keeps every action reachable. Adding has nothing to
/// give up, so it refuses rather than stranding anyone — hence the bool.
/// Nothing is written on a refusal.
fn assign<A, B>(map: &mut HashMap<A, Vec<B>>, action: A, input: B, replace: bool) -> bool
where
    A: Copy + Eq + Hash,
    B: Copy + Eq,
{
    let previous = map.get(&action).and_then(|list| list.first()).copied();
    if !replace
        && map
            .iter()
            .any(|(other, list)| *other != action && list.len() == 1 && list[0] == input)
    {
        return false;
    }
    for (other, list) in map.iter_mut() {
        if *other == action {
            continue;
        }
        let had = list.len();
        list.retain(|held| *held != input);
        if list.is_empty() && had > 0 && let Some(previous) = previous {
            list.push(previous);
        }
    }
    let list = map.entry(action).or_default();
    if replace {
        list.clear();
    } else {
        // Pressing the same input twice on one row is a no-op rather than
        // a duplicate entry.
        list.retain(|held| *held != input);
    }
    list.push(input);
    true
}

/// Drop `action`'s last-added input. Refuses to drop the only one.
fn unassign_last<A, B>(map: &mut HashMap<A, Vec<B>>, action: A) -> bool
where
    A: Copy + Eq + Hash,
{
    match map.get_mut(&action) {
        Some(list) if list.len() > 1 => {
            list.pop();
            true
        }
        _ => false,
    }
}

/// The inputs `action` holds.
///
/// Empty only if a hand-edited file emptied it and the load-time repair
/// somehow missed it, which is why this reads rather than panics: an
/// action nobody can press is survivable, a crash on startup is not.
fn held<'a, A, B>(map: &'a HashMap<A, Vec<B>>, action: &A) -> &'a [B]
where
    A: Eq + Hash,
{
    map.get(action).map(Vec::as_slice).unwrap_or(&[])
}

impl GameSettings {
    pub fn keys_for(&self, action: Action) -> &[KeyCode] {
        held(&self.bindings, &action)
    }

    /// The first key bound to `action` — what a label shows when there is
    /// only room for one, and what a stolen binding hands back.
    pub fn key_for(&self, action: Action) -> KeyCode {
        self.keys_for(action)
            .first()
            .copied()
            .expect("every action always has a binding")
    }

    /// Player 2's keys for `action`. Falls back to player 1's bindings for
    /// anything player 2 has no separate copy of (only Pause, today).
    pub fn keys2_for(&self, action: Action) -> &[KeyCode] {
        match self.bindings2.get(&action) {
            Some(list) if !list.is_empty() => list,
            _ => self.keys_for(action),
        }
    }

    /// Rebind one of player 2's keys — within player 2's own set only, so
    /// the two players are free to overlap if someone really wants that.
    pub fn bind2(&mut self, action: Action, key: KeyCode) -> bool {
        assign(&mut self.bindings2, action, key, true)
    }

    pub fn add_bind2(&mut self, action: Action, key: KeyCode) -> bool {
        assign(&mut self.bindings2, action, key, false)
    }

    pub fn drop_bind2(&mut self, action: Action) -> bool {
        unassign_last(&mut self.bindings2, action)
    }

    pub fn bind(&mut self, action: Action, key: KeyCode) -> bool {
        assign(&mut self.bindings, action, key, true)
    }

    /// Add a second (third, ..) key for `action` rather than replacing.
    pub fn add_bind(&mut self, action: Action, key: KeyCode) -> bool {
        assign(&mut self.bindings, action, key, false)
    }

    /// Take away the most recently added key. False if it was the only one.
    pub fn drop_bind(&mut self, action: Action) -> bool {
        unassign_last(&mut self.bindings, action)
    }

    pub fn pads_for(&self, action: Action) -> &[GamepadButton] {
        held(&self.pad_bindings, &action)
    }

    pub fn bind_pad(&mut self, action: Action, button: GamepadButton) -> bool {
        assign(&mut self.pad_bindings, action, button, true)
    }

    pub fn add_bind_pad(&mut self, action: Action, button: GamepadButton) -> bool {
        assign(&mut self.pad_bindings, action, button, false)
    }

    pub fn drop_bind_pad(&mut self, action: Action) -> bool {
        unassign_last(&mut self.pad_bindings, action)
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

    pub fn ui_keys(&self, action: UiAction) -> &[KeyCode] {
        held(&self.ui_bindings, &action)
    }

    pub fn ui_buttons(&self, action: UiAction) -> &[GamepadButton] {
        held(&self.ui_pad_bindings, &action)
    }

    /// Is this input already spoken for by some menu action?
    ///
    /// Asked about Escape and pad B, which the menus otherwise honour as a
    /// way out no matter what the bindings say. That fallback has to step
    /// aside when the player has deliberately given the input to something
    /// else — otherwise swapping confirm and cancel leaves B doing both,
    /// and since backing out is checked first, confirm never happens.
    pub fn ui_key_is_bound(&self, key: KeyCode) -> bool {
        self.ui_bindings.values().any(|list| list.contains(&key))
    }

    pub fn ui_button_is_bound(&self, button: GamepadButton) -> bool {
        self.ui_pad_bindings
            .values()
            .any(|list| list.contains(&button))
    }

    pub fn bind_ui(&mut self, action: UiAction, key: KeyCode) -> bool {
        assign(&mut self.ui_bindings, action, key, true)
    }

    pub fn add_bind_ui(&mut self, action: UiAction, key: KeyCode) -> bool {
        assign(&mut self.ui_bindings, action, key, false)
    }

    pub fn drop_bind_ui(&mut self, action: UiAction) -> bool {
        unassign_last(&mut self.ui_bindings, action)
    }

    pub fn bind_ui_pad(&mut self, action: UiAction, button: GamepadButton) -> bool {
        assign(&mut self.ui_pad_bindings, action, button, true)
    }

    pub fn add_bind_ui_pad(&mut self, action: UiAction, button: GamepadButton) -> bool {
        assign(&mut self.ui_pad_bindings, action, button, false)
    }

    pub fn drop_bind_ui_pad(&mut self, action: UiAction) -> bool {
        unassign_last(&mut self.ui_pad_bindings, action)
    }

    pub fn bgm_linear(&self) -> f32 {
        (self.master_volume as f32 / 10.0) * (self.bgm_volume as f32 / 10.0)
    }

    pub fn sfx_linear(&self) -> f32 {
        (self.master_volume as f32 / 10.0) * (self.sfx_volume as f32 / 10.0)
    }

    pub fn voice_linear(&self) -> f32 {
        (self.master_volume as f32 / 10.0) * (self.voice_volume as f32 / 10.0)
    }

    fn to_file(&self) -> SettingsFile {
        SettingsFile {
            bindings: self
                .bindings
                .iter()
                .map(|(a, keys)| (*a, Bound::from(&names(keys, key_name))))
                .collect(),
            bindings2: self
                .bindings2
                .iter()
                .map(|(a, keys)| (*a, Bound::from(&names(keys, key_name))))
                .collect(),
            pad_bindings: self
                .pad_bindings
                .iter()
                .map(|(a, bs)| (*a, Bound::from(&names(bs, pad_button_name))))
                .collect(),
            ui_bindings: self
                .ui_bindings
                .iter()
                .map(|(a, keys)| (*a, Bound::from(&names(keys, key_name))))
                .collect(),
            ui_pad_bindings: self
                .ui_pad_bindings
                .iter()
                .map(|(a, bs)| (*a, Bound::from(&names(bs, pad_button_name))))
                .collect(),
            das_ms: self.das_ms,
            arr_ms: self.arr_ms,
            master_volume: self.master_volume,
            bgm_volume: self.bgm_volume,
            sfx_volume: self.sfx_volume,
            voice_volume: self.voice_volume,
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
        for (action, bound) in &file.bindings {
            if let Some(keys) = parse_bound(bound, parse_key) {
                settings.bindings.insert(*action, keys);
            }
        }
        // Older files may predate an action (e.g. Zone); its default key
        // could then collide with a user rebinding. Give any action missing
        // from the file a key nothing else uses.
        for action in Action::ALL {
            let taken_by_other = Action::ALL.iter().any(|a| {
                *a != action
                    && settings
                        .keys_for(*a)
                        .iter()
                        .any(|k| settings.keys_for(action).contains(k))
            });
            if taken_by_other
                && !file.bindings.contains_key(&action)
                && let Some(free) = bindable_keys()
                    .into_iter()
                    .find(|k| Action::ALL.iter().all(|a| !settings.keys_for(*a).contains(k)))
            {
                settings.bindings.insert(action, vec![free]);
            }
        }
        // Player 2's set. A file written before local versus existed has
        // none, so choose a layout that clears whatever player 1 rebound
        // themselves to rather than handing both players the same keys.
        if file.bindings2.is_empty() {
            settings.bindings2 = fit_bindings2(&settings.bindings);
        }
        for (action, bound) in &file.bindings2 {
            if let Some(keys) = parse_bound(bound, parse_key) {
                settings.bindings2.insert(*action, keys);
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
            let mine = settings.keys2_for(action).to_vec();
            let clashes = Action::PLAYER2
                .iter()
                .any(|a| *a != action && settings.keys2_for(*a).iter().any(|k| mine.contains(k)))
                || Action::ALL
                    .iter()
                    .any(|a| settings.keys_for(*a).iter().any(|k| mine.contains(k)));
            if clashes
                && let Some(free) = bindable_keys().into_iter().find(|k| {
                    Action::ALL.iter().all(|a| !settings.keys_for(*a).contains(k))
                        && Action::PLAYER2
                            .iter()
                            .all(|a| !settings.keys2_for(*a).contains(k))
                })
            {
                settings.bindings2.insert(action, vec![free]);
            }
        }
        // Pad bindings: same restore + collision fixup as the keyboard.
        for (action, bound) in &file.pad_bindings {
            if let Some(buttons) = parse_bound(bound, parse_pad_button) {
                settings.pad_bindings.insert(*action, buttons);
            }
        }
        // Menu navigation. A file written before these existed simply has
        // none, and the defaults already in `settings` are what the menus
        // always did — so an old file keeps behaving exactly as it did.
        for (action, bound) in &file.ui_bindings {
            if let Some(keys) = parse_bound(bound, parse_key) {
                settings.ui_bindings.insert(*action, keys);
            }
        }
        for (action, bound) in &file.ui_pad_bindings {
            if let Some(buttons) = parse_bound(bound, parse_pad_button) {
                settings.ui_pad_bindings.insert(*action, buttons);
            }
        }
        for action in Action::ALL {
            let mine = settings.pads_for(action).to_vec();
            let taken_by_other = Action::ALL
                .iter()
                .any(|a| *a != action && settings.pads_for(*a).iter().any(|b| mine.contains(b)));
            if taken_by_other
                && !file.pad_bindings.contains_key(&action)
                && let Some(free) = bindable_pad_buttons()
                    .into_iter()
                    .find(|b| Action::ALL.iter().all(|a| !settings.pads_for(*a).contains(b)))
            {
                settings.pad_bindings.insert(action, vec![free]);
            }
        }
        settings.das_ms = file.das_ms.clamp(0, 500);
        settings.arr_ms = file.arr_ms.clamp(0, 200);
        settings.master_volume = file.master_volume.min(10);
        settings.bgm_volume = file.bgm_volume.min(10);
        settings.sfx_volume = file.sfx_volume.min(10);
        settings.voice_volume = file.voice_volume.min(10);
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
    /// Every key of player 2's, against every key of player 1's.
    fn shares_with_player1(s: &GameSettings, a: Action) -> Option<KeyCode> {
        s.keys2_for(a)
            .iter()
            .copied()
            .find(|k| Action::ALL.iter().any(|p1| s.keys_for(*p1).contains(k)))
    }

    #[test]
    fn default_layouts_do_not_share_keys() {
        let s = GameSettings::default();
        for a in Action::PLAYER2 {
            assert_eq!(
                shares_with_player1(&s, a),
                None,
                "{a:?} collides with player 1"
            );
        }
    }

    #[test]
    fn player2_moves_to_the_numpad_when_player_1_took_wasd() {
        let mut p1 = GameSettings::default().bindings;
        p1.insert(Action::MoveLeft, vec![KeyCode::KeyA]);
        p1.insert(Action::MoveRight, vec![KeyCode::KeyD]);
        p1.insert(Action::SoftDrop, vec![KeyCode::KeyS]);
        p1.insert(Action::RotateCw, vec![KeyCode::KeyW]);
        let p2 = fit_bindings2(&p1);
        assert_eq!(p2[&Action::MoveLeft], vec![KeyCode::Numpad4]);
        assert_eq!(p2[&Action::HardDrop], vec![KeyCode::Numpad0]);
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
            old.bindings.insert(action, vec![key]);
        }
        let mut file = old.to_file();
        file.bindings2.clear(); // as if written by an older build
        let loaded = GameSettings::from_file(file);
        for a in Action::PLAYER2 {
            assert_eq!(
                shares_with_player1(&loaded, a),
                None,
                "{a:?} collides with player 1"
            );
        }
    }

    #[test]
    fn an_action_can_hold_more_than_one_input() {
        let mut s = GameSettings::default();
        s.add_bind(Action::Hold, KeyCode::KeyL);
        assert_eq!(s.keys_for(Action::Hold), [KeyCode::KeyC, KeyCode::KeyL]);
        // ..and survives the round trip through the file.
        let loaded = GameSettings::from_file(s.to_file());
        assert_eq!(loaded.keys_for(Action::Hold), [KeyCode::KeyC, KeyCode::KeyL]);
    }

    /// A file from before this existed wrote one bare name per action, and
    /// still has to load.
    #[test]
    fn a_single_name_in_the_file_still_loads() {
        let text = r#"(
            bindings: { MoveLeft: "Left", Hold: "C" },
            das_ms: 160, arr_ms: 40,
            master_volume: 8, bgm_volume: 6, sfx_volume: 8,
        )"#;
        let file: SettingsFile = ron::from_str(text).expect("old-shaped file should parse");
        let loaded = GameSettings::from_file(file);
        assert_eq!(loaded.keys_for(Action::Hold), [KeyCode::KeyC]);
        assert_eq!(loaded.keys_for(Action::MoveLeft), [KeyCode::ArrowLeft]);
    }

    /// One input, one action. Taking a key from an action that has only
    /// that key hands it the one the thief was using, so nothing is ever
    /// left with nothing.
    #[test]
    fn stealing_an_input_never_strands_the_loser() {
        let mut s = GameSettings::default();
        s.bind(Action::Hold, KeyCode::KeyZ); // RotateCcw's key
        assert_eq!(s.keys_for(Action::Hold), [KeyCode::KeyZ]);
        assert_eq!(s.keys_for(Action::RotateCcw), [KeyCode::KeyC]);
    }

    /// Adding has nothing to trade, so it will not take an input that is
    /// some other action's only one.
    #[test]
    fn adding_refuses_to_strand_another_action() {
        let mut s = GameSettings::default();
        assert!(
            !s.add_bind(Action::Hold, KeyCode::KeyZ),
            "Z is all RotateCcw has"
        );
        assert_eq!(s.keys_for(Action::Hold), [KeyCode::KeyC]);
        assert_eq!(s.keys_for(Action::RotateCcw), [KeyCode::KeyZ]);
        // Once RotateCcw has a spare, Hold is welcome to the original.
        assert!(s.add_bind(Action::RotateCcw, KeyCode::KeyX));
        assert!(s.add_bind(Action::Hold, KeyCode::KeyZ));
        assert_eq!(s.keys_for(Action::Hold), [KeyCode::KeyC, KeyCode::KeyZ]);
        assert_eq!(s.keys_for(Action::RotateCcw), [KeyCode::KeyX]);
    }

    #[test]
    fn the_last_binding_cannot_be_dropped() {
        let mut s = GameSettings::default();
        assert!(!s.drop_bind(Action::Hold), "one binding is the floor");
        s.add_bind(Action::Hold, KeyCode::KeyL);
        assert!(s.drop_bind(Action::Hold));
        assert_eq!(s.keys_for(Action::Hold), [KeyCode::KeyC]);
        assert!(!s.drop_bind(Action::Hold));
    }

    /// The bug this was all for: swapping menu confirm and cancel used to
    /// leave B doing both, and back is tested first.
    #[test]
    fn swapping_confirm_and_cancel_leaves_each_button_one_job() {
        let mut s = GameSettings::default();
        s.bind_ui_pad(UiAction::Confirm, GamepadButton::East);
        assert_eq!(s.ui_buttons(UiAction::Confirm), [GamepadButton::East]);
        assert_eq!(s.ui_buttons(UiAction::Back), [GamepadButton::South]);
        // ..and B is now spoken for, so the menus' unconditional way out
        // stands down rather than firing alongside confirm.
        assert!(s.ui_button_is_bound(GamepadButton::East));
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
