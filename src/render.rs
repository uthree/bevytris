//! Board visualization: cell grid, ghost piece, hold/next previews, HUD
//! text and the incoming-garbage danger bar.

use bevy::prelude::*;

use crate::core::board::{Cell, BOARD_WIDTH, VISIBLE_HEIGHT};
use crate::core::game::gravity_seconds;
use crate::core::piece::{cells, PieceKind, Rot};
use crate::session::{BoardIndex, Countdown, CpuControlled, GameSession, MatchState};
use crate::state::{AppState, GameMode, PlayState};

/// Rows rendered above the visible field so pieces peek over the skyline.
const PEEK_ROWS: i8 = 2;

#[derive(Component)]
pub struct BoardTheme {
    pub cell: f32,
}

/// Transient glow on the board's neon frame, set by the effects layer when
/// something spectacular happens (replaces the old fullscreen flash, which
/// tinted the playfield and got in the way of reading the stack).
#[derive(Component)]
pub struct FrameGlow {
    pub color: Color,
    /// 1.0 = full glow, decays to 0.
    pub t: f32,
    /// Persistent 0..1 danger level (written by the effects module);
    /// tints the frame red with a heartbeat pulse.
    pub danger: f32,
}

impl Default for FrameGlow {
    fn default() -> Self {
        Self {
            color: Color::WHITE,
            t: 0.0,
            danger: 0.0,
        }
    }
}

/// Marker for the four frame bars around a board.
#[derive(Component)]
struct FrameBar;

const FRAME_BASE_COLOR: Color = Color::srgb(0.25, 0.75, 0.95);
/// Frame tint while the board's stack is dangerously high.
const DANGER_FRAME_COLOR: Color = Color::srgb(1.0, 0.16, 0.12);

/// Spring-damper motion for a whole board: inputs push it around (pressing
/// into a wall leans it, rotations twist it, hard drops thump it down) and
/// the spring settles it back home. This is what makes the board feel like
/// it has inertia.
#[derive(Component)]
pub struct BoardKick {
    home: Vec3,
    offset: Vec2,
    vel: Vec2,
    rot: f32,
    rot_vel: f32,
}

impl BoardKick {
    pub fn new(home: Vec3) -> Self {
        Self {
            home,
            offset: Vec2::ZERO,
            vel: Vec2::ZERO,
            rot: 0.0,
            rot_vel: 0.0,
        }
    }

    /// Push the board (px/s velocity change).
    pub fn impulse(&mut self, v: Vec2) {
        self.vel += v;
    }

    /// Twist the board (rad/s angular velocity change; +z = CCW).
    pub fn spin(&mut self, w: f32) {
        self.rot_vel += w;
    }
}

fn update_board_kick(time: Res<Time>, mut boards: Query<(&mut BoardKick, &mut Transform)>) {
    // Underdamped spring: lively single overshoot, then settle.
    // Tuned stiff for a snappy response (~3.3 Hz positional, ~4.2 Hz
    // rotational); impulses in effects.rs are sized against these.
    const K: f32 = 420.0;
    const C: f32 = 16.0;
    const K_ROT: f32 = 700.0;
    const C_ROT: f32 = 22.0;
    let dt = time.delta_secs().min(0.05);
    for (mut kick, mut tf) in &mut boards {
        let acc = -K * kick.offset - C * kick.vel;
        kick.vel += acc * dt;
        let dv = kick.vel * dt;
        kick.offset = (kick.offset + dv).clamp_length_max(13.0);

        let rot_acc = -K_ROT * kick.rot - C_ROT * kick.rot_vel;
        kick.rot_vel += rot_acc * dt;
        kick.rot = (kick.rot + kick.rot_vel * dt).clamp(-0.045, 0.045);

        tf.translation = kick.home + kick.offset.extend(0.0);
        tf.rotation = Quat::from_rotation_z(kick.rot);
    }
}

#[derive(Component)]
struct CellSprite {
    x: i8,
    y: i8,
}

#[derive(Component)]
struct HoldSprite(usize);

#[derive(Component)]
struct NextSprite {
    slot: usize,
    i: usize,
}

#[derive(Component, PartialEq)]
enum HudText {
    Score,
    Level,
    Lines,
    Speed,
    Incoming,
}

#[derive(Component)]
struct DangerBar;

#[derive(Component)]
struct CountdownText;

/// Center-top line with stage, opponent archetype, round and match score.
#[derive(Component)]
struct MatchHudText;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                sync_cells,
                sync_previews,
                sync_hud,
                sync_danger_bar,
                sync_countdown,
                sync_frame_glow,
                sync_match_hud,
                update_board_kick,
            )
                .run_if(in_state(AppState::Playing)),
        );
    }
}

/// Called (chained after session spawn) from OnEnter(Playing) in main.rs.
pub fn setup_board_visuals(
    mut commands: Commands,
    mode: Res<GameMode>,
    boards: Query<(Entity, &BoardIndex), With<GameSession>>,
) {
    let vs = matches!(*mode, GameMode::VsCpu { .. });
    let cell = if vs { 26.0 } else { 30.0 };

    for (entity, index) in &boards {
        let x = if vs {
            if index.0 == 0 { -330.0 } else { 330.0 }
        } else {
            0.0
        };
        let w = cell * BOARD_WIDTH as f32;
        let h = cell * VISIBLE_HEIGHT as f32;

        commands
            .entity(entity)
            .insert((
                Transform::from_xyz(x, -14.0, 0.0),
                BoardTheme { cell },
                FrameGlow::default(),
                BoardKick::new(Vec3::new(x, -14.0, 0.0)),
            ))
            .with_children(|parent| {
                // Backdrop panel.
                parent.spawn((
                    Sprite::from_color(Color::srgba(0.02, 0.02, 0.05, 0.92), Vec2::new(w + 8.0, h + 8.0)),
                    Transform::from_xyz(0.0, 0.0, 0.5),
                ));
                // Neon frame: four edge bars (pulsed by FrameGlow).
                for (fx, fy, fw, fh) in [
                    (0.0, h / 2.0 + 5.0, w + 16.0, 3.0),
                    (0.0, -h / 2.0 - 5.0, w + 16.0, 3.0),
                    (-w / 2.0 - 6.5, 0.0, 3.0, h + 13.0),
                    (w / 2.0 + 6.5, 0.0, 3.0, h + 13.0),
                ] {
                    parent.spawn((
                        Sprite::from_color(FRAME_BASE_COLOR, Vec2::new(fw, fh)),
                        Transform::from_xyz(fx, fy, 0.6),
                        FrameBar,
                    ));
                }

                // Cell grid, including two hidden peek rows on top.
                for y in 0..(VISIBLE_HEIGHT + PEEK_ROWS) {
                    for x in 0..BOARD_WIDTH {
                        let pos = cell_pos(cell, x, y, h);
                        parent.spawn((
                            Sprite::from_color(empty_color(y), Vec2::splat(cell - 2.0)),
                            Transform::from_translation(pos.extend(1.0)),
                            CellSprite { x, y },
                        ));
                    }
                }

                // Hold box (upper left of the board).
                let hold_x = -w / 2.0 - cell * 2.8;
                let hold_y = h / 2.0 - cell * 1.2;
                parent.spawn((
                    Text2d::new("HOLD"),
                    TextFont { font_size: FontSize::Px(cell * 0.62), ..default() },
                    TextColor(Color::srgb(0.6, 0.7, 0.8)),
                    Transform::from_xyz(hold_x, hold_y + cell * 1.7, 2.0),
                ));
                parent.spawn((
                    Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, 0.55), Vec2::splat(cell * 4.0)),
                    Transform::from_xyz(hold_x, hold_y, 0.7),
                ));
                for i in 0..4 {
                    parent.spawn((
                        Sprite::from_color(Color::NONE, Vec2::splat(cell * 0.55 - 1.0)),
                        Transform::from_xyz(hold_x, hold_y, 2.0),
                        HoldSprite(i),
                        PreviewAnchor(Vec2::new(hold_x, hold_y)),
                    ));
                }

                // Next queue (right of the board).
                let next_x = w / 2.0 + cell * 2.8;
                parent.spawn((
                    Text2d::new("NEXT"),
                    TextFont { font_size: FontSize::Px(cell * 0.62), ..default() },
                    TextColor(Color::srgb(0.6, 0.7, 0.8)),
                    Transform::from_xyz(next_x, h / 2.0 + cell * 0.5, 2.0),
                ));
                for slot in 0..5 {
                    let slot_y = h / 2.0 - cell * 1.2 - slot as f32 * cell * 2.4;
                    parent.spawn((
                        Sprite::from_color(Color::srgba(0.0, 0.0, 0.0, 0.45), Vec2::splat(cell * 3.4)),
                        Transform::from_xyz(next_x, slot_y, 0.7),
                    ));
                    for i in 0..4 {
                        parent.spawn((
                            Sprite::from_color(Color::NONE, Vec2::splat(cell * 0.55 - 1.0)),
                            Transform::from_xyz(next_x, slot_y, 2.0),
                            NextSprite { slot, i },
                            PreviewAnchor(Vec2::new(next_x, slot_y)),
                        ));
                    }
                }

                // Danger bar hugging the left edge of the field.
                parent.spawn((
                    Sprite::from_color(Color::srgb(0.9, 0.15, 0.2), Vec2::new(6.0, 1.0)),
                    Transform::from_xyz(-w / 2.0 - 14.0, -h / 2.0, 2.0),
                    DangerBar,
                ));

                // HUD text column (left side, under hold box).
                let hud_x = -w / 2.0 - cell * 2.8;
                let entries: &[(&str, HudText, f32)] = &[
                    ("SCORE", HudText::Score, 0.0),
                    ("LEVEL", HudText::Level, 1.0),
                    ("LINES", HudText::Lines, 2.0),
                    ("SPEED", HudText::Speed, 3.0),
                    ("ATK IN", HudText::Incoming, 4.0),
                ];
                for (label, marker, row) in entries {
                    let base_y = hold_y - cell * 3.2 - row * cell * 2.2;
                    parent.spawn((
                        Text2d::new(*label),
                        TextFont { font_size: FontSize::Px(cell * 0.5), ..default() },
                        TextColor(Color::srgb(0.5, 0.6, 0.7)),
                        Transform::from_xyz(hud_x, base_y, 2.0),
                    ));
                    parent.spawn((
                        Text2d::new("0"),
                        TextFont { font_size: FontSize::Px(cell * 0.7), ..default() },
                        TextColor(Color::WHITE),
                        Transform::from_xyz(hud_x, base_y - cell * 0.85, 2.0),
                        match marker {
                            HudText::Score => HudText::Score,
                            HudText::Level => HudText::Level,
                            HudText::Lines => HudText::Lines,
                            HudText::Speed => HudText::Speed,
                            HudText::Incoming => HudText::Incoming,
                        },
                    ));
                }

                // Player name over the board.
                let name = if !vs {
                    "MARATHON"
                } else if index.0 == 0 {
                    "YOU"
                } else {
                    "CPU"
                };
                parent.spawn((
                    Text2d::new(name),
                    TextFont { font_size: FontSize::Px(cell * 0.75), ..default() },
                    TextColor(Color::srgb(0.85, 0.9, 1.0)),
                    Transform::from_xyz(0.0, h / 2.0 + cell * 1.5, 2.0),
                ));
            });
    }

    // Central countdown text.
    commands.spawn((
        Text2d::new("3"),
        TextFont { font_size: FontSize::Px(128.0), ..default() },
        TextColor(Color::srgb(1.0, 0.9, 0.3)),
        Transform::from_xyz(0.0, 40.0, 50.0),
        CountdownText,
        DespawnOnExit(AppState::Playing),
    ));

    // Match header (stage / round / score) for VS mode.
    if vs {
        commands.spawn((
            Text2d::new(""),
            TextFont { font_size: FontSize::Px(24.0), ..default() },
            TextColor(Color::srgb(0.85, 0.9, 1.0)),
            Transform::from_xyz(0.0, 332.0, 30.0),
            MatchHudText,
            DespawnOnExit(AppState::Playing),
        ));
    }
}

fn sync_match_hud(
    match_state: Option<Res<MatchState>>,
    cpus: Query<&CpuControlled>,
    mut texts: Query<&mut Text2d, With<MatchHudText>>,
) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    let Some(ms) = match_state else { return };
    let Some(stage) = ms.stage else { return };
    let archetype = cpus
        .single()
        .map(|c| c.profile.archetype.label())
        .unwrap_or("");
    let line = format!(
        "STAGE {stage:02} {archetype}   ROUND {}   YOU {} - {} CPU   (FIRST TO {})",
        ms.round, ms.player_wins, ms.cpu_wins, ms.wins_needed
    );
    if **text != line {
        **text = line;
    }
}

fn cell_pos(cell: f32, x: i8, y: i8, board_h: f32) -> Vec2 {
    Vec2::new(
        (x as f32 + 0.5) * cell - cell * BOARD_WIDTH as f32 / 2.0,
        (y as f32 + 0.5) * cell - board_h / 2.0,
    )
}

fn empty_color(y: i8) -> Color {
    if y >= VISIBLE_HEIGHT {
        Color::NONE
    } else {
        // Kept very faint: a strong grid competes with the stack for
        // attention (playtester feedback).
        Color::srgba(1.0, 1.0, 1.0, 0.012)
    }
}

fn piece_color(kind: PieceKind, mul: f32, alpha: f32) -> Color {
    let (r, g, b) = kind.color();
    Color::srgba(r * mul, g * mul, b * mul, alpha)
}

fn sync_cells(
    boards: Query<(&GameSession, &Children)>,
    mut sprites: Query<(&CellSprite, &mut Sprite)>,
) {
    for (session, children) in &boards {
        let game = &session.game;
        let ghost = game.ghost();
        let ghost_cells = ghost.board_cells();
        let active_cells = game.active.board_cells();

        for child in children.iter() {
            let Ok((cell, mut sprite)) = sprites.get_mut(child) else {
                continue;
            };
            let p = (cell.x, cell.y);
            let color = if !game.game_over && active_cells.contains(&p) {
                // HDR-boosted so the falling piece blooms.
                crate::emissive(piece_color(game.active.kind, 1.05, 1.0), 1.9)
            } else if let Some(locked) = game.board.cell(cell.x, cell.y) {
                match locked {
                    Cell::Piece(kind) => {
                        if game.game_over {
                            piece_color(kind, 0.45, 1.0)
                        } else {
                            // Slight HDR lift keeps the stack vivid without
                            // competing with the active piece's glow.
                            crate::emissive(piece_color(kind, 0.95, 1.0), 1.15)
                        }
                    }
                    Cell::Garbage => Color::srgb(0.42, 0.44, 0.50),
                }
            } else if !game.game_over && ghost_cells.contains(&p) {
                piece_color(game.active.kind, 1.1, 0.30)
            } else {
                empty_color(cell.y)
            };
            // Cells poking above the skyline render ghosted so they read as
            // "outside the field" instead of floating over the frame.
            let color = if cell.y >= VISIBLE_HEIGHT {
                color.with_alpha(color.alpha() * 0.5)
            } else {
                color
            };
            sprite.color = color;
        }
    }
}

/// Fixed layout anchor for hold/next preview sprites (parent space).
#[derive(Component)]
struct PreviewAnchor(Vec2);

/// Position of preview sprite `i` when drawing `kind`, centered on the
/// anchor by the piece's actual occupied cells.
fn layout_mini(kind: PieceKind, scale: f32, anchor: Vec2, i: usize) -> Vec2 {
    let cs = cells(kind, Rot::R0);
    let (cx, cy) = (cs[i].0 as f32, cs[i].1 as f32);
    let min_x = cs.iter().map(|c| c.0).min().unwrap() as f32;
    let max_x = cs.iter().map(|c| c.0).max().unwrap() as f32;
    let min_y = cs.iter().map(|c| c.1).min().unwrap() as f32;
    let max_y = cs.iter().map(|c| c.1).max().unwrap() as f32;
    let off = Vec2::new((min_x + max_x + 1.0) / 2.0, (min_y + max_y + 1.0) / 2.0);
    anchor + (Vec2::new(cx + 0.5, cy + 0.5) - off) * scale
}

fn sync_previews(
    boards: Query<(&GameSession, &BoardTheme, &Children)>,
    mut holds: Query<
        (&HoldSprite, &PreviewAnchor, &mut Sprite, &mut Transform),
        Without<NextSprite>,
    >,
    mut nexts: Query<
        (&NextSprite, &PreviewAnchor, &mut Sprite, &mut Transform),
        Without<HoldSprite>,
    >,
) {
    for (session, theme, children) in &boards {
        let game = &session.game;
        let scale = theme.cell * 0.55;
        for child in children.iter() {
            if let Ok((hold, anchor, mut sprite, mut tf)) = holds.get_mut(child) {
                match game.hold {
                    Some(kind) => {
                        let pos = layout_mini(kind, scale, anchor.0, hold.0);
                        tf.translation.x = pos.x;
                        tf.translation.y = pos.y;
                        let mul = if game.hold_used { 0.4 } else { 1.0 };
                        sprite.color = piece_color(kind, mul, 1.0);
                    }
                    None => sprite.color = Color::NONE,
                }
            }
            if let Ok((next, anchor, mut sprite, mut tf)) = nexts.get_mut(child) {
                if let Some(kind) = game.queue.get(next.slot).copied() {
                    let pos = layout_mini(kind, scale, anchor.0, next.i);
                    tf.translation.x = pos.x;
                    tf.translation.y = pos.y;
                    sprite.color = piece_color(kind, 1.0, 1.0);
                } else {
                    sprite.color = Color::NONE;
                }
            }
        }
    }
}

fn sync_hud(
    boards: Query<(&GameSession, &Children)>,
    mut texts: Query<(&HudText, &mut Text2d, &mut TextColor)>,
) {
    for (session, children) in &boards {
        let game = &session.game;
        for child in children.iter() {
            let Ok((kind, mut text, mut color)) = texts.get_mut(child) else {
                continue;
            };
            match kind {
                HudText::Score => {
                    let s = game.score.to_string();
                    if **text != s {
                        **text = s;
                    }
                }
                HudText::Level => {
                    let s = game.level.to_string();
                    if **text != s {
                        **text = s;
                    }
                }
                HudText::Lines => {
                    let s = game.lines.to_string();
                    if **text != s {
                        **text = s;
                    }
                }
                HudText::Speed => {
                    let g = 1.0 / gravity_seconds(game.level);
                    let s = format!("{g:.1}G");
                    if **text != s {
                        **text = s;
                    }
                }
                HudText::Incoming => {
                    let n = game.incoming_total();
                    let s = if n == 0 { "-".to_string() } else { format!("{n}!") };
                    if **text != s {
                        **text = s;
                    }
                    color.0 = if n == 0 {
                        Color::srgb(0.5, 0.6, 0.7)
                    } else {
                        Color::srgb(1.0, 0.25, 0.25)
                    };
                }
            }
        }
    }
}

fn sync_danger_bar(
    time: Res<Time>,
    boards: Query<(&GameSession, &BoardTheme, &Children)>,
    mut bars: Query<(&mut Sprite, &mut Transform), With<DangerBar>>,
) {
    for (session, theme, children) in &boards {
        let rows = session.game.incoming_total();
        let h = theme.cell * VISIBLE_HEIGHT as f32;
        for child in children.iter() {
            let Ok((mut sprite, mut tf)) = bars.get_mut(child) else {
                continue;
            };
            let bar_h = (rows as f32 * theme.cell).min(h);
            sprite.custom_size = Some(Vec2::new(6.0, bar_h.max(0.001)));
            tf.translation.y = -h / 2.0 + bar_h / 2.0;
            let pulse = 0.7 + 0.3 * (time.elapsed_secs() * 8.0).sin();
            sprite.color = if rows == 0 {
                Color::NONE
            } else {
                crate::emissive(Color::srgb(0.95, 0.15, 0.2), 2.2).with_alpha(pulse)
            };
        }
    }
}

/// Decay the frame glow and paint the frame bars accordingly: the bars
/// blend from their base cyan toward the glow color and swell slightly.
fn sync_frame_glow(
    time: Res<Time>,
    mut boards: Query<(&mut FrameGlow, &Children)>,
    mut bars: Query<(&mut Sprite, &mut Transform), With<FrameBar>>,
) {
    use bevy::color::Mix;
    for (mut glow, children) in &mut boards {
        glow.t = (glow.t - 2.6 * time.delta_secs()).max(0.0);
        let t = glow.t * glow.t; // ease-out: snappy attack, soft tail
        // Danger: steady red heartbeat underneath the event pulses.
        let heartbeat = (time.elapsed_secs() * crate::effects::DANGER_PULSE).sin() * 0.5 + 0.5;
        let danger_t = glow.danger * (0.35 + 0.55 * heartbeat);
        for child in children.iter() {
            let Ok((mut sprite, mut tf)) = bars.get_mut(child) else {
                continue;
            };
            let base = FRAME_BASE_COLOR.mix(&DANGER_FRAME_COLOR, danger_t.min(1.0));
            let mixed = base.mix(&glow.color, t.min(1.0));
            // Base frame glows a little; pulses push it deep into HDR.
            sprite.color = crate::emissive(mixed, 1.35 + 2.4 * t + 1.1 * danger_t);
            let swell = 1.0 + t * 1.8;
            // Bars are thin in exactly one axis; swell that axis only.
            let size = sprite.custom_size.unwrap_or(Vec2::ONE);
            if size.x < size.y {
                tf.scale = Vec3::new(swell, 1.0, 1.0);
            } else {
                tf.scale = Vec3::new(1.0, swell, 1.0);
            }
        }
    }
}

fn sync_countdown(
    time: Res<Time>,
    countdown: Option<Res<Countdown>>,
    state: Res<State<PlayState>>,
    mut query: Query<(&mut Text2d, &mut Visibility, &mut TextColor), With<CountdownText>>,
) {
    let Ok((mut text, mut vis, mut color)) = query.single_mut() else {
        return;
    };
    match state.get() {
        PlayState::Countdown => {
            *vis = Visibility::Inherited;
            if let Some(c) = countdown {
                let n = crate::session::countdown_display(&c);
                let s = n.to_string();
                if **text != s {
                    **text = s;
                }
                color.0 = Color::srgb(1.0, 0.9, 0.3);
            }
        }
        PlayState::Running => {
            // Brief "GO!" flash right after the countdown.
            if **text != "GO!" {
                **text = "GO!".to_string();
            }
            let a = color.0.alpha() - 1.2 * time.delta_secs();
            if a <= 0.0 {
                *vis = Visibility::Hidden;
            } else {
                color.0 = Color::srgba(0.4, 1.0, 0.5, a);
            }
        }
        _ => {}
    }
}
