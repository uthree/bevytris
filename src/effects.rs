//! Juice: particles, screen shake, flashes, floating banners, confetti and
//! the scrolling starfield background. Also maps game events to SFX.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rand::Rng;
use std::collections::HashMap;

use bevy::audio::Volume;

use crate::audio::{PlaySfx, Sfx, SfxBank};
use crate::background::StarSurge;
use crate::config::GameSettings;
use crate::core::board::{BOARD_WIDTH, VISIBLE_HEIGHT};
use crate::core::game::{ClearKind, GARBAGE_CAP_PER_PIECE, GameEvent};
use crate::emissive;
use crate::render::{BoardKick, BoardTheme, FrameGlow};
use crate::session::{BoardEvent, BoardIndex, GameSession, SessionResult};
use crate::state::{AppState, GameMode, PlayState};

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraShake>()
            .init_resource::<DangerLevel>()
            .add_systems(Startup, setup_effect_assets)
            .add_systems(
                Update,
                (
                    update_danger,
                    sync_danger_vignette,
                    sync_danger_alarm,
                    zone_presentation,
                )
                    .run_if(in_state(AppState::Playing)),
            )
            .add_systems(
                Update,
                (
                    map_events_to_effects.run_if(in_state(AppState::Playing)),
                    update_particles,
                    update_banners,
                    update_zone_tally,
                    update_flashes,
                    update_shockwaves,
                    update_streaks,
                ),
            )
            .add_systems(OnEnter(PlayState::Finished), finish_fanfare)
            .add_systems(Update, run_celebration)
            .add_systems(OnExit(PlayState::Finished), |mut commands: Commands| {
                commands.remove_resource::<Celebration>()
            })
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
    /// Soft-edged square used by particles (the whole game is styled on
    /// rectangles — no round shapes).
    pub glow: Handle<Image>,
    /// Feather-edged rectangle used by streaks (drop trails, row bars):
    /// stays crisp and rectangular when stretched, unlike the radial glow.
    pub bar: Handle<Image>,
    /// Screen-edge vignette (transparent center) for the danger overlay.
    pub vignette: Handle<Image>,
}

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

/// 64x64 white square with a flat core and a short feathered edge: the
/// particle sprite. Tighter falloff than the streak rectangle so small
/// particles still read as squares, not blobs.
fn square_glow_image() -> Image {
    let edge = |n: f32| {
        let t = ((1.0 - n.abs()) / 0.22).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    image_from_alpha(64, move |nx, ny| edge(nx) * edge(ny))
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

/// Transparent center with alpha ramping up toward the edges; stretched
/// over the whole window as the danger overlay.
fn vignette_image() -> Image {
    image_from_alpha(128, |nx, ny| {
        let d = nx.abs().max(ny.abs());
        ((d - 0.45) / 0.55).clamp(0.0, 1.0).powf(1.6)
    })
}

fn setup_effect_assets(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(EffectAssets {
        glow: images.add(square_glow_image()),
        bar: images.add(soft_rect_image()),
        vignette: images.add(vignette_image()),
    });
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
                .with_rotation(Quat::from_rotation_z(rng.random_range(0.0..std::f32::consts::PI))),
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

#[allow(clippy::too_many_arguments)]
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
        let scale = if t < 0.15 {
            0.3 + t / 0.15 * 0.9
        } else {
            1.2 - (t - 0.15) * 0.15
        };
        tf.scale = Vec3::splat(scale);
        tf.translation.y += 34.0 * dt;
        let alpha = if t > 0.6 { 1.0 - (t - 0.6) / 0.4 } else { 1.0 };
        color.0.set_alpha(alpha);
    }
}

// ---------------------------------------------------------------------------
// Zone payout tally: a board-covering line count
// ---------------------------------------------------------------------------

/// One element of the zone-end payout display: a gold sheet washing over
/// the whole board plus a giant line count that slams in, holds, and
/// bursts away. Text elements scale-animate; the sheet only fades.
#[derive(Component)]
struct ZoneTally {
    life: f32,
    max_life: f32,
}

fn spawn_zone_tally(
    commands: &mut Commands,
    bar: &Handle<Image>,
    center: Vec2,
    board_size: Vec2,
    lines: u32,
) {
    let gold = Color::srgb(1.0, 0.85, 0.25);
    let life = 1.7;
    let tally = || ZoneTally {
        life,
        max_life: life,
    };
    // Gold sheet over the entire playfield.
    commands.spawn((
        Sprite {
            image: bar.clone(),
            custom_size: Some(board_size * 1.08),
            color: emissive(gold, 1.5).with_alpha(0.0),
            ..default()
        },
        Transform::from_translation(center.extend(38.0)),
        tally(),
        DespawnOnExit(AppState::Playing),
    ));
    // The number, sized to span most of the board.
    commands.spawn((
        Text2d::new(lines.to_string()),
        TextFont {
            font_size: FontSize::Px(board_size.y * 0.42),
            ..default()
        },
        TextColor(emissive(gold, 2.1)),
        Transform::from_translation((center + Vec2::new(0.0, board_size.y * 0.07)).extend(41.0))
            .with_scale(Vec3::splat(3.2)),
        tally(),
        DespawnOnExit(AppState::Playing),
    ));
    commands.spawn((
        Text2d::new("LINES"),
        TextFont {
            font_size: FontSize::Px(board_size.y * 0.09),
            ..default()
        },
        TextColor(emissive(Color::WHITE, 1.6)),
        Transform::from_translation((center - Vec2::new(0.0, board_size.y * 0.24)).extend(41.0))
            .with_scale(Vec3::splat(3.2)),
        tally(),
        DespawnOnExit(AppState::Playing),
    ));
}

#[allow(clippy::type_complexity)]
fn update_zone_tally(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(
        Entity,
        &mut ZoneTally,
        &mut Transform,
        Option<&mut Sprite>,
        Option<&mut TextColor>,
    )>,
) {
    let dt = time.delta_secs();
    let elapsed = time.elapsed_secs();
    for (entity, mut tally, mut tf, sprite, text) in &mut query {
        tally.life -= dt;
        if tally.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let t = 1.0 - tally.life / tally.max_life;
        if let Some(mut text) = text {
            // Slam in from huge, hold with a heartbeat, burst away.
            let scale = if t < 0.14 {
                let k = 1.0 - t / 0.14;
                1.0 + 2.2 * k * k * k
            } else if t < 0.78 {
                1.0 + 0.05 * (elapsed * 11.0).sin().abs()
            } else {
                1.0 + (t - 0.78) / 0.22 * 0.5
            };
            tf.scale = Vec3::splat(scale);
            let alpha = if t < 0.06 {
                t / 0.06
            } else if t > 0.78 {
                1.0 - (t - 0.78) / 0.22
            } else {
                1.0
            };
            text.0.set_alpha(alpha);
        } else if let Some(mut sprite) = sprite {
            // The sheet flares fast, then bleeds out over the hold.
            let alpha = if t < 0.08 {
                t / 0.08 * 0.5
            } else {
                0.5 * (1.0 - (t - 0.08) / 0.92)
            };
            sprite.color.set_alpha(alpha);
        }
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
        sprite.color.set_alpha(f.peak_alpha * (f.life / f.max_life));
    }
}

// ---------------------------------------------------------------------------
// Shockwave frames (playfield-friendly replacement for fullscreen flashes:
// thin expanding square outlines never obscure the stack)
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
            // Square outline, matching the game's all-rectangles styling.
            gizmos.rect_2d(
                Isometry2d::from_translation(wave.center),
                Vec2::splat(r * 2.0),
                emissive(wave.color, 3.2).with_alpha(alpha),
            );
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

#[allow(clippy::too_many_arguments)]
fn map_events_to_effects(
    mut commands: Commands,
    mut events: MessageReader<BoardEvent>,
    mut sfx: MessageWriter<PlaySfx>,
    mode: Res<GameMode>,
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
    let pulse_frame =
        |glows: &mut Query<&mut FrameGlow>, board: Entity, color: Color, strength: f32| {
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
        // The CPU's sounds are quieter so the player's actions stay crisp —
        // and during the zone, only the player's own board escapes the
        // lowpass muffle.
        let own_input = index.0 == 0;
        let gain = if own_input { 1.0 } else { 0.4 };
        let center = Vec2::new(board_tf.translation.x, board_tf.translation.y);
        let mut play = |s: Sfx, g: f32| {
            sfx.write(PlaySfx {
                sfx: s,
                gain: g,
                crisp: own_input,
            });
        };
        match &msg.event {
            GameEvent::Moved { dx } => {
                play(Sfx::Move, 0.5 * gain);
                if own_input {
                    shake.add(0.06);
                    if let Ok(mut kick) = kicks.get_mut(msg.board) {
                        kick.impulse(Vec2::new(*dx as f32 * 24.0, 0.0));
                    }
                }
            }
            GameEvent::MoveBlocked { dx } => {
                // Pressing into the wall: the board leans that way and stays
                // leaned while the key is held (impulses repeat at ARR rate).
                if own_input
                    && let Ok(mut kick) = kicks.get_mut(msg.board)
                {
                    kick.impulse(Vec2::new(*dx as f32 * 85.0, 0.0));
                }
            }
            GameEvent::Rotated { kicked, cw, kick } => {
                play(Sfx::Rotate, if *kicked { 1.0 } else { 0.7 } * gain);
                if own_input {
                    shake.add(if *kicked { 0.10 } else { 0.07 });
                    // Board recoil only for kicked rotations (spins tucked
                    // into a slot, T-spin style) — reacting to every plain
                    // rotation reads as noise.
                    if *kicked
                        && let Ok(mut k) = kicks.get_mut(msg.board)
                    {
                        // Reaction torque: piece spins CW, board CCW.
                        k.spin(if *cw { 1.2 } else { -1.2 });
                        // The wall kick shoves the piece; the board
                        // recoils the opposite way.
                        if kick.0 != 0 {
                            k.impulse(Vec2::new(-kick.0 as f32 * 55.0, 0.0));
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
                        kick.impulse(Vec2::new(0.0, -(115.0 + *distance as f32 * 4.0)));
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
                if let Some(distance) = drop_distance.remove(&msg.board)
                    && distance >= 3
                {
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
            GameEvent::Held => play(Sfx::Hold, gain),
            GameEvent::HoldBlocked => play(Sfx::HoldFail, 0.5 * gain),
            GameEvent::SpinNoLines { piece, mini } => {
                play(Sfx::TSpin, 0.7 * gain);
                let label = format!("{piece:?}-SPIN{}", if *mini { " MINI" } else { "" });
                spawn_banner(
                    &mut commands,
                    center + Vec2::new(0.0, 40.0 + banner_stagger),
                    label,
                    Color::srgb(0.85, 0.4, 1.0),
                    theme.cell,
                );
                banner_stagger += 44.0;
            }
            GameEvent::Cleared(clear) => {
                let is_tspin = clear.kind != ClearKind::Normal;
                // One "headline" sound per clear so the jingle phrases never
                // stack: perfect clear > T-spin > tetris/normal clears.
                // Short accents (B2B coin, combo ladder) still layer on top.
                if clear.perfect_clear {
                    play(Sfx::PerfectClear, gain);
                } else if is_tspin {
                    play(Sfx::TSpinClear, gain);
                } else {
                    play(Sfx::Clear(clear.lines), gain);
                }
                if clear.b2b {
                    play(Sfx::B2b, 0.9 * gain);
                }
                if clear.combo >= 1 {
                    play(Sfx::Combo(clear.combo), gain);
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
                    let row_center = Vec2::new(center.x, cell_world(board_tf, theme, 0, row).y);
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
                    // A spin is named after the piece that made it, the
                    // way a T-spin is: "S-SPIN DOUBLE".
                    (ClearKind::Spin, n) => format!(
                        "{:?}-SPIN {}",
                        clear.piece,
                        ["", "SINGLE", "DOUBLE", "TRIPLE", "QUAD"][n.min(4) as usize]
                    ),
                };
                if clear.b2b {
                    // The count only appears once it is worth something
                    // more than the first one was.
                    label = match clear.b2b_chain {
                        0 | 1 => format!("B2B {label}"),
                        n => format!("B2B x{n} {label}"),
                    };
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
                // A marathon-only celebration (10 lines earned). Under the
                // timed VS ramp the level rises every 25 s on a clock —
                // fanfare there would only pull focus from the match, so
                // VS modes get no level-up presentation at all. (Player
                // board only: under timed leveling both boards level up on
                // the same frame, which would double-trigger the sfx.)
                if index.0 == 0 && *mode == GameMode::Single {
                    play(Sfx::LevelUp, gain);
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
            GameEvent::ZoneReady => {
                // Unmissable "gauge full" cue: bright chime, gold banner
                // and a ripple off the board.
                play(Sfx::ZoneReady, gain);
                let gold = Color::srgb(1.0, 0.85, 0.25);
                pulse_frame(&mut glows, msg.board, gold, 0.9);
                spawn_shockwave(&mut commands, center, gold, false);
                spawn_burst(
                    &mut commands,
                    &glow_tex,
                    center,
                    gold,
                    24,
                    260.0,
                    10.0,
                    0.7,
                    90.0,
                );
                spawn_banner(
                    &mut commands,
                    center + Vec2::new(0.0, 60.0),
                    "ZONE READY!".to_string(),
                    gold,
                    theme.cell * 1.1,
                );
                if index.0 == 0 {
                    shake.add(0.15);
                }
            }
            GameEvent::ZoneActivated => {
                // Time stops: deep descending boom, full-screen flash,
                // particle nova, heavy shake and a star surge.
                play(Sfx::ZoneBoom, gain);
                let cyan = Color::srgb(0.3, 0.9, 1.0);
                pulse_frame(&mut glows, msg.board, cyan, 1.0);
                spawn_shockwave(&mut commands, center, cyan, true);
                spawn_burst(
                    &mut commands,
                    &glow_tex,
                    center,
                    cyan,
                    80,
                    520.0,
                    15.0,
                    0.9,
                    60.0,
                );
                spawn_banner(
                    &mut commands,
                    center + Vec2::new(0.0, 60.0),
                    "ZONE!".to_string(),
                    cyan,
                    theme.cell * 1.6,
                );
                if let Ok(mut kick) = kicks.get_mut(msg.board) {
                    kick.spin(1.2);
                }
                if index.0 == 0 {
                    spawn_flash(&mut commands, cyan, 0.38, 0.7);
                    shake.add(0.55);
                    surge.0 = surge.0.max(9.0);
                } else {
                    spawn_flash(&mut commands, cyan, 0.14, 0.4);
                }
            }
            GameEvent::ZoneLines { banked, total } => {
                // Rising coin ladder as the payload stacks up; the board
                // physically heaves upward with each banked wave.
                play(Sfx::Combo(*total), 0.9 * gain);
                let cyan = Color::srgb(0.3, 0.9, 1.0);
                pulse_frame(&mut glows, msg.board, cyan, 0.75);
                let h = theme.cell * VISIBLE_HEIGHT as f32;
                spawn_burst(
                    &mut commands,
                    &glow_tex,
                    center + Vec2::new(0.0, -h / 2.0),
                    cyan,
                    8 + 6 * *banked as usize,
                    300.0,
                    11.0,
                    0.6,
                    -160.0,
                );
                if let Ok(mut kick) = kicks.get_mut(msg.board) {
                    kick.impulse(Vec2::new(0.0, 70.0 + *banked as f32 * 12.0));
                }
                spawn_banner(
                    &mut commands,
                    center,
                    format!("x{total}"),
                    cyan,
                    theme.cell * (0.9 + 0.07 * *total as f32).min(1.8),
                );
                if index.0 == 0 {
                    shake.add(0.08 + 0.03 * *banked as f32);
                }
            }
            GameEvent::ZoneEnded { lines, attack } => {
                if *lines > 0 {
                    // Payday: fanfare, gold flash, confetti scaled by the
                    // haul, a heavy slam — and the line count filling the
                    // entire board (sheet + giant number + burst-away).
                    play(
                        if *lines >= 8 {
                            Sfx::Clear(4)
                        } else {
                            Sfx::TSpinClear
                        },
                        gain,
                    );
                    let gold = Color::srgb(1.0, 0.85, 0.25);
                    pulse_frame(&mut glows, msg.board, gold, 1.0);
                    spawn_shockwave(&mut commands, center, gold, true);
                    spawn_shockwave(&mut commands, center, Color::WHITE, true);
                    spawn_burst(
                        &mut commands,
                        &glow_tex,
                        center,
                        gold,
                        (40 + 14 * *lines as usize).min(180),
                        520.0,
                        14.0,
                        0.9,
                        140.0,
                    );
                    let board_size = Vec2::new(
                        theme.cell * BOARD_WIDTH as f32,
                        theme.cell * VISIBLE_HEIGHT as f32,
                    );
                    spawn_zone_tally(&mut commands, &bar_tex, center, board_size, *lines);
                    spawn_confetti_wave(
                        &mut commands,
                        center,
                        center.y + board_size.y / 2.0,
                        (30 + 12 * *lines as usize).min(160),
                    );
                    if let Ok(mut kick) = kicks.get_mut(msg.board) {
                        kick.impulse(Vec2::new(0.0, -160.0));
                    }
                    if index.0 == 0 {
                        spawn_flash(&mut commands, gold, 0.34, 0.8);
                        shake.add(0.4 + *lines as f32 * 0.06);
                        surge.0 = 9.0;
                    }
                    if *attack > 0 && boards.iter().count() > 1 {
                        play(Sfx::GarbageWarn, 0.8);
                    }
                }
            }
            GameEvent::AttackRamp { multiplier } => {
                // Both boards ramp on the same frame; announce once, over
                // the player's board.
                if index.0 == 0 {
                    play(Sfx::GarbageWarn, 1.0);
                    let orange = Color::srgb(1.0, 0.55, 0.15);
                    pulse_frame(&mut glows, msg.board, orange, 0.9);
                    shake.add(0.2);
                    spawn_banner(
                        &mut commands,
                        center + Vec2::new(0.0, 40.0),
                        format!("ATTACK x{multiplier}"),
                        orange,
                        theme.cell * 1.15,
                    );
                }
            }
            GameEvent::GarbageRose { rows } => {
                // Everything here scales with the wave: one row is a light
                // bump, a full 8-row slam rattles the whole screen.
                let r = *rows as f32;
                let heavy = *rows >= 4;
                let red = Color::srgb(1.0, 0.2, 0.2);
                play(Sfx::GarbageRise(*rows), gain);
                pulse_frame(&mut glows, msg.board, red, 0.35 + 0.09 * r);
                // Debris thrown up from the bottom edge where the rows
                // punched in (negative gravity: it flies upward).
                let h = theme.cell * VISIBLE_HEIGHT as f32;
                spawn_burst(
                    &mut commands,
                    &glow_tex,
                    center + Vec2::new(0.0, -h / 2.0),
                    Color::srgb(1.0, 0.35, 0.25),
                    6 + 5 * *rows as usize,
                    140.0 + 40.0 * r,
                    9.0,
                    0.5,
                    -220.0,
                );
                // The rising garbage shoves the board upward (both boards —
                // it reads as the stack physically pushing in).
                if let Ok(mut kick) = kicks.get_mut(msg.board) {
                    kick.impulse(Vec2::new(0.0, 55.0 + 22.0 * r));
                }
                if heavy {
                    // A big wave is a hit in its own right.
                    spawn_shockwave(&mut commands, center, red, *rows >= 6);
                }
                if index.0 == 0 {
                    shake.add(0.05 + 0.055 * r);
                    if heavy {
                        spawn_flash(&mut commands, red, 0.10 + 0.025 * r, 0.45);
                    }
                }
            }
            // Zen's wipe. Deliberately nothing like a top-out: a soft
            // white bloom and a settling nudge, no red, no thud, no
            // shake. The field ran out of room, and the run goes on.
            GameEvent::BoardReset => {
                play(Sfx::ZoneReady, gain * 0.7);
                if let Ok(mut kick) = kicks.get_mut(msg.board) {
                    kick.impulse(Vec2::new(0.0, -60.0));
                }
                spawn_flash(&mut commands, Color::srgb(0.75, 0.9, 1.0), 0.16, 0.5);
                // The stack is already gone by the time this arrives, so
                // the motes come from the field itself rather than from
                // the cells that were standing in it.
                for y in (0..VISIBLE_HEIGHT).step_by(2) {
                    for x in (0..BOARD_WIDTH).step_by(2) {
                        spawn_burst(
                            &mut commands,
                            &glow_tex,
                            cell_world(board_tf, theme, x, y),
                            Color::srgb(0.6, 0.85, 1.0),
                            1,
                            90.0,
                            theme.cell * 0.22,
                            1.4,
                            60.0,
                        );
                    }
                }
            }
            GameEvent::TopOut => {
                play(Sfx::GameOver, gain);
                shake.add(0.55);
                if let Ok(mut kick) = kicks.get_mut(msg.board) {
                    kick.impulse(Vec2::new(0.0, -200.0));
                    kick.spin(1.8);
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

/// Sustained match-end celebration: repeated fireworks, rings and confetti
/// waves for a few seconds after a VS win.
#[derive(Resource)]
struct Celebration {
    total: Timer,
    spawn: Timer,
    center: Vec2,
}

fn spawn_confetti_wave(commands: &mut Commands, center: Vec2, top_y: f32, count: usize) {
    let mut rng = rand::rng();
    for _ in 0..count {
        let color = Color::srgb(
            rng.random_range(0.4..1.0),
            rng.random_range(0.4..1.0),
            rng.random_range(0.4..1.0),
        );
        let pos = Vec2::new(
            center.x + rng.random_range(-190.0..190.0),
            top_y + rng.random_range(0.0..140.0),
        );
        commands.spawn((
            Sprite::from_color(emissive(color, 1.9), Vec2::new(6.0, 10.0)),
            Transform::from_translation(pos.extend(25.0)),
            Particle {
                vel: Vec2::new(rng.random_range(-45.0..45.0), rng.random_range(-40.0..10.0)),
                gravity: 140.0,
                damping: 0.4,
                spin: rng.random_range(-10.0..10.0),
                life: rng.random_range(1.6..3.2),
                max_life: 3.2,
            },
            DespawnOnExit(AppState::Playing),
        ));
    }
}

/// Match-end effects: big celebration on a player win, a proper collapse
/// on a loss.
#[allow(clippy::too_many_arguments)]
fn finish_fanfare(
    mut commands: Commands,
    result: Option<Res<SessionResult>>,
    boards: Query<(&Transform, &BoardTheme, &BoardIndex, &GameSession)>,
    mut sfx: MessageWriter<PlaySfx>,
    mut shake: ResMut<CameraShake>,
    mut surge: ResMut<StarSurge>,
    mut glows: Query<&mut FrameGlow>,
    mut kicks: Query<(Entity, &mut BoardKick)>,
    assets: Res<EffectAssets>,
) {
    let Some(result) = result else { return };

    let board_of = |slot: usize| boards.iter().find(|(_, _, i, _)| i.0 == slot);

    match *result {
        // A finished race celebrates like a match win.
        SessionResult::VsWin { winner: 0 } | SessionResult::RaceDone => {
            // ---- VICTORY: fireworks show over the player's board --------
            sfx.write(PlaySfx::new(Sfx::Win));
            if let Some((tf, theme, _, _)) = board_of(0) {
                let center = Vec2::new(tf.translation.x, tf.translation.y);
                let h = theme.cell * VISIBLE_HEIGHT as f32;
                spawn_confetti_wave(&mut commands, center, center.y + h / 2.0, 90);
                spawn_shockwave(&mut commands, center, Color::srgb(1.0, 0.9, 0.3), true);
                spawn_shockwave(&mut commands, center, Color::WHITE, true);
                shake.add(0.4);
                surge.0 = 9.0;
                commands.insert_resource(Celebration {
                    total: Timer::from_seconds(3.4, TimerMode::Once),
                    spawn: Timer::from_seconds(0.16, TimerMode::Repeating),
                    center,
                });
            }
            // Gold frames on both boards.
            for (entity, _) in &kicks {
                if let Ok(mut glow) = glows.get_mut(entity) {
                    glow.color = Color::srgb(1.0, 0.85, 0.25);
                    glow.t = 1.0;
                }
            }
        }
        SessionResult::VsWin { winner } => {
            // ---- DEFEAT: the player's board collapses --------------------
            sfx.write(PlaySfx::new(Sfx::Defeat));
            spawn_flash(&mut commands, Color::srgb(1.0, 0.2, 0.15), 0.38, 0.8);
            shake.add(0.85);
            if let Some((tf, theme, _, session)) = board_of(0) {
                let mut rng = rand::rng();
                for y in 0..VISIBLE_HEIGHT {
                    for x in 0..BOARD_WIDTH {
                        if session.game.board.cell(x, y).is_some() {
                            spawn_burst(
                                &mut commands,
                                &assets.glow,
                                cell_world(tf, theme, x, y),
                                Color::srgb(
                                    rng.random_range(0.55..0.8),
                                    rng.random_range(0.3..0.5),
                                    rng.random_range(0.25..0.4),
                                ),
                                1,
                                rng.random_range(260.0..430.0),
                                theme.cell * 0.3,
                                1.3,
                                420.0,
                            );
                        }
                    }
                }
            }
            for (entity, mut kick) in &mut kicks {
                if let Ok(mut glow) = glows.get_mut(entity) {
                    glow.color = Color::srgb(1.0, 0.2, 0.2);
                    glow.t = 1.0;
                }
                // The player's board takes the hit hardest.
                kick.impulse(Vec2::new(0.0, -170.0));
            }
            // Modest confetti for the CPU's corner.
            if let Some((tf, theme, _, _)) = board_of(winner) {
                let center = Vec2::new(tf.translation.x, tf.translation.y);
                let h = theme.cell * VISIBLE_HEIGHT as f32;
                spawn_confetti_wave(&mut commands, center, center.y + h / 2.0, 40);
            }
        }
        SessionResult::SoloOver => {
            // Marathon over: a weighty thud under the TopOut effects.
            sfx.write(PlaySfx {
                sfx: Sfx::Defeat,
                gain: 0.8,
                crisp: false,
            });
        }
    }
}

fn run_celebration(
    time: Res<Time>,
    mut commands: Commands,
    celebration: Option<ResMut<Celebration>>,
    assets: Option<Res<EffectAssets>>,
    mut sfx: MessageWriter<PlaySfx>,
    mut shake: ResMut<CameraShake>,
    mut surge: ResMut<StarSurge>,
) {
    let (Some(mut c), Some(assets)) = (celebration, assets) else {
        return;
    };
    c.total.tick(time.delta());
    if c.total.is_finished() {
        commands.remove_resource::<Celebration>();
        return;
    }
    if !c.spawn.tick(time.delta()).just_finished() {
        return;
    }

    // One firework per tick: a colored burst + ring at a random spot
    // around the winning board, plus a fresh confetti wave now and then.
    let mut rng = rand::rng();
    let pos = c.center
        + Vec2::new(
            rng.random_range(-230.0..230.0),
            rng.random_range(-160.0..230.0),
        );
    let palette = [
        Color::srgb(1.0, 0.85, 0.3),
        Color::srgb(0.4, 0.9, 1.0),
        Color::srgb(0.9, 0.5, 1.0),
        Color::srgb(0.5, 1.0, 0.6),
        Color::srgb(1.0, 0.55, 0.4),
    ];
    let color = palette[rng.random_range(0..palette.len())];
    spawn_burst(
        &mut commands,
        &assets.glow,
        pos,
        color,
        26,
        340.0,
        7.0,
        0.85,
        170.0,
    );
    spawn_shockwave(&mut commands, pos, color, false);
    if rng.random_range(0..3) == 0 {
        spawn_confetti_wave(&mut commands, c.center, c.center.y + 260.0, 26);
    }
    sfx.write(PlaySfx {
        sfx: Sfx::Combo(rng.random_range(2..9)),
        gain: 0.3,
        crisp: false,
    });
    shake.add(0.07);
    surge.0 = surge.0.max(2.5);
}

// ---------------------------------------------------------------------------
// Danger (pinch) presentation
// ---------------------------------------------------------------------------

/// Smoothed 0..1 danger per board, driven by the locked-stack height plus
/// the garbage that will rise on the next non-clearing lock.
#[derive(Resource, Default)]
pub struct DangerLevel(pub [f32; 2]);

#[derive(Component)]
struct DangerVignette;

#[derive(Component)]
struct DangerAlarm;

/// Effective height (rows) where the danger presentation starts / peaks.
/// The visible field is 20 rows; below 15 the player still has plenty of
/// room, so the vignette only creeps in from 15 and the alarm waits for
/// [`DANGER_ALARM_LEVEL`] (~16 rows). "Effective" height includes queued
/// garbage about to rise, so the warning fires before the wave lands.
const DANGER_START: f32 = 15.0;
const DANGER_FULL: f32 = 19.0;
/// Danger level (0..1) above which the alarm loop starts.
const DANGER_ALARM_LEVEL: f32 = 0.25;
/// Heartbeat frequency (rad/s) shared by the vignette and the board frame.
pub const DANGER_PULSE: f32 = 6.0;

fn update_danger(
    time: Res<Time>,
    state: Res<State<PlayState>>,
    mode: Res<GameMode>,
    settings: Res<GameSettings>,
    mut danger: ResMut<DangerLevel>,
    mut boards: Query<(&GameSession, &BoardIndex, &mut FrameGlow)>,
) {
    // An opponent mid-zone is a loaded gun: whatever their zone would
    // fire if it ended right now counts toward this board's threat, with
    // the same handicap scaling the real attack will get when it routes.
    let mut zone_threat = [0u32; 2];
    for (session, index, _) in boards.iter() {
        let mut threat = session.game.zone_projected_attack();
        if threat > 0 && *mode == GameMode::Custom {
            let pct = if index.0 == 0 {
                settings.custom.player_attack_pct
            } else {
                settings.custom.cpu_attack_pct
            };
            threat = (threat * pct + 50) / 100;
        }
        zone_threat[1 - index.0.min(1)] = threat;
    }

    for (session, index, mut glow) in &mut boards {
        // Only the deadly center columns can actually kill, so only they
        // drive the warning — a tall side stack is untidy, not lethal.
        let stack = session.game.deadly_height() as f32;
        // Queued garbage (plus the opponent's pending zone payout) lands
        // on top of the stack after the next non-clearing lock; only one
        // piece's cap can rise at once, so anything past the cap is not
        // an immediate threat yet.
        let pending = (session.game.incoming_total() + zone_threat[index.0.min(1)])
            .min(GARBAGE_CAP_PER_PIECE) as f32;
        let height = stack + pending;
        // Inside the zone the banked lines pile up by design and garbage
        // is frozen — an alarm there would be pure noise.
        let target = if session.game.zone_active() {
            0.0
        } else if matches!(state.get(), PlayState::Running | PlayState::Paused) {
            ((height - DANGER_START) / (DANGER_FULL - DANGER_START)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let slot = &mut danger.0[index.0.min(1)];
        // Ease in fast, out slower, so the effect breathes instead of popping.
        let rate = if target > *slot { 4.0 } else { 1.8 };
        *slot += (target - *slot) * (rate * time.delta_secs()).min(1.0);
        if *slot < 0.005 {
            *slot = 0.0;
        }
        glow.danger = *slot;
    }
}

/// Cyan full-screen tint while the player's zone runs, plus a breathing
/// frame glow on every board with an active zone.
#[derive(Component)]
struct ZoneVignette;

fn zone_presentation(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<EffectAssets>,
    boards: Query<(Entity, &BoardIndex, &GameSession)>,
    mut glows: Query<&mut FrameGlow>,
    mut vignette: Query<&mut ImageNode, With<ZoneVignette>>,
) {
    let t = time.elapsed_secs();
    for (entity, _, session) in &boards {
        if session.game.zone_active()
            && let Ok(mut glow) = glows.get_mut(entity)
        {
            glow.color = Color::srgb(0.3, 0.9, 1.0);
            glow.t = glow.t.max(0.45 + 0.3 * (t * 9.0).sin().abs());
        }
    }
    let Ok(mut node) = vignette.single_mut() else {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ImageNode {
                image: assets.vignette.clone(),
                color: Color::NONE,
                image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                ..default()
            },
            GlobalZIndex(-1),
            Pickable::IGNORE,
            ZoneVignette,
            DespawnOnExit(AppState::Playing),
        ));
        return;
    };
    let player_active = boards
        .iter()
        .any(|(_, i, s)| i.0 == 0 && s.game.zone_active());
    let alpha = if player_active {
        0.17 + 0.05 * (t * 8.0).sin()
    } else {
        0.0
    };
    node.color = Color::srgba(0.25, 0.85, 1.0, alpha);
}

/// Red screen-edge vignette that pulses with the human board's danger.
/// Spawned lazily so it also works when the app boots straight into
/// `Playing` (dev shortcut) before `Startup` assets existed.
fn sync_danger_vignette(
    mut commands: Commands,
    assets: Res<EffectAssets>,
    danger: Res<DangerLevel>,
    time: Res<Time>,
    mut vignette: Query<&mut ImageNode, With<DangerVignette>>,
) {
    let Ok(mut node) = vignette.single_mut() else {
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ImageNode {
                image: assets.vignette.clone(),
                color: Color::NONE,
                // Fill the node exactly; Auto would letterbox the square
                // texture in the middle of the screen instead.
                image_mode: bevy::ui::widget::NodeImageMode::Stretch,
                ..default()
            },
            // Behind every other UI node (HUD, overlays) — the vignette only
            // needs to cover the world-space playfield.
            GlobalZIndex(-1),
            Pickable::IGNORE,
            DangerVignette,
            DespawnOnExit(AppState::Playing),
        ));
        return;
    };
    let level = danger.0[0];
    let pulse = 0.7 + 0.3 * (time.elapsed_secs() * DANGER_PULSE).sin();
    node.color = Color::srgba(1.0, 0.12, 0.10, 0.24 * level * pulse);
}

/// Low-health alarm loop while the human board is in danger.
fn sync_danger_alarm(
    mut commands: Commands,
    danger: Res<DangerLevel>,
    state: Res<State<PlayState>>,
    settings: Res<GameSettings>,
    bank: Res<SfxBank>,
    alarm: Query<Entity, With<DangerAlarm>>,
    mut sinks: Query<&mut AudioSink, With<DangerAlarm>>,
) {
    let level = danger.0[0];
    let running = matches!(state.get(), PlayState::Running);
    // Hysteresis: start above the threshold, stop a notch below it, so the
    // loop doesn't flutter when the stack hovers right at the line.
    if running && level > DANGER_ALARM_LEVEL {
        if alarm.is_empty() {
            debug!("danger: alarm on (level {:.2})", level);
            commands.spawn((
                AudioPlayer::new(bank.danger_alarm()),
                PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
                DangerAlarm,
                DespawnOnExit(AppState::Playing),
            ));
        }
    } else if !running || level < DANGER_ALARM_LEVEL - 0.08 {
        for e in &alarm {
            commands.entity(e).despawn();
        }
    }
    for mut sink in &mut sinks {
        sink.set_volume(Volume::Linear(settings.sfx_linear() * (0.18 + 0.4 * level)));
    }
}
