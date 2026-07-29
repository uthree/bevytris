//! Localization: every user-facing UI string lives in a per-language
//! `Strings` table, selected by the `Locale` resource. The language
//! defaults to the OS locale and can be overridden in the settings
//! screen. In-game flavor terms (banner exclamations like "TETRIS!",
//! music titles, CPU archetype names) intentionally stay English.
//!
//! Adding a language = adding a `Strings` const + a `Lang` variant (and
//! making sure the game font covers its glyphs; the bundled Misaki font
//! covers ASCII + Japanese).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

use crate::config::{Action, GameSettings};

/// A concrete display language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    English,
    Japanese,
}

impl Lang {
    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::English => &EN,
            Lang::Japanese => &JA,
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Lang::English => "EN",
            Lang::Japanese => "JA",
        }
    }
}

/// What the settings store: follow the OS or force a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LangChoice {
    #[default]
    Auto,
    En,
    Ja,
}

impl LangChoice {
    pub fn resolve(self) -> Lang {
        match self {
            LangChoice::Auto => system_lang(),
            LangChoice::En => Lang::English,
            LangChoice::Ja => Lang::Japanese,
        }
    }

    /// Cycle Auto -> English -> Japanese (either direction).
    pub fn cycled(self, dir: i32) -> Self {
        const ORDER: [LangChoice; 3] = [LangChoice::Auto, LangChoice::En, LangChoice::Ja];
        let i = ORDER.iter().position(|c| *c == self).unwrap_or(0) as i32;
        ORDER[(i + dir).rem_euclid(ORDER.len() as i32) as usize]
    }
}

/// OS display language, detected once.
pub fn system_lang() -> Lang {
    static DETECTED: OnceLock<Lang> = OnceLock::new();
    *DETECTED.get_or_init(|| {
        match sys_locale::get_locale() {
            Some(tag) if tag.to_ascii_lowercase().starts_with("ja") => Lang::Japanese,
            _ => Lang::English,
        }
    })
}

/// The active language, updated live when the setting changes.
#[derive(Resource, Clone, Copy, PartialEq)]
pub struct Locale(pub Lang);

impl Locale {
    pub fn s(&self) -> &'static Strings {
        self.0.strings()
    }
}

pub struct I18nPlugin;

impl Plugin for I18nPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sync_locale);
    }
}

fn sync_locale(settings: Res<GameSettings>, mut locale: ResMut<Locale>) {
    if settings.is_changed() {
        let lang = settings.language.resolve();
        if locale.0 != lang {
            locale.0 = lang;
        }
    }
}

/// Localized label for a rebindable action (settings screen).
pub fn action_label(s: &Strings, action: Action) -> &'static str {
    match action {
        Action::MoveLeft => s.act_move_left,
        Action::MoveRight => s.act_move_right,
        Action::SoftDrop => s.act_soft_drop,
        Action::HardDrop => s.act_hard_drop,
        Action::RotateCw => s.act_rotate_cw,
        Action::RotateCcw => s.act_rotate_ccw,
        Action::Hold => s.act_hold,
        Action::Zone => s.act_zone,
        Action::Pause => s.act_pause,
    }
}

/// All user-facing UI text for one language. `{...}` placeholders are
/// substituted with `str::replace` at the call site.
pub struct Strings {
    // Title & mode menus
    pub solo: &'static str,
    pub vs_cpu: &'static str,
    pub zone_battle: &'static str,
    pub custom_match: &'static str,
    pub settings: &'static str,
    pub quit: &'static str,
    pub marathon: &'static str,
    pub sprint: &'static str,
    pub dig: &'static str,
    pub select_stage: &'static str,
    pub title_hint: &'static str,
    pub list_hint: &'static str,
    pub grid_hint: &'static str,
    // Menu footers
    pub desc_solo: &'static str,
    pub desc_vs: &'static str,
    pub desc_zone: &'static str,
    pub desc_custom: &'static str,
    pub desc_settings: &'static str,
    pub desc_quit: &'static str,
    pub desc_marathon: &'static str,
    /// `{n}` = goal lines.
    pub desc_sprint: &'static str,
    /// `{n}` = garbage rows.
    pub desc_dig: &'static str,
    pub best_label: &'static str,
    // Stage picker footer
    pub stage_prefix: &'static str,
    pub zone_prefix: &'static str,
    pub type_label: &'static str,
    /// `{n}` = wins needed.
    pub first_to: &'static str,
    pub locked: &'static str,
    pub not_cleared: &'static str,
    pub best_colon: &'static str,
    // Settings rows
    pub act_move_left: &'static str,
    pub act_move_right: &'static str,
    pub act_soft_drop: &'static str,
    pub act_hard_drop: &'static str,
    pub act_rotate_cw: &'static str,
    pub act_rotate_ccw: &'static str,
    pub act_hold: &'static str,
    pub act_zone: &'static str,
    pub act_pause: &'static str,
    pub press_key: &'static str,
    pub press_button: &'static str,
    pub sdf_setting: &'static str,
    pub master_vol: &'static str,
    pub bgm_vol: &'static str,
    pub sfx_vol: &'static str,
    pub vsync_off: &'static str,
    pub fullscreen: &'static str,
    pub language: &'static str,
    pub back: &'static str,
    pub settings_hint: &'static str,
    pub pad_hint: &'static str,
    // Settings section headings
    pub sec_keys: &'static str,
    pub sec_pad: &'static str,
    pub sec_handling: &'static str,
    pub sec_audio: &'static str,
    pub sec_display: &'static str,
    // Custom match setup rows
    pub cm_cpu_level: &'static str,
    pub cm_style: &'static str,
    pub cm_first_to: &'static str,
    pub cm_zone: &'static str,
    pub cm_margin: &'static str,
    pub cm_speed: &'static str,
    /// Timed-ramp value shown for the gravity row.
    pub cm_speed_auto: &'static str,
    pub cm_player_atk: &'static str,
    pub cm_cpu_atk: &'static str,
    pub cm_garbage: &'static str,
    pub cm_start: &'static str,
    pub cm_hint: &'static str,
    pub sec_cm_cpu: &'static str,
    pub sec_cm_rules: &'static str,
    // Pause / round / result overlays
    pub paused: &'static str,
    /// `{key}` = pause key label.
    pub pause_hint: &'static str,
    pub round_win: &'static str,
    pub round_lost: &'static str,
    /// `{p}` / `{c}` = player / cpu wins.
    pub vs_score: &'static str,
    pub next_round: &'static str,
    /// `{stage}` = "STAGE 07"-style label.
    pub stage_clear: &'static str,
    pub stage_failed: &'static str,
    pub match_win: &'static str,
    pub match_lose: &'static str,
    pub finish: &'static str,
    pub game_over: &'static str,
    pub rank: &'static str,
    pub new_best: &'static str,
    pub time_label: &'static str,
    pub hint_next_stage: &'static str,
    pub hint_play_again: &'static str,
    // Stat / HUD labels
    pub score: &'static str,
    pub level: &'static str,
    pub lines: &'static str,
    pub max_combo: &'static str,
    pub rounds: &'static str,
    pub attack: &'static str,
    pub tetris: &'static str,
    pub tspin: &'static str,
    pub combo: &'static str,
    pub pc: &'static str,
    pub pieces: &'static str,
    pub pps: &'static str,
    pub hold: &'static str,
    pub next: &'static str,
    pub speed: &'static str,
    pub atk_in: &'static str,
    pub time: &'static str,
    pub left: &'static str,
    pub zone_gauge: &'static str,
    pub you: &'static str,
    pub cpu: &'static str,
    pub round: &'static str,
    /// `{m}` = multiplier.
    pub atk_mult: &'static str,
    /// Match HUD title for custom matches.
    pub custom_prefix: &'static str,
}

pub const EN: Strings = Strings {
    solo: "SOLO",
    vs_cpu: "VS CPU",
    zone_battle: "ZONE BATTLE",
    custom_match: "CUSTOM MATCH",
    settings: "SETTINGS",
    quit: "QUIT",
    marathon: "MARATHON",
    sprint: "SPRINT",
    dig: "DIG",
    select_stage: "SELECT STAGE",
    title_hint: "Up/Down: select    ENTER: confirm",
    list_hint: "Up/Down: select    ENTER: start    ESC: back",
    grid_hint: "Arrows: select    ENTER: start    ESC: back",
    desc_solo: "SINGLE-PLAYER MODES: MARATHON / SPRINT / DIG",
    desc_vs: "BATTLE CPU RIVALS THROUGH 30 STAGES",
    desc_zone: "VS WITH A SUPER MOVE: CANCEL OR DIG GARBAGE TO CHARGE, STOP TIME, STRIKE BIG",
    desc_custom: "BUILD YOUR OWN VS MATCH: CPU SKILL & STYLE, RULES AND HANDICAPS",
    desc_settings: "KEY BINDINGS, HANDLING AND VOLUME",
    desc_quit: "EXIT THE GAME",
    desc_marathon: "CLASSIC ENDLESS MODE - GRAVITY RISES EVERY 10 LINES",
    desc_sprint: "RACE: CLEAR {n} LINES AS FAST AS YOU CAN",
    desc_dig: "RACE: DIG THROUGH {n} ROWS OF GARBAGE",
    best_label: "BEST",
    stage_prefix: "STAGE",
    zone_prefix: "ZONE",
    type_label: "TYPE",
    first_to: "FIRST TO {n}",
    locked: "LOCKED",
    not_cleared: "NOT CLEARED",
    best_colon: "BEST:",
    act_move_left: "Move Left",
    act_move_right: "Move Right",
    act_soft_drop: "Soft Drop",
    act_hard_drop: "Hard Drop",
    act_rotate_cw: "Rotate CW",
    act_rotate_ccw: "Rotate CCW",
    act_hold: "Hold",
    act_zone: "Zone",
    act_pause: "Pause",
    press_key: "PRESS KEY...",
    press_button: "PRESS BUTTON...",
    sdf_setting: "Soft Drop",
    master_vol: "Master Vol",
    bgm_vol: "BGM Vol",
    sfx_vol: "SFX Vol",
    vsync_off: "OFF (fast)",
    fullscreen: "Fullscreen",
    language: "Language",
    back: "BACK",
    settings_hint: "ENTER: rebind    Left/Right: adjust    ESC: back",
    pad_hint: "GAMEPAD: menus are fixed (D-PAD/STICK move, A confirm, B back); the stick always moves the piece (up = hard drop)",
    sec_keys: "- KEY BINDINGS -",
    sec_pad: "- GAMEPAD BUTTONS -",
    sec_handling: "- HANDLING -",
    sec_audio: "- AUDIO -",
    sec_display: "- DISPLAY & LANGUAGE -",
    cm_cpu_level: "CPU Level",
    cm_style: "CPU Style",
    cm_first_to: "First To",
    cm_zone: "Zone Gauge",
    cm_margin: "Margin Time",
    cm_speed: "Gravity",
    cm_speed_auto: "AUTO (RAMP)",
    cm_player_atk: "Your Attack",
    cm_cpu_atk: "CPU Attack",
    cm_garbage: "Start Garbage",
    cm_start: "START MATCH",
    cm_hint: "Left/Right: change    ENTER: start    ESC: back",
    sec_cm_cpu: "- OPPONENT -",
    sec_cm_rules: "- RULES -",
    paused: "PAUSED",
    pause_hint: "{key}: resume    R: restart    Q: quit to title",
    round_win: "ROUND WIN!",
    round_lost: "ROUND LOST",
    vs_score: "YOU {p} - {c} CPU",
    next_round: "next round...",
    stage_clear: "{stage} CLEAR!",
    stage_failed: "{stage} FAILED...",
    match_win: "YOU WIN!",
    match_lose: "YOU LOSE...",
    finish: "FINISH!",
    game_over: "GAME OVER",
    rank: "RANK",
    new_best: "NEW BEST!",
    time_label: "TIME",
    hint_next_stage: "ENTER: next stage    R: replay    Q: title",
    hint_play_again: "R / ENTER: play again    Q: title",
    score: "SCORE",
    level: "LEVEL",
    lines: "LINES",
    max_combo: "MAX COMBO",
    rounds: "ROUNDS",
    attack: "ATTACK",
    tetris: "TETRIS",
    tspin: "T-SPIN",
    combo: "COMBO",
    pc: "PC",
    pieces: "PIECES",
    pps: "PPS",
    hold: "HOLD",
    next: "NEXT",
    speed: "SPEED",
    atk_in: "ATK IN",
    time: "TIME",
    left: "LEFT",
    zone_gauge: "ZONE",
    you: "YOU",
    cpu: "CPU",
    round: "ROUND",
    atk_mult: "ATK x{m}",
    custom_prefix: "CUSTOM",
};

pub const JA: Strings = Strings {
    solo: "ソロ",
    vs_cpu: "VS CPU",
    zone_battle: "ゾーンバトル",
    custom_match: "カスタムマッチ",
    settings: "設定",
    quit: "終了",
    marathon: "マラソン",
    sprint: "スプリント",
    dig: "ディグ",
    select_stage: "ステージ選択",
    title_hint: "↑↓: 選択    ENTER: 決定",
    list_hint: "↑↓: 選択    ENTER: 開始    ESC: 戻る",
    grid_hint: "矢印: 選択    ENTER: 開始    ESC: 戻る",
    desc_solo: "1人用モード: マラソン / スプリント / ディグ",
    desc_vs: "全30ステージのCPU対戦",
    desc_zone: "必殺技つき対戦: ガベージの相殺や掘りでゲージを溜め、時を止めて一撃",
    desc_custom: "ルールを自由に設定して対戦: CPUの強さ・タイプ・ハンデなど",
    desc_settings: "キー設定・操作感・音量",
    desc_quit: "ゲームを終了",
    desc_marathon: "定番のエンドレス。10ラインごとに落下が速くなる",
    desc_sprint: "{n}ライン消去のタイムアタック",
    desc_dig: "ガベージ{n}段を掘り切るタイムアタック",
    best_label: "ベスト",
    stage_prefix: "ステージ",
    zone_prefix: "ZONE",
    type_label: "タイプ",
    first_to: "{n}本先取",
    locked: "未解放",
    not_cleared: "未クリア",
    best_colon: "ベスト:",
    act_move_left: "左移動",
    act_move_right: "右移動",
    act_soft_drop: "ソフトドロップ",
    act_hard_drop: "ハードドロップ",
    act_rotate_cw: "右回転",
    act_rotate_ccw: "左回転",
    act_hold: "ホールド",
    act_zone: "ゾーン",
    act_pause: "ポーズ",
    press_key: "キーを入力...",
    press_button: "ボタンを入力...",
    sdf_setting: "SD倍率",
    master_vol: "主音量",
    bgm_vol: "BGM音量",
    sfx_vol: "効果音量",
    vsync_off: "OFF (低遅延)",
    fullscreen: "フルスクリーン",
    language: "言語",
    back: "戻る",
    settings_hint: "ENTER: キー変更    ←→: 調整    ESC: 戻る",
    pad_hint: "パッド: メニューは固定(十字/スティック移動・A決定・B戻る)。スティックは常に移動(上=ハードドロップ)",
    sec_keys: "- キー設定 -",
    sec_pad: "- パッドボタン -",
    sec_handling: "- 操作感 -",
    sec_audio: "- サウンド -",
    sec_display: "- 表示・言語 -",
    cm_cpu_level: "CPUレベル",
    cm_style: "CPUタイプ",
    cm_first_to: "先取本数",
    cm_zone: "ゾーンゲージ",
    cm_margin: "マージンタイム",
    cm_speed: "落下速度",
    cm_speed_auto: "自動(加速)",
    cm_player_atk: "自分の攻撃力",
    cm_cpu_atk: "CPUの攻撃力",
    cm_garbage: "初期ガベージ",
    cm_start: "対戦開始",
    cm_hint: "←→: 変更    ENTER: 開始    ESC: 戻る",
    sec_cm_cpu: "- 対戦相手 -",
    sec_cm_rules: "- ルール -",
    paused: "ポーズ中",
    pause_hint: "{key}: 再開    R: リスタート    Q: タイトルへ",
    round_win: "ラウンド勝利!",
    round_lost: "ラウンド敗北",
    vs_score: "あなた {p} - {c} CPU",
    next_round: "次のラウンド...",
    stage_clear: "{stage} クリア!",
    stage_failed: "{stage} 敗北...",
    match_win: "勝利!",
    match_lose: "敗北...",
    finish: "フィニッシュ!",
    game_over: "ゲームオーバー",
    rank: "ランク",
    new_best: "自己ベスト更新!",
    time_label: "タイム",
    hint_next_stage: "ENTER: 次のステージ    R: 再戦    Q: タイトル",
    hint_play_again: "R / ENTER: もう一度    Q: タイトル",
    score: "スコア",
    level: "レベル",
    lines: "ライン",
    max_combo: "最大コンボ",
    rounds: "ラウンド",
    attack: "攻撃",
    tetris: "テトリス",
    tspin: "Tスピン",
    combo: "コンボ",
    pc: "パフェ",
    pieces: "ピース",
    pps: "PPS",
    hold: "ホールド",
    next: "ネクスト",
    speed: "速度",
    atk_in: "被弾",
    time: "タイム",
    left: "残り",
    zone_gauge: "ゾーン",
    you: "あなた",
    cpu: "CPU",
    round: "ラウンド",
    atk_mult: "攻撃 x{m}",
    custom_prefix: "カスタム",
};
