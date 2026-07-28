//! Game session orchestration: board entities, human input (DAS/ARR),
//! the CPU driver, garbage routing, and the first-to-n round/match flow
//! with stage grading.

use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::{PlaySfx, Sfx};
use crate::config::{Action, GameSettings};
use crate::core::ai::{self, AiProfile, Plan};
use crate::core::game::{Game, GameEvent, Stats};
use crate::progress::{Grade, Progress};
use crate::state::{AppState, GameMode, PlayState};

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
    pub profile: AiProfile,
    rng: StdRng,
    plan: Option<Plan>,
    /// `stats.pieces` value the current plan was made for.
    planned_piece: Option<u32>,
    hold_done: bool,
    timer: f32,
}

impl CpuControlled {
    fn new(profile: AiProfile, seed: u64) -> Self {
        Self {
            profile,
            rng: StdRng::seed_from_u64(seed ^ 0xC0FFEE),
            plan: None,
            planned_piece: None,
            hold_done: false,
            timer: profile.think_time,
        }
    }

    fn reset_for_round(&mut self) {
        self.plan = None;
        self.planned_piece = None;
        self.hold_done = false;
        self.timer = self.profile.think_time;
    }
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

/// Final outcome of the whole match.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionResult {
    /// Single player topped out.
    SoloOver,
    /// VS: which board index won the match.
    VsWin { winner: usize },
}

/// Who took the round that just ended (drives the RoundOver overlay).
#[derive(Resource, Debug, Clone, Copy)]
pub struct LastRound {
    pub winner: usize,
}

/// Set when the player clears a stage; consumed by the result overlay.
#[derive(Resource, Debug, Clone, Copy)]
pub struct StageClear {
    /// Kept for symmetry/logging; the overlay derives the stage from
    /// `GameMode` so it can also label failed attempts.
    #[allow(dead_code)]
    pub stage: u32,
    pub grade: Grade,
    pub new_best: bool,
}

/// Player statistics accumulated across all rounds of a match.
#[derive(Debug, Clone, Copy, Default)]
pub struct MatchAggregate {
    pub attack: u32,
    pub tetrises: u32,
    pub tspins: u32,
    pub max_combo: u32,
    pub perfect_clears: u32,
    pub time: f64,
}

impl MatchAggregate {
    fn absorb(&mut self, stats: &Stats) {
        self.attack += stats.attack_sent;
        self.tetrises += stats.tetrises;
        self.tspins += stats.tspins;
        self.max_combo = self.max_combo.max(stats.max_combo);
        self.perfect_clears += stats.perfect_clears;
        self.time += stats.time;
    }
}

/// First-to-n match bookkeeping.
#[derive(Resource, Debug, Clone)]
pub struct MatchState {
    pub stage: Option<u32>,
    pub wins_needed: u32,
    pub player_wins: u32,
    pub cpu_wins: u32,
    /// 1-based round counter.
    pub round: u32,
    pub agg: MatchAggregate,
}

impl MatchState {
    fn new(mode: GameMode) -> Self {
        let (stage, wins_needed) = match mode {
            GameMode::Single => (None, 1),
            // Boss stages (10/20/30) are first-to-3, the rest first-to-2.
            GameMode::VsCpu { stage } => {
                (Some(stage), if stage % 10 == 0 { 3 } else { 2 })
            }
        };
        Self {
            stage,
            wins_needed,
            player_wins: 0,
            cpu_wins: 0,
            round: 1,
            agg: MatchAggregate::default(),
        }
    }
}

#[derive(Resource)]
pub struct Countdown {
    timer: Timer,
    remaining: u32,
}

#[derive(Resource)]
struct RoundOverTimer(Timer);

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BoardEvent>()
            .init_resource::<GameMode>()
            .add_systems(
                OnEnter(AppState::Playing),
                (spawn_session, crate::render::setup_board_visuals).chain(),
            )
            .add_systems(OnEnter(PlayState::Countdown), start_countdown)
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
            .add_systems(OnEnter(PlayState::RoundOver), round_over_enter)
            .add_systems(
                Update,
                round_over_tick.run_if(in_state(PlayState::RoundOver)),
            )
            .add_systems(
                Update,
                pause_toggle.run_if(in_state(PlayState::Running).or_else(in_state(PlayState::Paused))),
            );
    }
}

fn spawn_session(mut commands: Commands, mode: Res<GameMode>) {
    let seed: u64 = rand::rng().random();
    // Both players get the same piece sequence (standard for versus play).
    let start_level = 1;

    commands.remove_resource::<SessionResult>();
    commands.remove_resource::<LastRound>();
    commands.remove_resource::<StageClear>();
    commands.insert_resource(MatchState::new(*mode));

    commands.spawn((
        GameSession { game: Game::new(seed, start_level) },
        BoardIndex(0),
        HumanControlled,
        DasState::default(),
        DespawnOnExit(AppState::Playing),
        Transform::default(),
        Visibility::default(),
    ));

    if let GameMode::VsCpu { stage } = *mode {
        let profile = AiProfile::for_stage(stage);
        commands.spawn((
            GameSession { game: Game::new(seed, start_level) },
            BoardIndex(1),
            CpuControlled::new(profile, seed),
            DespawnOnExit(AppState::Playing),
            Transform::default(),
            Visibility::default(),
        ));
    }
}

fn start_countdown(mut commands: Commands, mut sfx: MessageWriter<PlaySfx>) {
    commands.insert_resource(Countdown {
        timer: Timer::from_seconds(0.8, TimerMode::Repeating),
        remaining: 3,
    });
    sfx.write(PlaySfx::new(Sfx::Countdown));
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

    // Keys already held when control resumes (countdown end, unpause) never
    // fire just_pressed; pick them up with a fully charged DAS so holding a
    // direction through the countdown slides the piece immediately.
    if das.dir == 0 && (keys.pressed(left) != keys.pressed(right)) {
        das.dir = if keys.pressed(left) { -1 } else { 1 };
        das.held_secs = das_secs;
        das.arr_acc = 0.0;
        game.move_horizontal(das.dir);
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
            let next2 = game.queue.get(1).copied();
            *plan = ai::plan(
                &game.board,
                game.active.kind,
                game.hold,
                next,
                next2,
                game.incoming_total(),
                profile,
                rng,
            );
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
            continue;
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

/// Arcade-style grade for a won stage.
fn compute_grade(ms: &MatchState) -> Grade {
    let mut pts = 0.0f32;
    // Dominance: dropping no rounds is worth a lot.
    pts += if ms.cpu_wins == 0 {
        35.0
    } else if ms.cpu_wins == 1 {
        18.0
    } else {
        8.0
    };
    // Attack per minute.
    let minutes = (ms.agg.time / 60.0).max(0.05) as f32;
    let apm = ms.agg.attack as f32 / minutes;
    pts += (apm * 1.1).min(35.0);
    // Style: big clears, spins, combos, perfect clears.
    let style = ms.agg.tetrises * 5
        + ms.agg.tspins * 7
        + ms.agg.max_combo * 2
        + ms.agg.perfect_clears * 12;
    pts += (style as f32).min(30.0);

    if pts >= 85.0 {
        Grade::S
    } else if pts >= 65.0 {
        Grade::A
    } else if pts >= 45.0 {
        Grade::B
    } else if pts >= 28.0 {
        Grade::C
    } else {
        Grade::D
    }
}

#[allow(clippy::too_many_arguments)]
fn tick_games(
    time: Res<Time>,
    mut query: Query<(Entity, &BoardIndex, &mut GameSession)>,
    mut events: MessageWriter<BoardEvent>,
    mut next: ResMut<NextState<PlayState>>,
    mut commands: Commands,
    mode: Res<GameMode>,
    mut match_state: ResMut<MatchState>,
    mut progress: ResMut<Progress>,
    result: Option<Res<SessionResult>>,
) {
    // Advance simulations and gather events.
    let mut outgoing: Vec<(usize, u32)> = Vec::new();
    for (entity, index, mut session) in &mut query {
        session.game.tick(time.delta_secs());
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

    // Round end detection (only once per round).
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

    // Bank the player's stats for this round.
    if let Some((_, _, player)) = query.iter().find(|(_, i, _)| i.0 == 0) {
        match_state.agg.absorb(&player.game.stats);
    }

    match *mode {
        GameMode::Single => {
            commands.insert_resource(SessionResult::SoloOver);
            next.set(PlayState::Finished);
        }
        GameMode::VsCpu { stage } => {
            // Ties go to the player, generously.
            let winner = if dead.contains(&1) { 0 } else { 1 };
            if winner == 0 {
                match_state.player_wins += 1;
            } else {
                match_state.cpu_wins += 1;
            }
            commands.insert_resource(LastRound { winner });

            let decided = match_state.player_wins >= match_state.wins_needed
                || match_state.cpu_wins >= match_state.wins_needed;
            if decided {
                let match_winner = if match_state.player_wins >= match_state.wins_needed {
                    0
                } else {
                    1
                };
                commands.insert_resource(SessionResult::VsWin { winner: match_winner });
                if match_winner == 0 {
                    let grade = compute_grade(&match_state);
                    let new_best = progress.record_clear(stage, grade);
                    commands.insert_resource(StageClear { stage, grade, new_best });
                }
                next.set(PlayState::Finished);
            } else {
                next.set(PlayState::RoundOver);
            }
        }
    }
}

fn round_over_enter(
    mut commands: Commands,
    last: Option<Res<LastRound>>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    commands.insert_resource(RoundOverTimer(Timer::from_seconds(2.8, TimerMode::Once)));
    if let Some(last) = last {
        sfx.write(if last.winner == 0 {
            PlaySfx::new(Sfx::LevelUp)
        } else {
            PlaySfx { sfx: Sfx::GameOver, gain: 0.55 }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(player_wins: u32, cpu_wins: u32, agg: MatchAggregate) -> MatchState {
        MatchState {
            stage: Some(10),
            wins_needed: player_wins.max(1),
            player_wins,
            cpu_wins,
            round: player_wins + cpu_wins,
            agg,
        }
    }

    #[test]
    fn dominant_fast_stylish_earns_s() {
        let agg = MatchAggregate {
            attack: 64,          // 32 APM over 2 minutes
            tetrises: 2,
            tspins: 1,
            max_combo: 3,
            perfect_clears: 0,
            time: 120.0,
        };
        assert_eq!(compute_grade(&state(2, 0, agg)), Grade::S);
    }

    #[test]
    fn scrappy_slow_win_earns_low_grade() {
        let agg = MatchAggregate {
            attack: 6, // ~1.2 APM over 5 minutes
            time: 300.0,
            ..Default::default()
        };
        let grade = compute_grade(&state(3, 2, agg));
        assert!(matches!(grade, Grade::D | Grade::C), "got {grade:?}");
    }

    #[test]
    fn grades_improve_with_dominance() {
        let agg = MatchAggregate {
            attack: 30,
            tetrises: 1,
            time: 120.0,
            ..Default::default()
        };
        let sweep = compute_grade(&state(2, 0, agg));
        let close = compute_grade(&state(2, 1, agg));
        assert!(
            sweep == close || sweep.better_than(close),
            "sweep {sweep:?} vs close {close:?}"
        );
    }
}

/// Wait out the intermission (or let the player skip it), then reset both
/// boards for the next round.
#[allow(clippy::too_many_arguments)]
fn round_over_tick(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    settings: Res<GameSettings>,
    mut timer: ResMut<RoundOverTimer>,
    mut match_state: ResMut<MatchState>,
    mut boards: Query<(
        &mut GameSession,
        Option<&mut CpuControlled>,
        Option<&mut DasState>,
    )>,
    mut next: ResMut<NextState<PlayState>>,
    mut commands: Commands,
) {
    let skip = keys.just_pressed(KeyCode::Enter)
        || keys.just_pressed(settings.key_for(Action::HardDrop));
    if !timer.0.tick(time.delta()).is_finished() && !skip {
        return;
    }

    let seed: u64 = rand::rng().random();
    for (mut session, cpu, das) in &mut boards {
        session.game = Game::new(seed, 1);
        if let Some(mut cpu) = cpu {
            cpu.reset_for_round();
        }
        if let Some(mut das) = das {
            *das = DasState::default();
        }
    }
    match_state.round += 1;
    commands.remove_resource::<RoundOverTimer>();
    commands.remove_resource::<LastRound>();
    next.set(PlayState::Countdown);
}
