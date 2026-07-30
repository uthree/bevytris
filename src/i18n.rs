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

use crate::config::{Action, GameSettings, UiAction};

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
    *DETECTED.get_or_init(|| match sys_locale::get_locale() {
        Some(tag) if tag.to_ascii_lowercase().starts_with("ja") => Lang::Japanese,
        _ => Lang::English,
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

pub fn ui_action_label(s: &Strings, action: UiAction) -> &'static str {
    match action {
        UiAction::Up => s.ui_up,
        UiAction::Down => s.ui_down,
        UiAction::Left => s.ui_left,
        UiAction::Right => s.ui_right,
        UiAction::Confirm => s.ui_confirm,
        UiAction::Back => s.ui_back,
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
    /// Listening room: pick a seed and a preset, watch the piano roll.
    pub jukebox: &'static str,
    pub jukebox_hint: &'static str,
    pub jukebox_now: &'static str,
    pub jukebox_auto: &'static str,
    pub jb_seed: &'static str,
    pub jb_preset: &'static str,
    pub jb_meter: &'static str,
    pub jb_kit: &'static str,
    /// Listening room: how stepwise the melodies are.
    pub jb_smooth: &'static str,
    /// Listening room: which instrument plays the melody.
    pub jb_lead: &'static str,
    pub jb_intensity: &'static str,
    pub jb_zone: &'static str,
    pub settings: &'static str,
    pub quit: &'static str,
    pub marathon: &'static str,
    pub sprint: &'static str,
    pub zen: &'static str,
    /// Zen HUD / footer label for the lifetime level.
    pub zen_level: &'static str,
    /// Zen HUD label: how many times this run has been wiped.
    pub zen_resets: &'static str,
    /// HUD gravity readout once the ramp has topped out, in place of a
    /// number that has stopped meaning anything.
    pub speed_max: &'static str,
    pub select_stage: &'static str,
    /// Character picker: screen title, and what to do when the roster is
    /// empty. A character's own name and flavour text come from its
    /// `metadata.json` and are deliberately not translated here — they
    /// belong to whoever authored the pack.
    pub select_character: &'static str,
    /// Second pass of the picker, when a custom match also needs a face
    /// for the other board.
    pub select_character_p2: &'static str,
    pub select_character_cpu: &'static str,
    pub no_characters: &'static str,
    pub characters_hint: &'static str,
    pub character_problems: &'static str,
    pub title_hint: &'static str,
    pub list_hint: &'static str,
    pub grid_hint: &'static str,
    // Menu footers
    pub desc_solo: &'static str,
    pub desc_vs: &'static str,
    pub desc_zone: &'static str,
    pub desc_custom: &'static str,
    pub desc_jukebox: &'static str,
    pub desc_settings: &'static str,
    pub desc_quit: &'static str,
    pub desc_marathon: &'static str,
    /// `{n}` = goal lines.
    pub desc_sprint: &'static str,
    pub desc_zen: &'static str,
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
    pub ui_up: &'static str,
    pub ui_down: &'static str,
    pub ui_left: &'static str,
    pub ui_right: &'static str,
    pub ui_confirm: &'static str,
    pub ui_back: &'static str,
    pub sdf_setting: &'static str,
    pub master_vol: &'static str,
    pub bgm_vol: &'static str,
    pub sfx_vol: &'static str,
    pub voice_vol: &'static str,
    pub vsync_off: &'static str,
    pub fullscreen: &'static str,
    pub language: &'static str,
    pub back: &'static str,
    pub settings_hint: &'static str,
    pub pad_hint: &'static str,
    // Settings section headings
    pub sec_keys: &'static str,
    /// Player 2's key set (local versus).
    pub sec_keys2: &'static str,
    /// Appended to a player 2 key that player 1 also uses.
    pub key_clash: &'static str,
    pub sec_pad: &'static str,
    pub sec_menu: &'static str,
    pub sec_handling: &'static str,
    pub sec_audio: &'static str,
    pub sec_display: &'static str,
    // Custom match setup rows
    pub cm_opponent: &'static str,
    pub cm_opp_cpu: &'static str,
    pub cm_opp_2p: &'static str,
    /// Shown for the CPU rows while a human holds the other board.
    pub cm_na: &'static str,
    pub cm_hold: &'static str,
    pub cm_previews: &'static str,
    pub cm_messiness: &'static str,
    /// Messiness 0: every attack arrives as one clean well.
    pub cm_mess_clean: &'static str,
    /// Attack-handicap labels while both boards are human.
    pub cm_p1_atk: &'static str,
    pub cm_p2_atk: &'static str,
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
    /// Local versus round result; `{p}` = "1P" / "2P".
    pub round_win_by: &'static str,
    /// Local versus match result; `{p}` = "1P" / "2P".
    pub match_win_by: &'static str,
    /// `{p}` / `{c}` = player / cpu wins.
    pub vs_score: &'static str,
    /// Same, with both seats human.
    pub vs_score_2p: &'static str,
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
    /// Board names in local versus.
    pub p1: &'static str,
    pub p2: &'static str,
    /// Marker where the hold box would be when the rules ban hold.
    pub hold_off: &'static str,
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
    jukebox: "MUSIC",
    jukebox_hint: "UP/DOWN: SELECT    LEFT/RIGHT: ADJUST    ENTER: NEW SEED    ESC: BACK",
    jukebox_now: "",
    jukebox_auto: "AUTO",
    jb_seed: "SEED",
    jb_preset: "PRESET",
    jb_meter: "METER",
    jb_kit: "DRUMS",
    jb_smooth: "MELODY",
    jb_lead: "LEAD",
    jb_intensity: "INTENSITY",
    jb_zone: "ZONE",
    settings: "SETTINGS",
    quit: "QUIT",
    marathon: "MARATHON",
    sprint: "SPRINT",
    zen: "ZEN",
    zen_level: "ZEN LV",
    zen_resets: "RESETS",
    speed_max: "MAX",
    select_stage: "SELECT STAGE",
    select_character: "SELECT CHARACTER",
    select_character_p2: "SELECT 2P CHARACTER",
    select_character_cpu: "SELECT CPU CHARACTER",
    no_characters: "NO CHARACTERS INSTALLED",
    characters_hint: "Drop a folder into assets/characters to add one    ENTER: continue    ESC: back",
    character_problems: "{n} CHARACTER PROBLEM(S) - SEE THE CONSOLE LOG",
    title_hint: "Up/Down: select    ENTER: confirm",
    list_hint: "Up/Down: select    ENTER: start    ESC: back",
    grid_hint: "Arrows: select    ENTER: start    ESC: back",
    desc_solo: "SINGLE-PLAYER MODES: MARATHON / SPRINT / ZEN",
    desc_vs: "BATTLE CPU RIVALS THROUGH 30 STAGES",
    desc_zone: "VS WITH A SUPER MOVE: CANCEL OR DIG GARBAGE TO CHARGE, STOP TIME, STRIKE BIG",
    desc_custom: "BUILD YOUR OWN VS MATCH: CPU SKILL & STYLE, RULES AND HANDICAPS",
    desc_jukebox: "LISTEN TO GENERATED MUSIC AND WATCH THE PIANO ROLL",
    desc_settings: "KEY BINDINGS, HANDLING AND VOLUME",
    desc_quit: "EXIT THE GAME",
    desc_marathon: "CLASSIC ENDLESS MODE - GRAVITY RISES EVERY 10 LINES",
    desc_sprint: "RACE: CLEAR {n} LINES AS FAST AS YOU CAN",
    desc_zen: "ENDLESS AND UNLOSABLE - GRAVITY CLIMBS TO A PEAK, TOPPING OUT ONLY WIPES THE FIELD",
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
    ui_up: "MENU UP",
    ui_down: "MENU DOWN",
    ui_left: "MENU LEFT",
    ui_right: "MENU RIGHT",
    ui_confirm: "CONFIRM",
    ui_back: "BACK",
    sdf_setting: "Soft Drop",
    master_vol: "Master Vol",
    bgm_vol: "BGM Vol",
    sfx_vol: "SFX Vol",
    voice_vol: "Voice Vol",
    vsync_off: "OFF (fast)",
    fullscreen: "Fullscreen",
    language: "Language",
    back: "BACK",
    settings_hint: "ENTER: rebind    Left/Right: adjust    ESC: back",
    pad_hint: "GAMEPAD: menu buttons are under MENU CONTROLS; the stick always navigates and always moves the piece (up = hard drop), and B always backs out",
    sec_keys: "- KEY BINDINGS -",
    sec_keys2: "- PLAYER 2 KEYS (LOCAL VERSUS) -",
    key_clash: " ! ALSO 1P",
    sec_pad: "- GAMEPAD BUTTONS -",
    sec_menu: "- MENU CONTROLS -",
    sec_handling: "- HANDLING -",
    sec_audio: "- AUDIO -",
    sec_display: "- DISPLAY & LANGUAGE -",
    cm_opponent: "Opponent",
    cm_opp_cpu: "CPU",
    cm_opp_2p: "PLAYER 2 (LOCAL)",
    cm_na: "-",
    cm_hold: "Hold",
    cm_previews: "Previews",
    cm_messiness: "Messiness",
    cm_mess_clean: "CLEAN (ONE WELL)",
    cm_p1_atk: "1P Attack",
    cm_p2_atk: "2P Attack",
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
    round_win_by: "{p} TAKES THE ROUND!",
    match_win_by: "{p} WINS!",
    vs_score: "YOU {p} - {c} CPU",
    vs_score_2p: "1P {p} - {c} 2P",
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
    p1: "1P",
    p2: "2P",
    hold_off: "NO HOLD",
    round: "ROUND",
    atk_mult: "ATK x{m}",
    custom_prefix: "CUSTOM",
};

pub const JA: Strings = Strings {
    solo: "ソロ",
    vs_cpu: "VS CPU",
    zone_battle: "ゾーンバトル",
    custom_match: "カスタムマッチ",
    jukebox: "ミュージック",
    jukebox_hint: "↑↓: 選択    ←→: 変更    ENTER: シード再抽選    ESC: 戻る",
    jukebox_now: "",
    jukebox_auto: "おまかせ",
    jb_seed: "シード",
    jb_preset: "プリセット",
    jb_meter: "拍子",
    jb_kit: "ドラム",
    jb_smooth: "メロディ",
    jb_lead: "リード",
    jb_intensity: "テンション",
    jb_zone: "ゾーン",
    settings: "設定",
    quit: "終了",
    marathon: "マラソン",
    sprint: "スプリント",
    zen: "ゼン",
    zen_level: "ZEN LV",
    zen_resets: "リセット",
    speed_max: "MAX",
    select_stage: "ステージ選択",
    select_character: "キャラクター選択",
    select_character_p2: "2Pのキャラクター選択",
    select_character_cpu: "CPUのキャラクター選択",
    no_characters: "キャラクターが入っていません",
    characters_hint: "assets/characters にフォルダを置くと追加できます    ENTER: 続行    ESC: 戻る",
    character_problems: "キャラクターに{n}件の問題あり - コンソールログを確認",
    title_hint: "↑↓: 選択    ENTER: 決定",
    list_hint: "↑↓: 選択    ENTER: 開始    ESC: 戻る",
    grid_hint: "矢印: 選択    ENTER: 開始    ESC: 戻る",
    desc_solo: "1人用モード: マラソン / スプリント / ゼン",
    desc_vs: "全30ステージのCPU対戦",
    desc_zone: "必殺技つき対戦: ガベージの相殺や掘りでゲージを溜め、時を止めて一撃",
    desc_custom: "ルールを自由に設定して対戦: CPUの強さ・タイプ・ハンデなど",
    desc_jukebox: "自動生成された曲を聴きながらピアノロールを眺める",
    desc_settings: "キー設定・操作感・音量",
    desc_quit: "ゲームを終了",
    desc_marathon: "定番のエンドレス。10ラインごとに落下が速くなる",
    desc_sprint: "{n}ライン消去のタイムアタック",
    desc_zen: "終わりも負けもない。速度は最大まで上がり、詰んでも盤面が消えるだけ",
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
    ui_up: "メニュー上",
    ui_down: "メニュー下",
    ui_left: "メニュー左",
    ui_right: "メニュー右",
    ui_confirm: "決定",
    ui_back: "戻る",
    sdf_setting: "SD倍率",
    master_vol: "主音量",
    bgm_vol: "BGM音量",
    sfx_vol: "効果音量",
    voice_vol: "ボイス音量",
    vsync_off: "OFF (低遅延)",
    fullscreen: "フルスクリーン",
    language: "言語",
    back: "戻る",
    settings_hint: "ENTER: キー変更    ←→: 調整    ESC: 戻る",
    pad_hint: "パッド: メニュー操作は[メニュー操作]で変更可。スティックは常にカーソル移動と操作(上=ハードドロップ)、Bは常に戻る",
    sec_keys: "- キー設定 -",
    sec_keys2: "- 2Pキー設定 (ローカル対戦) -",
    key_clash: " ! 1Pと重複",
    sec_pad: "- パッドボタン -",
    sec_menu: "- メニュー操作 -",
    sec_handling: "- 操作感 -",
    sec_audio: "- サウンド -",
    sec_display: "- 表示・言語 -",
    cm_opponent: "対戦相手",
    cm_opp_cpu: "CPU",
    cm_opp_2p: "2P (ローカル対戦)",
    cm_na: "-",
    cm_hold: "ホールド",
    cm_previews: "ネクスト数",
    cm_messiness: "穴のばらけ",
    cm_mess_clean: "揃える(1列)",
    cm_p1_atk: "1Pの攻撃力",
    cm_p2_atk: "2Pの攻撃力",
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
    round_win_by: "{p} がラウンド獲得!",
    match_win_by: "{p} の勝利!",
    vs_score: "あなた {p} - {c} CPU",
    vs_score_2p: "1P {p} - {c} 2P",
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
    p1: "1P",
    p2: "2P",
    hold_off: "ホールド禁止",
    round: "ラウンド",
    atk_mult: "攻撃 x{m}",
    custom_prefix: "カスタム",
};
