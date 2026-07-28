//! Menus: title screen, settings (with live key rebinding), pause overlay
//! and the result screen. Fully keyboard-navigable, mouse also works.

use bevy::input::keyboard::KeyboardInput;
use bevy::input::ButtonState;
use bevy::prelude::*;

use crate::audio::{PlaySfx, Sfx};
use crate::config::{key_label, save_settings, Action, GameSettings};
use crate::core::ai::{AiProfile, MAX_STAGE};
use crate::progress::Progress;
use crate::session::{GameSession, HumanControlled, LastRound, MatchState, SessionResult, StageClear};
use crate::state::{AppState, GameMode, PlayState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Marathon,
    /// Open the stage picker.
    VsSelect,
    /// Start a VS match on this stage.
    Stage(u32),
    Settings,
    Quit,
    Bind(Action),
    AdjustDas,
    AdjustArr,
    AdjustSdf,
    AdjustMaster,
    AdjustBgm,
    AdjustSfx,
    ToggleVsync,
    Back,
}

#[derive(Component)]
struct MenuItem {
    index: usize,
    action: MenuAction,
}

#[derive(Component)]
struct MenuItemLabel;

/// Footer line on the stage select screen describing the focused stage.
#[derive(Component)]
struct StageFooter;

#[derive(Resource, Default)]
struct MenuCursor(usize);

/// Which action is waiting for a key press (settings screen).
/// `just_started` guards the frame the rebind began, so the confirming
/// Enter press / mouse click is never captured as the new binding.
#[derive(Resource, Default)]
struct Rebinding {
    action: Option<Action>,
    just_started: bool,
}

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuCursor>()
            .init_resource::<Rebinding>()
            .add_systems(OnEnter(AppState::Title), setup_title)
            .add_systems(
                OnEnter(AppState::Restarting),
                |mut next: ResMut<NextState<AppState>>| next.set(AppState::Playing),
            )
            .add_systems(OnEnter(AppState::Settings), setup_settings)
            .add_systems(OnEnter(AppState::StageSelect), setup_stage_select)
            .add_systems(OnExit(AppState::Settings), persist_settings)
            .add_systems(OnEnter(PlayState::Paused), setup_pause_overlay)
            .add_systems(OnEnter(PlayState::RoundOver), setup_round_overlay)
            .add_systems(OnEnter(PlayState::Finished), setup_result_overlay)
            .add_systems(
                Update,
                (
                    menu_keyboard_nav,
                    menu_mouse,
                    highlight_items,
                    refresh_settings_labels,
                    refresh_stage_footer.run_if(in_state(AppState::StageSelect)),
                )
                    .run_if(
                        in_state(AppState::Title)
                            .or_else(in_state(AppState::Settings))
                            .or_else(in_state(AppState::StageSelect)),
                    ),
            )
            .add_systems(
                Update,
                // Deterministically AFTER the nav systems: the keypress that
                // starts a rebind must not be read as the new binding.
                rebind_capture
                    .after(menu_keyboard_nav)
                    .after(menu_mouse)
                    .run_if(in_state(AppState::Settings)),
            )
            .add_systems(
                Update,
                overlay_input
                    .run_if(in_state(PlayState::Paused).or_else(in_state(PlayState::Finished))),
            );
    }
}

// ---------------------------------------------------------------------------
// Construction helpers
// ---------------------------------------------------------------------------

fn root_node() -> Node {
    Node {
        width: percent(100),
        height: percent(100),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        row_gap: px(8),
        ..default()
    }
}

fn item_bundle(index: usize, action: MenuAction, label: String, width: f32) -> impl Bundle {
    (
        Button,
        MenuItem { index, action },
        Node {
            width: px(width),
            height: px(42),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::all(px(2)),
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.08, 0.1, 0.16, 0.85)),
        BorderColor::all(Color::NONE),
        children![(
            Text::new(label),
            MenuItemLabel,
            TextFont {
                font_size: FontSize::Px(24.0),
                ..default()
            },
            TextColor(Color::srgb(0.85, 0.9, 1.0)),
        )],
    )
}

fn setup_title(mut commands: Commands, mut cursor: ResMut<MenuCursor>) {
    cursor.0 = 0;
    let items = vec![
        (MenuAction::Marathon, "MARATHON".to_string()),
        (MenuAction::VsSelect, "VS CPU".to_string()),
        (MenuAction::Settings, "SETTINGS".to_string()),
        (MenuAction::Quit, "QUIT".to_string()),
    ];
    commands
        .spawn((root_node(), DespawnOnExit(AppState::Title)))
        .with_children(|parent| {
            parent.spawn((
                Text::new("BEVYTRIS"),
                TextFont {
                    font_size: FontSize::Px(80.0),
                    ..default()
                },
                TextColor(Color::srgb(0.3, 0.85, 1.0)),
                TextShadow::default(),
                Node {
                    margin: UiRect::bottom(px(24)),
                    ..default()
                },
            ));
            for (i, (action, label)) in items.into_iter().enumerate() {
                parent.spawn(item_bundle(i, action, label, 300.0));
            }
            parent.spawn((
                Text::new("Up/Down: select    ENTER: confirm"),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.45, 0.5, 0.6)),
                Node {
                    margin: UiRect::top(px(28)),
                    ..default()
                },
            ));
        });
}

const STAGE_COLUMNS: usize = 6;

fn setup_stage_select(
    mut commands: Commands,
    mut cursor: ResMut<MenuCursor>,
    progress: Res<Progress>,
) {
    // Focus the newest unlocked stage.
    cursor.0 = (progress.unlocked.clamp(1, MAX_STAGE) - 1) as usize;

    commands
        .spawn((root_node(), DespawnOnExit(AppState::StageSelect)))
        .with_children(|parent| {
            parent.spawn((
                Text::new("SELECT STAGE"),
                TextFont {
                    font_size: FontSize::Px(40.0),
                    ..default()
                },
                TextColor(Color::srgb(0.3, 0.85, 1.0)),
                Node {
                    margin: UiRect::bottom(px(14)),
                    ..default()
                },
            ));
            parent
                .spawn(Node {
                    width: px(STAGE_COLUMNS as f32 * 92.0),
                    flex_wrap: FlexWrap::Wrap,
                    justify_content: JustifyContent::Center,
                    row_gap: px(8),
                    column_gap: px(8),
                    ..default()
                })
                .with_children(|grid| {
                    for i in 0..MAX_STAGE as usize {
                        let stage = (i + 1) as u32;
                        let unlocked = progress.is_unlocked(stage);
                        let grade = progress.grades.get(&stage).copied();
                        let label = if !unlocked {
                            format!("{stage:02}  -")
                        } else {
                            match grade {
                                Some(g) => format!("{stage:02}  {}", g.letter()),
                                None => format!("{stage:02}"),
                            }
                        };
                        let text_color = if !unlocked {
                            Color::srgba(0.5, 0.55, 0.65, 0.5)
                        } else {
                            match grade {
                                Some(g) => g.color(),
                                None => Color::srgb(0.9, 0.93, 1.0),
                            }
                        };
                        grid.spawn((
                            Button,
                            MenuItem {
                                index: i,
                                action: MenuAction::Stage(stage),
                            },
                            Node {
                                width: px(84),
                                height: px(52),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                border: UiRect::all(px(2)),
                                border_radius: BorderRadius::all(px(8)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.08, 0.1, 0.16, 0.85)),
                            BorderColor::all(Color::NONE),
                            children![(
                                Text::new(label),
                                MenuItemLabel,
                                TextFont {
                                    font_size: FontSize::Px(16.0),
                                    ..default()
                                },
                                TextColor(text_color),
                            )],
                        ));
                    }
                });
            parent.spawn((
                Text::new(""),
                StageFooter,
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.85, 0.95)),
                Node {
                    margin: UiRect::top(px(18)),
                    ..default()
                },
            ));
            parent.spawn((
                Text::new("Arrows: select    ENTER: start    ESC: back"),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.45, 0.5, 0.6)),
                Node {
                    margin: UiRect::top(px(8)),
                    ..default()
                },
            ));
        });
}

fn refresh_stage_footer(
    cursor: Res<MenuCursor>,
    progress: Res<Progress>,
    mut texts: Query<&mut Text, With<StageFooter>>,
) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    let stage = (cursor.0 as u32 + 1).clamp(1, MAX_STAGE);
    let profile = AiProfile::for_stage(stage);
    let first_to = if stage % 10 == 0 { 3 } else { 2 };
    let status = if !progress.is_unlocked(stage) {
        "LOCKED".to_string()
    } else {
        match progress.grades.get(&stage) {
            Some(g) => format!("BEST: {}", g.letter()),
            None => "NOT CLEARED".to_string(),
        }
    };
    let line = format!(
        "STAGE {stage:02}   TYPE: {}   FIRST TO {first_to}   {status}",
        profile.archetype.label()
    );
    if **text != line {
        **text = line;
    }
}

fn setup_settings(mut commands: Commands, mut cursor: ResMut<MenuCursor>, mut rebinding: ResMut<Rebinding>) {
    cursor.0 = 0;
    rebinding.action = None;
    rebinding.just_started = false;
    commands
        .spawn((root_node(), DespawnOnExit(AppState::Settings)))
        .with_children(|parent| {
            parent.spawn((
                Text::new("SETTINGS"),
                TextFont {
                    font_size: FontSize::Px(40.0),
                    ..default()
                },
                TextColor(Color::srgb(0.3, 0.85, 1.0)),
                Node {
                    margin: UiRect::bottom(px(12)),
                    ..default()
                },
            ));
            let mut index = 0;
            for action in Action::ALL {
                parent.spawn(item_bundle(
                    index,
                    MenuAction::Bind(action),
                    String::new(),
                    420.0,
                ));
                index += 1;
            }
            for action in [
                MenuAction::AdjustDas,
                MenuAction::AdjustArr,
                MenuAction::AdjustSdf,
                MenuAction::AdjustMaster,
                MenuAction::AdjustBgm,
                MenuAction::AdjustSfx,
                MenuAction::ToggleVsync,
                MenuAction::Back,
            ] {
                parent.spawn(item_bundle(index, action, String::new(), 420.0));
                index += 1;
            }
            parent.spawn((
                Text::new("ENTER: rebind    Left/Right: adjust    ESC: back"),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.45, 0.5, 0.6)),
                Node {
                    margin: UiRect::top(px(16)),
                    ..default()
                },
            ));
        });
}

fn settings_label(action: MenuAction, settings: &GameSettings, rebinding: &Rebinding) -> Option<String> {
    Some(match action {
        MenuAction::Bind(a) => {
            let key = if rebinding.action == Some(a) {
                "PRESS KEY...".to_string()
            } else {
                format!("[{}]", key_label(settings.key_for(a)))
            };
            format!("{:<12} {}", a.label(), key)
        }
        MenuAction::AdjustDas => format!("{:<12} {} ms", "DAS", settings.das_ms),
        MenuAction::AdjustArr => format!("{:<12} {} ms", "ARR", settings.arr_ms),
        MenuAction::AdjustSdf => format!("{:<12} {}", "Soft Drop", settings.sdf_label()),
        MenuAction::AdjustMaster => format!("{:<12} {}/10", "Master Vol", settings.master_volume),
        MenuAction::AdjustBgm => format!("{:<12} {}/10", "BGM Vol", settings.bgm_volume),
        MenuAction::AdjustSfx => format!("{:<12} {}/10", "SFX Vol", settings.sfx_volume),
        MenuAction::ToggleVsync => format!(
            "{:<12} {}",
            "VSync",
            if settings.vsync { "ON" } else { "OFF (fast)" }
        ),
        MenuAction::Back => "BACK".to_string(),
        _ => return None,
    })
}

fn refresh_settings_labels(
    settings: Res<GameSettings>,
    rebinding: Res<Rebinding>,
    items: Query<(&MenuItem, &Children)>,
    mut labels: Query<&mut Text, With<MenuItemLabel>>,
) {
    for (item, children) in &items {
        let Some(label) = settings_label(item.action, &settings, &rebinding) else {
            continue;
        };
        for child in children.iter() {
            if let Ok(mut text) = labels.get_mut(child) {
                if **text != label {
                    **text = label.clone();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Navigation & activation
// ---------------------------------------------------------------------------

fn item_count(items: &Query<(Entity, &MenuItem, &Interaction)>) -> usize {
    items.iter().count()
}

fn menu_keyboard_nav(
    keys: Res<ButtonInput<KeyCode>>,
    items: Query<(Entity, &MenuItem, &Interaction)>,
    mut cursor: ResMut<MenuCursor>,
    mut sfx: MessageWriter<PlaySfx>,
    activate: MenuActivateParams,
) {
    if activate.rebinding.action.is_some() {
        return;
    }
    let count = item_count(&items);
    if count == 0 {
        return;
    }
    // The stage picker is a grid: vertical steps jump a whole row and
    // left/right move the cursor instead of adjusting values.
    let grid = *activate.app_state.get() == AppState::StageSelect;
    let step = if grid { STAGE_COLUMNS } else { 1 };

    let mut moved = false;
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) {
        cursor.0 = (cursor.0 + step) % count;
        moved = true;
    }
    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) {
        cursor.0 = (cursor.0 + count - step) % count;
        moved = true;
    }
    if grid {
        if keys.just_pressed(KeyCode::ArrowRight) {
            cursor.0 = (cursor.0 + 1) % count;
            moved = true;
        }
        if keys.just_pressed(KeyCode::ArrowLeft) {
            cursor.0 = (cursor.0 + count - 1) % count;
            moved = true;
        }
    }
    if moved {
        sfx.write(PlaySfx::quiet(Sfx::MenuMove));
    }

    let adjust = if grid {
        0
    } else if keys.just_pressed(KeyCode::ArrowRight) {
        1
    } else if keys.just_pressed(KeyCode::ArrowLeft) {
        -1
    } else {
        0
    };
    let confirm = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::Space);
    let back = keys.just_pressed(KeyCode::Escape);

    if adjust == 0 && !confirm && !back {
        return;
    }
    let selected = items
        .iter()
        .find(|(_, item, _)| item.index == cursor.0)
        .map(|(_, item, _)| item.action);
    run_menu_action(selected, confirm, adjust, back, activate, sfx);
}

fn menu_mouse(
    items: Query<(Entity, &MenuItem, &Interaction), Changed<Interaction>>,
    mut cursor: ResMut<MenuCursor>,
    sfx: MessageWriter<PlaySfx>,
    activate: MenuActivateParams,
) {
    if activate.rebinding.action.is_some() {
        return;
    }
    let mut clicked: Option<MenuAction> = None;
    for (_, item, interaction) in &items {
        match interaction {
            Interaction::Hovered => cursor.0 = item.index,
            Interaction::Pressed => {
                cursor.0 = item.index;
                clicked = Some(item.action);
            }
            Interaction::None => {}
        }
    }
    if clicked.is_some() {
        run_menu_action(clicked, true, 0, false, activate, sfx);
    }
}

/// Everything menu activation needs, grouped to keep system signatures sane.
#[derive(bevy::ecs::system::SystemParam)]
struct MenuActivateParams<'w> {
    next_app: ResMut<'w, NextState<AppState>>,
    mode: ResMut<'w, GameMode>,
    settings: ResMut<'w, GameSettings>,
    rebinding: ResMut<'w, Rebinding>,
    exit: MessageWriter<'w, AppExit>,
    app_state: Res<'w, State<AppState>>,
    progress: Res<'w, Progress>,
}

fn run_menu_action(
    action: Option<MenuAction>,
    confirm: bool,
    adjust: i32,
    back: bool,
    mut p: MenuActivateParams,
    mut sfx: MessageWriter<PlaySfx>,
) {
    if back {
        if matches!(
            *p.app_state.get(),
            AppState::Settings | AppState::StageSelect
        ) {
            sfx.write(PlaySfx::new(Sfx::MenuBack));
            p.next_app.set(AppState::Title);
        }
        return;
    }
    let Some(action) = action else { return };

    if adjust != 0 {
        let s = &mut *p.settings;
        let changed = match action {
            MenuAction::AdjustDas => {
                s.das_ms = (s.das_ms as i32 + adjust * 10).clamp(30, 400) as u32;
                true
            }
            MenuAction::AdjustArr => {
                s.arr_ms = (s.arr_ms as i32 + adjust * 5).clamp(0, 150) as u32;
                true
            }
            MenuAction::AdjustSdf => {
                s.adjust_sdf(adjust);
                true
            }
            MenuAction::AdjustMaster => {
                s.master_volume = (s.master_volume as i32 + adjust).clamp(0, 10) as u32;
                true
            }
            MenuAction::AdjustBgm => {
                s.bgm_volume = (s.bgm_volume as i32 + adjust).clamp(0, 10) as u32;
                true
            }
            MenuAction::AdjustSfx => {
                s.sfx_volume = (s.sfx_volume as i32 + adjust).clamp(0, 10) as u32;
                true
            }
            MenuAction::ToggleVsync => {
                s.vsync = !s.vsync;
                true
            }
            _ => false,
        };
        if changed {
            // Persist immediately so closing the app from this screen
            // doesn't silently drop slider changes.
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        return;
    }

    if !confirm {
        return;
    }
    match action {
        MenuAction::Marathon => {
            *p.mode = GameMode::Single;
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::Playing);
        }
        MenuAction::VsSelect => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::StageSelect);
        }
        MenuAction::Stage(stage) => {
            if p.progress.is_unlocked(stage) {
                *p.mode = GameMode::VsCpu { stage };
                sfx.write(PlaySfx::new(Sfx::MenuSelect));
                p.next_app.set(AppState::Playing);
            } else {
                sfx.write(PlaySfx::new(Sfx::RotateFail));
            }
        }
        MenuAction::Settings => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::Settings);
        }
        MenuAction::Quit => {
            p.exit.write(AppExit::Success);
        }
        MenuAction::Bind(action) => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.rebinding.action = Some(action);
            p.rebinding.just_started = true;
        }
        MenuAction::ToggleVsync => {
            // Enter toggles too (same as left/right).
            p.settings.vsync = !p.settings.vsync;
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        MenuAction::Back => {
            sfx.write(PlaySfx::new(Sfx::MenuBack));
            p.next_app.set(AppState::Title);
        }
        _ => {}
    }
}

fn highlight_items(
    cursor: Res<MenuCursor>,
    mut items: Query<(&MenuItem, &mut BackgroundColor, &mut BorderColor)>,
) {
    for (item, mut bg, mut border) in &mut items {
        if item.index == cursor.0 {
            *bg = BackgroundColor(Color::srgba(0.16, 0.3, 0.45, 0.95));
            *border = BorderColor::all(Color::srgb(0.4, 0.9, 1.0));
        } else {
            *bg = BackgroundColor(Color::srgba(0.08, 0.1, 0.16, 0.85));
            *border = BorderColor::all(Color::NONE);
        }
    }
}

fn rebind_capture(
    mut keyboard: MessageReader<KeyboardInput>,
    mut rebinding: ResMut<Rebinding>,
    mut settings: ResMut<GameSettings>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    if rebinding.just_started {
        // Swallow the keypress/click that opened the rebind prompt.
        keyboard.clear();
        rebinding.just_started = false;
        return;
    }
    let Some(action) = rebinding.action else {
        keyboard.clear();
        return;
    };
    for input in keyboard.read() {
        if input.state != ButtonState::Pressed || input.repeat {
            continue;
        }
        if input.key_code == KeyCode::Escape {
            rebinding.action = None;
            sfx.write(PlaySfx::new(Sfx::MenuBack));
            return;
        }
        if crate::config::bindable_keys().contains(&input.key_code) {
            settings.bind(action, input.key_code);
            save_settings(&settings);
            rebinding.action = None;
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            return;
        }
    }
}

fn persist_settings(settings: Res<GameSettings>) {
    save_settings(&settings);
}

// ---------------------------------------------------------------------------
// Pause & result overlays
// ---------------------------------------------------------------------------

fn overlay_root() -> impl Bundle {
    (
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(10),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.66)),
        GlobalZIndex(10),
    )
}

fn overlay_text(text: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
        TextShadow::default(),
    )
}

fn setup_pause_overlay(mut commands: Commands, settings: Res<GameSettings>) {
    let pause_key = key_label(settings.key_for(Action::Pause));
    commands
        .spawn((overlay_root(), DespawnOnExit(PlayState::Paused)))
        .with_children(|parent| {
            parent.spawn(overlay_text("PAUSED", 64.0, Color::srgb(0.9, 0.95, 1.0)));
            parent.spawn(overlay_text(
                &format!("{pause_key}: resume    R: restart    Q: quit to title"),
                20.0,
                Color::srgb(0.6, 0.7, 0.8),
            ));
        });
}

/// Intermission between rounds of a first-to-n match.
fn setup_round_overlay(
    mut commands: Commands,
    last: Option<Res<LastRound>>,
    match_state: Option<Res<MatchState>>,
) {
    let won = matches!(last.as_deref(), Some(LastRound { winner: 0 }));
    let (headline, color) = if won {
        ("ROUND WIN!", Color::srgb(1.0, 0.9, 0.3))
    } else {
        ("ROUND LOST", Color::srgb(0.9, 0.35, 0.35))
    };
    let score = match_state.map(|ms| {
        format!(
            "YOU {} - {} CPU    (FIRST TO {})",
            ms.player_wins, ms.cpu_wins, ms.wins_needed
        )
    });
    commands
        .spawn((overlay_root(), DespawnOnExit(PlayState::RoundOver)))
        .with_children(|parent| {
            parent.spawn(overlay_text(headline, 58.0, color));
            if let Some(score) = score {
                parent.spawn(overlay_text(&score, 26.0, Color::srgb(0.9, 0.93, 1.0)));
            }
            parent.spawn(overlay_text(
                "next round...",
                18.0,
                Color::srgb(0.6, 0.7, 0.8),
            ));
        });
}

fn setup_result_overlay(
    mut commands: Commands,
    result: Option<Res<SessionResult>>,
    stage_clear: Option<Res<StageClear>>,
    match_state: Option<Res<MatchState>>,
    mode: Res<GameMode>,
    players: Query<&GameSession, With<HumanControlled>>,
) {
    let stage = match *mode {
        GameMode::VsCpu { stage } => Some(stage),
        GameMode::Single => None,
    };
    let (headline, color) = match (result.as_deref(), stage) {
        (Some(SessionResult::VsWin { winner: 0 }), Some(s)) => {
            (format!("STAGE {s:02} CLEAR!"), Color::srgb(1.0, 0.9, 0.3))
        }
        (Some(SessionResult::VsWin { .. }), Some(s)) => {
            (format!("STAGE {s:02} FAILED..."), Color::srgb(0.9, 0.3, 0.3))
        }
        _ => ("GAME OVER".to_string(), Color::srgb(0.9, 0.4, 0.4)),
    };

    // VS matches report the whole-match aggregate; marathon reports the run.
    let stats = if let Some(ms) = match_state.as_ref().filter(|_| stage.is_some()) {
        Some(format!(
            "ROUNDS {}-{}    ATTACK {}    TETRIS {}    T-SPIN {}    COMBO {}    PC {}",
            ms.player_wins,
            ms.cpu_wins,
            ms.agg.attack,
            ms.agg.tetrises,
            ms.agg.tspins,
            ms.agg.max_combo,
            ms.agg.perfect_clears,
        ))
    } else {
        players.single().ok().map(|s| {
            let g = &s.game;
            format!(
                "SCORE {}    LEVEL {}    LINES {}    MAX COMBO {}",
                g.score, g.level, g.lines, g.stats.max_combo
            )
        })
    };

    commands
        .spawn((overlay_root(), DespawnOnExit(PlayState::Finished)))
        .with_children(|parent| {
            parent.spawn(overlay_text(&headline, 64.0, color));
            if let Some(clear) = stage_clear.as_deref() {
                parent.spawn(overlay_text("RANK", 20.0, Color::srgb(0.6, 0.7, 0.8)));
                parent.spawn((
                    Text::new(clear.grade.letter()),
                    TextFont {
                        font_size: FontSize::Px(112.0),
                        ..default()
                    },
                    TextColor(clear.grade.color()),
                    TextShadow::default(),
                ));
                if clear.new_best {
                    parent.spawn(overlay_text(
                        "NEW BEST!",
                        24.0,
                        Color::srgb(0.5, 1.0, 0.6),
                    ));
                }
            }
            if let Some(stats) = stats {
                parent.spawn(overlay_text(&stats, 21.0, Color::srgb(0.85, 0.9, 1.0)));
            }
            let won_stage = matches!(result.as_deref(), Some(SessionResult::VsWin { winner: 0 }))
                && stage.is_some();
            let hint = if won_stage && stage.is_some_and(|s| s < MAX_STAGE) {
                "ENTER: next stage    R: replay    Q: title"
            } else {
                "R / ENTER: play again    Q: title"
            };
            parent.spawn(overlay_text(hint, 20.0, Color::srgb(0.6, 0.7, 0.8)));
        });
}

fn overlay_input(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<PlayState>>,
    settings: Res<GameSettings>,
    result: Option<Res<SessionResult>>,
    mut mode: ResMut<GameMode>,
    mut next_app: ResMut<NextState<AppState>>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    let finished = *state.get() == PlayState::Finished;
    // While paused, the pause key resumes via pause_toggle; if the player
    // has rebound Pause to R or Q, that key must not also restart/quit.
    let pause_key = settings.key_for(Action::Pause);
    let shadowed = |key: KeyCode| !finished && pause_key == key;

    // After beating a stage, ENTER advances to the next one (R replays).
    let next_stage = match (*mode, result.as_deref()) {
        (GameMode::VsCpu { stage }, Some(SessionResult::VsWin { winner: 0 }))
            if finished && stage < MAX_STAGE =>
        {
            Some(stage + 1)
        }
        _ => None,
    };

    if finished && keys.just_pressed(KeyCode::Enter) {
        if let Some(stage) = next_stage {
            *mode = GameMode::VsCpu { stage };
        }
        sfx.write(PlaySfx::new(Sfx::MenuSelect));
        next_app.set(AppState::Restarting);
    } else if keys.just_pressed(KeyCode::KeyR) && !shadowed(KeyCode::KeyR) {
        sfx.write(PlaySfx::new(Sfx::MenuSelect));
        // Bounce through Restarting: real OnExit/OnEnter(Playing) run and
        // the PlayState sub-state resets to Countdown (a Playing→Playing
        // identity transition would leave it stuck at Finished/Paused).
        next_app.set(AppState::Restarting);
    } else if keys.just_pressed(KeyCode::KeyQ) && !shadowed(KeyCode::KeyQ) {
        sfx.write(PlaySfx::new(Sfx::MenuBack));
        next_app.set(AppState::Title);
    }
}
