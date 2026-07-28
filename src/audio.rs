//! Audio: sound effects are procedurally synthesized at startup into
//! in-memory WAV files; BGM streams from CC0 tracks by Juhani Junkala
//! (see assets/CREDITS.md).

use bevy::audio::{AudioSource, PlaybackSettings, Volume};
use bevy::prelude::*;
use rand::seq::IndexedRandom;

use crate::config::GameSettings;
use crate::session::SessionResult;
use crate::state::{AppState, PlayState};

const SAMPLE_RATE: u32 = 44_100;

// ---------------------------------------------------------------------------
// Synthesis primitives
// ---------------------------------------------------------------------------

fn sine(phase: f32) -> f32 {
    (phase * std::f32::consts::TAU).sin()
}

fn square(phase: f32, duty: f32) -> f32 {
    if phase.fract() < duty { 1.0 } else { -1.0 }
}

fn triangle(phase: f32) -> f32 {
    let p = phase.fract();
    if p < 0.5 { 4.0 * p - 1.0 } else { 3.0 - 4.0 * p }
}

/// Deterministic white noise (xorshift), independent of the `rand` crate.
struct Noise(u32);

impl Noise {
    fn new() -> Self {
        Noise(0x1234_5678)
    }
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

#[derive(Clone, Copy)]
enum Wave {
    Sine,
    Square(f32),
    Triangle,
    Noise,
}

/// A single synthesized tone: frequency glides exponentially from
/// `freq_start` to `freq_end`, amplitude follows attack/decay envelope.
struct Tone {
    wave: Wave,
    freq_start: f32,
    freq_end: f32,
    amp: f32,
    duration: f32,
    attack: f32,
    /// Exponential decay rate; higher = snappier.
    decay: f32,
}

impl Tone {
    fn render(&self, out: &mut Vec<f32>, at: f32) {
        let start = (at * SAMPLE_RATE as f32) as usize;
        let count = (self.duration * SAMPLE_RATE as f32) as usize;
        if out.len() < start + count {
            out.resize(start + count, 0.0);
        }
        let mut noise = Noise::new();
        let mut phase = 0.0f32;
        for i in 0..count {
            let t = i as f32 / count as f32;
            let freq = self.freq_start * (self.freq_end / self.freq_start).powf(t);
            phase += freq / SAMPLE_RATE as f32;
            let raw = match self.wave {
                Wave::Sine => sine(phase),
                Wave::Square(duty) => square(phase, duty),
                Wave::Triangle => triangle(phase),
                Wave::Noise => noise.next(),
            };
            let secs = i as f32 / SAMPLE_RATE as f32;
            let env_attack = if self.attack > 0.0 {
                (secs / self.attack).min(1.0)
            } else {
                1.0
            };
            let env_decay = (-self.decay * t).exp();
            // Short release ramp so clips never click at the end.
            let env_tail = ((1.0 - t) * 20.0).min(1.0);
            out[start + i] += raw * self.amp * env_attack * env_decay * env_tail;
        }
    }
}

/// Encode f32 samples (-1..1) as a 16-bit mono RIFF/WAVE file.
fn encode_wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

fn soft_clip(samples: &mut [f32]) {
    for s in samples.iter_mut() {
        *s = (*s).tanh();
    }
}

fn to_source(samples: &mut Vec<f32>) -> AudioSource {
    soft_clip(samples);
    AudioSource {
        bytes: encode_wav(samples).into(),
    }
}

/// Note frequency from semitone offset relative to A4 (440 Hz).
fn note(semi: i32) -> f32 {
    440.0 * 2f32.powf(semi as f32 / 12.0)
}

// ---------------------------------------------------------------------------
// Sound effect construction
// ---------------------------------------------------------------------------

fn blip(wave: Wave, f0: f32, f1: f32, dur: f32, amp: f32) -> Vec<f32> {
    let mut buf = Vec::new();
    Tone {
        wave,
        freq_start: f0,
        freq_end: f1,
        amp,
        duration: dur,
        attack: 0.002,
        decay: 3.0,
    }
    .render(&mut buf, 0.0);
    buf
}

/// A fast arpeggio of square-wave notes (the classic "line clear" sparkle).
fn arpeggio(semis: &[i32], step: f32, note_len: f32, amp: f32, octave_double: bool) -> Vec<f32> {
    let mut buf = Vec::new();
    for (i, &s) in semis.iter().enumerate() {
        let at = i as f32 * step;
        Tone {
            wave: Wave::Square(0.5),
            freq_start: note(s),
            freq_end: note(s),
            amp,
            duration: note_len,
            attack: 0.002,
            decay: 2.5,
        }
        .render(&mut buf, at);
        if octave_double {
            Tone {
                wave: Wave::Square(0.25),
                freq_start: note(s + 12),
                freq_end: note(s + 12),
                amp: amp * 0.4,
                duration: note_len,
                attack: 0.002,
                decay: 2.5,
            }
            .render(&mut buf, at);
        }
    }
    buf
}

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
    combos: Vec<Handle<AudioSource>>,
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
    fn get(&self, sfx: Sfx) -> Handle<AudioSource> {
        match sfx {
            Sfx::Move => self.move_tick.clone(),
            Sfx::Rotate => self.rotate.clone(),
            Sfx::RotateFail => self.rotate_fail.clone(),
            Sfx::SoftDropTick => self.soft_drop.clone(),
            Sfx::HardDrop => self.hard_drop.clone(),
            Sfx::Lock => self.lock.clone(),
            Sfx::Hold => self.hold.clone(),
            Sfx::HoldFail => self.hold_fail.clone(),
            Sfx::Clear(n) => self.clears[(n.clamp(1, 4) - 1) as usize].clone(),
            Sfx::TSpin => self.tspin.clone(),
            Sfx::PerfectClear => self.perfect.clone(),
            Sfx::B2b => self.b2b.clone(),
            Sfx::Combo(n) => {
                let i = (n as usize).min(self.combos.len() - 1);
                self.combos[i].clone()
            }
            Sfx::LevelUp => self.level_up.clone(),
            Sfx::GarbageWarn => self.garbage_warn.clone(),
            Sfx::GarbageRise => self.garbage_rise.clone(),
            Sfx::Countdown => self.countdown.clone(),
            Sfx::Go => self.go.clone(),
            Sfx::GameOver => self.game_over.clone(),
            Sfx::Defeat => self.defeat.clone(),
            Sfx::Win => self.win.clone(),
            Sfx::MenuMove => self.menu_move.clone(),
            Sfx::MenuSelect => self.menu_select.clone(),
            Sfx::MenuBack => self.menu_back.clone(),
        }
    }
}

fn build_sfx_bank(assets: &mut Assets<AudioSource>) -> SfxBank {
    let mut add = |mut samples: Vec<f32>| assets.add(to_source(&mut samples));

    // Movement: tiny dry tick.
    let move_tick = add(blip(Wave::Square(0.5), 880.0, 880.0, 0.03, 0.25));
    let soft_drop = add(blip(Wave::Square(0.5), 660.0, 660.0, 0.02, 0.15));
    let rotate = add(blip(Wave::Sine, 500.0, 900.0, 0.06, 0.35));
    let rotate_fail = add(blip(Wave::Square(0.5), 180.0, 150.0, 0.08, 0.3));

    // Hard drop: noise burst + falling thump.
    let hard_drop = add({
        let mut buf = blip(Wave::Noise, 1000.0, 1000.0, 0.06, 0.5);
        Tone {
            wave: Wave::Sine,
            freq_start: 300.0,
            freq_end: 60.0,
            amp: 0.8,
            duration: 0.12,
            attack: 0.001,
            decay: 4.0,
        }
        .render(&mut buf, 0.0);
        buf
    });

    let lock = add({
        let mut buf = blip(Wave::Noise, 0.0, 0.0, 0.03, 0.3);
        Tone {
            wave: Wave::Square(0.5),
            freq_start: 320.0,
            freq_end: 320.0,
            amp: 0.3,
            duration: 0.04,
            attack: 0.001,
            decay: 5.0,
        }
        .render(&mut buf, 0.0);
        buf
    });

    let hold = add(blip(Wave::Triangle, 900.0, 350.0, 0.1, 0.4));
    let hold_fail = add(blip(Wave::Square(0.5), 220.0, 200.0, 0.09, 0.3));

    // Line clears: increasingly triumphant arpeggios (A minor pentatonic).
    let clears = [
        add(arpeggio(&[3, 10], 0.05, 0.12, 0.4, false)),
        add(arpeggio(&[3, 10, 15], 0.05, 0.12, 0.4, false)),
        add(arpeggio(&[3, 10, 15, 19], 0.05, 0.14, 0.4, false)),
        add({
            // TETRIS! big two-octave fanfare.
            let mut buf = arpeggio(&[3, 7, 10, 15, 19, 22, 27], 0.055, 0.22, 0.45, true);
            Tone {
                wave: Wave::Noise,
                freq_start: 0.0,
                freq_end: 0.0,
                amp: 0.15,
                duration: 0.25,
                attack: 0.01,
                decay: 6.0,
            }
            .render(&mut buf, 0.0);
            buf
        }),
    ];

    // T-spin: glittery chromatic sparkle.
    let tspin = add(arpeggio(&[15, 19, 24, 27, 31], 0.04, 0.15, 0.4, true));
    let perfect = add(arpeggio(&[3, 10, 15, 19, 22, 27, 31, 34], 0.07, 0.3, 0.45, true));
    let b2b = add(arpeggio(&[19, 24], 0.05, 0.18, 0.35, true));

    // Combo ladder: single blip rising through a pentatonic scale.
    let pentatonic = [0, 3, 5, 7, 10, 12, 15, 17, 19, 22, 24, 27];
    let combos = pentatonic
        .iter()
        .map(|&s| add(blip(Wave::Square(0.5), note(s + 12), note(s + 12), 0.09, 0.4)))
        .collect();

    let level_up = add(arpeggio(&[0, 4, 7, 12, 16, 19, 24], 0.06, 0.25, 0.4, false));
    let garbage_warn = add(blip(Wave::Square(0.3), 140.0, 110.0, 0.25, 0.4));
    let garbage_rise = add({
        let mut buf = blip(Wave::Noise, 0.0, 0.0, 0.2, 0.35);
        Tone {
            wave: Wave::Triangle,
            freq_start: 80.0,
            freq_end: 160.0,
            amp: 0.5,
            duration: 0.22,
            attack: 0.01,
            decay: 2.0,
        }
        .render(&mut buf, 0.0);
        buf
    });

    let countdown = add(blip(Wave::Square(0.5), 660.0, 660.0, 0.12, 0.4));
    let go = add(arpeggio(&[12, 16, 19], 0.04, 0.3, 0.45, true));

    let game_over = add({
        let mut buf = Vec::new();
        for (i, s) in [7, 3, 0, -5, -9].iter().enumerate() {
            Tone {
                wave: Wave::Triangle,
                freq_start: note(*s),
                freq_end: note(*s) * 0.97,
                amp: 0.4,
                duration: 0.3,
                attack: 0.01,
                decay: 1.5,
            }
            .render(&mut buf, i as f32 * 0.22);
        }
        buf
    });

    // Heavy sub-bass boom with a noise rumble tail.
    let defeat = add({
        let mut buf = blip(Wave::Noise, 0.0, 0.0, 0.7, 0.45);
        Tone {
            wave: Wave::Sine,
            freq_start: 72.0,
            freq_end: 27.0,
            amp: 0.95,
            duration: 1.0,
            attack: 0.004,
            decay: 2.0,
        }
        .render(&mut buf, 0.0);
        Tone {
            wave: Wave::Square(0.5),
            freq_start: 110.0,
            freq_end: 50.0,
            amp: 0.22,
            duration: 0.45,
            attack: 0.005,
            decay: 3.0,
        }
        .render(&mut buf, 0.04);
        buf
    });

    let win = add(arpeggio(&[0, 4, 7, 12, 12, 16, 19, 24], 0.09, 0.35, 0.45, true));

    let menu_move = add(blip(Wave::Square(0.5), 700.0, 700.0, 0.03, 0.2));
    let menu_select = add(arpeggio(&[12, 19], 0.05, 0.12, 0.35, false));
    let menu_back = add(arpeggio(&[19, 12], 0.05, 0.12, 0.35, false));

    SfxBank {
        move_tick,
        rotate,
        rotate_fail,
        soft_drop,
        hard_drop,
        lock,
        hold,
        hold_fail,
        clears,
        tspin,
        perfect,
        b2b,
        combos,
        level_up,
        garbage_warn,
        garbage_rise,
        countdown,
        go,
        game_over,
        defeat,
        win,
        menu_move,
        menu_select,
        menu_back,
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
        app.add_message::<PlaySfx>()
            .init_resource::<CurrentBgm>()
            // PreStartup: the initial state's OnEnter may run before Startup
            // systems, and the title BGM needs the bank to exist by then.
            .add_systems(PreStartup, generate_audio)
            .add_systems(Update, (play_sfx, apply_bgm_volume))
            .add_systems(OnEnter(AppState::Title), start_title_bgm)
            .add_systems(OnEnter(AppState::Playing), start_game_bgm)
            .add_systems(OnEnter(PlayState::Paused), pause_bgm)
            .add_systems(OnExit(PlayState::Paused), resume_bgm)
            .add_systems(OnEnter(PlayState::Finished), (pause_bgm, play_victory_jingle));
    }
}

fn generate_audio(
    mut commands: Commands,
    mut assets: ResMut<Assets<AudioSource>>,
    asset_server: Res<AssetServer>,
) {
    let bank = build_sfx_bank(&mut assets);
    let bgm = BgmBank {
        title: asset_server.load("music/title.ogg"),
        game: vec![
            asset_server.load("music/level1.ogg"),
            asset_server.load("music/level2.ogg"),
            asset_server.load("music/level3.ogg"),
        ],
        victory: asset_server.load("music/ending.ogg"),
    };
    commands.insert_resource(bank);
    commands.insert_resource(bgm);
}

fn play_sfx(
    mut commands: Commands,
    mut reader: MessageReader<PlaySfx>,
    bank: Option<Res<SfxBank>>,
    settings: Res<GameSettings>,
) {
    let Some(bank) = bank else { return };
    let base = settings.sfx_linear();
    for msg in reader.read() {
        let volume = base * msg.gain;
        if volume <= 0.001 {
            continue;
        }
        commands.spawn((
            AudioPlayer::new(bank.get(msg.sfx)),
            PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume)),
        ));
    }
}

fn start_title_bgm(
    commands: Commands,
    bank: Option<Res<BgmBank>>,
    settings: Res<GameSettings>,
    current: ResMut<CurrentBgm>,
    sinks: Query<&AudioSink, With<Bgm>>,
) {
    if let Some(bank) = bank {
        swap_bgm(commands, bank.title.clone(), &settings, current, sinks);
    }
}

fn start_game_bgm(
    commands: Commands,
    bank: Option<Res<BgmBank>>,
    settings: Res<GameSettings>,
    current: ResMut<CurrentBgm>,
    sinks: Query<&AudioSink, With<Bgm>>,
) {
    if let Some(bank) = bank {
        let track = bank
            .game
            .choose(&mut rand::rng())
            .expect("game BGM list is never empty")
            .clone();
        swap_bgm(commands, track, &settings, current, sinks);
    }
}

/// On a VS win, celebrate with the (CC0) ending jingle.
fn play_victory_jingle(
    mut commands: Commands,
    bank: Option<Res<BgmBank>>,
    settings: Res<GameSettings>,
    result: Option<Res<SessionResult>>,
) {
    let Some(bank) = bank else { return };
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
