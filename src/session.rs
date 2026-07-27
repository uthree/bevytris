//! Game session orchestration: board entities, human input (DAS/ARR),
//! the CPU driver, garbage routing between boards and win/lose detection.

use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::{PlaySfx, Sfx};
use crate::config::{Action, GameSettings};
use crate::core::ai::{self, AiProfile, Plan};
use crate::core::game::{Game, GameEvent};
use crate::state::{AppState, CpuDifficulty, GameMode, PlayState};

/// One playfield (either the human's or the CPU's).
#[derive(Component)]
pub struct GameSession {
    pub game: Game,
}

/// Board slot: 0 = left (human), 1 = right (CPU in VS mode).
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub struct BoardIndex(pub usize);

#[derive(Component)]
pub struct HumanControlled;

/// Auto-shift state for the human player.
#[derive(Component, Default)]
pub struct DasState {
    dir: i8,
    held_secs: f32,
    arr_acc: f32,
}

#[derive(Component)]
pub struct CpuControlled {
    profile: AiProfile,
    rng: StdRng,
    plan: Option<Plan>,
    /// `stats.pieces` value the current plan was made for.
    planned_piece: Option<u32>,
    hold_done: bool,
    timer: f32,
}

/// Raw game events re-published as Bevy messages for effects/audio/UI.
#[derive(Message)]
pub struct BoardEvent {
    pub board: Entity,
    /// Originating board slot (0 = human). Currently consumers resolve the
    /// slot via the `BoardIndex` component, but the field stays for hooks.
    #[allow(dead_code)]
    pub index: usize,
    pub event: GameEvent,
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionResult {
    /// Single player topped out.
    SoloOver,
    /// VS: which board index won.
    VsWin { winner: usize },
}

#[derive(Resource)]
pub struct Countdown {
    timer: Timer,
    remaining: u32,
}

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BoardEvent>()
            .init_resource::<GameMode>()
            .add_systems(
                OnEnter(AppState::Playing),
                (spawn_session, crate::render::setup_board_visuals).chain(),
            )
            .add_systems(
                Update,
                countdown_tick.run_if(in_state(PlayState::Countdown)),
            )
            .add_systems(
                Update,
                (human_input, cpu_drive, tick_games)
                    .chain()
                    .run_if(in_state(PlayState::Running)),
            )
            .add_systems(
                Update,
                pause_toggle.run_if(in_state(PlayState::Running).or_else(in_state(PlayState::Paused))),
            );
    }
}

fn spawn_session(mut commands: Commands, mode: Res<GameMode>, mut sfx: MessageWriter<PlaySfx>) {
    let seed: u64 = rand::rng().random();
    // Both players get the same piece sequence (standard for versus play).
    let start_level = 1;

    commands.remove_resource::<SessionResult>();
    commands.insert_resource(Countdown {
        timer: Timer::from_seconds(0.8, TimerMode::Repeating),
        remaining: 3,
    });
    sfx.write(PlaySfx::new(Sfx::Countdown));

    commands.spawn((
        GameSession { game: Game::new(seed, start_level) },
        BoardIndex(0),
        HumanControlled,
        DasState::default(),
        DespawnOnExit(AppState::Playing),
        Transform::default(),
        Visibility::default(),
    ));

    if let GameMode::VsCpu(difficulty) = *mode {
        let profile = match difficulty {
            CpuDifficulty::Easy => AiProfile::easy(),
            CpuDifficulty::Normal => AiProfile::normal(),
            CpuDifficulty::Hard => AiProfile::hard(),
        };
        commands.spawn((
            GameSession { game: Game::new(seed, start_level) },
            BoardIndex(1),
            CpuControlled {
                profile,
                rng: StdRng::seed_from_u64(seed ^ 0xC0FFEE),
                plan: None,
                planned_piece: None,
                hold_done: false,
                timer: profile.think_time,
            },
            DespawnOnExit(AppState::Playing),
            Transform::default(),
            Visibility::default(),
        ));
    }
}

fn countdown_tick(
    time: Res<Time>,
    mut countdown: ResMut<Countdown>,
    mut next: ResMut<NextState<PlayState>>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    if countdown.timer.tick(time.delta()).just_finished() {
        countdown.remaining = countdown.remaining.saturating_sub(1);
        if countdown.remaining == 0 {
            sfx.write(PlaySfx::new(Sfx::Go));
            next.set(PlayState::Running);
        } else {
            sfx.write(PlaySfx::new(Sfx::Countdown));
        }
    }
}

pub fn countdown_display(countdown: &Countdown) -> u32 {
    countdown.remaining
}

fn pause_toggle(
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<GameSettings>,
    state: Res<State<PlayState>>,
    mut next: ResMut<NextState<PlayState>>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    if keys.just_pressed(settings.key_for(Action::Pause)) {
        match state.get() {
            PlayState::Running => {
                next.set(PlayState::Paused);
                sfx.write(PlaySfx::new(Sfx::MenuSelect));
            }
            PlayState::Paused => {
                next.set(PlayState::Running);
                sfx.write(PlaySfx::new(Sfx::MenuBack));
            }
            _ => {}
        }
    }
}

fn human_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<GameSettings>,
    mut query: Query<(&mut GameSession, &mut DasState), With<HumanControlled>>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    let Ok((mut session, mut das)) = query.single_mut() else {
        return;
    };
    let game = &mut session.game;
    if game.game_over {
        return;
    }

    let left = settings.key_for(Action::MoveLeft);
    let right = settings.key_for(Action::MoveRight);
    let das_secs = settings.das_ms as f32 / 1000.0;
    let arr_secs = settings.arr_ms as f32 / 1000.0;

    // --- Horizontal movement with DAS/ARR ---------------------------------
    // The most recently pressed direction wins while both are held.
    if keys.just_pressed(left) {
        das.dir = -1;
        das.held_secs = 0.0;
        das.arr_acc = 0.0;
        game.move_horizontal(-1);
    }
    if keys.just_pressed(right) {
        das.dir = 1;
        das.held_secs = 0.0;
        das.arr_acc = 0.0;
        game.move_horizontal(1);
    }
    let dir_key = if das.dir < 0 { left } else { right };
    if das.dir != 0 && !keys.pressed(dir_key) {
        // Released the active direction; fall back to the other if held.
        let other = if das.dir < 0 { right } else { left };
        if keys.pressed(other) {
            das.dir = -das.dir;
            das.held_secs = 0.0;
            das.arr_acc = 0.0;
            game.move_horizontal(das.dir);
        } else {
            das.dir = 0;
        }
    }
    if das.dir != 0 && keys.pressed(dir_key) {
        das.held_secs += time.delta_secs();
        if das.held_secs >= das_secs {
            if arr_secs <= 0.0 {
                // ARR 0: teleport to the wall.
                while game.move_horizontal(das.dir) {}
            } else {
                das.arr_acc += time.delta_secs();
                while das.arr_acc >= arr_secs {
                    das.arr_acc -= arr_secs;
                    if !game.move_horizontal(das.dir) {
                        das.arr_acc = 0.0;
                        break;
                    }
                }
            }
        }
    }

    // --- Everything else ---------------------------------------------------
    game.set_soft_drop(keys.pressed(settings.key_for(Action::SoftDrop)));
    if keys.just_pressed(settings.key_for(Action::RotateCw)) {
        game.rotate(true);
    }
    if keys.just_pressed(settings.key_for(Action::RotateCcw)) {
        game.rotate(false);
    }
    if keys.just_pressed(settings.key_for(Action::Hold)) {
        game.hold();
    }
    if keys.just_pressed(settings.key_for(Action::HardDrop)) {
        game.hard_drop();
    }
    let _ = &mut sfx; // sfx for inputs are driven by BoardEvents in tick_games
}

fn cpu_drive(
    time: Res<Time>,
    mut query: Query<(&mut GameSession, &mut CpuControlled)>,
) {
    for (mut session, mut cpu) in &mut query {
        let CpuControlled {
            profile,
            rng,
            plan,
            planned_piece,
            hold_done,
            timer,
        } = &mut *cpu;
        let game = &mut session.game;
        if game.game_over {
            continue;
        }

        // New piece? Re-plan after a "thinking" pause.
        let piece_id = game.stats.pieces;
        if *planned_piece != Some(piece_id) {
            *planned_piece = Some(piece_id);
            *plan = None;
            *hold_done = false;
            *timer = profile.think_time;
            continue;
        }

        *timer -= time.delta_secs();
        if *timer > 0.0 {
            continue;
        }

        if plan.is_none() {
            let next = game.queue.front().copied();
            *plan = ai::plan(&game.board, game.active.kind, game.hold, next, profile, rng);
            if plan.is_none() {
                // Nowhere to go: give up gracefully by dropping.
                game.hard_drop();
                continue;
            }
        }

        let Some(p) = *plan else { continue };
        *timer = profile.action_interval;

        // Execute one virtual key press per interval.
        if p.use_hold && !*hold_done {
            game.hold();
            *hold_done = true;
            // After hold a different piece is active; the plan targeted it.
            return;
        }
        let rot_now = game.active.rot;
        if rot_now != p.rot {
            let cw_steps = (4 + p.rot.index() as i8 - rot_now.index() as i8) % 4;
            let ok = if cw_steps == 3 {
                game.rotate(false)
            } else {
                game.rotate(true)
            };
            if !ok {
                // Rotation stuck (rare): just drop it where it is.
                game.hard_drop();
            }
            continue;
        }
        if game.active.x != p.x {
            let dir = if p.x > game.active.x { 1 } else { -1 };
            if !game.move_horizontal(dir) {
                game.hard_drop();
            }
            continue;
        }
        game.hard_drop();
    }
}

fn tick_games(
    time: Res<Time>,
    mut query: Query<(Entity, &BoardIndex, &mut GameSession)>,
    mut events: MessageWriter<BoardEvent>,
    mut next: ResMut<NextState<PlayState>>,
    mut commands: Commands,
    mode: Res<GameMode>,
    result: Option<Res<SessionResult>>,
) {
    // Advance simulations and gather events.
    let mut outgoing: Vec<(usize, u32)> = Vec::new();
    let mut boards: Vec<(Entity, usize)> = Vec::new();
    for (entity, index, mut session) in &mut query {
        session.game.tick(time.delta_secs());
        boards.push((entity, index.0));
        for event in session.game.take_events() {
            if let GameEvent::Cleared(clear) = &event {
                if clear.attack > 0 {
                    outgoing.push((index.0, clear.attack));
                }
            }
            events.write(BoardEvent {
                board: entity,
                index: index.0,
                event,
            });
        }
    }

    // Route attacks to the other board.
    for (from, attack) in outgoing {
        for (_, index, mut session) in &mut query {
            if index.0 != from {
                session.game.queue_garbage(attack);
            }
        }
    }

    // Win / lose detection (only once).
    if result.is_some() {
        return;
    }
    let dead: Vec<usize> = query
        .iter()
        .filter(|(_, _, s)| s.game.game_over)
        .map(|(_, i, _)| i.0)
        .collect();
    if dead.is_empty() {
        return;
    }
    let verdict = match *mode {
        GameMode::Single => SessionResult::SoloOver,
        GameMode::VsCpu(_) => {
            // If both died the same frame, the human loses ties generously:
            // call it a win for the board that survived, or player win on tie.
            let winner = if dead.contains(&0) && dead.contains(&1) {
                0
            } else if dead.contains(&0) {
                1
            } else {
                0
            };
            SessionResult::VsWin { winner }
        }
    };
    commands.insert_resource(verdict);
    next.set(PlayState::Finished);
}
