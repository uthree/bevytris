//! Menus: title screen, settings (with live key rebinding), pause overlay
//! and the result screen. Fully keyboard-navigable, mouse also works.

use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;

use crate::audio::{PlaySfx, Sfx};
use crate::config::{key_label, save_settings, Action, GameSettings};
use crate::i18n::{action_label, Locale, Strings};
use crate::input::{PadAction, PadInput};
use crate::core::ai::{AiProfile, MAX_STAGE};
use crate::progress::Progress;
use crate::session::{
    format_race_time, GameSession, HumanControlled, LastRound, MatchState, RaceResult,
    SessionResult, StageClear,
};
use crate::state::{AppState, GameMode, PlayState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    /// Open the solo mode picker (marathon / sprint / dig).
    SoloSelect,
    Marathon,
    /// 40-line race.
    Sprint,
    /// Garbage-digging race.
    Dig,
    /// Open the stage picker.
    VsSelect,
    /// Start a VS match on this stage.
    Stage(u32),
    /// Open the zone battle difficulty picker.
    ZoneSelect,
    /// Start a zone battle against this stage's CPU profile.
    ZoneStage(u32),
    /// Open the custom match rule sheet.
    CustomSetup,
    /// Start a match under the current rule sheet.
    CustomStart,
    CustomCpuLevel,
    CustomStyle,
    CustomFirstTo,
    CustomZone,
    CustomMargin,
    CustomSpeed,
    CustomPlayerAtk,
    CustomCpuAtk,
    CustomGarbage,
    Settings,
    Quit,
    Bind(Action),
    /// Rebind the gamepad button for an action.
    BindPad(Action),
    AdjustDas,
    AdjustArr,
    AdjustSdf,
    AdjustMaster,
    AdjustBgm,
    AdjustSfx,
    AdjustLanguage,
    ToggleVsync,
    ToggleFullscreen,
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

/// Bottom-of-screen line describing the focused menu item (title / solo
/// picker).
#[derive(Component)]
struct MenuFooter;

#[derive(Resource, Default)]
struct MenuCursor(usize);

/// Which action is waiting for a key press (settings screen).
/// `pad` selects what gets captured: a keyboard key or a gamepad button.
/// `just_started` guards the frame the rebind began, so the confirming
/// Enter press / A press / mouse click is never captured as the binding.
#[derive(Resource, Default)]
struct Rebinding {
    action: Option<Action>,
    pad: bool,
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
            .add_systems(OnEnter(AppState::SoloSelect), setup_solo_select)
            .add_systems(OnEnter(AppState::ZoneSelect), setup_zone_select)
            .add_systems(OnEnter(AppState::StageSelect), setup_stage_select)
            .add_systems(OnEnter(AppState::CustomSetup), setup_custom)
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
                    refresh_stage_footer.run_if(
                        in_state(AppState::StageSelect).or_else(in_state(AppState::ZoneSelect)),
                    ),
                    refresh_menu_footer.run_if(
                        in_state(AppState::Title).or_else(in_state(AppState::SoloSelect)),
                    ),
                )
                    .run_if(
                        in_state(AppState::Title)
                            .or_else(in_state(AppState::Settings))
                            .or_else(in_state(AppState::SoloSelect))
                            .or_else(in_state(AppState::ZoneSelect))
                            .or_else(in_state(AppState::StageSelect))
                            .or_else(in_state(AppState::CustomSetup)),
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
            )
            .add_systems(
                Update,
                (settings_scroll_wheel, settings_keep_cursor_visible).run_if(
                    in_state(AppState::Settings).or_else(in_state(AppState::CustomSetup)),
                ),
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

fn setup_title(mut commands: Commands, mut cursor: ResMut<MenuCursor>, locale: Res<Locale>) {
    let s = locale.s();
    cursor.0 = 0;
    let items = vec![
        (MenuAction::SoloSelect, s.solo.to_string()),
        (MenuAction::VsSelect, s.vs_cpu.to_string()),
        (MenuAction::ZoneSelect, s.zone_battle.to_string()),
        (MenuAction::CustomSetup, s.custom_match.to_string()),
        (MenuAction::Settings, s.settings.to_string()),
        (MenuAction::Quit, s.quit.to_string()),
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
                Text::new(s.title_hint),
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
    spawn_menu_footer(&mut commands, AppState::Title);
}

/// Solo mode picker: marathon / sprint / dig.
fn setup_solo_select(
    mut commands: Commands,
    mut cursor: ResMut<MenuCursor>,
    locale: Res<Locale>,
) {
    let s = locale.s();
    cursor.0 = 0;
    let items = vec![
        (MenuAction::Marathon, s.marathon.to_string()),
        (MenuAction::Sprint, s.sprint.to_string()),
        (MenuAction::Dig, s.dig.to_string()),
    ];
    commands
        .spawn((root_node(), DespawnOnExit(AppState::SoloSelect)))
        .with_children(|parent| {
            parent.spawn((
                Text::new(s.solo),
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
            for (i, (action, label)) in items.into_iter().enumerate() {
                parent.spawn(item_bundle(i, action, label, 300.0));
            }
            parent.spawn((
                Text::new(s.list_hint),
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
    spawn_menu_footer(&mut commands, AppState::SoloSelect);
}

/// Bottom-anchored description line for the focused item.
fn spawn_menu_footer(commands: &mut Commands, state: AppState) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(36),
            width: percent(100),
            justify_content: JustifyContent::Center,
            ..default()
        },
        DespawnOnExit(state),
        children![(
            Text::new(""),
            MenuFooter,
            TextFont {
                font_size: FontSize::Px(16.0),
                ..default()
            },
            TextColor(Color::srgb(0.8, 0.85, 0.95)),
        )],
    ));
}

/// What the footer says about a focused item; None hides the line.
fn action_description(action: MenuAction, progress: &Progress, s: &Strings) -> Option<String> {
    use crate::session::{DIG_ROWS, SPRINT_GOAL_LINES};
    let best = |ms: Option<u64>| match ms {
        Some(ms) => format!("    {} {}", s.best_label, format_race_time(ms as f64 / 1000.0)),
        None => String::new(),
    };
    Some(match action {
        MenuAction::SoloSelect => s.desc_solo.to_string(),
        MenuAction::VsSelect => s.desc_vs.to_string(),
        MenuAction::ZoneSelect => s.desc_zone.to_string(),
        MenuAction::CustomSetup => s.desc_custom.to_string(),
        MenuAction::Settings => s.desc_settings.to_string(),
        MenuAction::Quit => s.desc_quit.to_string(),
        MenuAction::Marathon => s.desc_marathon.to_string(),
        MenuAction::Sprint => format!(
            "{}{}",
            s.desc_sprint.replace("{n}", &SPRINT_GOAL_LINES.to_string()),
            best(progress.best_sprint_ms)
        ),
        MenuAction::Dig => format!(
            "{}{}",
            s.desc_dig.replace("{n}", &DIG_ROWS.to_string()),
            best(progress.best_dig_ms)
        ),
        _ => return None,
    })
}

fn refresh_menu_footer(
    cursor: Res<MenuCursor>,
    progress: Res<Progress>,
    locale: Res<Locale>,
    items: Query<&MenuItem>,
    mut texts: Query<&mut Text, With<MenuFooter>>,
) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    let line = items
        .iter()
        .find(|item| item.index == cursor.0)
        .and_then(|item| action_description(item.action, &progress, locale.s()))
        .unwrap_or_default();
    if **text != line {
        **text = line;
    }
}

const STAGE_COLUMNS: usize = 6;

fn setup_stage_select(
    mut commands: Commands,
    mut cursor: ResMut<MenuCursor>,
    progress: Res<Progress>,
    locale: Res<Locale>,
) {
    spawn_stage_grid(
        &mut commands,
        &mut cursor,
        locale.s().select_stage,
        locale.s(),
        AppState::StageSelect,
        progress.unlocked,
        &progress.grades,
        MenuAction::Stage,
    );
}

/// Zone battle mirrors the VS campaign: same 30-stage ladder, its own
/// unlock/grade track.
fn setup_zone_select(
    mut commands: Commands,
    mut cursor: ResMut<MenuCursor>,
    progress: Res<Progress>,
    locale: Res<Locale>,
) {
    spawn_stage_grid(
        &mut commands,
        &mut cursor,
        locale.s().zone_battle,
        locale.s(),
        AppState::ZoneSelect,
        progress.zone_unlocked,
        &progress.zone_grades,
        MenuAction::ZoneStage,
    );
}

/// Shared 30-stage picker grid used by both VS campaigns.
#[allow(clippy::too_many_arguments)]
fn spawn_stage_grid(
    commands: &mut Commands,
    cursor: &mut MenuCursor,
    title: &str,
    s: &'static Strings,
    state: AppState,
    unlocked_to: u32,
    grades: &std::collections::HashMap<u32, crate::progress::Grade>,
    make_action: fn(u32) -> MenuAction,
) {
    // Focus the newest unlocked stage.
    cursor.0 = (unlocked_to.clamp(1, MAX_STAGE) - 1) as usize;

    commands
        .spawn((root_node(), DespawnOnExit(state)))
        .with_children(|parent| {
            parent.spawn((
                Text::new(title),
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
                        let unlocked = stage <= unlocked_to;
                        let grade = grades.get(&stage).copied();
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
                                action: make_action(stage),
                            },
                            Node {
                                width: px(84),
                                height: px(52),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                border: UiRect::all(px(2)),
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
                Text::new(s.grid_hint),
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
    app_state: Res<State<AppState>>,
    cursor: Res<MenuCursor>,
    progress: Res<Progress>,
    locale: Res<Locale>,
    mut texts: Query<&mut Text, With<StageFooter>>,
) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    let s = locale.s();
    let zone = *app_state.get() == AppState::ZoneSelect;
    let (prefix, unlocked_to, grades) = if zone {
        (s.zone_prefix, progress.zone_unlocked, &progress.zone_grades)
    } else {
        (s.stage_prefix, progress.unlocked, &progress.grades)
    };
    let stage = (cursor.0 as u32 + 1).clamp(1, MAX_STAGE);
    let profile = AiProfile::for_stage(stage);
    let first_to = if stage % 10 == 0 { 3 } else { 2 };
    let status = if stage > unlocked_to {
        s.locked.to_string()
    } else {
        match grades.get(&stage) {
            Some(g) => format!("{} {}", s.best_colon, g.letter()),
            None => s.not_cleared.to_string(),
        }
    };
    let line = format!(
        "{prefix} {stage:02}   {}: {}   {}   {status}",
        s.type_label,
        profile.archetype.label(),
        s.first_to.replace("{n}", &first_to.to_string()),
    );
    if **text != line {
        **text = line;
    }
}

/// Marker for the scrollable list of settings rows.
#[derive(Component)]
struct SettingsScroll;

/// Non-interactive section heading inside the settings list.
fn section_heading(label: &'static str) -> impl Bundle {
    (
        Text::new(label),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::srgb(0.4, 0.75, 0.9)),
        Node {
            margin: UiRect {
                top: px(10),
                bottom: px(2),
                ..default()
            },
            ..default()
        },
    )
}

fn setup_settings(
    mut commands: Commands,
    mut cursor: ResMut<MenuCursor>,
    mut rebinding: ResMut<Rebinding>,
    locale: Res<Locale>,
) {
    let s = locale.s();
    cursor.0 = 0;
    rebinding.action = None;
    rebinding.pad = false;
    rebinding.just_started = false;
    commands
        .spawn((root_node(), DespawnOnExit(AppState::Settings)))
        .with_children(|parent| {
            parent.spawn((
                Text::new(s.settings),
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
            // Rows live in a scroll container so small windows can still
            // reach everything (wheel, or keyboard nav auto-scrolling).
            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: px(6),
                        max_height: px(470),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    ScrollPosition::default(),
                    SettingsScroll,
                ))
                .with_children(|list| {
                    let mut index = 0;
                    list.spawn(section_heading(s.sec_keys));
                    for action in Action::ALL {
                        list.spawn(item_bundle(
                            index,
                            MenuAction::Bind(action),
                            String::new(),
                            420.0,
                        ));
                        index += 1;
                    }
                    list.spawn(section_heading(s.sec_pad));
                    for action in Action::ALL {
                        list.spawn(item_bundle(
                            index,
                            MenuAction::BindPad(action),
                            String::new(),
                            420.0,
                        ));
                        index += 1;
                    }
                    list.spawn(section_heading(s.sec_handling));
                    for action in [
                        MenuAction::AdjustDas,
                        MenuAction::AdjustArr,
                        MenuAction::AdjustSdf,
                    ] {
                        list.spawn(item_bundle(index, action, String::new(), 420.0));
                        index += 1;
                    }
                    list.spawn(section_heading(s.sec_audio));
                    for action in [
                        MenuAction::AdjustMaster,
                        MenuAction::AdjustBgm,
                        MenuAction::AdjustSfx,
                    ] {
                        list.spawn(item_bundle(index, action, String::new(), 420.0));
                        index += 1;
                    }
                    list.spawn(section_heading(s.sec_display));
                    for action in [
                        MenuAction::AdjustLanguage,
                        MenuAction::ToggleVsync,
                        MenuAction::ToggleFullscreen,
                    ] {
                        list.spawn(item_bundle(index, action, String::new(), 420.0));
                        index += 1;
                    }
                    list.spawn(item_bundle(index, MenuAction::Back, String::new(), 420.0));
                });
            parent.spawn((
                Text::new(s.settings_hint),
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
            parent.spawn((
                Text::new(s.pad_hint),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.4, 0.45, 0.55)),
                Node {
                    margin: UiRect::top(px(6)),
                    ..default()
                },
            ));
        });
}

/// Custom match rule sheet: opponent and rules rows, then START.
fn setup_custom(mut commands: Commands, mut cursor: ResMut<MenuCursor>, locale: Res<Locale>) {
    let s = locale.s();
    cursor.0 = 0;
    commands
        .spawn((root_node(), DespawnOnExit(AppState::CustomSetup)))
        .with_children(|parent| {
            parent.spawn((
                Text::new(s.custom_match),
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
            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: px(6),
                        max_height: px(470),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    ScrollPosition::default(),
                    SettingsScroll,
                ))
                .with_children(|list| {
                    let mut index = 0;
                    list.spawn(section_heading(s.sec_cm_cpu));
                    for action in [MenuAction::CustomCpuLevel, MenuAction::CustomStyle] {
                        list.spawn(item_bundle(index, action, String::new(), 420.0));
                        index += 1;
                    }
                    list.spawn(section_heading(s.sec_cm_rules));
                    for action in [
                        MenuAction::CustomFirstTo,
                        MenuAction::CustomZone,
                        MenuAction::CustomMargin,
                        MenuAction::CustomSpeed,
                        MenuAction::CustomPlayerAtk,
                        MenuAction::CustomCpuAtk,
                        MenuAction::CustomGarbage,
                    ] {
                        list.spawn(item_bundle(index, action, String::new(), 420.0));
                        index += 1;
                    }
                    list.spawn(item_bundle(index, MenuAction::CustomStart, String::new(), 420.0));
                    index += 1;
                    list.spawn(item_bundle(index, MenuAction::Back, String::new(), 420.0));
                });
            parent.spawn((
                Text::new(s.cm_hint),
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

/// Mouse wheel scrolls the settings list.
fn settings_scroll_wheel(
    mut wheels: MessageReader<MouseWheel>,
    mut scrolls: Query<&mut ScrollPosition, With<SettingsScroll>>,
) {
    for ev in wheels.read() {
        let dy = match ev.unit {
            MouseScrollUnit::Line => ev.y * 36.0,
            MouseScrollUnit::Pixel => ev.y,
        };
        for mut pos in &mut scrolls {
            pos.0.y -= dy;
        }
    }
}

/// Keyboard/pad navigation drags the view along with the focused row.
fn settings_keep_cursor_visible(
    cursor: Res<MenuCursor>,
    items: Query<(&MenuItem, &ComputedNode, &UiGlobalTransform)>,
    mut scrolls: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<SettingsScroll>,
    >,
) {
    if !cursor.is_changed() {
        return;
    }
    let Ok((snode, stf, mut pos)) = scrolls.single_mut() else {
        return;
    };
    let Some((_, inode, itf)) = items.iter().find(|(item, ..)| item.index == cursor.0) else {
        return;
    };
    // Everything below is in physical pixels; ScrollPosition wants logical.
    let inv = snode.inverse_scale_factor();
    let margin = 10.0;
    let view_top = stf.affine().translation.y - snode.size().y * 0.5;
    let view_bottom = stf.affine().translation.y + snode.size().y * 0.5;
    let item_top = itf.affine().translation.y - inode.size().y * 0.5 - margin;
    let item_bottom = itf.affine().translation.y + inode.size().y * 0.5 + margin;
    if item_top < view_top {
        pos.0.y += (item_top - view_top) * inv;
    } else if item_bottom > view_bottom {
        pos.0.y += (item_bottom - view_bottom) * inv;
    }
}

fn settings_label(
    action: MenuAction,
    settings: &GameSettings,
    rebinding: &Rebinding,
    s: &Strings,
) -> Option<String> {
    use crate::i18n::LangChoice;
    Some(match action {
        MenuAction::Bind(a) => {
            let key = if rebinding.action == Some(a) && !rebinding.pad {
                s.press_key.to_string()
            } else {
                format!("[{}]", key_label(settings.key_for(a)))
            };
            format!("{:<12} {}", action_label(s, a), key)
        }
        MenuAction::BindPad(a) => {
            let button = if rebinding.action == Some(a) && rebinding.pad {
                s.press_button.to_string()
            } else {
                format!("[{}]", crate::config::pad_button_label(settings.pad_for(a)))
            };
            format!("{:<12} {}", action_label(s, a), button)
        }
        MenuAction::AdjustDas => format!("{:<12} {} ms", "DAS", settings.das_ms),
        MenuAction::AdjustArr => format!("{:<12} {} ms", "ARR", settings.arr_ms),
        MenuAction::AdjustSdf => format!("{:<12} {}", s.sdf_setting, settings.sdf_label()),
        MenuAction::AdjustMaster => {
            format!("{:<12} {}/10", s.master_vol, settings.master_volume)
        }
        MenuAction::AdjustBgm => format!("{:<12} {}/10", s.bgm_vol, settings.bgm_volume),
        MenuAction::AdjustSfx => format!("{:<12} {}/10", s.sfx_vol, settings.sfx_volume),
        MenuAction::AdjustLanguage => {
            let value = match settings.language {
                LangChoice::Auto => {
                    format!("AUTO ({})", crate::i18n::system_lang().short())
                }
                LangChoice::En => "ENGLISH".to_string(),
                LangChoice::Ja => "日本語".to_string(),
            };
            format!("{:<12} {}", s.language, value)
        }
        MenuAction::ToggleVsync => format!(
            "{:<12} {}",
            "VSync",
            if settings.vsync { "ON" } else { s.vsync_off }
        ),
        MenuAction::ToggleFullscreen => format!(
            "{:<12} {}",
            s.fullscreen,
            if settings.fullscreen { "ON" } else { "OFF" }
        ),
        MenuAction::CustomCpuLevel => {
            format!("{:<12} {:02} / 30", s.cm_cpu_level, settings.custom.cpu_level)
        }
        MenuAction::CustomStyle => {
            let value = match settings.custom.cpu_style {
                crate::config::CpuStyle::Auto => format!(
                    "AUTO ({})",
                    AiProfile::for_stage(settings.custom.cpu_level).archetype.label()
                ),
                style => style.label().to_string(),
            };
            format!("{:<12} {}", s.cm_style, value)
        }
        MenuAction::CustomFirstTo => {
            format!("{:<12} {}", s.cm_first_to, settings.custom.wins_needed)
        }
        MenuAction::CustomZone => format!(
            "{:<12} {}",
            s.cm_zone,
            if settings.custom.zone { "ON" } else { "OFF" }
        ),
        MenuAction::CustomMargin => {
            let value = if settings.custom.margin_secs == 0 {
                "OFF".to_string()
            } else {
                format!("{}s", settings.custom.margin_secs)
            };
            format!("{:<12} {}", s.cm_margin, value)
        }
        MenuAction::CustomSpeed => {
            let value = if settings.custom.speed_level == 0 {
                s.cm_speed_auto.to_string()
            } else {
                format!("LV {}", settings.custom.speed_level)
            };
            format!("{:<12} {}", s.cm_speed, value)
        }
        MenuAction::CustomPlayerAtk => {
            format!("{:<12} {}%", s.cm_player_atk, settings.custom.player_attack_pct)
        }
        MenuAction::CustomCpuAtk => {
            format!("{:<12} {}%", s.cm_cpu_atk, settings.custom.cpu_attack_pct)
        }
        MenuAction::CustomGarbage => {
            format!("{:<12} {}", s.cm_garbage, settings.custom.start_garbage)
        }
        MenuAction::CustomStart => s.cm_start.to_string(),
        MenuAction::Back => s.back.to_string(),
        _ => return None,
    })
}

fn refresh_settings_labels(
    settings: Res<GameSettings>,
    rebinding: Res<Rebinding>,
    locale: Res<Locale>,
    items: Query<(&MenuItem, &Children)>,
    mut labels: Query<&mut Text, With<MenuItemLabel>>,
) {
    for (item, children) in &items {
        let Some(label) = settings_label(item.action, &settings, &rebinding, locale.s()) else {
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
    pad: Res<PadInput>,
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
    // The stage pickers are grids: vertical steps jump a whole row and
    // left/right move the cursor instead of adjusting values.
    let grid = matches!(
        *activate.app_state.get(),
        AppState::StageSelect | AppState::ZoneSelect
    );
    let step = if grid { STAGE_COLUMNS } else { 1 };

    let down = keys.just_pressed(KeyCode::ArrowDown)
        || keys.just_pressed(KeyCode::KeyS)
        || pad.just_pressed(PadAction::Down);
    let up = keys.just_pressed(KeyCode::ArrowUp)
        || keys.just_pressed(KeyCode::KeyW)
        || pad.just_pressed(PadAction::Up);
    let right = keys.just_pressed(KeyCode::ArrowRight) || pad.just_pressed(PadAction::Right);
    let left = keys.just_pressed(KeyCode::ArrowLeft) || pad.just_pressed(PadAction::Left);

    let mut moved = false;
    if down {
        cursor.0 = (cursor.0 + step) % count;
        moved = true;
    }
    if up {
        cursor.0 = (cursor.0 + count - step) % count;
        moved = true;
    }
    if grid {
        if right {
            cursor.0 = (cursor.0 + 1) % count;
            moved = true;
        }
        if left {
            cursor.0 = (cursor.0 + count - 1) % count;
            moved = true;
        }
    }
    if moved {
        sfx.write(PlaySfx::quiet(Sfx::MenuMove));
    }

    let adjust = if grid {
        0
    } else if right {
        1
    } else if left {
        -1
    } else {
        0
    };
    let confirm = keys.just_pressed(KeyCode::Enter)
        || keys.just_pressed(KeyCode::Space)
        || pad.just_pressed(PadAction::Confirm);
    let back = keys.just_pressed(KeyCode::Escape) || pad.just_pressed(PadAction::Back);

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
            AppState::Settings
                | AppState::SoloSelect
                | AppState::ZoneSelect
                | AppState::StageSelect
                | AppState::CustomSetup
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
            MenuAction::AdjustLanguage => {
                s.language = s.language.cycled(adjust);
                true
            }
            MenuAction::ToggleVsync => {
                s.vsync = !s.vsync;
                true
            }
            MenuAction::ToggleFullscreen => {
                s.fullscreen = !s.fullscreen;
                true
            }
            MenuAction::CustomCpuLevel => {
                s.custom.cpu_level =
                    (s.custom.cpu_level as i32 + adjust).clamp(1, MAX_STAGE as i32) as u32;
                true
            }
            MenuAction::CustomStyle => {
                s.custom.cpu_style = s.custom.cpu_style.cycled(adjust);
                true
            }
            MenuAction::CustomFirstTo => {
                s.custom.wins_needed = (s.custom.wins_needed as i32 + adjust).clamp(1, 5) as u32;
                true
            }
            MenuAction::CustomZone => {
                s.custom.zone = !s.custom.zone;
                true
            }
            MenuAction::CustomMargin => {
                const STEPS: [u32; 5] = [0, 60, 90, 120, 180];
                let i = STEPS
                    .iter()
                    .position(|&m| m == s.custom.margin_secs)
                    .unwrap_or(0) as i32;
                s.custom.margin_secs =
                    STEPS[(i + adjust).rem_euclid(STEPS.len() as i32) as usize];
                true
            }
            MenuAction::CustomSpeed => {
                s.custom.speed_level = (s.custom.speed_level as i32 + adjust).clamp(0, 20) as u32;
                true
            }
            MenuAction::CustomPlayerAtk => {
                s.custom.player_attack_pct =
                    (s.custom.player_attack_pct as i32 + adjust * 25).clamp(50, 200) as u32;
                true
            }
            MenuAction::CustomCpuAtk => {
                s.custom.cpu_attack_pct =
                    (s.custom.cpu_attack_pct as i32 + adjust * 25).clamp(50, 200) as u32;
                true
            }
            MenuAction::CustomGarbage => {
                s.custom.start_garbage = (s.custom.start_garbage as i32 + adjust).clamp(0, 8) as u32;
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
        MenuAction::SoloSelect => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::SoloSelect);
        }
        MenuAction::Marathon => {
            *p.mode = GameMode::Single;
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::Playing);
        }
        MenuAction::Sprint => {
            *p.mode = GameMode::Sprint;
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::Playing);
        }
        MenuAction::Dig => {
            *p.mode = GameMode::Dig;
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
        MenuAction::ZoneSelect => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::ZoneSelect);
        }
        MenuAction::ZoneStage(stage) => {
            if p.progress.is_zone_unlocked(stage) {
                *p.mode = GameMode::ZoneBattle { stage };
                sfx.write(PlaySfx::new(Sfx::MenuSelect));
                p.next_app.set(AppState::Playing);
            } else {
                sfx.write(PlaySfx::new(Sfx::RotateFail));
            }
        }
        MenuAction::CustomSetup => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::CustomSetup);
        }
        MenuAction::CustomStart => {
            *p.mode = GameMode::Custom;
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::Playing);
        }
        MenuAction::CustomZone => {
            // Enter toggles too (same as left/right).
            p.settings.custom.zone = !p.settings.custom.zone;
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        MenuAction::CustomStyle => {
            // Enter cycles forward, same as pressing right.
            p.settings.custom.cpu_style = p.settings.custom.cpu_style.cycled(1);
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
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
            p.rebinding.pad = false;
            p.rebinding.just_started = true;
        }
        MenuAction::BindPad(action) => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.rebinding.action = Some(action);
            p.rebinding.pad = true;
            p.rebinding.just_started = true;
        }
        MenuAction::ToggleVsync => {
            // Enter toggles too (same as left/right).
            p.settings.vsync = !p.settings.vsync;
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        MenuAction::AdjustLanguage => {
            // Enter cycles forward, same as pressing right.
            p.settings.language = p.settings.language.cycled(1);
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        MenuAction::ToggleFullscreen => {
            p.settings.fullscreen = !p.settings.fullscreen;
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
    pad: Res<PadInput>,
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

    if rebinding.pad {
        // Pad rebinding: the next gamepad button becomes the binding —
        // ANY bindable button, including B, so cancelling is keyboard
        // ESC only.
        let mut escape = false;
        for input in keyboard.read() {
            if input.state == ButtonState::Pressed
                && !input.repeat
                && input.key_code == KeyCode::Escape
            {
                escape = true;
            }
        }
        if escape {
            rebinding.action = None;
            sfx.write(PlaySfx::new(Sfx::MenuBack));
            return;
        }
        if let Some(button) = pad.raw_just_pressed() {
            settings.bind_pad(action, button);
            save_settings(&settings);
            rebinding.action = None;
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
        }
        return;
    }

    // Keyboard rebinding; pad B backs out of the prompt so a pad-only
    // player is never stuck in it.
    if pad.just_pressed(PadAction::Back) {
        rebinding.action = None;
        sfx.write(PlaySfx::new(Sfx::MenuBack));
        keyboard.clear();
        return;
    }
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

fn setup_pause_overlay(
    mut commands: Commands,
    settings: Res<GameSettings>,
    locale: Res<Locale>,
) {
    let s = locale.s();
    let pause_key = key_label(settings.key_for(Action::Pause));
    commands
        .spawn((overlay_root(), DespawnOnExit(PlayState::Paused)))
        .with_children(|parent| {
            parent.spawn(overlay_text(s.paused, 64.0, Color::srgb(0.9, 0.95, 1.0)));
            parent.spawn(overlay_text(
                &s.pause_hint.replace("{key}", &pause_key),
                20.0,
                Color::srgb(0.6, 0.7, 0.8),
            ));
        });
}

/// "YOU {p} - {c} CPU    (FIRST TO {n})" in the current language.
fn vs_score_line(s: &Strings, player: u32, cpu: u32, wins_needed: u32) -> String {
    format!(
        "{}    ({})",
        s.vs_score
            .replace("{p}", &player.to_string())
            .replace("{c}", &cpu.to_string()),
        s.first_to.replace("{n}", &wins_needed.to_string())
    )
}

/// Intermission between rounds of a first-to-n match.
fn setup_round_overlay(
    mut commands: Commands,
    last: Option<Res<LastRound>>,
    match_state: Option<Res<MatchState>>,
    locale: Res<Locale>,
) {
    let s = locale.s();
    let won = matches!(last.as_deref(), Some(LastRound { winner: 0 }));
    let (headline, color) = if won {
        (s.round_win, Color::srgb(1.0, 0.9, 0.3))
    } else {
        (s.round_lost, Color::srgb(0.9, 0.35, 0.35))
    };
    let score = match_state.map(|ms| vs_score_line(s, ms.player_wins, ms.cpu_wins, ms.wins_needed));
    commands
        .spawn((overlay_root(), DespawnOnExit(PlayState::RoundOver)))
        .with_children(|parent| {
            parent.spawn(overlay_text(headline, 58.0, color));
            if let Some(score) = score {
                parent.spawn(overlay_text(&score, 26.0, Color::srgb(0.9, 0.93, 1.0)));
            }
            parent.spawn(overlay_text(s.next_round, 18.0, Color::srgb(0.6, 0.7, 0.8)));
        });
}

fn setup_result_overlay(
    mut commands: Commands,
    result: Option<Res<SessionResult>>,
    stage_clear: Option<Res<StageClear>>,
    race_result: Option<Res<RaceResult>>,
    match_state: Option<Res<MatchState>>,
    mode: Res<GameMode>,
    locale: Res<Locale>,
    players: Query<&GameSession, With<HumanControlled>>,
) {
    let t = locale.s();
    // Both VS campaigns share the stage flow; only the label differs.
    let stage_label = match *mode {
        GameMode::VsCpu { stage } => Some((t.stage_prefix, stage)),
        GameMode::ZoneBattle { stage } => Some((t.zone_prefix, stage)),
        _ => None,
    };
    let stage = stage_label.map(|(_, s)| s);
    let vs_match = matches!(
        *mode,
        GameMode::VsCpu { .. } | GameMode::ZoneBattle { .. } | GameMode::Custom
    );
    let (headline, color) = match (result.as_deref(), stage_label) {
        (Some(SessionResult::RaceDone), _) => (t.finish.to_string(), Color::srgb(1.0, 0.9, 0.3)),
        (Some(SessionResult::VsWin { winner: 0 }), Some((prefix, s))) => (
            t.stage_clear.replace("{stage}", &format!("{prefix} {s:02}")),
            Color::srgb(1.0, 0.9, 0.3),
        ),
        (Some(SessionResult::VsWin { .. }), Some((prefix, s))) => (
            t.stage_failed.replace("{stage}", &format!("{prefix} {s:02}")),
            Color::srgb(0.9, 0.3, 0.3),
        ),
        // Custom matches have no stage to clear: plain win/lose.
        (Some(SessionResult::VsWin { winner: 0 }), None) if vs_match => {
            (t.match_win.to_string(), Color::srgb(1.0, 0.9, 0.3))
        }
        (Some(SessionResult::VsWin { .. }), None) if vs_match => {
            (t.match_lose.to_string(), Color::srgb(0.9, 0.3, 0.3))
        }
        _ => (t.game_over.to_string(), Color::srgb(0.9, 0.4, 0.4)),
    };

    // VS matches report the whole-match aggregate; races report pace;
    // marathon reports the run.
    let stats = if let Some(ms) = match_state.as_ref().filter(|_| vs_match) {
        Some(format!(
            "{} {}-{}    {} {}    {} {}    {} {}    {} {}    {} {}",
            t.rounds,
            ms.player_wins,
            ms.cpu_wins,
            t.attack,
            ms.agg.attack,
            t.tetris,
            ms.agg.tetrises,
            t.tspin,
            ms.agg.tspins,
            t.combo,
            ms.agg.max_combo,
            t.pc,
            ms.agg.perfect_clears,
        ))
    } else if matches!(*mode, GameMode::Sprint | GameMode::Dig) {
        players.single().ok().map(|s| {
            let g = &s.game;
            let pps = g.stats.pieces as f64 / g.stats.time.max(0.001);
            format!(
                "{} {}    {} {}    {} {:.2}",
                t.lines, g.lines, t.pieces, g.stats.pieces, t.pps, pps
            )
        })
    } else {
        players.single().ok().map(|s| {
            let g = &s.game;
            format!(
                "{} {}    {} {}    {} {}    {} {}",
                t.score, g.score, t.level, g.level, t.lines, g.lines, t.max_combo, g.stats.max_combo
            )
        })
    };

    commands
        .spawn((overlay_root(), DespawnOnExit(PlayState::Finished)))
        .with_children(|parent| {
            parent.spawn(overlay_text(&headline, 64.0, color));
            if let Some(clear) = stage_clear.as_deref() {
                parent.spawn(overlay_text(t.rank, 20.0, Color::srgb(0.6, 0.7, 0.8)));
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
                    parent.spawn(overlay_text(t.new_best, 24.0, Color::srgb(0.5, 1.0, 0.6)));
                }
            }
            if let Some(race) = race_result.as_deref() {
                parent.spawn(overlay_text(t.time_label, 20.0, Color::srgb(0.6, 0.7, 0.8)));
                parent.spawn((
                    Text::new(format_race_time(race.time)),
                    TextFont {
                        font_size: FontSize::Px(84.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.97, 1.0)),
                    TextShadow::default(),
                ));
                if race.new_best {
                    parent.spawn(overlay_text(t.new_best, 24.0, Color::srgb(0.5, 1.0, 0.6)));
                } else {
                    parent.spawn(overlay_text(
                        &format!(
                            "{} {}",
                            t.best_label,
                            format_race_time(race.best_ms as f64 / 1000.0)
                        ),
                        20.0,
                        Color::srgb(0.6, 0.7, 0.8),
                    ));
                }
            }
            if let Some(stats) = stats {
                parent.spawn(overlay_text(&stats, 21.0, Color::srgb(0.85, 0.9, 1.0)));
            }
            let won_stage = matches!(result.as_deref(), Some(SessionResult::VsWin { winner: 0 }))
                && stage.is_some();
            let hint = if won_stage && stage.is_some_and(|s| s < MAX_STAGE) {
                t.hint_next_stage
            } else {
                t.hint_play_again
            };
            parent.spawn(overlay_text(hint, 20.0, Color::srgb(0.6, 0.7, 0.8)));
        });
}

fn overlay_input(
    keys: Res<ButtonInput<KeyCode>>,
    pad: Res<PadInput>,
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
    // Both VS campaigns advance within their own ladder.
    let next_mode = match (*mode, result.as_deref()) {
        (GameMode::VsCpu { stage }, Some(SessionResult::VsWin { winner: 0 }))
            if finished && stage < MAX_STAGE =>
        {
            Some(GameMode::VsCpu { stage: stage + 1 })
        }
        (GameMode::ZoneBattle { stage }, Some(SessionResult::VsWin { winner: 0 }))
            if finished && stage < MAX_STAGE =>
        {
            Some(GameMode::ZoneBattle { stage: stage + 1 })
        }
        _ => None,
    };

    // Pad: A advances/replays and B quits to title, but only on the result
    // screen — while paused only Start (pause_toggle) reacts, so a stray
    // button press cannot throw away a running match.
    if finished && (keys.just_pressed(KeyCode::Enter) || pad.just_pressed(PadAction::Confirm)) {
        if let Some(next) = next_mode {
            *mode = next;
        }
        sfx.write(PlaySfx::new(Sfx::MenuSelect));
        next_app.set(AppState::Restarting);
    } else if keys.just_pressed(KeyCode::KeyR) && !shadowed(KeyCode::KeyR) {
        sfx.write(PlaySfx::new(Sfx::MenuSelect));
        // Bounce through Restarting: real OnExit/OnEnter(Playing) run and
        // the PlayState sub-state resets to Countdown (a Playing→Playing
        // identity transition would leave it stuck at Finished/Paused).
        next_app.set(AppState::Restarting);
    } else if (keys.just_pressed(KeyCode::KeyQ) && !shadowed(KeyCode::KeyQ))
        || (finished && pad.just_pressed(PadAction::Back))
    {
        sfx.write(PlaySfx::new(Sfx::MenuBack));
        next_app.set(AppState::Title);
    }
}
