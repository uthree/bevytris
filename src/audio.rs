//! Audio: sound effects come from Juhani Junkala's CC0 "512 Sound Effects
//! (8-bit style)" collection (assets/sfx, peak-normalized; see
//! assets/CREDITS.md), BGM streams from his CC0 "Retro Game Music Pack".
//! Combo sounds are one coin sample pitched up a pentatonic step per combo.

use bevy::audio::{AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rand::seq::IndexedRandom;

use crate::config::GameSettings;
use crate::session::SessionResult;
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
    Clear(u32),
    TSpin,
    PerfectClear,
    B2b,
    /// Coin chime pitched up a pentatonic step per combo count.
    Combo(u32),
    LevelUp,
    GarbageWarn,
    GarbageRise,
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
    clears: [Handle<AudioSource>; 4],
    tspin: Handle<AudioSource>,
    perfect: Handle<AudioSource>,
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
            Sfx::HardDrop => (self.hard_drop.clone(), 0.8, 1.0),
            Sfx::Lock => (self.lock.clone(), 0.5, 1.0),
            Sfx::Hold => (self.hold.clone(), 0.5, 1.0),
            Sfx::HoldFail => (self.hold_fail.clone(), 0.5, 1.0),
            Sfx::Clear(n) => {
                let i = (n.clamp(1, 4) - 1) as usize;
                (self.clears[i].clone(), 0.85 + i as f32 * 0.05, 1.0)
            }
            Sfx::TSpin => (self.tspin.clone(), 0.9, 1.0),
            Sfx::PerfectClear => (self.perfect.clone(), 1.0, 1.0),
            Sfx::B2b => (self.b2b.clone(), 0.6, 1.0),
            Sfx::Combo(n) => {
                let i = (n.max(1) as usize - 1).min(COMBO_SEMITONES.len() - 1);
                let speed = 2f32.powf(COMBO_SEMITONES[i] / 12.0);
                let gain = (0.85 + 0.05 * n as f32).min(1.2);
                (self.combo.clone(), gain, speed)
            }
            Sfx::LevelUp => (self.level_up.clone(), 0.9, 1.0),
            Sfx::GarbageWarn => (self.garbage_warn.clone(), 0.7, 1.0),
            Sfx::GarbageRise => (self.garbage_rise.clone(), 0.85, 1.0),
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

fn build_sfx_bank(asset_server: &AssetServer) -> SfxBank {
    SfxBank {
        move_tick: asset_server.load("sfx/move.wav"),
        rotate: asset_server.load("sfx/rotate.wav"),
        rotate_fail: asset_server.load("sfx/rotate_fail.wav"),
        soft_drop: asset_server.load("sfx/soft_drop.wav"),
        hard_drop: asset_server.load("sfx/hard_drop.wav"),
        lock: asset_server.load("sfx/lock.wav"),
        hold: asset_server.load("sfx/hold.wav"),
        hold_fail: asset_server.load("sfx/hold_fail.wav"),
        clears: [
            asset_server.load("sfx/clear1.wav"),
            asset_server.load("sfx/clear2.wav"),
            asset_server.load("sfx/clear3.wav"),
            asset_server.load("sfx/clear4.wav"),
        ],
        tspin: asset_server.load("sfx/tspin.wav"),
        perfect: asset_server.load("sfx/perfect.wav"),
        b2b: asset_server.load("sfx/b2b.wav"),
        combo: asset_server.load("sfx/combo.wav"),
        level_up: asset_server.load("sfx/level_up.wav"),
        garbage_warn: asset_server.load("sfx/garbage_warn.wav"),
        garbage_rise: asset_server.load("sfx/garbage_rise.wav"),
        countdown: asset_server.load("sfx/countdown.wav"),
        go: asset_server.load("sfx/go.wav"),
        game_over: asset_server.load("sfx/game_over.wav"),
        defeat: asset_server.load("sfx/defeat.wav"),
        win: asset_server.load("sfx/win.wav"),
        menu_move: asset_server.load("sfx/menu_move.wav"),
        menu_select: asset_server.load("sfx/menu_select.wav"),
        menu_back: asset_server.load("sfx/menu_back.wav"),
    }
}

// ---------------------------------------------------------------------------
// Bevy plumbing
// ---------------------------------------------------------------------------

/// BGM streamed from CC0 assets ("Retro Game Music Pack" by Juhani Junkala,
/// see assets/CREDITS.md). In-game tracks are picked at random per match.
#[derive(Resource)]
pub struct BgmBank {
    title: Handle<AudioSource>,
    game: Vec<Handle<AudioSource>>,
    victory: Handle<AudioSource>,
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
}

impl PlaySfx {
    pub fn new(sfx: Sfx) -> Self {
        Self { sfx, gain: 1.0 }
    }
    pub fn quiet(sfx: Sfx) -> Self {
        Self { sfx, gain: 0.4 }
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
        app.insert_resource(build_sfx_bank(&asset_server));
        app.insert_resource(BgmBank {
            title: asset_server.load("music/title.ogg"),
            game: vec![
                asset_server.load("music/level1.ogg"),
                asset_server.load("music/level2.ogg"),
                asset_server.load("music/level3.ogg"),
            ],
            victory: asset_server.load("music/ending.ogg"),
        });

        app.add_message::<PlaySfx>()
            .init_resource::<CurrentBgm>()
            .add_systems(Update, (play_sfx, apply_bgm_volume))
            .add_systems(OnEnter(AppState::Title), start_title_bgm)
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
    settings: Res<GameSettings>,
) {
    let base = settings.sfx_linear();
    for msg in reader.read() {
        let (handle, gain, speed) = bank.params(msg.sfx);
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
    swap_bgm(commands, bank.title.clone(), &settings, current, sinks);
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
    swap_bgm(commands, track, &settings, current, sinks);
}

/// On a VS win, celebrate with the (CC0) ending jingle.
fn play_victory_jingle(
    mut commands: Commands,
    bank: Res<BgmBank>,
    settings: Res<GameSettings>,
    result: Option<Res<SessionResult>>,
) {
    if matches!(result.as_deref(), Some(SessionResult::VsWin { winner: 0 })) {
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
    track: Handle<AudioSource>,
    settings: &GameSettings,
    mut current: ResMut<CurrentBgm>,
    sinks: Query<&AudioSink, With<Bgm>>,
) {
    if current.0.as_ref() == Some(&track) {
        // Same track: keep playing where it is (it may have been paused by
        // a result screen — a match restart must un-pause it).
        for sink in &sinks {
            sink.play();
        }
        return;
    }
    current.0 = Some(track.clone());
    debug!("bgm: starting {:?}", track.path());
    commands.queue(|world: &mut World| {
        let old: Vec<Entity> = world
            .query_filtered::<Entity, With<Bgm>>()
            .iter(world)
            .collect();
        for e in old {
            world.entity_mut(e).despawn();
        }
    });
    commands.spawn((
        AudioPlayer::new(track),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(settings.bgm_linear())),
        Bgm,
    ));
}

/// Keep the BGM sink in sync with the settings sliders.
fn apply_bgm_volume(
    settings: Res<GameSettings>,
    mut sinks: Query<&mut AudioSink, With<Bgm>>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut sink in &mut sinks {
        sink.set_volume(Volume::Linear(settings.bgm_linear()));
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
