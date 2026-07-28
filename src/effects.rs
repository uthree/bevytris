//! Juice: particles, screen shake, flashes, floating banners, confetti and
//! the scrolling starfield background. Also maps game events to SFX.

use bevy::prelude::*;
use rand::Rng;

use crate::audio::{PlaySfx, Sfx};
use crate::core::board::{BOARD_WIDTH, VISIBLE_HEIGHT};
use crate::core::game::{ClearKind, GameEvent};
use crate::render::BoardTheme;
use crate::session::{BoardEvent, BoardIndex, GameSession, SessionResult};
use crate::state::{AppState, PlayState};

pub struct EffectsPlugin;

impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraShake>()
            .add_systems(Startup, spawn_starfield)
            .add_systems(
                Update,
                (
                    map_events_to_effects.run_if(in_state(AppState::Playing)),
                    update_particles,
                    update_banners,
                    update_flashes,
                    update_starfield,
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
    let mut rng = rand::rng();
    let s = shake.trauma * shake.trauma;
    for mut tf in &mut cameras {
        if s > 0.0001 {
            tf.translation.x = rng.random_range(-1.0..1.0) * s * 24.0;
            tf.translation.y = rng.random_range(-1.0..1.0) * s * 24.0;
            tf.rotation = Quat::from_rotation_z(rng.random_range(-1.0..1.0) * s * 0.03);
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

fn spawn_burst(
    commands: &mut Commands,
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
        let s = rng.random_range(0.5..1.2) * size;
        commands.spawn((
            Sprite::from_color(color, Vec2::splat(s)),
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
    commands.spawn((
        Text2d::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
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
// Starfield background
// ---------------------------------------------------------------------------

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

fn update_starfield(time: Res<Time>, mut query: Query<(&Star, &mut Transform)>) {
    for (star, mut tf) in &mut query {
        tf.translation.y -= star.speed * time.delta_secs();
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
    boards: Query<(&Transform, &BoardTheme, &BoardIndex, &GameSession)>,
) {
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

        match &msg.event {
            GameEvent::Moved => play(Sfx::Move, 0.5 * gain),
            GameEvent::Rotated { kicked } => {
                play(Sfx::Rotate, if *kicked { 1.0 } else { 0.7 } * gain)
            }
            GameEvent::RotationFailed => play(Sfx::RotateFail, 0.35 * gain),
            GameEvent::SoftDropStep => play(Sfx::SoftDropTick, 0.25 * gain),
            GameEvent::HardDrop { distance } => {
                play(Sfx::HardDrop, gain);
                if index.0 == 0 {
                    shake.add(0.14 + *distance as f32 * 0.004);
                }
            }
            GameEvent::Locked { piece } => {
                play(Sfx::Lock, 0.8 * gain);
                // Dust puff under the locked piece.
                for (x, y) in piece.board_cells() {
                    if y < VISIBLE_HEIGHT {
                        spawn_burst(
                            &mut commands,
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

                // Shake & flash scale with how spectacular the clear is.
                let spectacle = clear.lines as f32
                    + if is_tspin { 2.0 } else { 0.0 }
                    + if clear.perfect_clear { 4.0 } else { 0.0 };
                if index.0 == 0 || spectacle >= 4.0 {
                    shake.add(0.06 * spectacle + 0.06);
                }
                if spectacle >= 4.0 {
                    let color = if clear.perfect_clear {
                        Color::srgb(1.0, 1.0, 0.9)
                    } else if is_tspin {
                        Color::srgb(0.8, 0.3, 1.0)
                    } else {
                        Color::srgb(0.3, 0.8, 1.0)
                    };
                    spawn_flash(&mut commands, color, 0.22, 0.35);
                }

                // Particle spray along each cleared row.
                for &row in &clear.rows {
                    if row >= VISIBLE_HEIGHT {
                        continue;
                    }
                    for x in 0..BOARD_WIDTH {
                        let pos = cell_world(board_tf, theme, x, row);
                        let color = if is_tspin {
                            Color::srgb(0.9, 0.5, 1.0)
                        } else if clear.lines == 4 {
                            Color::srgb(0.4, 0.9, 1.0)
                        } else {
                            Color::srgb(1.0, 0.95, 0.7)
                        };
                        spawn_burst(
                            &mut commands,
                            pos,
                            color,
                            4,
                            220.0,
                            theme.cell * 0.22,
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
                    spawn_flash(&mut commands, Color::srgb(0.5, 1.0, 0.6), 0.15, 0.3);
                    spawn_banner(
                        &mut commands,
                        center + Vec2::new(0.0, 80.0),
                        format!("LEVEL {level}"),
                        Color::srgb(0.5, 1.0, 0.6),
                        theme.cell,
                    );
                }
            }
            GameEvent::GarbageRose { rows } => {
                play(Sfx::GarbageRise, gain);
                if index.0 == 0 {
                    shake.add(0.1 + *rows as f32 * 0.04);
                    spawn_flash(&mut commands, Color::srgb(1.0, 0.2, 0.2), 0.12, 0.25);
                }
            }
            GameEvent::TopOut => {
                play(Sfx::GameOver, gain);
                shake.add(0.55);
                spawn_flash(&mut commands, Color::srgb(1.0, 0.3, 0.2), 0.3, 0.6);
                // Board "explodes": scatter particles over the whole field.
                for y in 0..VISIBLE_HEIGHT {
                    for x in 0..BOARD_WIDTH {
                        if session.game.board.cell(x, y).is_some() && (x + y) % 2 == 0 {
                            spawn_burst(
                                &mut commands,
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
                    Sprite::from_color(color, Vec2::new(6.0, 10.0)),
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
