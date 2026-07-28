//! Menus: title screen, settings (with live key rebinding), pause overlay
//! and the result screen. Fully keyboard-navigable, mouse also works.

use bevy::input::keyboard::KeyboardInput;
use bevy::input::ButtonState;
use bevy::prelude::*;

use crate::audio::{PlaySfx, Sfx};
use crate::config::{key_label, save_settings, Action, GameSettings};
use crate::session::{GameSession, HumanControlled, SessionResult};
use crate::state::{AppState, CpuDifficulty, GameMode, PlayState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    Marathon,
    Vs(CpuDifficulty),
    Settings,
    Quit,
    Bind(Action),
    AdjustDas,
    AdjustArr,
    AdjustMaster,
    AdjustBgm,
    AdjustSfx,
    Back,
}

#[derive(Component)]
struct MenuItem {
    index: usize,
    action: MenuAction,
}

#[derive(Component)]
struct MenuItemLabel;

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
            .add_systems(OnExit(AppState::Settings), persist_settings)
            .add_systems(OnEnter(PlayState::Paused), setup_pause_overlay)
            .add_systems(OnEnter(PlayState::Finished), setup_result_overlay)
            .add_systems(
                Update,
                (
                    menu_keyboard_nav,
                    menu_mouse,
                    highlight_items,
                    refresh_settings_labels,
                )
                    .run_if(in_state(AppState::Title).or_else(in_state(AppState::Settings))),
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
                font_size: FontSize::Px(22.0),
                ..default()
            },
            TextColor(Color::srgb(0.85, 0.9, 1.0)),
        )],
    )
}

fn setup_title(mut commands: Commands, mut cursor: ResMut<MenuCursor>) {
    cursor.0 = 0;
    let mut items = vec![(MenuAction::Marathon, "MARATHON".to_string())];
    for d in [CpuDifficulty::Easy, CpuDifficulty::Normal, CpuDifficulty::Hard] {
        items.push((MenuAction::Vs(d), format!("VS CPU - {}", d.label())));
    }
    items.push((MenuAction::Settings, "SETTINGS".to_string()));
    items.push((MenuAction::Quit, "QUIT".to_string()));
    commands
        .spawn((root_node(), DespawnOnExit(AppState::Title)))
        .with_children(|parent| {
            parent.spawn((
                Text::new("BEVYTRIS"),
                TextFont {
                    font_size: FontSize::Px(84.0),
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
                    font_size: FontSize::Px(44.0),
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
                MenuAction::AdjustMaster,
                MenuAction::AdjustBgm,
                MenuAction::AdjustSfx,
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
        MenuAction::AdjustMaster => format!("{:<12} {}/10", "Master Vol", settings.master_volume),
        MenuAction::AdjustBgm => format!("{:<12} {}/10", "BGM Vol", settings.bgm_volume),
        MenuAction::AdjustSfx => format!("{:<12} {}/10", "SFX Vol", settings.sfx_volume),
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
    let mut moved = false;
    if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::KeyS) {
        cursor.0 = (cursor.0 + 1) % count;
        moved = true;
    }
    if keys.just_pressed(KeyCode::ArrowUp) || keys.just_pressed(KeyCode::KeyW) {
        cursor.0 = (cursor.0 + count - 1) % count;
        moved = true;
    }
    if moved {
        sfx.write(PlaySfx::quiet(Sfx::MenuMove));
    }

    let adjust = if keys.just_pressed(KeyCode::ArrowRight) {
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
        if *p.app_state.get() == AppState::Settings {
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
        MenuAction::Vs(difficulty) => {
            *p.mode = GameMode::VsCpu(difficulty);
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::Playing);
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

fn setup_result_overlay(
    mut commands: Commands,
    result: Option<Res<SessionResult>>,
    players: Query<&GameSession, With<HumanControlled>>,
) {
    let (headline, color) = match result.as_deref() {
        Some(SessionResult::VsWin { winner: 0 }) => ("YOU WIN!", Color::srgb(1.0, 0.9, 0.3)),
        Some(SessionResult::VsWin { .. }) => ("YOU LOSE...", Color::srgb(0.9, 0.3, 0.3)),
        _ => ("GAME OVER", Color::srgb(0.9, 0.4, 0.4)),
    };
    let stats = players.single().ok().map(|s| {
        let g = &s.game;
        format!(
            "SCORE {}    LEVEL {}    LINES {}    MAX COMBO {}",
            g.score, g.level, g.lines, g.stats.max_combo
        )
    });
    commands
        .spawn((overlay_root(), DespawnOnExit(PlayState::Finished)))
        .with_children(|parent| {
            parent.spawn(overlay_text(headline, 72.0, color));
            if let Some(stats) = stats {
                parent.spawn(overlay_text(&stats, 22.0, Color::srgb(0.85, 0.9, 1.0)));
            }
            parent.spawn(overlay_text(
                "R / ENTER: play again    Q: title",
                20.0,
                Color::srgb(0.6, 0.7, 0.8),
            ));
        });
}

fn overlay_input(
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<PlayState>>,
    settings: Res<GameSettings>,
    mut next_app: ResMut<NextState<AppState>>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    let finished = *state.get() == PlayState::Finished;
    // While paused, the pause key resumes via pause_toggle; if the player
    // has rebound Pause to R or Q, that key must not also restart/quit.
    let pause_key = settings.key_for(Action::Pause);
    let shadowed = |key: KeyCode| !finished && pause_key == key;

    let restart = (keys.just_pressed(KeyCode::KeyR) && !shadowed(KeyCode::KeyR))
        || (finished && keys.just_pressed(KeyCode::Enter));
    if restart {
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
