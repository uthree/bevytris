//! Audio: sound effects come from Juhani Junkala's CC0 "512 Sound Effects
//! (8-bit style)" collection (assets/sfx, peak-normalized; see
//! assets/CREDITS.md), BGM streams from his CC0 "Retro Game Music Pack".
//! Combo sounds are one coin sample pitched up a pentatonic step per combo.

use bevy::audio::{AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rand::seq::IndexedRandom;
use std::collections::HashMap;

use crate::config::GameSettings;
use crate::session::{GameSession, HumanControlled, SessionResult};
use crate::state::{AppState, PlayState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sfx {
    Move,
    Rotate,
    RotateFail,
    SoftDropTick,
    HardDrop,
    Lock,
    Hold,
    HoldFail,
    /// Clear(4) is the TETRIS arpeggio; 1-3 play its first bell alone.
    Clear(u32),
    /// Sparkle for spins that clear no lines.
    TSpin,
    /// Jingle phrase for T-spins that clear lines.
    TSpinClear,
    PerfectClear,
    /// The zone gauge just filled: a bright two-note bell chime.
    ZoneReady,
    /// Zone activation: a deep descending sub-bass boom.
    ZoneBoom,
    B2b,
    /// Coin chime pitched up a pentatonic step per combo count.
    Combo(u32),
    LevelUp,
    GarbageWarn,
    /// Garbage rows hit the board; bigger waves land louder and deeper.
    GarbageRise(u32),
    Countdown,
    Go,
    GameOver,
    /// Deep boom layered under GameOver when the whole match is lost.
    Defeat,
    Win,
    MenuMove,
    MenuSelect,
    MenuBack,
}

/// Semitone ladder for combo chimes (major pentatonic, two octaves):
/// combo 1 plays the root, combo 11+ caps at +2 octaves.
const COMBO_SEMITONES: [f32; 11] = [
    0.0, 2.0, 4.0, 7.0, 9.0, 12.0, 14.0, 16.0, 19.0, 21.0, 24.0,
];

#[derive(Resource)]
pub struct SfxBank {
    move_tick: Handle<AudioSource>,
    rotate: Handle<AudioSource>,
    rotate_fail: Handle<AudioSource>,
    soft_drop: Handle<AudioSource>,
    hard_drop: Handle<AudioSource>,
    lock: Handle<AudioSource>,
    hold: Handle<AudioSource>,
    hold_fail: Handle<AudioSource>,
    /// The tetris arpeggio's first bell note isolated, shared by all 1-3
    /// line clears — a modest single ding in the same voice as the fanfare.
    clear_note: Handle<AudioSource>,
    tetris: Handle<AudioSource>,
    tspin: Handle<AudioSource>,
    tspin_clear: Handle<AudioSource>,
    perfect: Handle<AudioSource>,
    zone_ready: Handle<AudioSource>,
    zone_boom: Handle<AudioSource>,
    danger_alarm: Handle<AudioSource>,
    b2b: Handle<AudioSource>,
    combo: Handle<AudioSource>,
    level_up: Handle<AudioSource>,
    garbage_warn: Handle<AudioSource>,
    garbage_rise: Handle<AudioSource>,
    countdown: Handle<AudioSource>,
    go: Handle<AudioSource>,
    game_over: Handle<AudioSource>,
    defeat: Handle<AudioSource>,
    win: Handle<AudioSource>,
    menu_move: Handle<AudioSource>,
    menu_select: Handle<AudioSource>,
    menu_back: Handle<AudioSource>,
}

impl SfxBank {
    /// Looping low-health alarm for the danger (pinch) presentation.
    pub fn danger_alarm(&self) -> Handle<AudioSource> {
        self.danger_alarm.clone()
    }

    /// Handle, base gain and playback speed for one effect. The samples on
    /// disk are peak-normalized, so base gains re-balance them (movement
    /// ticks stay subtle, rewards hit hard); speed != 1.0 pitch-shifts.
    fn params(&self, sfx: Sfx) -> (Handle<AudioSource>, f32, f32) {
        match sfx {
            // Action sounds are noise-based and sit well below the melodic
            // reward sounds — the loudness contrast is intentional.
            Sfx::Move => (self.move_tick.clone(), 0.4, 1.0),
            Sfx::Rotate => (self.rotate.clone(), 0.42, 1.0),
            Sfx::RotateFail => (self.rotate_fail.clone(), 0.45, 1.0),
            Sfx::SoftDropTick => (self.soft_drop.clone(), 0.28, 1.0),
            Sfx::HardDrop => (self.hard_drop.clone(), 0.55, 1.0),
            Sfx::Lock => (self.lock.clone(), 0.5, 1.0),
            Sfx::Hold => (self.hold.clone(), 0.5, 1.0),
            Sfx::HoldFail => (self.hold_fail.clone(), 0.5, 1.0),
            Sfx::Clear(n) => match n.clamp(1, 4) {
                // 1-3 share the fanfare's single opening bell; the count
                // is already told by score/combo cues.
                1..=3 => (self.clear_note.clone(), 0.9, 1.0),
                _ => (self.tetris.clone(), 1.0, 1.0),
            },
            Sfx::TSpin => (self.tspin.clone(), 0.9, 1.0),
            Sfx::TSpinClear => (self.tspin_clear.clone(), 1.0, 1.0),
            Sfx::PerfectClear => (self.perfect.clone(), 1.0, 1.0),
            Sfx::ZoneReady => (self.zone_ready.clone(), 0.95, 1.0),
            // The boom is synthesized dense (rms -8 dB), so it sits a bit
            // lower to leave room for the rest of the mix.
            Sfx::ZoneBoom => (self.zone_boom.clone(), 0.85, 1.0),
            Sfx::B2b => (self.b2b.clone(), 0.6, 1.0),
            Sfx::Combo(n) => {
                let i = (n.max(1) as usize - 1).min(COMBO_SEMITONES.len() - 1);
                let speed = 2f32.powf(COMBO_SEMITONES[i] / 12.0);
                let gain = (0.85 + 0.05 * n as f32).min(1.2);
                (self.combo.clone(), gain, speed)
            }
            Sfx::LevelUp => (self.level_up.clone(), 0.9, 1.0),
            Sfx::GarbageWarn => (self.garbage_warn.clone(), 0.7, 1.0),
            Sfx::GarbageRise(rows) => {
                // 1 row: a light thump. 8 rows: a loud, pitched-down slam.
                let r = rows.clamp(1, 8) as f32;
                let gain = (0.55 + 0.11 * r).min(1.4);
                let speed = (1.12 - 0.06 * r).max(0.62);
                (self.garbage_rise.clone(), gain, speed)
            }
            Sfx::Countdown => (self.countdown.clone(), 0.7, 1.0),
            Sfx::Go => (self.go.clone(), 0.9, 1.0),
            Sfx::GameOver => (self.game_over.clone(), 0.9, 1.0),
            Sfx::Defeat => (self.defeat.clone(), 1.0, 1.0),
            Sfx::Win => (self.win.clone(), 1.0, 1.0),
            Sfx::MenuMove => (self.menu_move.clone(), 0.4, 1.0),
            Sfx::MenuSelect => (self.menu_select.clone(), 0.7, 1.0),
            Sfx::MenuBack => (self.menu_back.clone(), 0.5, 1.0),
        }
    }
}

/// Maps each dry sample to its lowpassed twin in assets/sfx/muffled
/// (pre-rendered offline: 2x biquad LP at 750 Hz + soft limiter). While
/// the player's zone runs, every effect plays the muffled variant.
#[derive(Resource)]
pub struct MuffledSfx(HashMap<AssetId<AudioSource>, Handle<AudioSource>>);

fn build_sfx_bank(asset_server: &AssetServer) -> (SfxBank, MuffledSfx) {
    let mut muffled = HashMap::new();
    let mut load = |file: &str| -> Handle<AudioSource> {
        let dry: Handle<AudioSource> = asset_server.load(format!("sfx/{file}"));
        muffled.insert(dry.id(), asset_server.load(format!("sfx/muffled/{file}")));
        dry
    };
    let bank = SfxBank {
        move_tick: load("move.wav"),
        rotate: load("rotate.wav"),
        rotate_fail: load("rotate_fail.wav"),
        soft_drop: load("soft_drop.wav"),
        hard_drop: load("hard_drop.wav"),
        lock: load("lock.wav"),
        hold: load("hold.wav"),
        hold_fail: load("hold_fail.wav"),
        clear_note: load("clear_note.wav"),
        tetris: load("phrase_tetris.wav"),
        tspin: load("tspin.wav"),
        tspin_clear: load("phrase_tspin.wav"),
        perfect: load("phrase_perfect.wav"),
        zone_ready: load("zone_ready.wav"),
        zone_boom: load("zone_boom.wav"),
        danger_alarm: load("danger_alarm.wav"),
        b2b: load("b2b.wav"),
        combo: load("combo.wav"),
        level_up: load("level_up.wav"),
        garbage_warn: load("garbage_warn.wav"),
        garbage_rise: load("garbage_rise.wav"),
        countdown: load("countdown.wav"),
        go: load("go.wav"),
        game_over: load("game_over.wav"),
        defeat: load("defeat.wav"),
        win: load("win.wav"),
        menu_move: load("menu_move.wav"),
        menu_select: load("menu_select.wav"),
        menu_back: load("menu_back.wav"),
    };
    (bank, MuffledSfx(muffled))
}

/// True while the human player's zone super move is running — the cue
/// for the whole soundscape to go underwater.
pub fn player_zone_active(
    sessions: &Query<&GameSession, With<HumanControlled>>,
) -> bool {
    sessions.iter().any(|s| s.game.zone_active())
}

// ---------------------------------------------------------------------------
// Bevy plumbing
// ---------------------------------------------------------------------------

/// BGM streamed from CC0 assets (Juhani Junkala's "Retro Game Music Pack"
/// and "4 Chiptunes (Adventure)", SketchyLogic's "NES Shooter Music"; see
/// assets/CREDITS.md). In-game tracks are picked at random per match.
#[derive(Resource)]
pub struct BgmBank {
    title: BgmTrack,
    stage_select: BgmTrack,
    game: Vec<BgmTrack>,
    victory: Handle<AudioSource>,
}

/// A music track plus what the corner toast should call it.
#[derive(Clone)]
struct BgmTrack {
    name: &'static str,
    handle: Handle<AudioSource>,
}

/// Marker for the currently playing BGM entity.
#[derive(Component)]
pub struct Bgm;

/// Which track is currently playing, so re-entering a state (Settings →
/// Title, or a match restart) doesn't restart the music from bar one.
#[derive(Resource, Default)]
struct CurrentBgm(Option<Handle<AudioSource>>);

#[derive(Message)]
pub struct PlaySfx {
    pub sfx: Sfx,
    /// Extra gain for distance/importance scaling (1.0 = normal).
    pub gain: f32,
    /// Exempt from the zone lowpass: the player's own board stays crisp
    /// while the rest of the world goes underwater.
    pub crisp: bool,
}

impl PlaySfx {
    pub fn new(sfx: Sfx) -> Self {
        Self {
            sfx,
            gain: 1.0,
            crisp: false,
        }
    }
    pub fn quiet(sfx: Sfx) -> Self {
        Self {
            sfx,
            gain: 0.4,
            crisp: false,
        }
    }
}

pub struct GameAudioPlugin;

impl Plugin for GameAudioPlugin {
    fn build(&self, app: &mut App) {
        // The initial state's OnEnter runs in the first StateTransition
        // schedule, which bevy_state places BEFORE PreStartup — so the banks
        // must already exist when the app starts or the title BGM misses
        // launch. Build them here, at plugin-build time (this plugin must be
        // added after DefaultPlugins, which registers the AssetServer).
        let asset_server = app.world().resource::<AssetServer>().clone();
        let (bank, muffled) = build_sfx_bank(&asset_server);
        app.insert_resource(bank);
        app.insert_resource(muffled);
        app.init_resource::<BgmDuck>();
        let track = |name: &'static str, file: &str| BgmTrack {
            name,
            handle: asset_server.load(format!("music/{file}")),
        };
        app.insert_resource(BgmBank {
            title: track("Title Screen - Juhani Junkala", "title.ogg"),
            stage_select: track("Stage Select - Juhani Junkala", "stage_select.ogg"),
            game: vec![
                track("Level 1 - Juhani Junkala", "level1.ogg"),
                track("Level 2 - Juhani Junkala", "level2.ogg"),
                track("Level 3 - Juhani Junkala", "level3.ogg"),
                track("Stage 1 - Juhani Junkala", "stage1.ogg"),
                track("Stage 2 - Juhani Junkala", "stage2.ogg"),
                track("Boss Fight - Juhani Junkala", "boss_fight.ogg"),
                track("Venus - SketchyLogic", "venus.wav"),
                track("Map - SketchyLogic", "map.wav"),
                track("Mars - SketchyLogic", "mars.wav"),
                track("Mercury - SketchyLogic", "mercury.wav"),
            ],
            victory: asset_server.load("music/ending.ogg"),
        });

        app.add_message::<PlaySfx>()
            .init_resource::<CurrentBgm>()
            .add_systems(Update, (play_sfx, apply_bgm_volume, update_bgm_toasts))
            .add_systems(OnEnter(AppState::Title), start_title_bgm)
            .add_systems(OnEnter(AppState::SoloSelect), start_stage_select_bgm)
            .add_systems(OnEnter(AppState::ZoneSelect), start_stage_select_bgm)
            .add_systems(OnEnter(AppState::StageSelect), start_stage_select_bgm)
            .add_systems(OnEnter(AppState::CustomSetup), start_stage_select_bgm)
            .add_systems(OnEnter(AppState::Playing), start_game_bgm)
            .add_systems(OnEnter(PlayState::Paused), pause_bgm)
            .add_systems(OnExit(PlayState::Paused), resume_bgm)
            .add_systems(OnEnter(PlayState::Finished), (pause_bgm, play_victory_jingle));
    }
}

fn play_sfx(
    mut commands: Commands,
    mut reader: MessageReader<PlaySfx>,
    bank: Res<SfxBank>,
    muffled: Res<MuffledSfx>,
    settings: Res<GameSettings>,
    sessions: Query<&GameSession, With<HumanControlled>>,
) {
    let base = settings.sfx_linear();
    // Inside the player's zone the world goes underwater: every effect
    // swaps to its pre-rendered lowpassed variant.
    let muffle = player_zone_active(&sessions);
    for msg in reader.read() {
        let (mut handle, gain, speed) = bank.params(msg.sfx);
        if muffle && !msg.crisp {
            if let Some(wet) = muffled.0.get(&handle.id()) {
                handle = wet.clone();
            }
        }
        let volume = base * gain * msg.gain;
        if volume <= 0.001 {
            continue;
        }
        commands.spawn((
            AudioPlayer::new(handle),
            PlaybackSettings::DESPAWN
                .with_volume(Volume::Linear(volume))
                .with_speed(speed),
        ));
    }
}

fn start_title_bgm(
    commands: Commands,
    bank: Res<BgmBank>,
    settings: Res<GameSettings>,
    current: ResMut<CurrentBgm>,
    sinks: Query<&AudioSink, With<Bgm>>,
) {
    swap_bgm(commands, &bank.title, &settings, current, sinks);
}

fn start_stage_select_bgm(
    commands: Commands,
    bank: Res<BgmBank>,
    settings: Res<GameSettings>,
    current: ResMut<CurrentBgm>,
    sinks: Query<&AudioSink, With<Bgm>>,
) {
    swap_bgm(commands, &bank.stage_select, &settings, current, sinks);
}

fn start_game_bgm(
    commands: Commands,
    bank: Res<BgmBank>,
    settings: Res<GameSettings>,
    current: ResMut<CurrentBgm>,
    sinks: Query<&AudioSink, With<Bgm>>,
) {
    let track = bank
        .game
        .choose(&mut rand::rng())
        .expect("game BGM list is never empty")
        .clone();
    swap_bgm(commands, &track, &settings, current, sinks);
}

/// On a VS win or a finished race, celebrate with the (CC0) ending jingle.
fn play_victory_jingle(
    mut commands: Commands,
    bank: Res<BgmBank>,
    settings: Res<GameSettings>,
    result: Option<Res<SessionResult>>,
) {
    if matches!(
        result.as_deref(),
        Some(SessionResult::VsWin { winner: 0 } | SessionResult::RaceDone)
    ) {
        commands.spawn((
            AudioPlayer::new(bank.victory.clone()),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(settings.bgm_linear())),
            // Cut the jingle if the player leaves the result screen early.
            DespawnOnExit(PlayState::Finished),
        ));
    }
}

fn swap_bgm(
    mut commands: Commands,
    track: &BgmTrack,
    settings: &GameSettings,
    mut current: ResMut<CurrentBgm>,
    sinks: Query<&AudioSink, With<Bgm>>,
) {
    if current.0.as_ref() == Some(&track.handle) {
        // Same track: keep playing where it is (it may have been paused by
        // a result screen — a match restart must un-pause it).
        for sink in &sinks {
            sink.play();
        }
        return;
    }
    current.0 = Some(track.handle.clone());
    debug!("bgm: starting {:?}", track.handle.path());
    commands.queue(|world: &mut World| {
        let old: Vec<Entity> = world
            .query_filtered::<Entity, Or<(With<Bgm>, With<BgmToast>)>>()
            .iter(world)
            .collect();
        for e in old {
            world.entity_mut(e).despawn();
        }
    });
    commands.spawn((
        AudioPlayer::new(track.handle.clone()),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(settings.bgm_linear())),
        Bgm,
    ));
    spawn_bgm_toast(&mut commands, track.name);
}

// ---------------------------------------------------------------------------
// "Now playing" toast
// ---------------------------------------------------------------------------

/// Corner note naming the track whenever the BGM actually changes.
#[derive(Component)]
struct BgmToast {
    life: f32,
}

const BGM_TOAST_LIFE: f32 = 4.5;

fn spawn_bgm_toast(commands: &mut Commands, name: &str) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(14.0),
            bottom: Val::Px(10.0),
            ..default()
        },
        Text::new(format!("♪ {name}")),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::srgba(0.72, 0.82, 1.0, 0.0)),
        GlobalZIndex(30),
        Pickable::IGNORE,
        BgmToast {
            life: BGM_TOAST_LIFE,
        },
    ));
}

fn update_bgm_toasts(
    time: Res<Time>,
    mut commands: Commands,
    mut toasts: Query<(Entity, &mut BgmToast, &mut TextColor)>,
) {
    for (entity, mut toast, mut color) in &mut toasts {
        toast.life -= time.delta_secs();
        if toast.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        // Fade in over 0.4 s, hold, fade out over the last 0.9 s.
        let elapsed = BGM_TOAST_LIFE - toast.life;
        let alpha = (elapsed / 0.4).min(toast.life / 0.9).clamp(0.0, 1.0);
        color.0.set_alpha(alpha * 0.85);
    }
}

/// Smoothed BGM duck factor: the music sinks behind glass while the
/// player's zone runs (we can't lowpass a live stream, so distance
/// stands in for muffling — the SFX carry the real filter).
#[derive(Resource)]
struct BgmDuck(f32);

impl Default for BgmDuck {
    fn default() -> Self {
        BgmDuck(1.0)
    }
}

/// Keep the BGM sink in sync with the settings sliders and the zone duck.
fn apply_bgm_volume(
    time: Res<Time>,
    settings: Res<GameSettings>,
    sessions: Query<&GameSession, With<HumanControlled>>,
    mut duck: ResMut<BgmDuck>,
    mut sinks: Query<&mut AudioSink, With<Bgm>>,
) {
    let target = if player_zone_active(&sessions) { 0.3 } else { 1.0 };
    let next = duck.0 + (target - duck.0) * (5.0 * time.delta_secs()).min(1.0);
    if (next - duck.0).abs() > 0.0005 || settings.is_changed() {
        duck.0 = next;
        for mut sink in &mut sinks {
            sink.set_volume(Volume::Linear(settings.bgm_linear() * duck.0));
        }
    }
}

fn pause_bgm(sinks: Query<&AudioSink, With<Bgm>>) {
    for sink in &sinks {
        sink.pause();
    }
}

fn resume_bgm(sinks: Query<&AudioSink, With<Bgm>>) {
    for sink in &sinks {
        sink.play();
    }
}
