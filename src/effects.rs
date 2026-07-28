//! Juice: particles, screen shake, flashes, floating banners, confetti and
//! the scrolling starfield background. Also maps game events to SFX.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rand::Rng;
use std::collections::HashMap;

use crate::audio::{PlaySfx, Sfx};
use crate::core::board::{BOARD_WIDTH, VISIBLE_HEIGHT};
use crate::core::game::{ClearKind, GameEvent};
use crate::emissive;
use crate::render::{BoardKick, BoardTheme, FrameGlow};
use crate::session::{BoardEvent, BoardIndex, GameSession, SessionResult};
use crate::state::{AppState, PlayState};

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraShake>()
            .init_resource::<StarSurge>()
            .add_systems(Startup, (setup_effect_assets, spawn_starfield))
            .add_systems(
                Update,
                (
                    map_events_to_effects.run_if(in_state(AppState::Playing)),
                    update_particles,
                    update_banners,
                    update_flashes,
                    update_shockwaves,
                    update_streaks,
                    update_starfield,
                    drift_background,
                ),
            )
            .add_systems(OnEnter(PlayState::Finished), finish_fanfare)
            .add_systems(
                PostUpdate,
                apply_camera_shake.before(TransformSystems::Propagate),
            );
    }
}

// ---------------------------------------------------------------------------
// Generated effect textures & background art
// ---------------------------------------------------------------------------

#[derive(Resource)]
pub struct EffectAssets {
    /// Soft radial gradient used by particles.
    pub glow: Handle<Image>,
    /// Feather-edged rectangle used by streaks (drop trails, row bars):
    /// stays crisp and rectangular when stretched, unlike the radial glow.
    pub bar: Handle<Image>,
}

#[derive(Component)]
struct BackgroundArt;

fn image_from_alpha(size: u32, alpha: impl Fn(f32, f32) -> f32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    let max = (size - 1) as f32;
    for y in 0..size {
        for x in 0..size {
            // Normalized coordinates in -1..1.
            let nx = x as f32 / max * 2.0 - 1.0;
            let ny = y as f32 / max * 2.0 - 1.0;
            let a = alpha(nx, ny).clamp(0.0, 1.0);
            data.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// 64x64 white radial gradient with soft falloff.
fn radial_glow_image() -> Image {
    image_from_alpha(64, |nx, ny| {
        let d = Vec2::new(nx, ny).length();
        (1.0 - d).clamp(0.0, 1.0).powi(2)
    })
}

/// Rectangle with a flat core and feathered edges on both axes.
fn soft_rect_image() -> Image {
    let edge = |n: f32| {
        // 1.0 in the core, smooth falloff over the outer 35%.
        let t = ((1.0 - n.abs()) / 0.35).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    image_from_alpha(64, move |nx, ny| edge(nx) * edge(ny))
}

fn setup_effect_assets(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
) {
    commands.insert_resource(EffectAssets {
        glow: images.add(radial_glow_image()),
        bar: images.add(soft_rect_image()),
    });
    // CC0 space painting (Westbeam, see assets/CREDITS.md), dimmed so the
    // boards stay the brightest thing on screen.
    commands.spawn((
        Sprite {
            image: asset_server.load("images/space_bg.png"),
            custom_size: Some(Vec2::new(1560.0, 1170.0)),
            color: Color::srgb(0.5, 0.5, 0.62),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, -30.0),
        BackgroundArt,
    ));
}

/// Slow parallax drift so the backdrop feels alive.
fn drift_background(time: Res<Time>, mut query: Query<&mut Transform, With<BackgroundArt>>) {
    let t = time.elapsed_secs();
    for mut tf in &mut query {
        tf.translation.x = 14.0 * (t * 0.031).sin();
        tf.translation.y = 10.0 * (t * 0.023).cos();
    }
}

// ---------------------------------------------------------------------------
// Camera shake
// ---------------------------------------------------------------------------

#[derive(Resource, Default)]
pub struct CameraShake {
    trauma: f32,
}

impl CameraShake {
    pub fn add(&mut self, amount: f32) {
        self.trauma = (self.trauma + amount).min(1.0);
    }
}

fn apply_camera_shake(
    time: Res<Time>,
    mut shake: ResMut<CameraShake>,
    mut cameras: Query<&mut Transform, With<Camera2d>>,
) {
    // Smooth shake: sum of incommensurate sine waves instead of per-frame
    // random jumps — the camera sways instead of vibrating.
    let t = time.elapsed_secs();
    let s = shake.trauma * shake.trauma;
    for mut tf in &mut cameras {
        if s > 0.0001 {
            let x = (t * 31.7).sin() + 0.55 * (t * 57.3 + 1.1).sin();
            let y = (t * 27.1 + 0.7).sin() + 0.55 * (t * 47.9 + 2.3).sin();
            tf.translation.x = x * s * 14.0;
            tf.translation.y = y * s * 14.0;
            tf.rotation = Quat::from_rotation_z((t * 23.3 + 0.4).sin() * s * 0.022);
        } else {
            tf.translation.x = 0.0;
            tf.translation.y = 0.0;
            tf.rotation = Quat::IDENTITY;
        }
    }
    shake.trauma = (shake.trauma - 1.6 * time.delta_secs()).max(0.0);
}

// ---------------------------------------------------------------------------
// Particles
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Particle {
    vel: Vec2,
    gravity: f32,
    damping: f32,
    spin: f32,
    life: f32,
    max_life: f32,
}

#[allow(clippy::too_many_arguments)]
fn spawn_burst(
    commands: &mut Commands,
    glow: &Handle<Image>,
    center: Vec2,
    color: Color,
    count: usize,
    speed: f32,
    size: f32,
    life: f32,
    gravity: f32,
) {
    let mut rng = rand::rng();
    for _ in 0..count {
        let angle = rng.random_range(0.0..std::f32::consts::TAU);
        let v = rng.random_range(0.3..1.0) * speed;
        // Glow textures read much softer than solid quads, so scale up and
        // push the color into HDR for bloom.
        let s = rng.random_range(0.5..1.2) * size * 2.6;
        commands.spawn((
            Sprite {
                image: glow.clone(),
                custom_size: Some(Vec2::splat(s)),
                color: emissive(color, 3.0),
                ..default()
            },
            Transform::from_translation(center.extend(20.0))
                .with_rotation(Quat::from_rotation_z(rng.random_range(0.0..3.14))),
            Particle {
                vel: Vec2::new(angle.cos(), angle.sin()) * v,
                gravity,
                damping: 2.2,
                spin: rng.random_range(-8.0..8.0),
                life,
                max_life: life,
            },
            DespawnOnExit(AppState::Playing),
        ));
    }
}

// ---------------------------------------------------------------------------
// Streaks: light bars for cleared rows and hard-drop trails
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Streak {
    life: f32,
    max_life: f32,
    /// Vertical collapse (cleared rows) vs pure fade (drop trails).
    collapse: bool,
}

fn spawn_streak(
    commands: &mut Commands,
    glow: &Handle<Image>,
    center: Vec2,
    size: Vec2,
    color: Color,
    boost: f32,
    life: f32,
    collapse: bool,
) {
    commands.spawn((
        Sprite {
            image: glow.clone(),
            custom_size: Some(size),
            color: emissive(color, boost),
            ..default()
        },
        Transform::from_translation(center.extend(14.0)),
        Streak {
            life,
            max_life: life,
            collapse,
        },
        DespawnOnExit(AppState::Playing),
    ));
}

fn update_streaks(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut Streak, &mut Transform, &mut Sprite)>,
) {
    for (entity, mut streak, mut tf, mut sprite) in &mut query {
        streak.life -= time.delta_secs();
        if streak.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let fade = (streak.life / streak.max_life).clamp(0.0, 1.0);
        sprite.color.set_alpha(fade);
        if streak.collapse {
            // Squash vertically while widening a touch: reads as the row
            // being vaporized.
            tf.scale.y = fade;
            tf.scale.x = 1.0 + (1.0 - fade) * 0.12;
        }
    }
}

fn update_particles(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut Particle, &mut Transform, &mut Sprite)>,
) {
    let dt = time.delta_secs();
    for (entity, mut p, mut tf, mut sprite) in &mut query {
        p.life -= dt;
        if p.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let damping = p.damping;
        let gravity = p.gravity;
        p.vel *= 1.0 - damping * dt;
        p.vel.y -= gravity * dt;
        tf.translation.x += p.vel.x * dt;
        tf.translation.y += p.vel.y * dt;
        tf.rotate_z(p.spin * dt);
        let a = (p.life / p.max_life).clamp(0.0, 1.0);
        sprite.color.set_alpha(a);
    }
}

// ---------------------------------------------------------------------------
// Floating banners ("TETRIS!", "T-SPIN DOUBLE!" ...)
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Banner {
    life: f32,
    max_life: f32,
}

fn spawn_banner(commands: &mut Commands, pos: Vec2, text: String, color: Color, size: f32) {
    // Small jitter so back-to-back banners don't stack into one blob.
    let mut rng = rand::rng();
    let pos = pos + Vec2::new(rng.random_range(-14.0..14.0), rng.random_range(-8.0..8.0));
    commands.spawn((
        Text2d::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(emissive(color, 1.7)),
        Transform::from_translation(pos.extend(40.0)).with_scale(Vec3::splat(0.3)),
        Banner {
            life: 1.2,
            max_life: 1.2,
        },
        DespawnOnExit(AppState::Playing),
    ));
}

fn update_banners(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut Banner, &mut Transform, &mut TextColor)>,
) {
    let dt = time.delta_secs();
    for (entity, mut b, mut tf, mut color) in &mut query {
        b.life -= dt;
        if b.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let t = 1.0 - b.life / b.max_life;
        // Pop in, drift up, fade out.
        let scale = if t < 0.15 { 0.3 + t / 0.15 * 0.9 } else { 1.2 - (t - 0.15) * 0.15 };
        tf.scale = Vec3::splat(scale);
        tf.translation.y += 34.0 * dt;
        let alpha = if t > 0.6 { 1.0 - (t - 0.6) / 0.4 } else { 1.0 };
        color.0.set_alpha(alpha);
    }
}

// ---------------------------------------------------------------------------
// Fullscreen flashes
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Flash {
    life: f32,
    max_life: f32,
    peak_alpha: f32,
}

fn spawn_flash(commands: &mut Commands, color: Color, peak_alpha: f32, life: f32) {
    commands.spawn((
        Sprite::from_color(color, Vec2::new(4000.0, 3000.0)),
        Transform::from_xyz(0.0, 0.0, 80.0),
        Flash {
            life,
            max_life: life,
            peak_alpha,
        },
        DespawnOnExit(AppState::Playing),
    ));
}

fn update_flashes(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut Flash, &mut Sprite)>,
) {
    for (entity, mut f, mut sprite) in &mut query {
        f.life -= time.delta_secs();
        if f.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        sprite
            .color
            .set_alpha(f.peak_alpha * (f.life / f.max_life));
    }
}

// ---------------------------------------------------------------------------
// Shockwave rings (playfield-friendly replacement for fullscreen flashes:
// thin expanding outlines never obscure the stack)
// ---------------------------------------------------------------------------

#[derive(Component)]
struct Shockwave {
    center: Vec2,
    radius: f32,
    speed: f32,
    life: f32,
    max_life: f32,
    color: Color,
    rings: u32,
}

fn spawn_shockwave(commands: &mut Commands, center: Vec2, color: Color, big: bool) {
    let (radius, speed, life, rings) = if big {
        (30.0, 950.0, 0.5, 3)
    } else {
        (20.0, 550.0, 0.3, 2)
    };
    commands.spawn((
        Shockwave {
            center,
            radius,
            speed,
            life,
            max_life: life,
            color,
            rings,
        },
        DespawnOnExit(AppState::Playing),
    ));
}

fn update_shockwaves(
    time: Res<Time>,
    mut commands: Commands,
    mut gizmos: Gizmos,
    mut query: Query<(Entity, &mut Shockwave)>,
) {
    let dt = time.delta_secs();
    for (entity, mut wave) in &mut query {
        wave.life -= dt;
        if wave.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        wave.radius += wave.speed * dt;
        wave.speed *= 1.0 - 1.5 * dt; // ease out
        let fade = (wave.life / wave.max_life).clamp(0.0, 1.0);
        for i in 0..wave.rings {
            let r = wave.radius - i as f32 * 7.0;
            if r <= 1.0 {
                continue;
            }
            let alpha = fade * (1.0 - i as f32 * 0.28);
            gizmos
                .circle_2d(
                    wave.center,
                    r,
                    emissive(wave.color, 3.2).with_alpha(alpha),
                )
                .resolution(72);
        }
    }
}

// ---------------------------------------------------------------------------
// Starfield background
// ---------------------------------------------------------------------------

/// Temporary starfield speed multiplier; spikes on spectacular clears so the
/// whole background lunges without ever covering the boards.
#[derive(Resource)]
struct StarSurge(f32);

impl Default for StarSurge {
    fn default() -> Self {
        StarSurge(1.0)
    }
}

#[derive(Component)]
struct Star {
    speed: f32,
}

fn spawn_starfield(mut commands: Commands) {
    let mut rng = rand::rng();
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
}

fn update_starfield(
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
// Event mapping
// ---------------------------------------------------------------------------

/// World-space center of a board cell.
fn cell_world(board_tf: &Transform, theme: &BoardTheme, x: i8, y: i8) -> Vec2 {
    let h = theme.cell * VISIBLE_HEIGHT as f32;
    Vec2::new(
        board_tf.translation.x + (x as f32 + 0.5) * theme.cell
            - theme.cell * BOARD_WIDTH as f32 / 2.0,
        board_tf.translation.y + (y as f32 + 0.5) * theme.cell - h / 2.0,
    )
}

fn map_events_to_effects(
    mut commands: Commands,
    mut events: MessageReader<BoardEvent>,
    mut sfx: MessageWriter<PlaySfx>,
    mut shake: ResMut<CameraShake>,
    mut surge: ResMut<StarSurge>,
    mut glows: Query<&mut FrameGlow>,
    mut kicks: Query<&mut BoardKick>,
    boards: Query<(&Transform, &BoardTheme, &BoardIndex, &GameSession)>,
    assets: Res<EffectAssets>,
    // Hard-drop distance per board, consumed by the following Locked event
    // to draw the fall trail.
    mut drop_distance: Local<HashMap<Entity, i8>>,
) {
    let glow_tex = assets.glow.clone();
    let bar_tex = assets.bar.clone();
    let pulse_frame = |glows: &mut Query<&mut FrameGlow>, board: Entity, color: Color, strength: f32| {
        if let Ok(mut glow) = glows.get_mut(board) {
            glow.color = color;
            glow.t = glow.t.max(strength.min(1.0));
        }
    };
    let mut banner_stagger = 0.0f32;
    for msg in events.read() {
        let Ok((board_tf, theme, index, session)) = boards.get(msg.board) else {
            continue;
        };
        // The CPU's sounds are quieter so the player's actions stay crisp.
        let gain = if index.0 == 0 { 1.0 } else { 0.4 };
        let center = Vec2::new(board_tf.translation.x, board_tf.translation.y);
        let mut play = |s: Sfx, g: f32| {
            sfx.write(PlaySfx { sfx: s, gain: g });
        };

        // Input feedback is for the player's own board only — the CPU issues
        // inputs far too fast for per-input feedback to read well.
        let own_input = index.0 == 0;
        match &msg.event {
            GameEvent::Moved { dx } => {
                play(Sfx::Move, 0.5 * gain);
                if own_input {
                    shake.add(0.06);
                    if let Ok(mut kick) = kicks.get_mut(msg.board) {
                        kick.impulse(Vec2::new(*dx as f32 * 16.0, 0.0));
                    }
                }
            }
            GameEvent::MoveBlocked { dx } => {
                // Pressing into the wall: the board leans that way and stays
                // leaned while the key is held (impulses repeat at ARR rate).
                if own_input {
                    if let Ok(mut kick) = kicks.get_mut(msg.board) {
                        kick.impulse(Vec2::new(*dx as f32 * 46.0, 0.0));
                    }
                }
            }
            GameEvent::Rotated { kicked, cw, kick } => {
                play(Sfx::Rotate, if *kicked { 1.0 } else { 0.7 } * gain);
                if own_input {
                    shake.add(if *kicked { 0.10 } else { 0.07 });
                    if let Ok(mut k) = kicks.get_mut(msg.board) {
                        // Reaction torque: piece spins CW, board recoils CCW.
                        k.spin(if *cw { 0.95 } else { -0.95 });
                        // Wall kicks shove the piece; the board recoils the
                        // opposite way.
                        if kick.0 != 0 {
                            k.impulse(Vec2::new(-kick.0 as f32 * 34.0, 0.0));
                        }
                    }
                }
            }
            GameEvent::RotationFailed => play(Sfx::RotateFail, 0.35 * gain),
            GameEvent::SoftDropStep => {
                play(Sfx::SoftDropTick, 0.25 * gain);
                if own_input {
                    shake.add(0.03);
                }
            }
            GameEvent::HardDrop { distance } => {
                play(Sfx::HardDrop, gain);
                drop_distance.insert(msg.board, *distance);
                if own_input {
                    shake.add(0.10 + *distance as f32 * 0.003);
                    if let Ok(mut kick) = kicks.get_mut(msg.board) {
                        // The board takes the hit and dips.
                        kick.impulse(Vec2::new(0.0, -(85.0 + *distance as f32 * 3.0)));
                    }
                }
            }
            GameEvent::Locked { piece } => {
                play(Sfx::Lock, 0.8 * gain);
                // Dust puff under the locked piece.
                for (x, y) in piece.board_cells() {
                    if y < VISIBLE_HEIGHT {
                        spawn_burst(
                            &mut commands,
                            &glow_tex,
                            cell_world(board_tf, theme, x, y),
                            Color::srgba(0.9, 0.9, 1.0, 0.8),
                            2,
                            60.0,
                            3.0,
                            0.3,
                            60.0,
                        );
                    }
                }
                // Light trail down the columns the piece just fell through.
                if let Some(distance) = drop_distance.remove(&msg.board) {
                    if distance >= 3 {
                        let cells = piece.board_cells();
                        let color = {
                            let (r, g, b) = piece.kind.color();
                            Color::srgb(r, g, b)
                        };
                        for &(x, _) in &cells {
                            let top = cells
                                .iter()
                                .filter(|c| c.0 == x)
                                .map(|c| c.1)
                                .max()
                                .unwrap_or(0);
                            let from = (top + 1).min(VISIBLE_HEIGHT - 1);
                            let to = (top + distance).min(VISIBLE_HEIGHT - 1);
                            if to <= from {
                                continue;
                            }
                            let a = cell_world(board_tf, theme, x, from);
                            let b = cell_world(board_tf, theme, x, to);
                            spawn_streak(
                                &mut commands,
                                &bar_tex,
                                (a + b) / 2.0,
                                Vec2::new(theme.cell * 0.8, (b.y - a.y).abs() + theme.cell),
                                color.with_alpha(0.55),
                                2.4,
                                0.22,
                                false,
                            );
                        }
                    }
                }
            }
            GameEvent::Held => play(Sfx::Hold, gain),
            GameEvent::HoldBlocked => play(Sfx::HoldFail, 0.5 * gain),
            GameEvent::TSpinNoLines { mini } => {
                play(Sfx::TSpin, 0.7 * gain);
                let label = if *mini { "T-SPIN MINI" } else { "T-SPIN" };
                spawn_banner(
                    &mut commands,
                    center + Vec2::new(0.0, 40.0 + banner_stagger),
                    label.to_string(),
                    Color::srgb(0.85, 0.4, 1.0),
                    theme.cell,
                );
                banner_stagger += 44.0;
            }
            GameEvent::Cleared(clear) => {
                play(Sfx::Clear(clear.lines), gain);
                let is_tspin = clear.kind != ClearKind::Normal;
                if is_tspin {
                    play(Sfx::TSpin, gain);
                }
                if clear.b2b {
                    play(Sfx::B2b, 0.9 * gain);
                }
                if clear.combo >= 1 {
                    play(Sfx::Combo(clear.combo), gain);
                }
                if clear.perfect_clear {
                    play(Sfx::PerfectClear, gain);
                }

                // Shake & spectacle effects scale with how big the clear is.
                // Deliberately NO fullscreen flash here: tinting the whole
                // playfield made the stack hard to read mid-game. Instead:
                // frame glow + expanding ring + starfield surge, all of
                // which stay out of the playfield.
                let spectacle = clear.lines as f32
                    + if is_tspin { 2.0 } else { 0.0 }
                    + if clear.perfect_clear { 4.0 } else { 0.0 };
                if index.0 == 0 || spectacle >= 4.0 {
                    shake.add(0.06 * spectacle + 0.06);
                }
                let ring_color = if clear.perfect_clear {
                    Color::srgb(1.0, 0.95, 0.4)
                } else if is_tspin {
                    Color::srgb(0.85, 0.4, 1.0)
                } else if clear.lines == 4 {
                    Color::srgb(0.35, 0.85, 1.0)
                } else {
                    Color::srgb(0.9, 0.95, 1.0)
                };
                pulse_frame(&mut glows, msg.board, ring_color, 0.25 + 0.14 * spectacle);
                spawn_shockwave(&mut commands, center, ring_color, spectacle >= 4.0);
                if clear.perfect_clear {
                    // Double ring + hard background lunge for the big one.
                    spawn_shockwave(&mut commands, center, Color::WHITE, true);
                    surge.0 = 9.0;
                } else if spectacle >= 4.0 {
                    surge.0 = surge.0.max(5.5);
                }

                // Each cleared row: a collapsing light bar plus a spray of
                // glowing particles.
                let spray = if spectacle >= 4.0 { 6 } else { 4 };
                for &row in &clear.rows {
                    if row >= VISIBLE_HEIGHT {
                        continue;
                    }
                    let row_center = Vec2::new(
                        center.x,
                        cell_world(board_tf, theme, 0, row).y,
                    );
                    spawn_streak(
                        &mut commands,
                        &bar_tex,
                        row_center,
                        Vec2::new(theme.cell * BOARD_WIDTH as f32 * 1.06, theme.cell * 1.7),
                        ring_color,
                        3.5,
                        0.3,
                        true,
                    );
                    for x in 0..BOARD_WIDTH {
                        let pos = cell_world(board_tf, theme, x, row);
                        spawn_burst(
                            &mut commands,
                            &glow_tex,
                            pos,
                            ring_color,
                            spray,
                            240.0,
                            theme.cell * 0.24,
                            0.55,
                            240.0,
                        );
                    }
                }

                // Banner text.
                let mut label = match (clear.kind, clear.lines) {
                    (ClearKind::Normal, 1) => "SINGLE".to_string(),
                    (ClearKind::Normal, 2) => "DOUBLE".to_string(),
                    (ClearKind::Normal, 3) => "TRIPLE".to_string(),
                    (ClearKind::Normal, _) => "TETRIS!".to_string(),
                    (ClearKind::TSpin, n) => format!(
                        "T-SPIN {}",
                        ["", "SINGLE", "DOUBLE", "TRIPLE"][n.min(3) as usize]
                    ),
                    (ClearKind::TSpinMini, n) => format!(
                        "T-SPIN MINI {}",
                        ["", "SINGLE", "DOUBLE", ""][n.min(3) as usize]
                    ),
                };
                if clear.b2b {
                    label = format!("B2B {label}");
                }
                let color = if clear.perfect_clear {
                    Color::srgb(1.0, 0.95, 0.4)
                } else if is_tspin {
                    Color::srgb(0.9, 0.5, 1.0)
                } else if clear.lines == 4 {
                    Color::srgb(0.4, 0.9, 1.0)
                } else {
                    Color::srgb(0.95, 0.95, 1.0)
                };
                let size = if clear.lines >= 4 || is_tspin {
                    theme.cell * 1.15
                } else {
                    theme.cell * 0.8
                };
                spawn_banner(
                    &mut commands,
                    center + Vec2::new(0.0, 30.0 + banner_stagger),
                    label,
                    color,
                    size,
                );
                banner_stagger += 44.0;
                if clear.combo >= 2 {
                    spawn_banner(
                        &mut commands,
                        center + Vec2::new(0.0, 30.0 + banner_stagger),
                        format!("{} COMBO", clear.combo),
                        Color::srgb(1.0, 0.8, 0.3),
                        theme.cell * 0.7,
                    );
                    banner_stagger += 40.0;
                }
                if clear.perfect_clear {
                    spawn_banner(
                        &mut commands,
                        center + Vec2::new(0.0, 90.0 + banner_stagger),
                        "PERFECT CLEAR!".to_string(),
                        Color::srgb(1.0, 0.95, 0.4),
                        theme.cell * 1.3,
                    );
                    banner_stagger += 50.0;
                }
                // Warn the opponent about incoming garbage.
                if clear.attack > 0 && boards.iter().count() > 1 {
                    play(Sfx::GarbageWarn, 0.8);
                }
            }
            GameEvent::LevelUp { level } => {
                play(Sfx::LevelUp, gain);
                if index.0 == 0 {
                    shake.add(0.15);
                    let green = Color::srgb(0.5, 1.0, 0.6);
                    pulse_frame(&mut glows, msg.board, green, 0.8);
                    spawn_shockwave(&mut commands, center, green, true);
                    surge.0 = surge.0.max(4.0);
                    spawn_banner(
                        &mut commands,
                        center + Vec2::new(0.0, 80.0),
                        format!("LEVEL {level}"),
                        green,
                        theme.cell,
                    );
                }
            }
            GameEvent::GarbageRose { rows } => {
                play(Sfx::GarbageRise, gain);
                pulse_frame(
                    &mut glows,
                    msg.board,
                    Color::srgb(1.0, 0.2, 0.2),
                    0.5 + *rows as f32 * 0.06,
                );
                // The rising garbage shoves the board upward (both boards —
                // it reads as the stack physically pushing in).
                if let Ok(mut kick) = kicks.get_mut(msg.board) {
                    kick.impulse(Vec2::new(0.0, 55.0 + *rows as f32 * 6.0));
                }
                if index.0 == 0 {
                    shake.add(0.1 + *rows as f32 * 0.04);
                }
            }
            GameEvent::TopOut => {
                play(Sfx::GameOver, gain);
                shake.add(0.55);
                if let Ok(mut kick) = kicks.get_mut(msg.board) {
                    kick.impulse(Vec2::new(0.0, -150.0));
                    kick.spin(1.4);
                }
                spawn_flash(&mut commands, Color::srgb(1.0, 0.3, 0.2), 0.3, 0.6);
                // Board "explodes": scatter particles over the whole field.
                for y in 0..VISIBLE_HEIGHT {
                    for x in 0..BOARD_WIDTH {
                        if session.game.board.cell(x, y).is_some() && (x + y) % 2 == 0 {
                            spawn_burst(
                                &mut commands,
                                &glow_tex,
                                cell_world(board_tf, theme, x, y),
                                Color::srgb(0.7, 0.7, 0.75),
                                1,
                                340.0,
                                theme.cell * 0.3,
                                1.1,
                                420.0,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// Confetti + fanfare when a VS match ends with a human win.
fn finish_fanfare(
    mut commands: Commands,
    result: Option<Res<SessionResult>>,
    boards: Query<(&Transform, &BoardTheme, &BoardIndex), With<GameSession>>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    let Some(result) = result else { return };
    if let SessionResult::VsWin { winner } = *result {
        if winner == 0 {
            sfx.write(PlaySfx::new(Sfx::Win));
        }
        for (tf, theme, index) in &boards {
            if index.0 != winner {
                continue;
            }
            let mut rng = rand::rng();
            let h = theme.cell * VISIBLE_HEIGHT as f32;
            for _ in 0..140 {
                let color = Color::srgb(
                    rng.random_range(0.4..1.0),
                    rng.random_range(0.4..1.0),
                    rng.random_range(0.4..1.0),
                );
                let pos = Vec2::new(
                    tf.translation.x + rng.random_range(-160.0..160.0),
                    tf.translation.y + h / 2.0 + rng.random_range(0.0..120.0),
                );
                commands.spawn((
                    Sprite::from_color(emissive(color, 1.9), Vec2::new(6.0, 10.0)),
                    Transform::from_translation(pos.extend(25.0)),
                    Particle {
                        vel: Vec2::new(rng.random_range(-40.0..40.0), rng.random_range(-30.0..10.0)),
                        gravity: 140.0,
                        damping: 0.4,
                        spin: rng.random_range(-10.0..10.0),
                        life: rng.random_range(1.6..3.0),
                        max_life: 3.0,
                    },
                    DespawnOnExit(AppState::Playing),
                ));
            }
        }
    }
}
