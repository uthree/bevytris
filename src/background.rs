//! Procedural background scenes, all built from glowing rectangles:
//!
//! * FORMATION — square particles forming morphing 3D figures (cube
//!   wireframe, sine surface, Lissajous knot, double helix), spun and
//!   perspective-projected.
//! * CYBER — matrix-style glyph rain plus drifting lines of this game's
//!   actual source code.
//! * GALAXY — the space painting, a slowly turning spiral galaxy of
//!   square stars, and the occasional shooting star.
//! * VISUALIZER — EQ bars along the screen edges and a waveform of
//!   squares through the middle.
//!
//! Scenes crossfade on a 40-70 s timer (random next pick). Everything
//! breathes with [`AudioPulse`], an energy level fed by every sound
//! effect the game plays — an audio visualizer driven by the game's own
//! soundscape. The classic falling starfield stays on across all scenes
//! (and still lunges via [`StarSurge`] on big clears).

use bevy::prelude::*;
use rand::Rng;

use crate::audio::PlaySfx;
use crate::emissive;
use crate::session::{GameSession, HumanControlled};
use crate::state::AppState;

/// Pseudo-spectrum band count for the visualizer.
pub const BANDS: usize = 24;

/// Temporary starfield speed multiplier; spikes on spectacular clears so
/// the whole background lunges without ever covering the boards.
#[derive(Resource)]
pub struct StarSurge(pub f32);

impl Default for StarSurge {
    fn default() -> Self {
        StarSurge(1.0)
    }
}

/// Audio energy driving the scenes: every played sound effect adds a hit
/// that decays quickly, and a pseudo-spectrum wiggles per band on top.
#[derive(Resource)]
pub struct AudioPulse {
    /// Overall energy, ~0.1 (idle breathing) .. 1.6 (heavy action).
    pub energy: f32,
    /// Slow-smoothed energy for MOTION parameters (rotation speed,
    /// amplitude): spiky raw energy makes movement stutter, so anything
    /// that moves follows this instead and only brightness may twitch.
    pub slow: f32,
    pub bands: [f32; BANDS],
}

impl Default for AudioPulse {
    fn default() -> Self {
        AudioPulse {
            energy: 0.15,
            slow: 0.15,
            bands: [0.0; BANDS],
        }
    }
}

const SCENE_COUNT: usize = 4;
const FORMATION: usize = 0;
const CYBER: usize = 1;
const GALAXY: usize = 2;
const VISUALIZER: usize = 3;

/// One color scheme shared by every scene.
struct Palette {
    formation: [Color; 4],
    /// Glyph-rain columns.
    glyph: Color,
    /// Drifting code blocks.
    code: Color,
    /// Galaxy: core, then the two arm tints.
    galaxy: [Color; 3],
    /// Visualizer bar gradient ends.
    eq: (Color, Color),
    wave: Color,
}

/// Preset palettes; every incoming scene draws a random one, so the
/// backdrop changes hue over a session instead of being eternally cyan.
const PALETTES: [Palette; 5] = [
    // NEON — the original cyan/magenta arcade look.
    Palette {
        formation: [
            Color::srgb(0.35, 0.9, 1.0),
            Color::srgb(0.95, 0.4, 1.0),
            Color::srgb(0.95, 0.97, 1.0),
            Color::srgb(1.0, 0.85, 0.3),
        ],
        glyph: Color::srgb(0.25, 1.0, 0.5),
        code: Color::srgb(0.3, 0.75, 0.9),
        galaxy: [
            Color::srgb(1.0, 0.9, 0.7),
            Color::srgb(0.6, 0.8, 1.0),
            Color::srgb(0.7, 0.5, 1.0),
        ],
        eq: (Color::srgb(0.2, 0.9, 1.0), Color::srgb(1.0, 0.3, 0.9)),
        wave: Color::srgb(0.5, 0.95, 1.0),
    },
    // EMBER — warm oranges and reds.
    Palette {
        formation: [
            Color::srgb(1.0, 0.55, 0.2),
            Color::srgb(1.0, 0.3, 0.25),
            Color::srgb(1.0, 0.9, 0.4),
            Color::srgb(1.0, 0.8, 0.6),
        ],
        glyph: Color::srgb(1.0, 0.7, 0.25),
        code: Color::srgb(0.95, 0.55, 0.3),
        galaxy: [
            Color::srgb(1.0, 0.85, 0.5),
            Color::srgb(1.0, 0.6, 0.3),
            Color::srgb(0.95, 0.35, 0.3),
        ],
        eq: (Color::srgb(1.0, 0.8, 0.3), Color::srgb(1.0, 0.25, 0.2)),
        wave: Color::srgb(1.0, 0.7, 0.4),
    },
    // MATRIX — all greens.
    Palette {
        formation: [
            Color::srgb(0.3, 1.0, 0.5),
            Color::srgb(0.2, 0.9, 0.7),
            Color::srgb(0.85, 1.0, 0.9),
            Color::srgb(0.7, 1.0, 0.3),
        ],
        glyph: Color::srgb(0.25, 1.0, 0.45),
        code: Color::srgb(0.4, 0.9, 0.5),
        galaxy: [
            Color::srgb(0.85, 1.0, 0.8),
            Color::srgb(0.4, 0.95, 0.5),
            Color::srgb(0.2, 0.8, 0.6),
        ],
        eq: (Color::srgb(0.3, 1.0, 0.5), Color::srgb(0.15, 0.75, 0.6)),
        wave: Color::srgb(0.5, 1.0, 0.6),
    },
    // ICE — whites and pale blues.
    Palette {
        formation: [
            Color::srgb(0.95, 0.98, 1.0),
            Color::srgb(0.6, 0.85, 1.0),
            Color::srgb(0.4, 0.95, 1.0),
            Color::srgb(0.7, 0.7, 1.0),
        ],
        glyph: Color::srgb(0.6, 0.85, 1.0),
        code: Color::srgb(0.75, 0.85, 1.0),
        galaxy: [
            Color::srgb(1.0, 1.0, 1.0),
            Color::srgb(0.65, 0.85, 1.0),
            Color::srgb(0.45, 0.6, 1.0),
        ],
        eq: (Color::srgb(0.9, 0.97, 1.0), Color::srgb(0.35, 0.6, 1.0)),
        wave: Color::srgb(0.8, 0.92, 1.0),
    },
    // VAPOR — pinks and purples.
    Palette {
        formation: [
            Color::srgb(1.0, 0.5, 0.8),
            Color::srgb(0.7, 0.4, 1.0),
            Color::srgb(0.45, 0.6, 1.0),
            Color::srgb(1.0, 0.85, 0.95),
        ],
        glyph: Color::srgb(1.0, 0.55, 0.8),
        code: Color::srgb(0.75, 0.55, 1.0),
        galaxy: [
            Color::srgb(1.0, 0.8, 0.9),
            Color::srgb(0.9, 0.45, 0.9),
            Color::srgb(0.55, 0.45, 1.0),
        ],
        eq: (Color::srgb(0.7, 0.4, 1.0), Color::srgb(1.0, 0.45, 0.75)),
        wave: Color::srgb(1.0, 0.6, 0.85),
    },
];

/// Which palette each scene is currently wearing. An incoming scene
/// re-rolls its palette, so colors shift with every crossfade without
/// ever popping mid-display.
#[derive(Resource)]
struct ScenePalettes([usize; SCENE_COUNT]);

impl ScenePalettes {
    fn random() -> Self {
        let mut rng = rand::rng();
        ScenePalettes(std::array::from_fn(|_| rng.random_range(0..PALETTES.len())))
    }

    fn of(&self, scene: usize) -> &'static Palette {
        &PALETTES[self.0[scene]]
    }
}

/// Seconds a crossfade takes.
const FADE_SECS: f32 = 3.0;

#[derive(Component)]
struct SceneRoot(usize);

#[derive(Resource)]
struct SceneState {
    active: usize,
    prev: usize,
    /// Crossfade progress toward `active` (1 = fully shown).
    fade: f32,
    /// Master gate: scenes only show during gameplay. Menus keep the
    /// calm starfield so the title screen stays quiet.
    master: f32,
    timer: Timer,
    /// Dev pin (`BEVYTRIS_SCENE=formation|cyber|galaxy|visualizer`).
    pinned: bool,
}

/// Per-scene display weight (0..1), refreshed every frame, plus the
/// eased master gate (0 in menus, 1 mid-game).
#[derive(Resource, Default)]
struct SceneWeights {
    scenes: [f32; SCENE_COUNT],
    master: f32,
}

/// The zone's own minimal show: while the player's zone runs, every
/// scene blacks out (`gate` -> 1) and square shards stream radially
/// from the screen center instead — less information, more focus.
#[derive(Resource, Default)]
struct ZoneFx {
    gate: f32,
    spawn_acc: f32,
}

#[derive(Component)]
struct ZoneShard {
    vel: Vec2,
    life: f32,
    max_life: f32,
}

pub struct BackgroundPlugin;

impl Plugin for BackgroundPlugin {
    fn build(&self, app: &mut App) {
        let (start, pinned) = match std::env::var("BEVYTRIS_SCENE").as_deref() {
            Ok("formation") => (FORMATION, true),
            Ok("cyber") => (CYBER, true),
            Ok("galaxy") => (GALAXY, true),
            Ok("visualizer") => (VISUALIZER, true),
            _ => (rand::rng().random_range(0..SCENE_COUNT), false),
        };
        app.init_resource::<StarSurge>()
            .init_resource::<AudioPulse>()
            .init_resource::<SceneWeights>()
            .init_resource::<ZoneFx>()
            .insert_resource(ScenePalettes::random())
            .insert_resource(SceneState {
                active: start,
                prev: start,
                fade: 1.0,
                master: 0.0,
                timer: Timer::from_seconds(rand::rng().random_range(40.0..70.0), TimerMode::Once),
                pinned,
            })
            .add_systems(Startup, setup_background)
            .add_systems(
                Update,
                (
                    update_audio_pulse,
                    update_zone_fx,
                    update_scene_state,
                    animate_stars,
                    animate_formation,
                    animate_cyber,
                    animate_galaxy,
                    animate_visualizer,
                )
                    .chain(),
            );
    }
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Star {
    speed: f32,
    alpha: f32,
}

#[derive(Component)]
struct FormationDot {
    idx: usize,
}

#[derive(Component)]
struct CodeColumn {
    speed: f32,
}

#[derive(Component)]
struct CodeBlock {
    speed: f32,
}

#[derive(Component)]
struct GalaxyStar {
    angle: f32,
    radius: f32,
    phase: f32,
    size: f32,
    /// Palette slot: 0 = core, 1/2 = arm tints.
    kind: usize,
}

#[derive(Component)]
struct GalaxyArt;

#[derive(Component)]
struct ShootingStar {
    vel: Vec2,
    life: f32,
}

/// Spawns shooting stars while the galaxy scene is up.
#[derive(Resource)]
struct ShootingTimer(Timer);

#[derive(Component)]
struct EqBar {
    band: usize,
    /// Mirrored copy along the top edge.
    top: bool,
}

#[derive(Component)]
struct WaveDot {
    idx: usize,
}

const FORMATION_DOTS: usize = 240;
const CODE_COLUMNS: usize = 24;
const GALAXY_STARS: usize = 300;
const WAVE_DOTS: usize = 72;

/// Real multi-line fragments of this repository, drifting through the
/// cyber scene. Whole nested blocks read far more like a program than
/// scattered single lines.
const CODE_BLOCKS: [&str; 4] = [
    "pub fn rotate(&mut self, clockwise: bool) -> bool {\n    let from = self.active.rot;\n    let to = if clockwise { from.cw() } else { from.ccw() };\n    for (i, &(dx, dy)) in kicks(self.active.kind, from, to) {\n        let candidate = self.active.rotated(to).shifted(dx, dy);\n        if self.board.fits(&candidate) {\n            self.active = candidate;\n            self.last_rotation_kick = Some(i);\n            return true;\n        }\n    }\n    false\n}",
    "pub fn attack_multiplier(&self) -> f32 {\n    let Some(margin) = self.margin_time else {\n        return 1.0;\n    };\n    let over = self.stats.time as f32 - margin;\n    if over < 0.0 {\n        1.0\n    } else if over < 30.0 {\n        1.5\n    } else {\n        4.0\n    }\n}",
    "fn lock_active(&mut self) {\n    let piece = self.active;\n    let tspin = self.detect_tspin();\n    self.board.lock(&piece);\n    self.stats.pieces += 1;\n    if self.zone_active() {\n        let banked = self.board.bank_full_rows();\n        if banked > 0 {\n            zone.lines += banked;\n        }\n    }\n}",
    "while let Some((piece, kick, steps)) = queue.pop_front() {\n    let grounded = !board.fits(&piece.shifted(0, -1));\n    if grounded {\n        let tspin = t_spin_kind(board, &piece, kick);\n        out.push(Placement { piece, steps });\n    }\n    for (dx, step) in [(-1, Step::Left), (1, Step::Right)] {\n        let cand = piece.shifted(dx, 0);\n        if board.fits(&cand) {\n            queue.push_back(cand);\n        }\n    }\n}",
];

const GLYPHS: &[char] = &[
    'ア', 'イ', 'ウ', 'エ', 'オ', 'カ', 'キ', 'ク', 'ケ', 'コ', 'サ', 'シ', 'ス', 'セ', 'ソ',
    'タ', 'チ', 'ツ', 'テ', 'ト', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B',
    'C', 'D', 'E', 'F', '<', '>', '/', '*', '+', '-', '=', '#', '$', '%', '&', '?',
];

fn random_column_text(rng: &mut impl Rng) -> String {
    let len = rng.random_range(10..22);
    let mut s = String::new();
    for i in 0..len {
        if i > 0 {
            s.push('\n');
        }
        s.push(GLYPHS[rng.random_range(0..GLYPHS.len())]);
    }
    s
}

fn setup_background(mut commands: Commands, asset_server: Res<AssetServer>) {
    let mut rng = rand::rng();

    // Falling starfield for the menus; it fades out while the scenes run
    // so each scene keeps its own distinct look.
    for _ in 0..110 {
        let b = rng.random_range(0.15..0.7);
        let size = rng.random_range(1.0..3.5);
        commands.spawn((
            Sprite::from_color(Color::srgba(b, b, b * 1.2, 0.8), Vec2::splat(size)),
            Transform::from_xyz(
                rng.random_range(-660.0..660.0),
                rng.random_range(-380.0..380.0),
                -10.0,
            ),
            Star {
                speed: rng.random_range(12.0..55.0),
                alpha: 0.8,
            },
        ));
    }

    // --- FORMATION -------------------------------------------------------
    let palette = [
        Color::srgb(0.35, 0.9, 1.0),
        Color::srgb(0.95, 0.4, 1.0),
        Color::srgb(0.95, 0.97, 1.0),
        Color::srgb(1.0, 0.85, 0.3),
    ];
    commands
        .spawn((
            SceneRoot(FORMATION),
            Transform::from_xyz(0.0, 0.0, -20.0),
            Visibility::default(),
        ))
        .with_children(|parent| {
            for i in 0..FORMATION_DOTS {
                parent.spawn((
                    Sprite::from_color(emissive(palette[i % palette.len()], 2.4), Vec2::splat(4.0)),
                    Transform::default(),
                    FormationDot { idx: i },
                ));
            }
        });

    // --- CYBER -----------------------------------------------------------
    commands
        .spawn((
            SceneRoot(CYBER),
            Transform::from_xyz(0.0, 0.0, -18.0),
            Visibility::default(),
        ))
        .with_children(|parent| {
            for i in 0..CODE_COLUMNS {
                let x = -640.0 + (i as f32 + 0.5) * 1280.0 / CODE_COLUMNS as f32
                    + rng.random_range(-14.0..14.0);
                parent.spawn((
                    Text2d::new(random_column_text(&mut rng)),
                    TextFont {
                        font_size: FontSize::Px(15.0),
                        ..default()
                    },
                    TextColor(emissive(Color::srgb(0.25, 1.0, 0.5), 1.5)),
                    Transform::from_xyz(x, rng.random_range(-360.0..420.0), 0.0),
                    CodeColumn {
                        speed: rng.random_range(40.0..130.0),
                    },
                ));
            }
            for (i, block) in CODE_BLOCKS.iter().enumerate() {
                parent.spawn((
                    Text2d::new(*block),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    // Left-justified so the indentation actually reads.
                    TextLayout::justify(Justify::Left),
                    TextColor(emissive(Color::srgb(0.3, 0.75, 0.9), 1.2)),
                    Transform::from_xyz(
                        rng.random_range(-640.0..640.0),
                        [230.0, -20.0, -260.0, 120.0][i % 4] + rng.random_range(-30.0..30.0),
                        -0.5,
                    ),
                    CodeBlock {
                        speed: rng.random_range(14.0..38.0),
                    },
                ));
            }
        });

    // --- GALAXY ----------------------------------------------------------
    commands
        .spawn((
            SceneRoot(GALAXY),
            Transform::from_xyz(0.0, 0.0, -24.0),
            Visibility::default(),
        ))
        .with_children(|parent| {
            // CC0 space painting (Westbeam, see assets/CREDITS.md).
            parent.spawn((
                Sprite {
                    image: asset_server.load("images/space_bg.png"),
                    custom_size: Some(Vec2::new(1560.0, 1170.0)),
                    color: Color::srgba(0.5, 0.5, 0.62, 0.0),
                    ..default()
                },
                Transform::from_xyz(0.0, 0.0, -4.0),
                GalaxyArt,
            ));
            // Two-armed logarithmic spiral of square stars.
            for i in 0..GALAXY_STARS {
                let arm = (i % 2) as f32 * std::f32::consts::PI;
                let t = (i / 2) as f32 / (GALAXY_STARS / 2) as f32;
                let radius = 30.0 + 530.0 * t.powf(0.8);
                let angle = arm + t * 4.2 + rng.random_range(-0.16..0.16);
                let kind = if t < 0.25 {
                    0
                } else if rng.random_bool(0.25) {
                    2
                } else {
                    1
                };
                parent.spawn((
                    Sprite::from_color(
                        emissive(Color::WHITE, 1.6),
                        Vec2::splat(rng.random_range(1.5..4.5)),
                    ),
                    Transform::default(),
                    GalaxyStar {
                        angle,
                        radius,
                        phase: rng.random_range(0.0..std::f32::consts::TAU),
                        size: rng.random_range(1.5..4.5),
                        kind,
                    },
                ));
            }
        });

    // --- VISUALIZER ------------------------------------------------------
    commands
        .spawn((
            SceneRoot(VISUALIZER),
            Transform::from_xyz(0.0, 0.0, -22.0),
            Visibility::default(),
        ))
        .with_children(|parent| {
            let w = 1280.0 / BANDS as f32;
            for band in 0..BANDS {
                let x = -640.0 + (band as f32 + 0.5) * w;
                let f = band as f32 / (BANDS - 1) as f32;
                let color = Color::srgb(0.2 + 0.8 * f, 0.9 - 0.6 * f, 1.0 - 0.2 * f);
                for top in [false, true] {
                    parent.spawn((
                        Sprite::from_color(emissive(color, 1.3), Vec2::new(w - 6.0, 8.0)),
                        Transform::from_xyz(x, if top { 366.0 } else { -366.0 }, 0.0),
                        EqBar { band, top },
                    ));
                }
            }
            for idx in 0..WAVE_DOTS {
                parent.spawn((
                    Sprite::from_color(emissive(Color::srgb(0.5, 0.95, 1.0), 1.4), Vec2::splat(4.0)),
                    Transform::from_xyz(
                        -640.0 + (idx as f32 + 0.5) * 1280.0 / WAVE_DOTS as f32,
                        0.0,
                        -0.5,
                    ),
                    WaveDot { idx },
                ));
            }
        });

    commands.insert_resource(ShootingTimer(Timer::from_seconds(5.0, TimerMode::Once)));
}

// ---------------------------------------------------------------------------
// Audio pulse & scene switching
// ---------------------------------------------------------------------------

fn update_audio_pulse(
    time: Res<Time>,
    mut pulse: ResMut<AudioPulse>,
    mut sfx: MessageReader<PlaySfx>,
) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    // Every sound effect is a hit; louder effects hit harder.
    let mut hit = 0.0;
    for msg in sfx.read() {
        hit += (msg.gain.min(1.5)) * 0.3;
    }
    pulse.energy = (pulse.energy + hit).min(1.6);
    // Fast decay toward a gentle idle breath, so quiet menus still move.
    let idle = 0.12 + 0.05 * (t * 0.8).sin().abs();
    pulse.energy += (idle - pulse.energy) * (2.2 * dt).min(1.0);
    // The motion-side envelope follows slowly in both directions.
    let energy = pulse.energy;
    pulse.slow += (energy - pulse.slow) * (1.2 * dt).min(1.0);

    // Pseudo-spectrum: incommensurate sine mixes per band, scaled by the
    // energy envelope. Reads like an equalizer without needing an FFT.
    let energy = pulse.energy;
    for (i, band) in pulse.bands.iter_mut().enumerate() {
        let fi = i as f32;
        let wobble = ((t * (2.1 + fi * 0.37)).sin() * (t * (1.3 + fi * 0.11) + fi).cos()).abs();
        let target = (0.12 + wobble) * (0.35 + energy);
        *band += (target - *band) * (8.0 * dt).min(1.0);
    }
}

/// The zone's dedicated show: scenes black out and square shards stream
/// radially from the screen center — minimal, monochrome, focused.
fn update_zone_fx(
    time: Res<Time>,
    sessions: Query<&GameSession, With<HumanControlled>>,
    mut fx: ResMut<ZoneFx>,
    mut commands: Commands,
    mut shards: Query<(Entity, &mut ZoneShard, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    let active = sessions.iter().any(|s| s.game.zone_active());
    let target = if active { 1.0 } else { 0.0 };
    let rate = if active { 5.0 } else { 2.5 };
    fx.gate += (target - fx.gate) * (rate * dt).min(1.0);

    // Steady radial stream while the gate is open.
    if fx.gate > 0.1 {
        fx.spawn_acc += dt * 80.0 * fx.gate;
        let mut rng = rand::rng();
        while fx.spawn_acc >= 1.0 {
            fx.spawn_acc -= 1.0;
            let a = rng.random_range(0.0..std::f32::consts::TAU);
            let dir = Vec2::new(a.cos(), a.sin());
            let start = dir * rng.random_range(14.0..70.0) + Vec2::new(0.0, -14.0);
            commands.spawn((
                Sprite::from_color(
                    emissive(Color::srgb(0.75, 0.95, 1.0), 2.0),
                    Vec2::splat(rng.random_range(2.0..5.0)),
                ),
                Transform::from_translation(start.extend(-20.0)),
                ZoneShard {
                    vel: dir * rng.random_range(50.0..140.0),
                    life: 1.5,
                    max_life: 1.5,
                },
                DespawnOnExit(AppState::Playing),
            ));
        }
    }

    for (entity, mut shard, mut tf, mut sprite) in &mut shards {
        shard.life -= dt;
        if shard.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        // Warp feel: shards accelerate as they fly outward.
        shard.vel *= 1.0 + 2.0 * dt;
        tf.translation.x += shard.vel.x * dt;
        tf.translation.y += shard.vel.y * dt;
        let k = 1.0 - shard.life / shard.max_life;
        let alpha = (k / 0.12).min(shard.life / 0.4).clamp(0.0, 1.0);
        sprite.color.set_alpha(alpha * 0.8);
    }
}

fn update_scene_state(
    time: Res<Time>,
    app_state: Res<State<AppState>>,
    fx: Res<ZoneFx>,
    mut state: ResMut<SceneState>,
    mut weights: ResMut<SceneWeights>,
    mut palettes: ResMut<ScenePalettes>,
    mut roots: Query<(&SceneRoot, &mut Visibility)>,
) {
    let dt = time.delta_secs();
    state.fade = (state.fade + dt / FADE_SECS).min(1.0);

    // The show only runs during gameplay; menus fade back to the quiet
    // starfield (in over ~2 s, out faster so the title calms right down).
    let playing = matches!(*app_state.get(), AppState::Playing | AppState::Restarting);
    let rate = if playing { dt / 2.0 } else { dt / 0.8 };
    let target = if playing { 1.0 } else { 0.0 };
    state.master += (target - state.master).clamp(-rate, rate);

    if !state.pinned && state.timer.tick(time.delta()).is_finished() {
        let mut rng = rand::rng();
        let mut next = rng.random_range(0..SCENE_COUNT);
        if next == state.active {
            next = (next + 1) % SCENE_COUNT;
        }
        state.prev = state.active;
        state.active = next;
        state.fade = 0.0;
        state.timer = Timer::from_seconds(rng.random_range(40.0..70.0), TimerMode::Once);
        // The incoming scene picks a fresh color scheme.
        palettes.0[next] = rng.random_range(0..PALETTES.len());
    }

    let ease = state.fade * state.fade * (3.0 - 2.0 * state.fade);
    let master = state.master * state.master * (3.0 - 2.0 * state.master);
    // The zone show replaces the scenes: they black out while it runs.
    let show = master * (1.0 - fx.gate);
    weights.scenes = [0.0; SCENE_COUNT];
    weights.scenes[state.prev] = (1.0 - ease) * show;
    weights.scenes[state.active] = ease * show;
    weights.master = master;

    for (root, mut vis) in &mut roots {
        let target = if weights.scenes[root.0] > 0.001 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != target {
            *vis = target;
        }
    }
}

fn animate_stars(
    time: Res<Time>,
    weights: Res<SceneWeights>,
    mut surge: ResMut<StarSurge>,
    mut query: Query<(&Star, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    surge.0 += (1.0 - surge.0) * (3.0 * dt).min(1.0);
    let visible = 1.0 - weights.master;
    for (star, mut tf, mut sprite) in &mut query {
        tf.translation.y -= star.speed * surge.0 * dt;
        if tf.translation.y < -380.0 {
            tf.translation.y = 380.0;
        }
        sprite.color.set_alpha(star.alpha * visible);
    }
}

// ---------------------------------------------------------------------------
// FORMATION: morphing 3D figures of square particles
// ---------------------------------------------------------------------------

/// Point `i` of `n` on figure `shape` (unit-ish coordinates), animated by `t`.
fn figure_point(shape: usize, i: usize, n: usize, t: f32) -> Vec3 {
    let f = i as f32 / n as f32;
    match shape % 4 {
        // Cube wireframe: points distributed along the 12 edges.
        0 => {
            const C: [[f32; 3]; 8] = [
                [-1.0, -1.0, -1.0],
                [1.0, -1.0, -1.0],
                [1.0, 1.0, -1.0],
                [-1.0, 1.0, -1.0],
                [-1.0, -1.0, 1.0],
                [1.0, -1.0, 1.0],
                [1.0, 1.0, 1.0],
                [-1.0, 1.0, 1.0],
            ];
            const E: [(usize, usize); 12] = [
                (0, 1),
                (1, 2),
                (2, 3),
                (3, 0),
                (4, 5),
                (5, 6),
                (6, 7),
                (7, 4),
                (0, 4),
                (1, 5),
                (2, 6),
                (3, 7),
            ];
            let per = (n / 12).max(1);
            let (a, b) = E[(i / per).min(11)];
            let k = (i % per) as f32 / per as f32;
            (Vec3::from_array(C[a]).lerp(Vec3::from_array(C[b]), k)) * 0.72
        }
        // Rippling sine surface.
        1 => {
            let cols = 16;
            let rows = n / cols;
            let x = (i % cols) as f32 / (cols - 1) as f32 * 2.0 - 1.0;
            let z = (i / cols).min(rows - 1) as f32 / (rows - 1) as f32 * 2.0 - 1.0;
            let y = 0.35 * (x * 3.4 + t * 1.4).sin() + 0.25 * (z * 3.0 - t).cos();
            Vec3::new(x, y * 0.9, z)
        }
        // Lissajous knot.
        2 => {
            let s = f * std::f32::consts::TAU;
            Vec3::new(
                (3.0 * s + t * 0.35).sin(),
                (4.0 * s).sin() * 0.8,
                (5.0 * s).cos(),
            ) * 0.85
        }
        // Double helix with rungs.
        _ => {
            let strand = i % 2;
            let k = f * 2.0 % 1.0;
            let a = k * std::f32::consts::TAU * 2.6 + strand as f32 * std::f32::consts::PI;
            if i % 9 == 8 {
                // Rung between the strands.
                let r = (i as f32 * 0.618).fract() * 2.0 - 1.0;
                let base = k * std::f32::consts::TAU * 2.6;
                Vec3::new(base.sin() * r * 0.55, k * 2.0 - 1.0, base.cos() * r * 0.55)
            } else {
                Vec3::new(a.sin() * 0.55, k * 2.0 - 1.0, a.cos() * 0.55)
            }
        }
    }
}

fn animate_formation(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    // Integrated rotation angle. Never derive an angle from
    // `elapsed * speed(t)` — a changing speed makes the angle jump.
    mut spin: Local<f32>,
    mut dots: Query<(&FormationDot, &mut Transform, &mut Sprite)>,
) {
    let weight = weights.scenes[FORMATION];
    if weight <= 0.001 {
        return;
    }
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    let palette = palettes.of(FORMATION);
    // A new figure every 14 s, morphing over 3 s.
    let cycle = t / 14.0;
    let shape = cycle as usize;
    let morph = ((cycle.fract() * 14.0) / 3.0).min(1.0);
    let morph = morph * morph * (3.0 - 2.0 * morph);

    // Motion follows the slow envelope only: smooth swell, no stutter.
    *spin += dt * (0.22 + 0.18 * pulse.slow);
    let tilt = 0.45 + 0.25 * (t * 0.13).sin();
    let scale = 250.0 * (1.0 + 0.12 * pulse.slow);
    let (sy, cy) = spin.sin_cos();
    let (sx, cx) = tilt.sin_cos();

    for (dot, mut tf, mut sprite) in &mut dots {
        let a = figure_point(shape, dot.idx, FORMATION_DOTS, t);
        let b = figure_point(shape + 1, dot.idx, FORMATION_DOTS, t);
        let p = a.lerp(b, morph);
        // Rotate around Y then X, then perspective-project.
        let p = Vec3::new(p.x * cy + p.z * sy, p.y, -p.x * sy + p.z * cy);
        let p = Vec3::new(p.x, p.y * cx - p.z * sx, p.y * sx + p.z * cx);
        let persp = 2.6 / (2.6 + p.z);
        tf.translation.x = p.x * scale * persp;
        tf.translation.y = p.y * scale * persp;
        tf.translation.z = -p.z; // painter's order inside the scene
        let size = (4.6 * persp).clamp(1.5, 7.0);
        sprite.custom_size = Some(Vec2::splat(size));
        let depth_fade = ((persp - 0.55) * 1.6).clamp(0.15, 1.0);
        let alpha = weight * depth_fade * (0.6 + 0.4 * pulse.slow.min(1.0));
        sprite.color =
            emissive(palette.formation[dot.idx % 4], 2.4).with_alpha(alpha);
    }
}

// ---------------------------------------------------------------------------
// CYBER: glyph rain + drifting source code
// ---------------------------------------------------------------------------

fn animate_cyber(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    mut columns: Query<(&CodeColumn, &mut Transform, &mut Text2d, &mut TextColor), Without<CodeBlock>>,
    mut blocks: Query<(&CodeBlock, &mut Transform, &mut TextColor), (With<CodeBlock>, Without<CodeColumn>)>,
) {
    let weight = weights.scenes[CYBER];
    if weight <= 0.001 {
        return;
    }
    let dt = time.delta_secs();
    let palette = palettes.of(CYBER);
    let speed_mul = 0.7 + 0.6 * pulse.slow;
    let mut rng = rand::rng();
    for (col, mut tf, mut text, mut color) in &mut columns {
        tf.translation.y -= col.speed * speed_mul * dt;
        if tf.translation.y < -640.0 {
            tf.translation.y = rng.random_range(420.0..640.0);
            **text = random_column_text(&mut rng);
        }
        color.0 = emissive(palette.glyph, 1.5)
            .with_alpha(weight * (0.28 + 0.25 * pulse.slow.min(1.0)));
    }
    for (block, mut tf, mut color) in &mut blocks {
        tf.translation.x -= block.speed * speed_mul * dt;
        if tf.translation.x < -900.0 {
            tf.translation.x = rng.random_range(750.0..1000.0);
        }
        color.0 = emissive(palette.code, 1.2).with_alpha(weight * 0.3);
    }
}

// ---------------------------------------------------------------------------
// GALAXY: spiral of square stars + shooting stars
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn animate_galaxy(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    mut commands: Commands,
    mut timer: ResMut<ShootingTimer>,
    // Integrated disc rotation (see animate_formation's spin note).
    mut turn: Local<f32>,
    mut stars: Query<(&GalaxyStar, &mut Transform, &mut Sprite), Without<GalaxyArt>>,
    mut art: Query<&mut Sprite, (With<GalaxyArt>, Without<GalaxyStar>)>,
    mut shooting: Query<
        (Entity, &mut ShootingStar, &mut Transform, &mut Sprite),
        (Without<GalaxyStar>, Without<GalaxyArt>),
    >,
) {
    let weight = weights.scenes[GALAXY];
    let t = time.elapsed_secs();
    let dt = time.delta_secs();

    // Shooting stars live in world space and finish their run even while
    // the scene fades out.
    for (entity, mut star, mut tf, mut sprite) in &mut shooting {
        star.life -= dt;
        if star.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        tf.translation.x += star.vel.x * dt;
        tf.translation.y += star.vel.y * dt;
        sprite.color.set_alpha((star.life / 0.9).min(1.0) * 0.8);
    }

    if let Ok(mut sprite) = art.single_mut() {
        sprite.color.set_alpha(weight);
    }
    if weight <= 0.001 {
        return;
    }

    let palette = palettes.of(GALAXY);
    *turn += dt * (0.02 + 0.015 * pulse.slow);
    for (star, mut tf, mut sprite) in &mut stars {
        let a = star.angle + *turn * (1.0 + 60.0 / star.radius);
        tf.translation.x = a.cos() * star.radius;
        tf.translation.y = a.sin() * star.radius * 0.55; // inclined disc
        let twinkle = 0.55 + 0.45 * (t * 2.6 + star.phase).sin().abs();
        let alpha = weight * twinkle * (0.5 + 0.35 * pulse.slow.min(1.0));
        sprite.color = emissive(palette.galaxy[star.kind], 1.6).with_alpha(alpha);
        sprite.custom_size = Some(Vec2::splat(star.size * (0.85 + 0.3 * pulse.slow)));
    }

    if timer.0.tick(time.delta()).is_finished() {
        let mut rng = rand::rng();
        timer.0 = Timer::from_seconds(rng.random_range(3.0..8.0), TimerMode::Once);
        let from_left = rng.random_bool(0.5);
        let x = if from_left { -700.0 } else { 700.0 };
        let vx = if from_left { 1.0 } else { -1.0 } * rng.random_range(700.0..1000.0);
        commands.spawn((
            Sprite::from_color(
                emissive(Color::srgb(0.9, 0.95, 1.0), 2.2),
                Vec2::new(26.0, 3.0),
            ),
            Transform::from_xyz(x, rng.random_range(-100.0..340.0), -23.0)
                .with_rotation(Quat::from_rotation_z(if from_left { -0.28 } else { 0.28 })),
            ShootingStar {
                vel: Vec2::new(vx, -rng.random_range(180.0..300.0)),
                life: 0.9,
            },
        ));
    }
}

// ---------------------------------------------------------------------------
// VISUALIZER: EQ bars + waveform
// ---------------------------------------------------------------------------

fn animate_visualizer(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    mut bars: Query<(&EqBar, &mut Transform, &mut Sprite), Without<WaveDot>>,
    mut dots: Query<(&WaveDot, &mut Transform, &mut Sprite), Without<EqBar>>,
) {
    let weight = weights.scenes[VISUALIZER];
    if weight <= 0.001 {
        return;
    }
    let t = time.elapsed_secs();
    let palette = palettes.of(VISUALIZER);
    for (bar, mut tf, mut sprite) in &mut bars {
        let h = 8.0 + pulse.bands[bar.band] * 120.0;
        let size = sprite.custom_size.unwrap_or(Vec2::new(40.0, 8.0));
        sprite.custom_size = Some(Vec2::new(size.x, h));
        tf.translation.y = if bar.top {
            366.0 - h / 2.0
        } else {
            -366.0 + h / 2.0
        };
        let f = bar.band as f32 / (BANDS - 1) as f32;
        let color = palette.eq.0.mix(&palette.eq.1, f);
        sprite.color =
            emissive(color, 1.3).with_alpha(weight * if bar.top { 0.14 } else { 0.26 });
    }
    for (dot, mut tf, mut sprite) in &mut dots {
        let f = dot.idx as f32 / WAVE_DOTS as f32;
        let band = pulse.bands[(dot.idx * BANDS / WAVE_DOTS).min(BANDS - 1)];
        let y = (f * 21.0 + t * 3.2).sin() * (10.0 + 70.0 * band)
            + (f * 9.0 - t * 1.4).sin() * 8.0;
        tf.translation.y = y;
        sprite.color = emissive(palette.wave, 1.4).with_alpha(weight * 0.3);
    }
}
