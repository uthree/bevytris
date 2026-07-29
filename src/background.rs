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
//! * GARDEN, FRACTAL, SUNSET, AUTOMATON — plants, a Sierpinski carpet
//!   zoom, a pixel sunset, and Conway's Life.
//! * PIANOROLL — the score the auto-composer is generating right now,
//!   scrolling past a fixed playhead.
//!
//! Scenes crossfade on a 40-70 s timer (random next pick). Everything
//! breathes with [`AudioPulse`], which is fed by the synthesizer's own
//! envelope plus every sound effect the game plays — so the visualizer
//! bars really are the notes you are hearing. The classic falling
//! starfield stays on across all scenes (and still lunges via
//! [`StarSurge`] on big clears).

use bevy::prelude::*;
use bevytris_chiptune::Voice;
use rand::Rng;

use crate::audio::PlaySfx;
use crate::emissive;
use crate::music::{MusicEngine, ScoreFeed};
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

const SCENE_COUNT: usize = 9;
const FORMATION: usize = 0;
const CYBER: usize = 1;
const GALAXY: usize = 2;
const VISUALIZER: usize = 3;
const GARDEN: usize = 4;
const FRACTAL: usize = 5;
const SUNSET: usize = 6;
const AUTOMATON: usize = 7;
const PIANOROLL: usize = 8;

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
    /// Full-screen backdrop gradient (top, bottom) so the base color
    /// isn't eternally navy.
    bg: (Color, Color),
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
        bg: (Color::srgb(0.04, 0.03, 0.11), Color::srgb(0.02, 0.06, 0.11)),
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
        bg: (Color::srgb(0.10, 0.03, 0.05), Color::srgb(0.08, 0.04, 0.02)),
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
        bg: (Color::srgb(0.02, 0.07, 0.04), Color::srgb(0.01, 0.05, 0.05)),
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
        bg: (Color::srgb(0.04, 0.06, 0.12), Color::srgb(0.06, 0.09, 0.14)),
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
        bg: (Color::srgb(0.09, 0.03, 0.11), Color::srgb(0.05, 0.04, 0.12)),
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
            Ok("garden") => (GARDEN, true),
            Ok("fractal") => (FRACTAL, true),
            Ok("sunset") => (SUNSET, true),
            Ok("automaton") => (AUTOMATON, true),
            Ok("pianoroll") => (PIANOROLL, true),
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
                    animate_backdrops,
                    animate_formation,
                    animate_cyber,
                    animate_galaxy,
                    animate_visualizer,
                    animate_garden,
                    animate_fractal,
                    animate_sunset,
                    animate_automaton,
                    animate_pianoroll,
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

/// Palette-tinted full-screen gradient behind a scene's content, so the
/// base color isn't the same navy in every scene.
#[derive(Component)]
struct SceneBackdrop {
    scene: usize,
    bottom: bool,
}

#[derive(Component)]
struct GardenDot {
    plant: usize,
    root: Vec2,
    rel: Vec2,
    /// 0..1 order of appearance while the plant grows.
    birth: f32,
    /// 0..1 distance from the root; deeper parts sway more.
    depth: f32,
    /// 0 stem, 1 leaf, 2 flower.
    kind: u8,
}

#[derive(Component)]
struct CarpetCell {
    /// Center in unit carpet space (-0.5..0.5).
    pos: Vec2,
    /// Edge length in unit space.
    size: f32,
    depth: u8,
}

#[derive(Component)]
struct SkyBand(usize);

#[derive(Component)]
struct SunsetSun;

#[derive(Component)]
struct SunsetCloud {
    speed: f32,
}

#[derive(Component)]
struct MountainBar;

#[derive(Component)]
struct LifeCell {
    idx: usize,
    alpha: f32,
}

/// Conway's Game of Life state for the AUTOMATON scene.
#[derive(Resource)]
struct LifeGrid {
    w: usize,
    h: usize,
    cells: Vec<bool>,
    prev: Vec<bool>,
    step_timer: f32,
    reseed_timer: f32,
}

const LIFE_W: usize = 60;
const LIFE_H: usize = 34;
const LIFE_CELL: f32 = 22.0;

/// Muted sunset sky gradient, top of the screen to the horizon.
const SKY_COLORS: [(f32, f32, f32); 8] = [
    (0.10, 0.05, 0.20),
    (0.16, 0.07, 0.22),
    (0.25, 0.09, 0.22),
    (0.38, 0.12, 0.21),
    (0.52, 0.16, 0.19),
    (0.68, 0.24, 0.16),
    (0.82, 0.34, 0.15),
    (0.92, 0.48, 0.18),
];

struct GardenSeed {
    plant: usize,
    root: Vec2,
    rel: Vec2,
    birth: f32,
    depth: f32,
    kind: u8,
}

/// Grow a row of branching plants (stems as chains of dots, leaves on
/// the way, a bloom at every tip).
fn garden_seeds(rng: &mut impl Rng) -> Vec<GardenSeed> {
    let mut out = Vec::new();
    let roots = [-560.0, -420.0, -180.0, 150.0, 420.0, 560.0];
    for (plant, &rx) in roots.iter().enumerate() {
        let root = Vec2::new(rx + rng.random_range(-25.0..25.0), -372.0);
        let main_len = rng.random_range(260.0..420.0);
        let mut stack = vec![(
            Vec2::ZERO,
            std::f32::consts::FRAC_PI_2 + rng.random_range(-0.1..0.1),
            main_len,
            0u32,
            0.0f32,
        )];
        while let Some((start, mut angle, len, level, dist0)) = stack.pop() {
            let mut pos = start;
            let steps = ((len / 11.0) as usize).max(2);
            for s in 0..steps {
                pos += Vec2::from_angle(angle) * 11.0;
                angle += rng.random_range(-0.1..0.1);
                let dist = dist0 + (s + 1) as f32 * 11.0;
                let reach = (dist / (main_len * 1.5)).min(1.0);
                out.push(GardenSeed {
                    plant,
                    root,
                    rel: pos,
                    birth: reach * 0.9,
                    depth: reach,
                    kind: 0,
                });
                if level >= 1 && rng.random_bool(0.25) {
                    let side = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
                    out.push(GardenSeed {
                        plant,
                        root,
                        rel: pos + Vec2::from_angle(angle + side * 1.3) * 9.0,
                        birth: (reach * 0.9 + 0.03).min(1.0),
                        depth: reach,
                        kind: 1,
                    });
                }
                if level < 2 && len > 70.0 && rng.random_bool(0.14) {
                    let side = if rng.random_bool(0.5) { 1.0 } else { -1.0 };
                    stack.push((
                        pos,
                        angle + side * rng.random_range(0.35..0.7),
                        len * 0.55,
                        level + 1,
                        dist,
                    ));
                }
            }
            out.push(GardenSeed {
                plant,
                root,
                rel: pos,
                birth: ((dist0 + len) / (main_len * 1.5) * 0.92).min(0.98),
                depth: 1.0,
                kind: 2,
            });
        }
    }
    out
}

/// Sierpinski carpet cells: 8 blocks per level, recursively.
fn carpet_cells(out: &mut Vec<(Vec2, f32, u8)>, center: Vec2, size: f32, depth: u8) {
    for dy in -1..=1i32 {
        for dx in -1..=1i32 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let c = center + Vec2::new(dx as f32, dy as f32) * size / 3.0;
            out.push((c, size / 3.0, depth));
            if depth < 3 {
                carpet_cells(out, c, size / 3.0, depth + 1);
            }
        }
    }
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

    // --- GARDEN ----------------------------------------------------------
    commands
        .spawn((
            SceneRoot(GARDEN),
            Transform::from_xyz(0.0, 0.0, -21.0),
            Visibility::default(),
        ))
        .with_children(|parent| {
            for seed in garden_seeds(&mut rng) {
                parent.spawn((
                    Sprite::from_color(Color::NONE, Vec2::splat(4.0)),
                    Transform::default(),
                    GardenDot {
                        plant: seed.plant,
                        root: seed.root,
                        rel: seed.rel,
                        birth: seed.birth,
                        depth: seed.depth,
                        kind: seed.kind,
                    },
                ));
            }
        });

    // --- FRACTAL ---------------------------------------------------------
    let mut cells = Vec::with_capacity(584);
    carpet_cells(&mut cells, Vec2::ZERO, 1.0, 1);
    commands
        .spawn((
            SceneRoot(FRACTAL),
            Transform::from_xyz(0.0, 0.0, -23.0),
            Visibility::default(),
        ))
        .with_children(|parent| {
            for (pos, size, depth) in cells {
                parent.spawn((
                    Sprite::from_color(Color::NONE, Vec2::splat(4.0)),
                    Transform::default(),
                    CarpetCell { pos, size, depth },
                ));
            }
        });

    // --- SUNSET ----------------------------------------------------------
    commands
        .spawn((
            SceneRoot(SUNSET),
            Transform::from_xyz(0.0, 0.0, -24.0),
            Visibility::default(),
        ))
        .with_children(|parent| {
            // Gradient sky bands, top to horizon.
            for (i, &(r, g, b)) in SKY_COLORS.iter().enumerate() {
                parent.spawn((
                    Sprite::from_color(
                        Color::srgba(r, g, b, 0.0),
                        Vec2::new(1420.0, 96.0),
                    ),
                    Transform::from_xyz(0.0, 340.0 - (i as f32 + 0.5) * 92.0, -3.0),
                    SkyBand(i),
                ));
            }
            // The square sun, slowly setting behind the ridge.
            parent.spawn((
                Sprite::from_color(emissive(Color::srgb(1.0, 0.72, 0.32), 1.9), Vec2::splat(88.0)),
                Transform::from_xyz(-170.0, 120.0, -2.5),
                SunsetSun,
            ));
            // Drifting clouds.
            for _ in 0..6 {
                parent.spawn((
                    Sprite::from_color(
                        Color::srgba(1.0, 0.62, 0.5, 0.0),
                        Vec2::new(rng.random_range(120.0..230.0), rng.random_range(10.0..18.0)),
                    ),
                    Transform::from_xyz(
                        rng.random_range(-720.0..720.0),
                        rng.random_range(60.0..320.0),
                        -2.2,
                    ),
                    SunsetCloud {
                        speed: rng.random_range(6.0..18.0),
                    },
                ));
            }
            // Two ridges of pixel mountains: bars anchored to the bottom.
            for (layer, (color, base, amp, width, z)) in [
                (Color::srgb(0.14, 0.07, 0.18), 140.0, 90.0, 46.0, -1.6),
                (Color::srgb(0.05, 0.03, 0.09), 80.0, 60.0, 52.0, -0.9),
            ]
            .into_iter()
            .enumerate()
            {
                let count = (1480.0 / width) as usize + 1;
                for i in 0..count {
                    let fi = i as f32 + layer as f32 * 0.5;
                    let h = base
                        + amp * ((fi * 1.31).sin() + 0.6 * (fi * 0.47 + 1.3).sin()).abs();
                    parent.spawn((
                        Sprite::from_color(color.with_alpha(0.0), Vec2::new(width, h)),
                        Transform::from_xyz(
                            -740.0 + (i as f32 + 0.5) * width,
                            -370.0 + h / 2.0,
                            z,
                        ),
                        MountainBar,
                    ));
                }
            }
        });

    // --- AUTOMATON -------------------------------------------------------
    let mut life_cells = vec![false; LIFE_W * LIFE_H];
    for cell in life_cells.iter_mut() {
        *cell = rng.random_bool(0.18);
    }
    commands.insert_resource(LifeGrid {
        w: LIFE_W,
        h: LIFE_H,
        prev: life_cells.clone(),
        cells: life_cells,
        step_timer: 0.3,
        reseed_timer: 18.0,
    });
    commands
        .spawn((
            SceneRoot(AUTOMATON),
            Transform::from_xyz(0.0, 0.0, -25.5),
            Visibility::default(),
        ))
        .with_children(|parent| {
            for idx in 0..LIFE_W * LIFE_H {
                let x = (idx % LIFE_W) as f32;
                let y = (idx / LIFE_W) as f32;
                parent.spawn((
                    Sprite::from_color(Color::NONE, Vec2::splat(LIFE_CELL - 3.0)),
                    Transform::from_xyz(
                        (x + 0.5) * LIFE_CELL - LIFE_W as f32 * LIFE_CELL / 2.0,
                        (y + 0.5) * LIFE_CELL - LIFE_H as f32 * LIFE_CELL / 2.0,
                        0.0,
                    ),
                    LifeCell { idx, alpha: 0.0 },
                ));
            }
        });

    // PIANOROLL — the composer's own score, scrolling right to left past
    // a fixed playhead. Horizontal on purpose: tetrominoes already own
    // the vertical axis, and falling notes make the piece look like it is
    // drifting sideways. It also survives the opaque board slab, since
    // streaks pass behind it and come out the other side.
    commands
        .spawn((
            SceneRoot(PIANOROLL),
            Transform::from_xyz(0.0, 0.0, -26.0),
            Visibility::default(),
        ))
        .with_children(|parent| {
            // Notes and nothing else. There is no grid, no row banding and
            // no playhead line: a piano roll is already the most
            // structured thing on screen, and behind a board that the
            // player is reading closely, all that scaffolding just
            // competes. The note-on flash marks where "now" is on its own.
            for i in 0..ROLL_NOTES {
                parent.spawn((
                    Sprite::from_color(Color::NONE, Vec2::ZERO),
                    Transform::from_xyz(0.0, 0.0, 0.1 + 0.01 * (i % 4) as f32),
                    RollPart(i),
                ));
            }
        });

    // Palette-tinted backdrop gradients (sunset paints its own sky).
    for scene in [
        FORMATION, CYBER, GALAXY, VISUALIZER, GARDEN, FRACTAL, AUTOMATON, PIANOROLL,
    ] {
        commands.spawn((
            Sprite::from_color(Color::NONE, Vec2::new(1440.0, 800.0)),
            Transform::from_xyz(0.0, 0.0, -29.0),
            SceneBackdrop {
                scene,
                bottom: false,
            },
        ));
        commands.spawn((
            Sprite::from_color(Color::NONE, Vec2::new(1440.0, 400.0)),
            Transform::from_xyz(0.0, -200.0, -28.9),
            SceneBackdrop {
                scene,
                bottom: true,
            },
        ));
    }

    commands.insert_resource(ShootingTimer(Timer::from_seconds(5.0, TimerMode::Once)));
}

// ---------------------------------------------------------------------------
// Audio pulse & scene switching
// ---------------------------------------------------------------------------

fn update_audio_pulse(
    time: Res<Time>,
    engine: Res<MusicEngine>,
    feed: Res<ScoreFeed>,
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
    // The synthesizer publishes its own envelope, so the background now
    // reacts to the music itself and not just to a proxy.
    let music = engine.link.energy().min(1.0);
    pulse.energy = (pulse.energy.max(0.35 * music) + hit).min(1.6);
    // Fast decay toward a gentle idle breath, so quiet menus still move.
    let idle = 0.12 + 0.05 * (t * 0.8).sin().abs();
    pulse.energy += (idle - pulse.energy) * (2.2 * dt).min(1.0);
    // The motion-side envelope follows slowly in both directions.
    let energy = pulse.energy;
    pulse.slow += (energy - pulse.slow) * (1.2 * dt).min(1.0);

    // Real spectrum: twelve pitch classes in two registers, taken from
    // the notes actually sounding right now. A sine wobble stands in
    // underneath so quiet menus still have something to show.
    let mut real = [0.0f32; BANDS];
    for n in &feed.notes {
        // Drums carry no pitch and would all pile into one band.
        if n.voice >= Voice::Perc as u8 || n.at > feed.pos || feed.pos >= n.end {
            continue;
        }
        let idx = (n.midi % 12) as usize + if n.midi >= 60 { 12 } else { 0 };
        real[idx.min(BANDS - 1)] += n.vel as f32 / 127.0;
    }
    for (i, band) in pulse.bands.iter_mut().enumerate() {
        let fi = i as f32;
        let wobble = ((t * (2.1 + fi * 0.37)).sin() * (t * (1.3 + fi * 0.11) + fi).cos()).abs();
        let floor = (0.12 + wobble) * (0.35 + energy) * 0.5;
        let target = real[i].min(1.4).max(floor);
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

/// Palette-tinted gradients behind each scene's content.
fn animate_backdrops(
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    mut backdrops: Query<(&SceneBackdrop, &mut Sprite)>,
) {
    for (backdrop, mut sprite) in &mut backdrops {
        let weight = weights.scenes[backdrop.scene];
        let palette = palettes.of(backdrop.scene);
        let color = if backdrop.bottom {
            palette.bg.1
        } else {
            palette.bg.0
        };
        sprite.color = color.with_alpha(weight);
    }
}

// ---------------------------------------------------------------------------
// GARDEN: plants growing, swaying and blooming
// ---------------------------------------------------------------------------

fn animate_garden(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    mut dots: Query<(&GardenDot, &mut Transform, &mut Sprite)>,
) {
    let weight = weights.scenes[GARDEN];
    if weight <= 0.001 {
        return;
    }
    let t = time.elapsed_secs();
    let palette = palettes.of(GARDEN);
    // Grow for half the cycle, hold in bloom, wither, regrow.
    let cycle = (t / 30.0).fract();
    let grow = (cycle / 0.5).min(1.0);
    let fade = if cycle > 0.86 {
        1.0 - (cycle - 0.86) / 0.14
    } else {
        1.0
    };

    for (dot, mut tf, mut sprite) in &mut dots {
        if dot.birth >= grow {
            sprite.color.set_alpha(0.0);
            continue;
        }
        // Wind: the whole plant leans, deeper parts lean further.
        let sway = (t * 0.9 + dot.root.x * 0.012).sin() * (0.045 + 0.05 * pulse.slow) * dot.depth;
        let rel = Vec2::from_angle(sway).rotate(dot.rel);
        let pos = dot.root + rel;
        tf.translation.x = pos.x;
        tf.translation.y = pos.y;
        tf.translation.z = dot.kind as f32 * 0.1;

        let pop = ((grow - dot.birth) / 0.06).clamp(0.0, 1.0);
        let (color, size, boost, base_alpha) = match dot.kind {
            0 => (
                Color::srgb(0.25, 0.55, 0.3).mix(&Color::srgb(0.4, 0.8, 0.4), dot.depth),
                4.0,
                1.2,
                0.7,
            ),
            1 => (Color::srgb(0.45, 0.9, 0.45), 5.5, 1.3, 0.75),
            _ => (palette.formation[dot.plant % 4], 7.5, 1.9, 0.95),
        };
        let size = if dot.kind == 2 {
            size * (1.0 + 0.12 * (t * 2.6 + dot.rel.x).sin())
        } else {
            size
        };
        sprite.custom_size = Some(Vec2::splat(size));
        sprite.color = emissive(color, boost)
            .with_alpha(weight * fade * base_alpha * (0.4 + 0.6 * pop));
    }
}

// ---------------------------------------------------------------------------
// FRACTAL: Sierpinski carpet, endlessly zooming
// ---------------------------------------------------------------------------

fn animate_fractal(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    // Integrated zoom/rotation (see animate_formation's spin note).
    mut zoom: Local<f32>,
    mut rot: Local<f32>,
    mut cells: Query<(&CarpetCell, &mut Transform, &mut Sprite)>,
) {
    let weight = weights.scenes[FRACTAL];
    if weight <= 0.001 {
        return;
    }
    let dt = time.delta_secs();
    let palette = palettes.of(FRACTAL);
    *zoom += dt * (0.045 + 0.03 * pulse.slow);
    *rot += dt * 0.04;
    // The carpet is self-similar at 3x: zooming by 3 lands on itself,
    // so the phase wraps seamlessly.
    let scale = 760.0 * 3f32.powf(zoom.fract());
    let rotv = Vec2::from_angle(*rot);

    for (cell, mut tf, mut sprite) in &mut cells {
        let pos = rotv.rotate(cell.pos) * scale;
        tf.translation.x = pos.x;
        tf.translation.y = pos.y;
        tf.translation.z = cell.depth as f32 * 0.01;
        let px = cell.size * scale;
        sprite.custom_size = Some(Vec2::splat(px * 0.86));
        // Fade blocks that outgrew the frame or shrank into grain; keep
        // the whole thing translucent — it's a backdrop, not a wall.
        let big_fade = ((620.0 - px) / 260.0).clamp(0.0, 1.0);
        let small_fade = ((px - 2.0) / 6.0).clamp(0.0, 1.0);
        let f = cell.depth as f32 / 3.0;
        let color = palette.eq.0.mix(&palette.eq.1, f);
        sprite.color = emissive(color, 1.1).with_alpha(
            weight * 0.19 * big_fade * small_fade * (0.7 + 0.3 * pulse.slow.min(1.0)),
        );
    }
}

// ---------------------------------------------------------------------------
// SUNSET: gradient sky, setting sun, ridges and clouds
// ---------------------------------------------------------------------------

fn animate_sunset(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    mut bands: Query<
        (&SkyBand, &mut Sprite),
        (Without<SunsetSun>, Without<SunsetCloud>, Without<MountainBar>),
    >,
    mut sun: Query<
        (&mut Transform, &mut Sprite),
        (With<SunsetSun>, Without<SkyBand>, Without<SunsetCloud>, Without<MountainBar>),
    >,
    mut clouds: Query<
        (&SunsetCloud, &mut Transform, &mut Sprite),
        (Without<SunsetSun>, Without<SkyBand>, Without<MountainBar>),
    >,
    mut mountains: Query<
        &mut Sprite,
        (With<MountainBar>, Without<SunsetSun>, Without<SkyBand>, Without<SunsetCloud>),
    >,
) {
    let weight = weights.scenes[SUNSET];
    if weight <= 0.001 {
        return;
    }
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    let warm = 0.85 + 0.12 * pulse.slow.min(1.0);
    for (band, mut sprite) in &mut bands {
        let (r, g, b) = SKY_COLORS[band.0];
        sprite.color = Color::srgba(r * warm, g * warm, b * warm, weight * 0.9);
    }
    // The sun takes a long minute to sink behind the ridge.
    if let Ok((mut tf, mut sprite)) = sun.single_mut() {
        let cycle = (t / 70.0).fract();
        tf.translation.y = 300.0 - 540.0 * cycle;
        sprite.color = emissive(Color::srgb(1.0, 0.72, 0.32), 1.6 + 0.5 * pulse.slow)
            .with_alpha(weight);
    }
    for (cloud, mut tf, mut sprite) in &mut clouds {
        tf.translation.x += cloud.speed * (0.6 + 0.8 * pulse.slow) * dt;
        if tf.translation.x > 780.0 {
            tf.translation.x = -780.0;
        }
        sprite.color.set_alpha(weight * 0.3);
    }
    for mut sprite in &mut mountains {
        sprite.color.set_alpha(weight);
    }
}

// ---------------------------------------------------------------------------
// AUTOMATON: Conway's Game of Life
// ---------------------------------------------------------------------------

fn animate_automaton(
    time: Res<Time>,
    pulse: Res<AudioPulse>,
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    mut grid: ResMut<LifeGrid>,
    mut cells: Query<(&mut LifeCell, &mut Sprite)>,
) {
    let weight = weights.scenes[AUTOMATON];
    if weight <= 0.001 {
        return;
    }
    let dt = time.delta_secs();
    let palette = palettes.of(AUTOMATON);

    grid.step_timer -= dt;
    grid.reseed_timer -= dt;
    let stepped = grid.step_timer <= 0.0;
    if stepped {
        // Faster generations while the action is loud.
        grid.step_timer = 0.30 - 0.13 * pulse.slow.min(1.0);
        let (w, h) = (grid.w, grid.h);
        let grid = &mut *grid; // split field borrows through the ResMut
        grid.prev.copy_from_slice(&grid.cells);
        let mut population = 0usize;
        for y in 0..h {
            for x in 0..w {
                let mut neighbors = 0;
                for dy in [h - 1, 0, 1] {
                    for dx in [w - 1, 0, 1] {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        if grid.prev[(y + dy) % h * w + (x + dx) % w] {
                            neighbors += 1;
                        }
                    }
                }
                let alive = matches!(
                    (grid.prev[y * w + x], neighbors),
                    (true, 2) | (true, 3) | (false, 3)
                );
                grid.cells[y * w + x] = alive;
                population += alive as usize;
            }
        }
        // Dying board (or just time): drop in a few random soups so the
        // dish never goes still.
        if population < w * h / 50 || grid.reseed_timer <= 0.0 {
            grid.reseed_timer = 18.0;
            let mut rng = rand::rng();
            for _ in 0..3 {
                let cx = rng.random_range(0..w);
                let cy = rng.random_range(0..h);
                for dy in 0..5 {
                    for dx in 0..5 {
                        if rng.random_bool(0.5) {
                            let i = (cy + dy) % h * w + (cx + dx) % w;
                            grid.cells[i] = true;
                        }
                    }
                }
            }
        }
    }

    for (mut cell, mut sprite) in &mut cells {
        let alive = grid.cells[cell.idx];
        if stepped && alive && !grid.prev[cell.idx] {
            cell.alpha = 0.95; // newborn flash
        }
        let target = if alive { 0.5 } else { 0.0 };
        cell.alpha += (target - cell.alpha) * (9.0 * dt).min(1.0);
        sprite.color = emissive(palette.glyph, 1.4).with_alpha(cell.alpha * weight);
    }
}

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


// ---------------------------------------------------------------------------
// PIANOROLL
// ---------------------------------------------------------------------------

/// A pooled note rectangle for the piano roll.
#[derive(Component)]
struct RollPart(usize);

/// Lowest drawn pitch (A1) — the triangle bass bottoms out around here.
const ROLL_LOW_MIDI: i32 = 33;
const ROLL_ROWS: usize = 56;
const ROLL_ROW_H: f32 = 12.0;
/// Eight voices at up to sixteen notes a bar over four bars of visible
/// score; the AUTOMATON scene already ships 2040 sprites, so there is
/// plenty of headroom.
const ROLL_NOTES: usize = 512;
/// Scroll rate. Sized so the visible future is a little under the
/// composer's lookahead, otherwise notes pop in at the right edge.
const ROLL_PX_PER_SEC: f32 = 240.0;
/// Where "now" sits: left of centre, so most of the screen is the music
/// that has not happened yet.
const ROLL_PLAYHEAD_X: f32 = -150.0;
const ROLL_DRUM_Y: f32 = -348.0;

fn row_y(row: usize) -> f32 {
    -336.0 + row as f32 * ROLL_ROW_H + ROLL_ROW_H / 2.0
}

/// What one pool slot should draw this frame.
#[derive(Clone, Copy)]
struct RollDraw {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: Color,
    alpha: f32,
    glow: f32,
}

fn animate_pianoroll(
    weights: Res<SceneWeights>,
    palettes: Res<ScenePalettes>,
    pulse: Res<AudioPulse>,
    surge: Res<StarSurge>,
    feed: Res<ScoreFeed>,
    mut parts: Query<(&RollPart, &mut Transform, &mut Sprite)>,
) {
    let weight = weights.scenes[PIANOROLL];
    if weight <= 0.001 {
        return;
    }
    let palette = palettes.of(PIANOROLL);
    // Brightness may follow the spiky energy; nothing that MOVES may.
    let glow_boost = 1.0 + 0.35 * (surge.0 - 1.0).clamp(0.0, 1.5);
    let global = weight * (0.9 + 0.25 * pulse.slow.min(1.0));

    // Lay the visible notes out into pool slots.
    let mut draws: [Option<RollDraw>; ROLL_NOTES] = [None; ROLL_NOTES];
    let mut slot = 0usize;
    for n in &feed.notes {
        if slot >= ROLL_NOTES {
            break;
        }
        let x0 = ROLL_PLAYHEAD_X + feed.secs_from_now(n.at) * ROLL_PX_PER_SEC;
        let x1 = ROLL_PLAYHEAD_X + feed.secs_from_now(n.end) * ROLL_PX_PER_SEC;
        // A sixteenth at 140 BPM is 107 ms; below a couple of pixels the
        // sliver just flickers.
        let w = (x1 - x0).max(14.0);
        let cx = x0 + w / 2.0;
        if cx + w / 2.0 < -660.0 || cx - w / 2.0 > 660.0 {
            continue;
        }
        let drum = n.voice >= Voice::Perc as u8;
        let y = if drum {
            ROLL_DRUM_Y
        } else {
            let row = (n.midi as i32 - ROLL_LOW_MIDI).rem_euclid(ROLL_ROWS as i32) as usize;
            row_y(row)
        };
        let vel = 0.55 + 0.45 * (n.vel as f32 / 127.0);
        let playing = n.at <= feed.pos && feed.pos < n.end;
        let past = feed.pos >= n.end;
        // With no grid behind them the notes carry the whole scene, so
        // they sit a little brighter than they used to. The note-on flash
        // is the only bloom source, and the column of flashes it makes as
        // notes cross is what tells you where "now" is.
        let (base_alpha, glow) = if playing {
            (0.62 * vel, 2.4 * glow_boost)
        } else if past {
            (0.10 * vel, 1.0)
        } else {
            (0.20 * vel, 1.0)
        };
        // Fade at both edges so nothing pops in or out.
        let edge = ((cx + 640.0) / 170.0).clamp(0.0, 1.0) * ((640.0 - cx) / 130.0).clamp(0.0, 1.0);
        draws[slot] = Some(RollDraw {
            x: cx,
            y,
            w: if drum { 9.0 } else { w },
            h: if drum { 9.0 } else { 9.0 * (0.75 + 0.25 * n.vel as f32 / 127.0) },
            color: palette.formation[voice_hue(n.voice)],
            alpha: base_alpha * edge * global,
            glow,
        });
        slot += 1;
    }

    for (part, mut tf, mut sprite) in &mut parts {
        match draws[part.0] {
            Some(d) => {
                tf.translation.x = d.x;
                tf.translation.y = d.y;
                sprite.custom_size = Some(Vec2::new(d.w, d.h));
                sprite.color = emissive(d.color, d.glow).with_alpha(d.alpha);
            }
            None => {
                // Cheaper than hiding the entity, and keeps the pool in
                // one archetype.
                sprite.color = Color::NONE;
            }
        }
    }
}

/// Eight voices share the palette's four tints, grouped by role so
/// related lines read as one part: melody, harmony, low end, percussion.
fn voice_hue(voice: u8) -> usize {
    match voice {
        v if v == Voice::Lead as u8 || v == Voice::Counter as u8 => 0,
        v if v == Voice::Harmony as u8 || v == Voice::Wave as u8 => 1,
        v if v == Voice::Bass as u8 || v == Voice::Saw as u8 => 2,
        _ => 3,
    }
}
