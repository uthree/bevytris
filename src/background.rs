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
    pub bands: [f32; BANDS],
}

impl Default for AudioPulse {
    fn default() -> Self {
        AudioPulse {
            energy: 0.15,
            bands: [0.0; BANDS],
        }
    }
}

const SCENE_COUNT: usize = 4;
const FORMATION: usize = 0;
const CYBER: usize = 1;
const GALAXY: usize = 2;
const VISUALIZER: usize = 3;

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
    timer: Timer,
    /// Dev pin (`BEVYTRIS_SCENE=formation|cyber|galaxy|visualizer`).
    pinned: bool,
}

/// Per-scene display weight (0..1), refreshed every frame.
#[derive(Resource, Default)]
struct SceneWeights([f32; SCENE_COUNT]);

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
            .insert_resource(SceneState {
                active: start,
                prev: start,
                fade: 1.0,
                timer: Timer::from_seconds(rand::rng().random_range(40.0..70.0), TimerMode::Once),
                pinned,
            })
            .add_systems(Startup, setup_background)
            .add_systems(
                Update,
                (
                    update_audio_pulse,
                    update_scene_state,
                    animate_stars,
                    animate_formation,
                    animate_cyber,
                    animate_galaxy,
                    animate_visualizer,
                ),
            );
    }
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Star {
    speed: f32,
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
struct CodeLine {
    speed: f32,
}

#[derive(Component)]
struct GalaxyStar {
    angle: f32,
    radius: f32,
    phase: f32,
    size: f32,
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
const CODE_LINES: usize = 7;
const GALAXY_STARS: usize = 300;
const WAVE_DOTS: usize = 72;

/// Real lines from this repository, drifting through the cyber scene.
const CODE_SNIPPETS: [&str; 12] = [
    "let ease = t * t * (3.0 - 2.0 * t);",
    "self.gravity_acc += dt / sec_per_row.max(1e-6);",
    "let difficult = lines == 4 || tspin.is_some();",
    "attack = self.cancel_incoming(attack);",
    "for &rot in distinct_rotations(kind) {",
    "zone.charge = (zone.charge + charge_gain).min(1.0);",
    "while board.fits(&piece.shifted(0, -1)) {",
    "if front_filled == 2 || kick == 4 {",
    "let seed: u64 = rand::rng().random();",
    "score += 2 * distance as u64;",
    "nodes.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));",
    "fn t_spin_kind(board, piece, last_kick)",
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

    // Global falling starfield (shared by every scene).
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
            for i in 0..CODE_LINES {
                parent.spawn((
                    Text2d::new(CODE_SNIPPETS[i % CODE_SNIPPETS.len()]),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(emissive(Color::srgb(0.3, 0.75, 0.9), 1.2)),
                    Transform::from_xyz(
                        rng.random_range(-640.0..640.0),
                        -340.0 + i as f32 * 110.0 + rng.random_range(-20.0..20.0),
                        -0.5,
                    ),
                    CodeLine {
                        speed: rng.random_range(25.0..70.0),
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
                let core = t < 0.25;
                let color = if core {
                    Color::srgb(1.0, 0.9, 0.7)
                } else if rng.random_bool(0.25) {
                    Color::srgb(0.7, 0.5, 1.0)
                } else {
                    Color::srgb(0.6, 0.8, 1.0)
                };
                parent.spawn((
                    Sprite::from_color(
                        emissive(color, 1.6),
                        Vec2::splat(rng.random_range(1.5..4.5)),
                    ),
                    Transform::default(),
                    GalaxyStar {
                        angle,
                        radius,
                        phase: rng.random_range(0.0..std::f32::consts::TAU),
                        size: rng.random_range(1.5..4.5),
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
                        Sprite::from_color(emissive(color, 1.8), Vec2::new(w - 6.0, 10.0)),
                        Transform::from_xyz(x, if top { 366.0 } else { -366.0 }, 0.0),
                        EqBar { band, top },
                    ));
                }
            }
            for idx in 0..WAVE_DOTS {
                parent.spawn((
                    Sprite::from_color(emissive(Color::srgb(0.5, 0.95, 1.0), 1.8), Vec2::splat(5.0)),
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

fn update_scene_state(
    time: Res<Time>,
    mut state: ResMut<SceneState>,
    mut weights: ResMut<SceneWeights>,
    mut roots: Query<(&SceneRoot, &mut Visibility)>,
) {
    let dt = time.delta_secs();
    state.fade = (state.fade + dt / FADE_SECS).min(1.0);
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
    }

    let ease = state.fade * state.fade * (3.0 - 2.0 * state.fade);
    weights.0 = [0.0; SCENE_COUNT];
    weights.0[state.prev] = 1.0 - ease;
    weights.0[state.active] = ease;

    for (root, mut vis) in &mut roots {
        let target = if weights.0[root.0] > 0.001 {
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
    mut surge: ResMut<StarSurge>,
    mut query: Query<(&Star, &mut Transform)>,
) {
    let dt = time.delta_secs();
    surge.0 += (1.0 - surge.0) * (3.0 * dt).min(1.0);
    for (star, mut tf) in &mut query {
        tf.translation.y -= star.speed * surge.0 * dt;
        if tf.translation.y < -380.0 {
            tf.translation.y = 380.0;
        }
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
    mut dots: Query<(&FormationDot, &mut Transform, &mut Sprite)>,
) {
    let weight = weights.0[FORMATION];
    if weight <= 0.001 {
        return;
    }
    let t = time.elapsed_secs();
    // A new figure every 14 s, morphing over 3 s.
    let cycle = t / 14.0;
    let shape = cycle as usize;
    let morph = ((cycle.fract() * 14.0) / 3.0).min(1.0);
    let morph = morph * morph * (3.0 - 2.0 * morph);

    let spin = t * (0.25 + 0.25 * pulse.energy);
    let tilt = 0.45 + 0.25 * (t * 0.13).sin();
    let scale = 250.0 * (1.0 + 0.18 * pulse.energy);
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
        sprite.color.set_alpha(weight * depth_fade * (0.55 + 0.45 * pulse.energy.min(1.0)));
    }
}

// ---------------------------------------------------------------------------
// CYBER: glyph rain + drifting source code
// ---------------------------------------------------------------------------

fn animate_cyber(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    mut columns: Query<(&CodeColumn, &mut Transform, &mut Text2d, &mut TextColor), Without<CodeLine>>,
    mut lines: Query<(&CodeLine, &mut Transform, &mut TextColor), (With<CodeLine>, Without<CodeColumn>)>,
) {
    let weight = weights.0[CYBER];
    if weight <= 0.001 {
        return;
    }
    let dt = time.delta_secs();
    let speed_mul = 0.7 + 0.8 * pulse.energy;
    let mut rng = rand::rng();
    for (col, mut tf, mut text, mut color) in &mut columns {
        tf.translation.y -= col.speed * speed_mul * dt;
        if tf.translation.y < -640.0 {
            tf.translation.y = rng.random_range(420.0..640.0);
            **text = random_column_text(&mut rng);
        }
        color.0.set_alpha(weight * (0.28 + 0.3 * pulse.energy.min(1.0)));
    }
    for (line, mut tf, mut color) in &mut lines {
        tf.translation.x -= line.speed * speed_mul * dt;
        if tf.translation.x < -820.0 {
            tf.translation.x = rng.random_range(700.0..900.0);
        }
        color.0.set_alpha(weight * 0.22);
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
    mut commands: Commands,
    mut timer: ResMut<ShootingTimer>,
    mut stars: Query<(&GalaxyStar, &mut Transform, &mut Sprite), Without<GalaxyArt>>,
    mut art: Query<&mut Sprite, (With<GalaxyArt>, Without<GalaxyStar>)>,
    mut shooting: Query<
        (Entity, &mut ShootingStar, &mut Transform, &mut Sprite),
        (Without<GalaxyStar>, Without<GalaxyArt>),
    >,
) {
    let weight = weights.0[GALAXY];
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

    let turn = t * (0.02 + 0.02 * pulse.energy);
    for (star, mut tf, mut sprite) in &mut stars {
        let a = star.angle + turn * (1.0 + 60.0 / star.radius);
        tf.translation.x = a.cos() * star.radius;
        tf.translation.y = a.sin() * star.radius * 0.55; // inclined disc
        let twinkle = 0.55 + 0.45 * (t * 2.6 + star.phase).sin().abs();
        sprite.color.set_alpha(weight * twinkle * (0.5 + 0.4 * pulse.energy.min(1.0)));
        sprite.custom_size = Some(Vec2::splat(star.size * (0.8 + 0.4 * pulse.energy)));
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
    mut bars: Query<(&EqBar, &mut Transform, &mut Sprite), Without<WaveDot>>,
    mut dots: Query<(&WaveDot, &mut Transform, &mut Sprite), Without<EqBar>>,
) {
    let weight = weights.0[VISUALIZER];
    if weight <= 0.001 {
        return;
    }
    let t = time.elapsed_secs();
    for (bar, mut tf, mut sprite) in &mut bars {
        let h = 10.0 + pulse.bands[bar.band] * 240.0;
        let size = sprite.custom_size.unwrap_or(Vec2::new(40.0, 10.0));
        sprite.custom_size = Some(Vec2::new(size.x, h));
        tf.translation.y = if bar.top {
            366.0 - h / 2.0
        } else {
            -366.0 + h / 2.0
        };
        sprite
            .color
            .set_alpha(weight * if bar.top { 0.30 } else { 0.5 });
    }
    for (dot, mut tf, mut sprite) in &mut dots {
        let f = dot.idx as f32 / WAVE_DOTS as f32;
        let band = pulse.bands[(dot.idx * BANDS / WAVE_DOTS).min(BANDS - 1)];
        let y = (f * 21.0 + t * 3.2).sin() * (14.0 + 130.0 * band)
            + (f * 9.0 - t * 1.4).sin() * 10.0;
        tf.translation.y = y;
        sprite.color.set_alpha(weight * 0.5);
    }
}
