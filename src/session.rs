//! Game session orchestration: board entities, human input (DAS/ARR),
//! the CPU driver, garbage routing, and the first-to-n round/match flow
//! with stage grading.

use bevy::prelude::*;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::audio::{PlaySfx, Sfx};
use crate::config::{Action, CustomMatchConfig, GameSettings, Opponent};
use crate::core::ai::{self, AiProfile, Plan, Step};
use crate::core::board::BOARD_WIDTH;
use crate::core::game::{Game, GameEvent, Leveling, MAX_TIMED_LEVEL, SpinRule, Stats, Zone};
use crate::input::PadInput;
use crate::progress::{Grade, Progress};
use crate::render::BoardKick;
use crate::state::{AppState, GameMode, PlayState};

/// VS rounds speed up on a clock — one gravity level every this many
/// seconds, for both players alike — instead of by cleared lines. Playing
/// well (downstacking a lot) no longer drags your own gravity up faster
/// than the opponent's, and no round can stay slow forever. 25 s/level
/// roughly matches the old pace at a typical ~20 lines/minute.
const VS_SECONDS_PER_LEVEL: f32 = 25.0;

/// Sprint race: clear this many lines to finish.
pub const SPRINT_GOAL_LINES: u32 = 40;

/// Where zen's gravity ramp stops: the top of the guideline curve, which
/// zen climbs on marathon's ten-lines-a-level rule. Nothing in zen can be
/// lost, so the ceiling is not a difficulty wall but a destination —
/// reaching it changes the backdrop and the music (see [`ZenPeak`]).
pub const ZEN_MAX_LEVEL: u32 = MAX_TIMED_LEVEL;

/// VS margin time: past this many seconds into a round, both players'
/// attack scales up (x1.5 -> x4, see `Game::attack_multiplier`) so long
/// rounds escalate to a finish instead of stalling.
const MARGIN_TIME_SECS: f32 = 90.0;

/// Cheese-style garbage: one hole per row, never directly above the
/// previous row's hole (a shared well would trivialize it).
fn stack_cheese(game: &mut Game, seed: u64, rows: u32) {
    let mut rng = StdRng::seed_from_u64(seed ^ 0xD16);
    let mut prev: i8 = -1;
    for _ in 0..rows {
        let mut hole = rng.random_range(0..BOARD_WIDTH);
        while hole == prev {
            hole = rng.random_range(0..BOARD_WIDTH);
        }
        game.board.add_garbage(1, hole);
        prev = hole;
    }
}

/// A fresh board for `mode`: VS boards use the timed gravity ramp, zen
/// climbs marathon's up to [`ZEN_MAX_LEVEL`], and sprint alone pins
/// gravity at level 1. Custom matches apply the user's rule sheet.
fn new_game(seed: u64, mode: GameMode, custom: &CustomMatchConfig) -> Game {
    let start_level = match mode {
        GameMode::Custom if custom.speed_level > 0 => custom.speed_level,
        // Dev helper: `BEVYTRIS_ZEN_LEVEL=<n>` starts a zen run part-way
        // up the ramp, so the peak — and the backdrop and music that come
        // with it — can be auditioned without first clearing two hundred
        // lines.
        GameMode::Zen => std::env::var("BEVYTRIS_ZEN_LEVEL")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .map_or(1, |level| level.clamp(1, ZEN_MAX_LEVEL)),
        _ => 1,
    };
    let mut game = Game::new(seed, start_level);
    match mode {
        GameMode::VsCpu { .. } => {
            game.leveling = Leveling::Timed {
                seconds_per_level: VS_SECONDS_PER_LEVEL,
            };
            game.margin_time = Some(MARGIN_TIME_SECS);
        }
        GameMode::ZoneBattle { .. } => {
            game.leveling = Leveling::Timed {
                seconds_per_level: VS_SECONDS_PER_LEVEL,
            };
            game.margin_time = Some(MARGIN_TIME_SECS);
            game.zone = Some(Zone::default());
            // The campaign that is already about firing off everything you
            // have: every piece may spin, so a stack full of awkward gaps
            // is an attack waiting to happen rather than a mess to dig.
            game.spin_rule = SpinRule::AllSpin;
        }
        GameMode::Sprint => game.leveling = Leveling::Fixed,
        GameMode::Zen => {
            // Marathon's ramp, stopped at the top of the curve. Nothing
            // can end the run, so the rising floor is not pressure — it
            // is the only thing giving an endless mode a shape, and the
            // ceiling is somewhere to arrive rather than somewhere to
            // fail.
            game.leveling = Leveling::Lines;
            game.max_level = ZEN_MAX_LEVEL;
            game.endless = true;
        }
        GameMode::Custom => {
            game.leveling = if custom.speed_level > 0 {
                Leveling::Fixed
            } else {
                Leveling::Timed {
                    seconds_per_level: VS_SECONDS_PER_LEVEL,
                }
            };
            if custom.margin_secs > 0 {
                game.margin_time = Some(custom.margin_secs as f32);
            }
            if custom.zone {
                game.zone = Some(Zone::default());
            }
            if custom.start_garbage > 0 {
                stack_cheese(&mut game, seed, custom.start_garbage);
            }
            // Rules that apply equally to both boards.
            if custom.all_spin {
                game.spin_rule = SpinRule::AllSpin;
            }
            game.b2b_escalation = custom.b2b_chain;
            game.garbage_messiness = custom.messiness;
            game.hold_enabled = custom.hold;
            game.visible_previews = custom.previews as usize;
        }
        GameMode::Single => {}
    }
    // Dev helper: BEVYTRIS_ZONE_CHARGE=1 starts every zone gauge full so
    // zone flows can be tested without earning the charge first.
    if std::env::var("BEVYTRIS_ZONE_CHARGE").is_ok()
        && let Some(zone) = &mut game.zone
    {
        zone.charge = 1.0;
    }
    game
}

/// Elapsed race time as "m:ss.cc".
pub fn format_race_time(secs: f64) -> String {
    let ms = (secs * 1000.0).round() as u64;
    format!(
        "{}:{:02}.{:02}",
        ms / 60_000,
        ms / 1000 % 60,
        ms % 1000 / 10
    )
}

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

/// Board 0 — the one the camera effects, the danger audio, the music
/// intensity and the solo result screen all speak for. Always present and
/// always unique, which `HumanControlled` stopped being once local versus
/// put a second human on the right-hand board.
#[derive(Component)]
pub struct PrimaryPlayer;

/// Which human is at the controls: 0 uses the main key set (and gamepad
/// slot 0), 1 uses the player 2 set (and gamepad slot 1).
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub struct PlayerSeat(pub usize);

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
    /// Next step of the plan to execute.
    step_idx: usize,
    /// `stats.pieces` value the current plan was made for.
    planned_piece: Option<u32>,
    hold_done: bool,
    timer: f32,
    /// How long the CPU has been sitting on a full zone gauge.
    zone_full_secs: f32,
}

impl CpuControlled {
    fn new(profile: AiProfile, seed: u64) -> Self {
        Self {
            profile,
            rng: StdRng::seed_from_u64(seed ^ 0xC0FFEE),
            plan: None,
            step_idx: 0,
            planned_piece: None,
            hold_done: false,
            timer: profile.think_time,
            zone_full_secs: 0.0,
        }
    }

    fn reset_for_round(&mut self) {
        self.plan = None;
        self.step_idx = 0;
        self.planned_piece = None;
        self.hold_done = false;
        self.timer = self.profile.think_time;
        self.zone_full_secs = 0.0;
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
    /// A race (sprint / dig) reached its goal.
    RaceDone,
    /// VS: which board index won the match.
    VsWin { winner: usize },
}

/// Set when a race reaches its goal; consumed by the result overlay.
#[derive(Resource, Debug, Clone, Copy)]
pub struct RaceResult {
    /// Finish time in seconds.
    pub time: f64,
    pub new_best: bool,
    /// Personal best after this run, in milliseconds.
    pub best_ms: u64,
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
    fn new(mode: GameMode, custom: &CustomMatchConfig) -> Self {
        let (stage, wins_needed) = match mode {
            GameMode::Single | GameMode::Sprint | GameMode::Zen => (None, 1),
            // Boss stages (10/20/30) are first-to-3, the rest first-to-2.
            GameMode::VsCpu { stage } | GameMode::ZoneBattle { stage } => {
                (Some(stage), if stage % 10 == 0 { 3 } else { 2 })
            }
            GameMode::Custom => (None, custom.wins_needed.clamp(1, 5)),
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

/// The whole intermission between rounds. The losing board leaves at once
/// and takes about three quarters of a second to clear the bottom of the
/// screen; the rest is the winner holding their pose alone before the next
/// countdown.
const ROUND_OVER_SECS: f32 = 4.0;

/// Lines already added to the lifetime zen total this session. Zen has no
/// finish line to bank at, so lines are handed over as they are cleared;
/// this watermark is what keeps a restart from counting them twice.
#[derive(Resource, Default)]
struct ZenBanked(u32);

/// Zen reached [`ZEN_MAX_LEVEL`] — the run is at terminal gravity. The
/// backdrop and the music both change on this, so it is published once
/// here rather than re-derived on each side.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZenPeak(pub bool);

/// Latch [`ZenPeak`] the moment the ramp tops out. It only ever latches
/// on: a wipe costs the field, not the speed the run earned.
fn watch_zen_peak(
    mode: Res<GameMode>,
    mut peak: ResMut<ZenPeak>,
    boards: Query<&GameSession, With<PrimaryPlayer>>,
) {
    if peak.0 || *mode != GameMode::Zen {
        return;
    }
    if boards.iter().any(|s| s.game.level >= ZEN_MAX_LEVEL) {
        peak.0 = true;
    }
}

/// Feed cleared lines into the lifetime zen total as they happen.
fn bank_zen_lines(
    mode: Res<GameMode>,
    mut banked: ResMut<ZenBanked>,
    mut progress: ResMut<Progress>,
    boards: Query<&GameSession, With<PrimaryPlayer>>,
) {
    if *mode != GameMode::Zen {
        return;
    }
    let Ok(session) = boards.single() else {
        return;
    };
    let lines = session.game.lines;
    if lines > banked.0 {
        progress.add_zen_lines((lines - banked.0) as u64);
        banked.0 = lines;
    }
}

/// `add_zen_lines` only touches the disk on a level-up, so leaving zen
/// part-way to the next level would otherwise drop those lines.
fn save_zen_on_exit(mode: Res<GameMode>, progress: Res<Progress>) {
    if *mode == GameMode::Zen {
        crate::progress::save_progress(&progress);
    }
}

pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BoardEvent>()
            .init_resource::<GameMode>()
            .add_systems(
                OnEnter(AppState::Playing),
                // Ordering is load-bearing twice over: the visuals need the
                // boards, and the portraits need the `BoardTheme` the
                // visuals insert.
                (
                    spawn_session,
                    crate::render::setup_board_visuals,
                    crate::character::spawn_portraits,
                )
                    .chain(),
            )
            .add_systems(OnEnter(PlayState::Countdown), start_countdown)
            .add_systems(
                Update,
                countdown_tick.run_if(in_state(PlayState::Countdown)),
            )
            .init_resource::<ZenBanked>()
            .init_resource::<ZenPeak>()
            .add_systems(OnExit(AppState::Playing), save_zen_on_exit)
            .add_systems(
                Update,
                (
                    human_input,
                    cpu_drive,
                    tick_games,
                    bank_zen_lines,
                    watch_zen_peak,
                )
                    .chain()
                    .run_if(in_state(PlayState::Running)),
            )
            .add_systems(
                OnEnter(PlayState::RoundOver),
                (round_over_enter, drop_losing_board),
            )
            .add_systems(OnEnter(PlayState::Finished), drop_losing_board)
            // A new round reuses the board entities, so anything the last
            // one did to them has to be undone before it starts.
            .add_systems(OnEnter(PlayState::Countdown), recover_boards)
            .add_systems(
                Update,
                round_over_tick.run_if(in_state(PlayState::RoundOver)),
            )
            .add_systems(
                Update,
                pause_toggle
                    .run_if(in_state(PlayState::Running).or_else(in_state(PlayState::Paused))),
            );
    }
}

fn spawn_session(mut commands: Commands, mode: Res<GameMode>, settings: Res<GameSettings>) {
    let seed: u64 = rand::rng().random();
    let custom = settings.custom;
    // Both players get the same piece sequence (standard for versus play).

    commands.remove_resource::<SessionResult>();
    commands.remove_resource::<LastRound>();
    commands.remove_resource::<StageClear>();
    commands.remove_resource::<RaceResult>();
    commands.insert_resource(MatchState::new(*mode, &custom));
    commands.insert_resource(ZenBanked::default());
    commands.insert_resource(ZenPeak::default());

    commands.spawn((
        GameSession {
            game: new_game(seed, *mode, &custom),
        },
        BoardIndex(0),
        HumanControlled,
        PrimaryPlayer,
        PlayerSeat(0),
        DasState::default(),
        DespawnOnExit(AppState::Playing),
        Transform::default(),
        Visibility::default(),
    ));

    // A local-versus custom match puts a second human on board 1; every
    // other versus mode puts the CPU there.
    if *mode == GameMode::Custom && custom.opponent == Opponent::Human {
        commands.spawn((
            GameSession {
                game: new_game(seed, *mode, &custom),
            },
            BoardIndex(1),
            HumanControlled,
            PlayerSeat(1),
            DasState::default(),
            DespawnOnExit(AppState::Playing),
            Transform::default(),
            Visibility::default(),
        ));
        return;
    }

    let cpu_profile = match *mode {
        GameMode::VsCpu { stage } | GameMode::ZoneBattle { stage } => {
            Some(AiProfile::for_stage(stage))
        }
        GameMode::Custom => {
            let mut profile =
                AiProfile::for_stage_styled(custom.cpu_level, custom.cpu_style.archetype());
            // A banned hold slot binds the CPU too, or it plans around a
            // resource the rules just took away from both boards.
            profile.uses_hold &= custom.hold;
            Some(profile)
        }
        _ => None,
    };
    if let Some(profile) = cpu_profile {
        commands.spawn((
            GameSession {
                game: new_game(seed, *mode, &custom),
            },
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
    pad: Res<PadInput>,
    settings: Res<GameSettings>,
    state: Res<State<PlayState>>,
    mut next: ResMut<NextState<PlayState>>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    if keys.any_just_pressed(settings.keys_for(Action::Pause).iter().copied())
        || pad.action_just_pressed(Action::Pause)
    {
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

/// Where one seat's inputs come from: its key set, plus either every
/// connected pad (single-player: any controller just works) or one
/// specific pad (local versus: player 2 must not steer player 1).
struct Controls<'a> {
    keys: &'a ButtonInput<KeyCode>,
    pad: &'a PadInput,
    settings: &'a GameSettings,
    seat: usize,
    /// None = read all pads merged.
    pad_slot: Option<usize>,
}

impl Controls<'_> {
    /// Every key this seat has for `action` — an action can hold more than
    /// one, so hold on both shoulder buttons is a thing you can ask for.
    fn keys_for(&self, action: Action) -> &[KeyCode] {
        if self.seat == 0 {
            self.settings.keys_for(action)
        } else {
            self.settings.keys2_for(action)
        }
    }

    fn pressed(&self, action: Action) -> bool {
        self.keys.any_pressed(self.keys_for(action).iter().copied())
            || match self.pad_slot {
                Some(slot) => self.pad.slot_pressed(slot, action),
                None => self.pad.action_pressed(action),
            }
    }

    fn just_pressed(&self, action: Action) -> bool {
        self.keys
            .any_just_pressed(self.keys_for(action).iter().copied())
            || match self.pad_slot {
                Some(slot) => self.pad.slot_just_pressed(slot, action),
                None => self.pad.action_just_pressed(action),
            }
    }
}

fn human_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    pad: Res<PadInput>,
    settings: Res<GameSettings>,
    mut query: Query<(&PlayerSeat, &mut GameSession, &mut DasState), With<HumanControlled>>,
) {
    // With a second human on the board, each player is pinned to one pad;
    // alone, any pad drives the only board there is.
    let versus = query.iter().count() > 1;
    for (seat, mut session, mut das) in &mut query {
        let c = Controls {
            keys: &keys,
            pad: &pad,
            settings: &settings,
            seat: seat.0,
            pad_slot: versus.then_some(seat.0),
        };
        drive_board(&time, &c, &settings, &mut session.game, &mut das);
    }
}

fn drive_board(
    time: &Time,
    c: &Controls,
    settings: &GameSettings,
    game: &mut Game,
    das: &mut DasState,
) {
    if game.game_over {
        return;
    }

    // Keyboard and gamepad merge into one digital state; DAS/ARR then
    // treats both sources identically.
    let left_just = c.just_pressed(Action::MoveLeft);
    let right_just = c.just_pressed(Action::MoveRight);
    let left_down = c.pressed(Action::MoveLeft);
    let right_down = c.pressed(Action::MoveRight);
    let das_secs = settings.das_ms as f32 / 1000.0;
    let arr_secs = settings.arr_ms as f32 / 1000.0;

    // --- Horizontal movement with DAS/ARR ---------------------------------
    // The most recently pressed direction wins while both are held.
    if left_just {
        das.dir = -1;
        das.held_secs = 0.0;
        das.arr_acc = 0.0;
        game.move_horizontal(-1);
    }
    if right_just {
        das.dir = 1;
        das.held_secs = 0.0;
        das.arr_acc = 0.0;
        game.move_horizontal(1);
    }

    // Inputs already held when control resumes (countdown end, unpause)
    // never fire just_pressed; pick them up with a fully charged DAS so
    // holding a direction through the countdown slides the piece
    // immediately.
    if das.dir == 0 && (left_down != right_down) {
        das.dir = if left_down { -1 } else { 1 };
        das.held_secs = das_secs;
        das.arr_acc = 0.0;
        game.move_horizontal(das.dir);
    }

    let dir_down = if das.dir < 0 { left_down } else { right_down };
    if das.dir != 0 && !dir_down {
        // Released the active direction; fall back to the other if held.
        let other_down = if das.dir < 0 { right_down } else { left_down };
        if other_down {
            das.dir = -das.dir;
            das.held_secs = 0.0;
            das.arr_acc = 0.0;
            game.move_horizontal(das.dir);
        } else {
            das.dir = 0;
        }
    }
    if das.dir != 0 && dir_down {
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
    game.soft_drop_factor = settings.sdf_factor();
    game.set_soft_drop(c.pressed(Action::SoftDrop));
    if c.just_pressed(Action::RotateCw) {
        game.rotate(true);
    }
    if c.just_pressed(Action::RotateCcw) {
        game.rotate(false);
    }
    if c.just_pressed(Action::Hold) {
        game.hold();
    }
    if c.just_pressed(Action::HardDrop) {
        game.hard_drop();
    }
    if c.just_pressed(Action::Zone) {
        game.activate_zone();
    }
}

fn cpu_drive(time: Res<Time>, mut query: Query<(&mut GameSession, &mut CpuControlled)>) {
    for (mut session, mut cpu) in &mut query {
        let CpuControlled {
            profile,
            rng,
            plan,
            step_idx,
            planned_piece,
            hold_done,
            timer,
            zone_full_secs,
        } = &mut *cpu;
        let game = &mut session.game;
        if game.game_over {
            continue;
        }

        // Zone policy: fire when in trouble (tall stack or garbage on the
        // way); if the gauge just sits full for a while, use it anyway.
        if game
            .zone
            .as_ref()
            .is_some_and(|z| z.charge >= 1.0 && z.active.is_none())
        {
            *zone_full_secs += time.delta_secs();
            let stack = game.board.column_heights().into_iter().max().unwrap_or(0);
            if stack >= 10 || game.incoming_total() >= 4 || *zone_full_secs > 8.0 {
                game.activate_zone();
            }
        } else {
            *zone_full_secs = 0.0;
        }

        // New piece? Re-plan after a "thinking" pause.
        let piece_id = game.stats.pieces;
        if *planned_piece != Some(piece_id) {
            *planned_piece = Some(piece_id);
            *plan = None;
            *step_idx = 0;
            *hold_done = false;
            *timer = profile.think_time;
            continue;
        }

        *timer -= time.delta_secs();
        if *timer > 0.0 {
            continue;
        }

        if plan.is_none() {
            // The CPU reads exactly the previews the rules put on screen,
            // so a shortened NEXT queue handicaps it the same way.
            let queue: Vec<_> = game
                .queue
                .iter()
                .copied()
                .take(game.visible_previews)
                .collect();
            *plan = ai::plan(
                &game.board,
                game.active,
                game.hold,
                &queue,
                game.incoming_total(),
                game.b2b_chain(),
                game.combo(),
                game.spin_rule,
                profile,
                rng,
            );
            *step_idx = 0;
            if plan.is_none() {
                // Nowhere to go: give up gracefully by dropping.
                game.hard_drop();
                continue;
            }
        }

        let Some(p) = plan.as_ref() else { continue };
        *timer = profile.action_interval;

        // Execute one virtual key press per interval.
        if p.use_hold && !*hold_done {
            game.hold();
            *hold_done = true;
            // After hold a different piece is active; the remaining steps
            // target it (they were planned from its spawn state).
            continue;
        }
        match p.steps.get(*step_idx).copied() {
            Some(step) => {
                *step_idx += 1;
                let ok = match step {
                    Step::Left => game.move_horizontal(-1),
                    Step::Right => game.move_horizontal(1),
                    Step::Cw => game.rotate(true),
                    Step::Ccw => game.rotate(false),
                    Step::Drop => {
                        game.sonic_drop();
                        true
                    }
                };
                if !ok {
                    // Gravity (or a misread) drifted the piece off the
                    // planned path: commit where it stands.
                    game.hard_drop();
                }
            }
            None => game.hard_drop(),
        }
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
    let style =
        ms.agg.tetrises * 5 + ms.agg.tspins * 7 + ms.agg.max_combo * 2 + ms.agg.perfect_clears * 12;
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
    settings: Res<GameSettings>,
    mut match_state: ResMut<MatchState>,
    mut progress: ResMut<Progress>,
    result: Option<Res<SessionResult>>,
) {
    // Advance simulations and gather events.
    let mut outgoing: Vec<(usize, u32)> = Vec::new();
    for (entity, index, mut session) in &mut query {
        session.game.tick(time.delta_secs());
        for event in session.game.take_events() {
            match &event {
                GameEvent::Cleared(clear) if clear.attack > 0 => {
                    outgoing.push((index.0, clear.attack));
                }
                // Zone payloads travel like any other attack.
                GameEvent::ZoneEnded { attack, .. } if *attack > 0 => {
                    outgoing.push((index.0, *attack));
                }
                _ => {}
            }
            events.write(BoardEvent {
                board: entity,
                index: index.0,
                event,
            });
        }
    }

    // Route attacks to the other board. Custom matches scale each side's
    // outgoing garbage by its handicap percentage (rounded half-up).
    for (from, attack) in outgoing {
        let attack = if *mode == GameMode::Custom {
            let pct = if from == 0 {
                settings.custom.player_attack_pct
            } else {
                settings.custom.cpu_attack_pct
            };
            (attack * pct + 50) / 100
        } else {
            attack
        };
        if attack == 0 {
            continue;
        }
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

    // Race goal detection — checked before top-out handling so a run that
    // finishes on its very last piece still counts as a finish.
    if *mode == GameMode::Sprint
        && let Some((_, _, player)) = query.iter().find(|(_, i, _)| i.0 == 0)
    {
        let game = &player.game;
        if !game.game_over && game.lines >= SPRINT_GOAL_LINES {
            match_state.agg.absorb(&game.stats);
            let time = game.stats.time;
            let ms = (time * 1000.0).round() as u64;
            let new_best = progress.record_sprint(ms);
            commands.insert_resource(RaceResult {
                time,
                new_best,
                best_ms: progress.best_sprint_ms.unwrap_or(ms),
            });
            commands.insert_resource(SessionResult::RaceDone);
            next.set(PlayState::Finished);
            return;
        }
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
        // Zen cannot reach this arm: `endless` keeps `game_over` false, so
        // the `dead` list above is always empty. The arm stays exhaustive
        // rather than special-casing it.
        GameMode::Single | GameMode::Sprint | GameMode::Zen => {
            commands.insert_resource(SessionResult::SoloOver);
            next.set(PlayState::Finished);
        }
        GameMode::VsCpu { .. } | GameMode::ZoneBattle { .. } | GameMode::Custom => {
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
                commands.insert_resource(SessionResult::VsWin {
                    winner: match_winner,
                });
                // Both VS campaigns grade wins; each unlocks its own track.
                if match_winner == 0 {
                    let grade = compute_grade(&match_state);
                    match *mode {
                        GameMode::VsCpu { stage } => {
                            let new_best = progress.record_clear(stage, grade);
                            commands.insert_resource(StageClear {
                                stage,
                                grade,
                                new_best,
                            });
                        }
                        GameMode::ZoneBattle { stage } => {
                            let new_best = progress.record_zone_clear(stage, grade);
                            commands.insert_resource(StageClear {
                                stage,
                                grade,
                                new_best,
                            });
                        }
                        _ => {}
                    }
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
    commands.insert_resource(RoundOverTimer(Timer::from_seconds(
        ROUND_OVER_SECS,
        TimerMode::Once,
    )));
    if let Some(last) = last {
        sfx.write(if last.winner == 0 {
            PlaySfx::new(Sfx::LevelUp)
        } else {
            PlaySfx {
                sfx: Sfx::GameOver,
                gain: 0.55,
                crisp: false,
            }
        });
    }
}

/// Send the losing board out of the bottom of the screen.
///
/// Run on the way into both endings a round has: an intermission between
/// rounds and the end of the whole match. It fires the instant the round
/// is decided, on the same frame the stack reached the ceiling — the board
/// falls because it just lost, so any pause between the two reads as the
/// game thinking about it rather than as a consequence.
///
/// Solo has no loser to single out — the one board on screen *is* the
/// player — so nothing drops and the field stays put under the result
/// screen.
fn drop_losing_board(
    mode: Res<GameMode>,
    result: Option<Res<SessionResult>>,
    last: Option<Res<LastRound>>,
    mut boards: Query<(&BoardIndex, &mut BoardKick)>,
) {
    if !matches!(
        *mode,
        GameMode::VsCpu { .. } | GameMode::ZoneBattle { .. } | GameMode::Custom
    ) {
        return;
    }
    // A decided match names its winner; an undecided one falls back to the
    // round that just ended, which is the same question one round smaller.
    let winner = match result.as_deref() {
        Some(SessionResult::VsWin { winner }) => *winner,
        _ => match last.as_deref() {
            Some(round) => round.winner,
            None => return,
        },
    };
    let loser = 1 - winner.min(1);
    for (index, mut kick) in &mut boards {
        if index.0.min(1) == loser {
            // Topple away from the middle of the screen, so the two boards
            // never look like they are leaning into each other.
            kick.drop_away(if loser == 1 { -0.9 } else { 0.9 });
        }
    }
}

/// Put every board back where it belongs. A first-to-n match reuses the
/// same board entities from round to round, so a board that fell out of
/// the screen last round would otherwise start the next one still falling.
fn recover_boards(mut boards: Query<&mut BoardKick>) {
    for mut kick in &mut boards {
        kick.land();
    }
}

/// Wait out the intermission (deliberately not skippable: round results
/// should register before the next countdown starts), then reset both
/// boards for the next round.
#[allow(clippy::too_many_arguments)]
fn round_over_tick(
    time: Res<Time>,
    mode: Res<GameMode>,
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
    if !timer.0.tick(time.delta()).is_finished() {
        return;
    }

    let seed: u64 = rand::rng().random();
    for (mut session, cpu, das) in &mut boards {
        session.game = new_game(seed, *mode, &settings.custom);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One board, one mode, and the latch under test.
    fn peak_app(mode: GameMode, level: u32) -> (App, Entity) {
        let mut app = App::new();
        app.insert_resource(mode)
            .init_resource::<ZenPeak>()
            .add_systems(Update, watch_zen_peak);
        let mut game = Game::new(1, 1);
        game.level = level;
        let board = app
            .world_mut()
            .spawn((GameSession { game }, PrimaryPlayer))
            .id();
        (app, board)
    }

    fn set_level(app: &mut App, board: Entity, level: u32) {
        app.world_mut()
            .get_mut::<GameSession>(board)
            .expect("board")
            .game
            .level = level;
    }

    fn peaked(app: &App) -> bool {
        app.world().resource::<ZenPeak>().0
    }

    #[test]
    fn zen_latches_its_peak_at_the_top_of_the_ramp() {
        let (mut app, board) = peak_app(GameMode::Zen, 1);
        app.update();
        assert!(!peaked(&app), "a fresh run has not peaked");

        set_level(&mut app, board, ZEN_MAX_LEVEL);
        app.update();
        assert!(peaked(&app));

        // The latch only ever closes: a wipe costs the field, not the
        // speed the run earned, and the backdrop and music that came with
        // the peak must not flicker back either.
        set_level(&mut app, board, 1);
        app.update();
        assert!(peaked(&app), "the peak came undone");
    }

    #[test]
    fn only_zen_has_a_peak_to_reach() {
        // Marathon runs the same leveling rule and sails straight past
        // this number; nothing about it should change the scenery.
        let (mut app, _) = peak_app(GameMode::Single, ZEN_MAX_LEVEL + 5);
        app.update();
        assert!(!peaked(&app));
    }

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
            attack: 64, // 32 APM over 2 minutes
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
