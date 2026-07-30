//! Menus: title screen, settings (with live key rebinding), pause overlay
//! and the result screen. Fully keyboard-navigable, mouse also works.

use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::ui::UiGlobalTransform;

use crate::audio::{PlaySfx, Sfx};
use crate::config::{Action, GameSettings, Opponent, key_label, save_settings};
use crate::core::ai::{AiProfile, MAX_STAGE};
use crate::i18n::{Locale, Strings, action_label};
use crate::input::{PadAction, PadInput};
use crate::music::{Jukebox, MusicEngine};
use crate::progress::Progress;
use crate::session::{
    GameSession, LastRound, MatchState, PrimaryPlayer, RaceResult, SessionResult, StageClear,
    format_race_time,
};
use crate::state::{AppState, GameMode, PlayState};
use bevytris_chiptune::Inst;
use bevytris_chiptune::compose::{Kit, Meter, Profile};
use rand::Rng as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    /// Open the solo mode picker (marathon / sprint / zen).
    SoloSelect,
    Marathon,
    /// 40-line race.
    Sprint,
    /// Endless, failure-free line clearing.
    Zen,
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
    /// CPU or a second local human on the right-hand board.
    CustomOpponent,
    CustomCpuLevel,
    CustomStyle,
    CustomFirstTo,
    CustomZone,
    CustomMargin,
    CustomSpeed,
    CustomPlayerAtk,
    CustomCpuAtk,
    CustomGarbage,
    /// How scattered the holes in incoming garbage are.
    CustomMessiness,
    CustomHold,
    CustomPreviews,
    /// Play as this character (index into the roster).
    Character(usize),
    /// Start the match with nobody: what the picker offers when no
    /// character packs are installed.
    CharacterNone,
    /// Open the listening room.
    Jukebox,
    Settings,
    Quit,
    Bind(Action),
    /// Rebind player 2's key for an action (local versus).
    Bind2(Action),
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

/// Footer line on the character picker describing the focused character.
#[derive(Component)]
struct CharacterFooter;

/// The character picker's heading, which changes between the two passes a
/// custom match needs.
#[derive(Component)]
struct CharacterTitle;

/// Bottom-of-screen line describing the focused menu item (title / solo
/// picker).
#[derive(Component)]
struct MenuFooter;

#[derive(Resource, Default)]
struct MenuCursor(usize);

/// Where the character picker was opened from, so escaping out of it goes
/// back to the screen the player was actually on rather than to the title.
/// `GameMode` already carries *what* is about to start; this is the only
/// other thing the detour needs to remember.
#[derive(Resource, Default)]
struct PickerReturn(AppState);

/// Which action is waiting for a key press (settings screen).
/// `pad` selects what gets captured: a keyboard key or a gamepad button.
/// `just_started` guards the frame the rebind began, so the confirming
/// Enter press / A press / mouse click is never captured as the binding.
#[derive(Resource, Default)]
struct Rebinding {
    action: Option<Action>,
    pad: bool,
    /// Capturing into player 2's key set rather than player 1's.
    player2: bool,
    just_started: bool,
}

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuCursor>()
            .init_resource::<Rebinding>()
            .init_resource::<PickerReturn>()
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
            .add_systems(OnEnter(AppState::CharacterSelect), setup_character_select)
            .init_resource::<JukeboxCursor>()
            .add_systems(OnEnter(AppState::Jukebox), setup_jukebox)
            .add_systems(
                Update,
                (jukebox_input, refresh_jukebox)
                    .chain()
                    .run_if(in_state(AppState::Jukebox)),
            )
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
                    (refresh_character_footer, refresh_character_title)
                        .run_if(in_state(AppState::CharacterSelect)),
                    refresh_menu_footer
                        .run_if(in_state(AppState::Title).or_else(in_state(AppState::SoloSelect))),
                )
                    .run_if(
                        in_state(AppState::Title)
                            .or_else(in_state(AppState::Settings))
                            .or_else(in_state(AppState::SoloSelect))
                            .or_else(in_state(AppState::ZoneSelect))
                            .or_else(in_state(AppState::StageSelect))
                            .or_else(in_state(AppState::CustomSetup))
                            .or_else(in_state(AppState::CharacterSelect)),
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
                (settings_scroll_wheel, settings_keep_cursor_visible)
                    // After the nav systems so a keypress scrolls on the same
                    // frame it moves the cursor, not the frame after.
                    .after(menu_keyboard_nav)
                    .after(menu_mouse)
                    .run_if(in_state(AppState::Settings).or_else(in_state(AppState::CustomSetup))),
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
        (MenuAction::Jukebox, s.jukebox.to_string()),
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

/// Solo mode picker: marathon / sprint / zen.
fn setup_solo_select(mut commands: Commands, mut cursor: ResMut<MenuCursor>, locale: Res<Locale>) {
    let s = locale.s();
    cursor.0 = 0;
    let items = vec![
        (MenuAction::Marathon, s.marathon.to_string()),
        (MenuAction::Sprint, s.sprint.to_string()),
        (MenuAction::Zen, s.zen.to_string()),
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
    use crate::session::SPRINT_GOAL_LINES;
    let best = |ms: Option<u64>| match ms {
        Some(ms) => format!(
            "    {} {}",
            s.best_label,
            format_race_time(ms as f64 / 1000.0)
        ),
        None => String::new(),
    };
    Some(match action {
        MenuAction::SoloSelect => s.desc_solo.to_string(),
        MenuAction::VsSelect => s.desc_vs.to_string(),
        MenuAction::ZoneSelect => s.desc_zone.to_string(),
        MenuAction::CustomSetup => s.desc_custom.to_string(),
        MenuAction::Jukebox => s.desc_jukebox.to_string(),
        MenuAction::Settings => s.desc_settings.to_string(),
        MenuAction::Quit => s.desc_quit.to_string(),
        MenuAction::Marathon => s.desc_marathon.to_string(),
        MenuAction::Sprint => format!(
            "{}{}",
            s.desc_sprint.replace("{n}", &SPRINT_GOAL_LINES.to_string()),
            best(progress.best_sprint_ms)
        ),
        // Zen has no best time to chase; what it has is a level that only
        // ever goes up, so the footer reports that instead.
        MenuAction::Zen => format!(
            "{}    {} {}  ({}/{})",
            s.desc_zen,
            s.zen_level,
            progress.zen_level(),
            progress.zen_level_progress(),
            crate::progress::ZEN_LINES_PER_LEVEL,
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

const CHARACTER_COLUMNS: usize = 4;

/// Who you are playing as. Every start passes through here; restarts do
/// not (they bounce through `Restarting`, which never visits this screen),
/// so choosing a character is a decision per *match*, not per attempt.
fn setup_character_select(
    mut commands: Commands,
    mut cursor: ResMut<MenuCursor>,
    roster: Res<crate::character::CharacterRoster>,
    issues: Res<crate::character::CharacterIssues>,
    settings: Res<GameSettings>,
    locale: Res<Locale>,
    asset_server: Res<AssetServer>,
) {
    let s = locale.s();
    // Open on whoever they played last.
    cursor.0 = roster
        .0
        .iter()
        .position(|c| c.id == settings.character)
        .unwrap_or(0);
    commands
        .spawn((root_node(), DespawnOnExit(AppState::CharacterSelect)))
        .with_children(|parent| {
            parent.spawn((
                Text::new(s.select_character),
                CharacterTitle,
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
            if roster.0.is_empty() {
                // Nothing installed is a legitimate state — the game ships
                // with no characters — so say what to do about it rather
                // than showing an empty box.
                parent.spawn((
                    Text::new(s.no_characters),
                    TextFont {
                        font_size: FontSize::Px(18.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.8, 0.7, 0.4)),
                    Node {
                        margin: UiRect::bottom(px(10)),
                        ..default()
                    },
                ));
                // Characters are an optional extra, so a roster-less
                // install must still be able to start the match — this
                // screen is a detour, never a gate.
                parent.spawn(item_bundle(
                    0,
                    MenuAction::CharacterNone,
                    s.cm_start.to_string(),
                    300.0,
                ));
            } else {
                parent
                    .spawn(Node {
                        width: px(CHARACTER_COLUMNS as f32 * 168.0),
                        flex_wrap: FlexWrap::Wrap,
                        justify_content: JustifyContent::Center,
                        row_gap: px(8),
                        column_gap: px(8),
                        ..default()
                    })
                    .with_children(|grid| {
                        for (i, character) in roster.0.iter().enumerate() {
                            // The label is baked in at spawn time, the way
                            // the stage grid does it: the settings-row
                            // pattern of filling labels in later would
                            // render a blank tile for an action that
                            // `settings_label` does not know about.
                            let label = if character.issues.is_empty() {
                                character.ascii_name.clone()
                            } else {
                                format!("{} !", character.ascii_name)
                            };
                            let text_color = if character.issues.is_empty() {
                                Color::srgb(0.9, 0.93, 1.0)
                            } else {
                                Color::srgb(1.0, 0.72, 0.35)
                            };
                            // icon.png if the pack drew one, otherwise the
                            // standing art — a name on its own tells you
                            // nothing about who you are picking.
                            let art = character
                                .icon
                                .as_deref()
                                .or_else(|| character.standing_for(0))
                                .map(|path| asset_server.load::<Image>(path.to_string()));
                            grid.spawn((
                                Button,
                                MenuItem {
                                    index: i,
                                    action: MenuAction::Character(i),
                                },
                                Node {
                                    width: px(160),
                                    height: px(132),
                                    flex_direction: FlexDirection::Column,
                                    align_items: AlignItems::Center,
                                    justify_content: JustifyContent::Center,
                                    row_gap: px(4),
                                    padding: UiRect::all(px(4)),
                                    border: UiRect::all(px(2)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.08, 0.1, 0.16, 0.85)),
                                BorderColor::all(Color::NONE),
                            ))
                            .with_children(|tile| {
                                if let Some(image) = art {
                                    // Fixed height, automatic width: the
                                    // tile keeps whatever aspect the pack
                                    // drew instead of squashing it.
                                    tile.spawn((
                                        ImageNode::new(image),
                                        Node {
                                            height: px(84),
                                            max_width: px(148),
                                            ..default()
                                        },
                                        Pickable::IGNORE,
                                    ));
                                }
                                tile.spawn((
                                    Text::new(label),
                                    MenuItemLabel,
                                    TextFont {
                                        font_size: FontSize::Px(16.0),
                                        ..default()
                                    },
                                    TextColor(text_color),
                                    Pickable::IGNORE,
                                ));
                            });
                        }
                    });
            }
            parent.spawn((
                Text::new(""),
                CharacterFooter,
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
                Text::new(if roster.0.is_empty() {
                    s.characters_hint
                } else {
                    s.grid_hint
                }),
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
            if !issues.0.is_empty() {
                parent.spawn((
                    Text::new(
                        s.character_problems
                            .replace("{n}", &issues.0.len().to_string()),
                    ),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.55, 0.2)),
                    Node {
                        margin: UiRect::top(px(10)),
                        ..default()
                    },
                ));
            }
        });
}

/// True while the picker is on its second pass, choosing for the other
/// board. Only a custom match asks — everywhere else the opponent is the
/// stage's, and picking their face would be picking their identity.
fn picking_opponent(mode: &GameMode, chosen: &crate::character::MatchCharacters) -> bool {
    *mode == GameMode::Custom && chosen.sides[0].is_some()
}

/// Relabel the heading for whichever side is being chosen.
fn refresh_character_title(
    mode: Res<GameMode>,
    chosen: Res<crate::character::MatchCharacters>,
    settings: Res<GameSettings>,
    locale: Res<Locale>,
    mut texts: Query<&mut Text, With<CharacterTitle>>,
) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    let s = locale.s();
    let line = if !picking_opponent(&mode, &chosen) {
        s.select_character
    } else if settings.custom.opponent == crate::config::Opponent::Human {
        s.select_character_p2
    } else {
        s.select_character_cpu
    };
    if **text != line {
        **text = line.to_string();
    }
}

/// Flavour text for the focused character.
fn refresh_character_footer(
    cursor: Res<MenuCursor>,
    roster: Res<crate::character::CharacterRoster>,
    mut texts: Query<&mut Text, With<CharacterFooter>>,
) {
    let Ok(mut text) = texts.single_mut() else {
        return;
    };
    let line = match roster.get(cursor.0) {
        Some(c) => {
            let mut line = c.display_name.clone();
            if !c.flavor.is_empty() {
                line.push_str(" — ");
                line.push_str(&c.flavor);
            }
            // Credit whoever made the pack, since anyone can make one.
            if !c.author.is_empty() {
                line.push_str(" (");
                line.push_str(&c.author);
                line.push(')');
            }
            line
        }
        None => String::new(),
    };
    if **text != line {
        **text = line;
    }
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
    let first_to = if stage.is_multiple_of(10) { 3 } else { 2 };
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
    rebinding.player2 = false;
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
                    list.spawn(section_heading(s.sec_keys2));
                    for action in Action::PLAYER2 {
                        list.spawn(item_bundle(
                            index,
                            MenuAction::Bind2(action),
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
                    for action in [
                        MenuAction::CustomOpponent,
                        MenuAction::CustomCpuLevel,
                        MenuAction::CustomStyle,
                    ] {
                        list.spawn(item_bundle(index, action, String::new(), 420.0));
                        index += 1;
                    }
                    list.spawn(section_heading(s.sec_cm_rules));
                    for action in [
                        MenuAction::CustomFirstTo,
                        MenuAction::CustomZone,
                        MenuAction::CustomMargin,
                        MenuAction::CustomSpeed,
                        MenuAction::CustomHold,
                        MenuAction::CustomPreviews,
                        MenuAction::CustomMessiness,
                        MenuAction::CustomPlayerAtk,
                        MenuAction::CustomCpuAtk,
                        MenuAction::CustomGarbage,
                    ] {
                        list.spawn(item_bundle(index, action, String::new(), 420.0));
                        index += 1;
                    }
                    list.spawn(item_bundle(
                        index,
                        MenuAction::CustomStart,
                        String::new(),
                        420.0,
                    ));
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

// ---------------------------------------------------------------------------
// Listening room
// ---------------------------------------------------------------------------

/// One adjustable row of the listening room. Deliberately not built on
/// [`MenuAction`]: those rows all read and write `GameSettings`, and none
/// of this belongs in a file that gets saved to disk.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
struct JukeboxRow(usize);

const JUKEBOX_ROWS: usize = 8;

/// The melody instruments the LEAD row cycles through, plus AUTO.
const JUKEBOX_LEADS: [Inst; 7] = [
    Inst::Soft,
    Inst::Pluck,
    Inst::Sustain,
    Inst::Piano,
    Inst::Guitar,
    Inst::Marimba,
    Inst::Brass,
];

#[derive(Resource, Default)]
struct JukeboxCursor(usize);

fn setup_jukebox(mut commands: Commands, mut cursor: ResMut<JukeboxCursor>, locale: Res<Locale>) {
    let s = locale.s();
    cursor.0 = 0;
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(6),
                ..default()
            },
            DespawnOnExit(AppState::Jukebox),
        ))
        .with_children(|outer| {
            // The piano roll runs right behind this, and a bright note
            // crossing a line of text makes it unreadable. A panel is
            // cheaper than fighting the contrast.
            outer
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: px(6),
                        padding: UiRect::axes(px(48), px(28)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.03, 0.06, 0.82)),
                ))
                .with_children(|parent| {
                    parent.spawn((
                        Text::new(s.jukebox),
                        TextFont {
                            font_size: FontSize::Px(40.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.3, 0.85, 1.0)),
                        TextShadow::default(),
                        Node {
                            margin: UiRect::bottom(px(14)),
                            ..default()
                        },
                    ));
                    for i in 0..JUKEBOX_ROWS {
                        parent.spawn((
                            Text::new(""),
                            TextFont {
                                font_size: FontSize::Px(20.0),
                                ..default()
                            },
                            TextColor(Color::srgb(0.75, 0.8, 0.9)),
                            TextShadow::default(),
                            JukeboxRow(i),
                        ));
                    }
                    parent.spawn((
                        Text::new(s.jukebox_now),
                        TextFont {
                            font_size: FontSize::Px(18.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.55, 0.95, 0.85)),
                        TextShadow::default(),
                        Node {
                            margin: UiRect::top(px(18)),
                            ..default()
                        },
                        JukeboxRow(JUKEBOX_ROWS),
                    ));
                    parent.spawn((
                        Text::new(s.jukebox_hint),
                        TextFont {
                            font_size: FontSize::Px(16.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.45, 0.5, 0.6)),
                        Node {
                            margin: UiRect::top(px(20)),
                            ..default()
                        },
                    ));
                });
        });
}

fn jukebox_input(
    keys: Res<ButtonInput<KeyCode>>,
    pad: Res<PadInput>,
    mut cursor: ResMut<JukeboxCursor>,
    mut jukebox: ResMut<Jukebox>,
    mut sfx: MessageWriter<PlaySfx>,
    mut next_app: ResMut<NextState<AppState>>,
) {
    if keys.just_pressed(KeyCode::Escape) || pad.just_pressed(PadAction::Back) {
        sfx.write(PlaySfx::new(Sfx::MenuBack));
        next_app.set(AppState::Title);
        return;
    }
    let down = keys.just_pressed(KeyCode::ArrowDown)
        || keys.just_pressed(KeyCode::KeyS)
        || pad.just_pressed(PadAction::Down);
    let up = keys.just_pressed(KeyCode::ArrowUp)
        || keys.just_pressed(KeyCode::KeyW)
        || pad.just_pressed(PadAction::Up);
    if down {
        cursor.0 = (cursor.0 + 1) % JUKEBOX_ROWS;
        sfx.write(PlaySfx::new(Sfx::MenuMove));
    }
    if up {
        cursor.0 = (cursor.0 + JUKEBOX_ROWS - 1) % JUKEBOX_ROWS;
        sfx.write(PlaySfx::new(Sfx::MenuMove));
    }

    // WASD alongside the arrows, matching the up/down keys above.
    let right = keys.just_pressed(KeyCode::ArrowRight)
        || keys.just_pressed(KeyCode::KeyD)
        || pad.just_pressed(PadAction::Right);
    let left = keys.just_pressed(KeyCode::ArrowLeft)
        || keys.just_pressed(KeyCode::KeyA)
        || pad.just_pressed(PadAction::Left);
    // Confirm re-rolls the seed wherever the cursor is: it is the thing
    // you want most often, and there is nothing else to confirm.
    let reroll = keys.just_pressed(KeyCode::Enter)
        || keys.just_pressed(KeyCode::KeyR)
        || pad.just_pressed(PadAction::Confirm);
    if reroll {
        jukebox.seed = rand::rng().random();
        sfx.write(PlaySfx::new(Sfx::MenuSelect));
        return;
    }
    let step = if right {
        1
    } else if left {
        -1
    } else {
        return;
    };
    sfx.write(PlaySfx::new(Sfx::MenuMove));
    match cursor.0 {
        // Walking the seed one at a time is useless; a nudge that lands
        // somewhere else entirely is the point.
        0 => jukebox.seed = jukebox.seed.wrapping_add(step as u64),
        1 => {
            let i = Profile::ALL
                .iter()
                .position(|p| *p == jukebox.profile)
                .unwrap_or(0);
            jukebox.profile =
                Profile::ALL[(i as i32 + step).rem_euclid(Profile::ALL.len() as i32) as usize];
        }
        2 => jukebox.meter = cycle_option(jukebox.meter, &Meter::ALL, step),
        3 => jukebox.kit = cycle_option(jukebox.kit, &Kit::ALL, step),
        // AUTO sits below 0%, so stepping left off the bottom hands the
        // choice back to the preset rather than clamping at "0".
        4 => {
            jukebox.smooth = match (jukebox.smooth, step) {
                (None, 1) => Some(0.0),
                (None, _) => None,
                (Some(v), _) => {
                    let next = v + 0.1 * step as f32;
                    if next < -0.01 {
                        None
                    } else {
                        Some(next.min(1.0))
                    }
                }
            }
        }
        5 => jukebox.lead = cycle_option(jukebox.lead, &JUKEBOX_LEADS, step),
        6 => jukebox.intensity = (jukebox.intensity + 0.1 * step as f32).clamp(0.0, 1.0),
        _ => jukebox.zone = !jukebox.zone,
    }
}

/// Cycle through `AUTO` and then every value, so the roll can always be
/// handed back to the composer.
fn cycle_option<T: Copy + PartialEq>(current: Option<T>, all: &[T], step: i32) -> Option<T> {
    let len = all.len() as i32 + 1;
    let i = match current {
        None => 0,
        Some(v) => all.iter().position(|x| *x == v).unwrap_or(0) as i32 + 1,
    };
    let next = (i + step).rem_euclid(len);
    if next == 0 {
        None
    } else {
        Some(all[(next - 1) as usize])
    }
}

fn refresh_jukebox(
    locale: Res<Locale>,
    cursor: Res<JukeboxCursor>,
    jukebox: Res<Jukebox>,
    engine: Res<MusicEngine>,
    mut rows: Query<(&JukeboxRow, &mut Text, &mut TextColor)>,
) {
    let s = locale.s();
    let auto = s.jukebox_auto;
    for (row, mut text, mut color) in &mut rows {
        if row.0 == JUKEBOX_ROWS {
            // The engine's own report, so what is on screen is what is
            // actually sounding rather than what was asked for.
            text.0 = match engine.now_playing() {
                Some(info) => format!("♪ {}  [{}]", info.label(), info.layer_list()),
                None => String::new(),
            };
            continue;
        }
        let (label, value) = match row.0 {
            0 => (s.jb_seed, format!("0x{:x}", jukebox.seed)),
            1 => (s.jb_preset, jukebox.profile.name().to_string()),
            2 => (
                s.jb_meter,
                jukebox
                    .meter
                    .map_or(auto.to_string(), |m| m.name().to_string()),
            ),
            3 => (
                s.jb_kit,
                jukebox
                    .kit
                    .map_or(auto.to_string(), |k| k.name().to_string()),
            ),
            4 => (
                s.jb_smooth,
                jukebox
                    .smooth
                    .map_or(auto.to_string(), |v| format!("{:.0}%", v * 100.0)),
            ),
            5 => (
                s.jb_lead,
                jukebox.lead.map_or(auto.to_string(), |i| {
                    bevytris_chiptune::compose::lead_name(i).to_uppercase()
                }),
            ),
            6 => (s.jb_intensity, format!("{:.0}%", jukebox.intensity * 100.0)),
            _ => (
                s.jb_zone,
                if jukebox.zone { "ON" } else { "OFF" }.to_string(),
            ),
        };
        let selected = cursor.0 == row.0;
        text.0 = format!(
            "{} {:<11} {}",
            if selected { ">" } else { " " },
            label,
            value
        );
        color.0 = if selected {
            Color::srgb(0.95, 0.98, 1.0)
        } else {
            Color::srgb(0.6, 0.65, 0.75)
        };
    }
}

#[cfg(test)]
mod jukebox_tests {
    use super::*;

    #[test]
    fn cycling_an_option_visits_auto_and_every_value() {
        // Right from AUTO walks the values in order and comes back.
        let mut m = None;
        let mut seen = Vec::new();
        for _ in 0..Meter::ALL.len() + 1 {
            m = cycle_option(m, &Meter::ALL, 1);
            seen.push(m);
        }
        assert_eq!(seen.len(), 4);
        assert_eq!(seen[0], Some(Meter::Four));
        assert_eq!(seen[1], Some(Meter::Three));
        assert_eq!(seen[2], Some(Meter::Six));
        assert_eq!(seen[3], None, "the cycle must return to AUTO");

        // Left is the exact inverse.
        assert_eq!(cycle_option(None, &Meter::ALL, -1), Some(Meter::Six));
        assert_eq!(cycle_option(Some(Meter::Four), &Meter::ALL, -1), None);

        // Two values behave the same way.
        assert_eq!(cycle_option(None, &Kit::ALL, 1), Some(Kit::Chip));
        assert_eq!(cycle_option(Some(Kit::Sampled), &Kit::ALL, 1), None);
    }

    #[test]
    fn every_jukebox_row_is_reachable_and_labelled() {
        // The cursor wraps over exactly the rows the screen spawns, and
        // the extra row index is the now-playing line rather than a
        // seventh setting.
        assert_eq!(JUKEBOX_ROWS, 8);
        let s = crate::i18n::EN;
        for label in [
            s.jb_seed,
            s.jb_preset,
            s.jb_meter,
            s.jb_kit,
            s.jb_smooth,
            s.jb_lead,
            s.jb_intensity,
            s.jb_zone,
        ] {
            assert!(!label.is_empty());
        }
    }

    /// The LEAD row offers every melody instrument the engine has, so a
    /// new one cannot be added and then be unreachable for auditioning.
    #[test]
    fn the_lead_row_offers_every_melody_instrument() {
        use bevytris_chiptune::ALL_INSTRUMENTS;
        for inst in ALL_INSTRUMENTS {
            if inst.is_melody() {
                assert!(
                    JUKEBOX_LEADS.contains(&inst),
                    "{inst:?} cannot be picked in the listening room"
                );
            }
        }
        for inst in JUKEBOX_LEADS {
            assert!(inst.is_melody(), "{inst:?} cannot carry a tune");
        }
    }

    /// The MELODY row steps 0..100% and drops off the bottom into AUTO,
    /// which is the only way back to "let the preset decide".
    #[test]
    fn the_melody_row_walks_auto_through_full() {
        let step = |v: Option<f32>, step: i32| -> Option<f32> {
            match (v, step) {
                (None, 1) => Some(0.0),
                (None, _) => None,
                (Some(v), _) => {
                    let next = v + 0.1 * step as f32;
                    if next < -0.01 {
                        None
                    } else {
                        Some(next.min(1.0))
                    }
                }
            }
        };
        assert_eq!(step(None, 1), Some(0.0));
        assert_eq!(step(Some(0.0), -1), None, "left off 0% returns to AUTO");
        assert_eq!(step(None, -1), None, "AUTO is the bottom of the range");
        // Climbs to 100% and stops there.
        let mut v = Some(0.0);
        for _ in 0..20 {
            v = step(v, 1);
        }
        assert_eq!(v, Some(1.0));
    }
}

/// How far down the list can be scrolled, in the logical pixels
/// [`ScrollPosition`] is measured in. Layout clamps what it *draws* to this
/// range but leaves the component alone, so anything writing a scroll
/// position has to clamp itself or the two silently drift apart.
fn max_scroll_y(node: &ComputedNode) -> f32 {
    let overflow = node.content_size.y - node.size().y + node.scrollbar_size.y;
    (overflow * node.inverse_scale_factor()).max(0.0)
}

/// Mouse wheel scrolls the settings list.
fn settings_scroll_wheel(
    mut wheels: MessageReader<MouseWheel>,
    mut scrolls: Query<(&ComputedNode, &mut ScrollPosition), With<SettingsScroll>>,
) {
    for ev in wheels.read() {
        let dy = match ev.unit {
            MouseScrollUnit::Line => ev.y * 36.0,
            MouseScrollUnit::Pixel => ev.y,
        };
        for (node, mut pos) in &mut scrolls {
            pos.0.y = (pos.0.y - dy).clamp(0.0, max_scroll_y(node));
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
    let delta = if item_top < view_top {
        item_top - view_top
    } else if item_bottom > view_bottom {
        item_bottom - view_bottom
    } else {
        return;
    };
    // The transforms above are where the rows were actually drawn, i.e. they
    // already carry the *clamped* scroll offset. Start from that so the follow
    // stays exact instead of building on a position layout never honoured.
    let drawn = snode.scroll_position.y * inv;
    pos.0.y = (drawn + delta * inv).clamp(0.0, max_scroll_y(snode));
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
            let key = if rebinding.action == Some(a) && !rebinding.pad && !rebinding.player2 {
                s.press_key.to_string()
            } else {
                format!("[{}]", key_label(settings.key_for(a)))
            };
            format!("{:<12} {}", action_label(s, a), key)
        }
        MenuAction::Bind2(a) => {
            let key = if rebinding.action == Some(a) && !rebinding.pad && rebinding.player2 {
                s.press_key.to_string()
            } else {
                // A key both players share would move both boards at
                // once, so say so rather than letting it be discovered
                // mid-match.
                let bound = settings.key2_for(a);
                let shared = Action::ALL.iter().any(|p1| settings.key_for(*p1) == bound);
                let mark = if shared { s.key_clash } else { "" };
                format!("[{}]{}", key_label(bound), mark)
            };
            format!("{:<12} {}", action_label(s, a), key)
        }
        MenuAction::BindPad(a) => {
            let button = if rebinding.action == Some(a) && rebinding.pad && !rebinding.player2 {
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
        MenuAction::CustomOpponent => {
            let value = match settings.custom.opponent {
                Opponent::Cpu => s.cm_opp_cpu,
                Opponent::Human => s.cm_opp_2p,
            };
            format!("{:<12} {}", s.cm_opponent, value)
        }
        // The CPU rows stay listed but read as N/A once a human is in the
        // other seat, so it is obvious why they stopped mattering.
        MenuAction::CustomCpuLevel if settings.custom.opponent == Opponent::Human => {
            format!("{:<12} {}", s.cm_cpu_level, s.cm_na)
        }
        MenuAction::CustomStyle if settings.custom.opponent == Opponent::Human => {
            format!("{:<12} {}", s.cm_style, s.cm_na)
        }
        MenuAction::CustomCpuLevel => {
            format!(
                "{:<12} {:02} / 30",
                s.cm_cpu_level, settings.custom.cpu_level
            )
        }
        MenuAction::CustomStyle => {
            let value = match settings.custom.cpu_style {
                crate::config::CpuStyle::Auto => format!(
                    "AUTO ({})",
                    AiProfile::for_stage(settings.custom.cpu_level)
                        .archetype
                        .label()
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
            let label = if settings.custom.opponent == Opponent::Human {
                s.cm_p1_atk
            } else {
                s.cm_player_atk
            };
            format!("{:<12} {}%", label, settings.custom.player_attack_pct)
        }
        MenuAction::CustomCpuAtk => {
            let label = if settings.custom.opponent == Opponent::Human {
                s.cm_p2_atk
            } else {
                s.cm_cpu_atk
            };
            format!("{:<12} {}%", label, settings.custom.cpu_attack_pct)
        }
        MenuAction::CustomHold => format!(
            "{:<12} {}",
            s.cm_hold,
            if settings.custom.hold { "ON" } else { "OFF" }
        ),
        MenuAction::CustomPreviews => {
            format!("{:<12} {}", s.cm_previews, settings.custom.previews)
        }
        MenuAction::CustomMessiness => {
            let value = if settings.custom.messiness == 0 {
                s.cm_mess_clean.to_string()
            } else {
                format!("{}%", settings.custom.messiness)
            };
            format!("{:<12} {}", s.cm_messiness, value)
        }
        MenuAction::CustomGarbage => {
            format!("{:<12} {}", s.cm_garbage, settings.custom.start_garbage)
        }
        MenuAction::CustomStart => s.cm_start.to_string(),
        MenuAction::Back => s.back.to_string(),
        MenuAction::CharacterNone => s.cm_start.to_string(),
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
            if let Ok(mut text) = labels.get_mut(child)
                && **text != label
            {
                **text = label.clone();
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
    // The pickers are grids: vertical steps jump a whole row and left/right
    // move the cursor instead of adjusting values. Deriving `grid` from the
    // step is what keeps left/right from being read as a value adjustment on
    // any screen that gains a grid later.
    let step = match *activate.app_state.get() {
        AppState::StageSelect | AppState::ZoneSelect => STAGE_COLUMNS,
        AppState::CharacterSelect => CHARACTER_COLUMNS,
        _ => 1,
    };
    let grid = step > 1;

    let down = keys.just_pressed(KeyCode::ArrowDown)
        || keys.just_pressed(KeyCode::KeyS)
        || pad.just_pressed(PadAction::Down);
    let up = keys.just_pressed(KeyCode::ArrowUp)
        || keys.just_pressed(KeyCode::KeyW)
        || pad.just_pressed(PadAction::Up);
    let right = keys.just_pressed(KeyCode::ArrowRight) || pad.just_pressed(PadAction::Right);
    let left = keys.just_pressed(KeyCode::ArrowLeft) || pad.just_pressed(PadAction::Left);

    // A grid whose whole roster fits on one row has no rows to jump
    // between, so vertical movement walks one tile at a time instead.
    // Without this, `count - step` underflows — 30 stages over 6 columns
    // never could, but a picker with two characters over four columns
    // does, and in a debug build that is a panic rather than a wrap.
    let vstep = if count <= step { 1 } else { step };
    let mut moved = false;
    if down {
        cursor.0 = (cursor.0 + vstep) % count;
        moved = true;
    }
    if up {
        cursor.0 = (cursor.0 as i64 - vstep as i64).rem_euclid(count as i64) as usize;
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
    run_menu_action(selected, confirm, adjust, back, activate, sfx, &mut cursor);
}

fn menu_mouse(
    items: Query<(Entity, &MenuItem, &Interaction), Changed<Interaction>>,
    mut moves: MessageReader<CursorMoved>,
    mut cursor: ResMut<MenuCursor>,
    sfx: MessageWriter<PlaySfx>,
    activate: MenuActivateParams,
) {
    // Hover is only allowed to move the cursor when the pointer itself moved.
    // A scrolling list slides rows *under* a resting pointer, which counts as
    // a fresh hover and would otherwise yank the keyboard cursor back to
    // whatever row happened to arrive beneath the mouse.
    let pointer_moved = moves.read().count() > 0;
    if activate.rebinding.action.is_some() {
        return;
    }
    let mut clicked: Option<MenuAction> = None;
    for (_, item, interaction) in &items {
        match interaction {
            Interaction::Hovered => {
                if pointer_moved {
                    cursor.0 = item.index;
                }
            }
            Interaction::Pressed => {
                cursor.0 = item.index;
                clicked = Some(item.action);
            }
            Interaction::None => {}
        }
    }
    if clicked.is_some() {
        run_menu_action(clicked, true, 0, false, activate, sfx, &mut cursor);
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
    picker_return: ResMut<'w, PickerReturn>,
    roster: Res<'w, crate::character::CharacterRoster>,
    characters: ResMut<'w, crate::character::MatchCharacters>,
}

/// Head for the character picker, remembering where to come back to.
///
/// The pending pick is cleared here rather than in the picker's `OnEnter`
/// so that a match started without ever visiting this screen (a restart, or
/// `BEVYTRIS_SCREEN=playing`) keeps whatever was already chosen.
fn open_picker(p: &mut MenuActivateParams, from: AppState) {
    p.picker_return.0 = from;
    p.characters.sides = [None, None];
    p.next_app.set(AppState::CharacterSelect);
}

/// `cursor` is passed in rather than living in [`MenuActivateParams`]:
/// both callers already hold it, and a system that asked for it twice
/// would fail to initialize.
fn run_menu_action(
    action: Option<MenuAction>,
    confirm: bool,
    adjust: i32,
    back: bool,
    mut p: MenuActivateParams,
    mut sfx: MessageWriter<PlaySfx>,
    cursor: &mut MenuCursor,
) {
    if back {
        // The picker is a detour on the way into a match, so escaping it
        // returns to whichever screen sent us here.
        if *p.app_state.get() == AppState::CharacterSelect {
            sfx.write(PlaySfx::new(Sfx::MenuBack));
            // On the second pass, step back to the first rather than out
            // of the screen: the player is mid-decision, not leaving.
            if picking_opponent(&p.mode, &p.characters) {
                let back_to = p.characters.sides[0].unwrap_or(0);
                p.characters.sides = [None, None];
                cursor.0 = back_to;
                return;
            }
            p.next_app.set(p.picker_return.0);
            return;
        }
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
                s.custom.margin_secs = STEPS[(i + adjust).rem_euclid(STEPS.len() as i32) as usize];
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
                s.custom.start_garbage =
                    (s.custom.start_garbage as i32 + adjust).clamp(0, 8) as u32;
                true
            }
            MenuAction::CustomOpponent => {
                s.custom.opponent = s.custom.opponent.toggled();
                true
            }
            MenuAction::CustomHold => {
                s.custom.hold = !s.custom.hold;
                true
            }
            MenuAction::CustomPreviews => {
                s.custom.previews = (s.custom.previews as i32 + adjust).clamp(0, 5) as u32;
                true
            }
            MenuAction::CustomMessiness => {
                s.custom.messiness = (s.custom.messiness as i32 + adjust * 25).clamp(0, 100) as u32;
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
            open_picker(&mut p, AppState::SoloSelect);
        }
        MenuAction::Sprint => {
            *p.mode = GameMode::Sprint;
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            open_picker(&mut p, AppState::SoloSelect);
        }
        MenuAction::Zen => {
            *p.mode = GameMode::Zen;
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            open_picker(&mut p, AppState::SoloSelect);
        }
        MenuAction::VsSelect => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::StageSelect);
        }
        MenuAction::Stage(stage) => {
            if p.progress.is_unlocked(stage) {
                *p.mode = GameMode::VsCpu { stage };
                sfx.write(PlaySfx::new(Sfx::MenuSelect));
                open_picker(&mut p, AppState::StageSelect);
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
                open_picker(&mut p, AppState::ZoneSelect);
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
            open_picker(&mut p, AppState::CustomSetup);
        }
        MenuAction::CustomZone => {
            // Enter toggles too (same as left/right).
            p.settings.custom.zone = !p.settings.custom.zone;
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        MenuAction::CustomOpponent => {
            p.settings.custom.opponent = p.settings.custom.opponent.toggled();
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        MenuAction::CustomHold => {
            p.settings.custom.hold = !p.settings.custom.hold;
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        MenuAction::CustomStyle => {
            // Enter cycles forward, same as pressing right.
            p.settings.custom.cpu_style = p.settings.custom.cpu_style.cycled(1);
            save_settings(&p.settings);
            sfx.write(PlaySfx::quiet(Sfx::MenuMove));
        }
        MenuAction::CharacterNone => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.characters.sides = [None, None];
            p.next_app.set(AppState::Playing);
        }
        MenuAction::Character(index) => {
            if p.roster.get(index).is_none() {
                sfx.write(PlaySfx::new(Sfx::RotateFail));
                return;
            }
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            if picking_opponent(&p.mode, &p.characters) {
                p.characters.sides[1] = Some(index);
                p.next_app.set(AppState::Playing);
                return;
            }
            // Only the player's own pick is remembered across sessions;
            // who they set up as an opponent is per-match.
            p.characters.sides = [Some(index), None];
            if let Some(character) = p.roster.get(index) {
                p.settings.character = character.id.clone();
                save_settings(&p.settings);
            }
            if *p.mode == GameMode::Custom {
                // Stay here for the other side. The cursor starts on the
                // player's pick, which is the likeliest neighbour of what
                // they want next.
                cursor.0 = index;
                return;
            }
            // Everywhere else the opponent is left unset and
            // `resolve_match_characters` fills it in deterministically on
            // the way into the match, so a restart keeps the same pairing.
            p.next_app.set(AppState::Playing);
        }
        MenuAction::Jukebox => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.next_app.set(AppState::Jukebox);
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
            p.rebinding.player2 = false;
            p.rebinding.just_started = true;
        }
        MenuAction::Bind2(action) => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.rebinding.action = Some(action);
            p.rebinding.pad = false;
            p.rebinding.player2 = true;
            p.rebinding.just_started = true;
        }
        MenuAction::BindPad(action) => {
            sfx.write(PlaySfx::new(Sfx::MenuSelect));
            p.rebinding.action = Some(action);
            p.rebinding.pad = true;
            p.rebinding.player2 = false;
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
            if rebinding.player2 {
                settings.bind2(action, input.key_code);
            } else {
                settings.bind(action, input.key_code);
            }
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

fn setup_pause_overlay(mut commands: Commands, settings: Res<GameSettings>, locale: Res<Locale>) {
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
fn vs_score_line(s: &Strings, two_player: bool, player: u32, cpu: u32, wins_needed: u32) -> String {
    let template = if two_player {
        s.vs_score_2p
    } else {
        s.vs_score
    };
    format!(
        "{}    ({})",
        template
            .replace("{p}", &player.to_string())
            .replace("{c}", &cpu.to_string()),
        s.first_to.replace("{n}", &wins_needed.to_string())
    )
}

/// True when the right-hand board is a second person rather than the CPU.
fn is_two_player(mode: &GameMode, settings: &GameSettings) -> bool {
    *mode == GameMode::Custom && settings.custom.opponent == Opponent::Human
}

/// Intermission between rounds of a first-to-n match.
fn setup_round_overlay(
    mut commands: Commands,
    last: Option<Res<LastRound>>,
    match_state: Option<Res<MatchState>>,
    mode: Res<GameMode>,
    settings: Res<GameSettings>,
    locale: Res<Locale>,
) {
    let s = locale.s();
    let two_player = is_two_player(&mode, &settings);
    let won = matches!(last.as_deref(), Some(LastRound { winner: 0 }));
    // With two people watching, "ROUND LOST" is meaningless — say who won.
    let (headline, color) = if two_player {
        (
            s.round_win_by.replace("{p}", if won { s.p1 } else { s.p2 }),
            Color::srgb(1.0, 0.9, 0.3),
        )
    } else if won {
        (s.round_win.to_string(), Color::srgb(1.0, 0.9, 0.3))
    } else {
        (s.round_lost.to_string(), Color::srgb(0.9, 0.35, 0.35))
    };
    let score = match_state
        .map(|ms| vs_score_line(s, two_player, ms.player_wins, ms.cpu_wins, ms.wins_needed));
    commands
        .spawn((overlay_root(), DespawnOnExit(PlayState::RoundOver)))
        .with_children(|parent| {
            parent.spawn(overlay_text(&headline, 58.0, color));
            if let Some(score) = score {
                parent.spawn(overlay_text(&score, 26.0, Color::srgb(0.9, 0.93, 1.0)));
            }
            parent.spawn(overlay_text(s.next_round, 18.0, Color::srgb(0.6, 0.7, 0.8)));
        });
}

#[allow(clippy::too_many_arguments)]
fn setup_result_overlay(
    mut commands: Commands,
    result: Option<Res<SessionResult>>,
    stage_clear: Option<Res<StageClear>>,
    race_result: Option<Res<RaceResult>>,
    match_state: Option<Res<MatchState>>,
    mode: Res<GameMode>,
    settings: Res<GameSettings>,
    locale: Res<Locale>,
    players: Query<&GameSession, With<PrimaryPlayer>>,
) {
    let t = locale.s();
    let two_player = is_two_player(&mode, &settings);
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
            t.stage_clear
                .replace("{stage}", &format!("{prefix} {s:02}")),
            Color::srgb(1.0, 0.9, 0.3),
        ),
        (Some(SessionResult::VsWin { .. }), Some((prefix, s))) => (
            t.stage_failed
                .replace("{stage}", &format!("{prefix} {s:02}")),
            Color::srgb(0.9, 0.3, 0.3),
        ),
        // Local versus: name the winner instead of addressing one of the
        // two people at the keyboard as "you".
        (Some(SessionResult::VsWin { winner }), None) if two_player => (
            t.match_win_by
                .replace("{p}", if *winner == 0 { t.p1 } else { t.p2 }),
            Color::srgb(1.0, 0.9, 0.3),
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
    } else if *mode == GameMode::Sprint {
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
                t.score,
                g.score,
                t.level,
                g.level,
                t.lines,
                g.lines,
                t.max_combo,
                g.stats.max_combo
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

#[allow(clippy::too_many_arguments)]
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

#[cfg(test)]
mod picker_tests {
    use super::*;
    use crate::character::{CharacterRoster, MatchCharacters};
    use bevy::ecs::system::RunSystemOnce;
    use bevy::state::app::StatesPlugin;

    /// A world with everything `run_menu_action` reads, sitting on `screen`.
    fn world_on(screen: AppState) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin));
        app.insert_state(screen);
        app.add_message::<PlaySfx>();
        app.add_message::<AppExit>();
        app.init_resource::<GameMode>()
            .init_resource::<Rebinding>()
            .init_resource::<PickerReturn>()
            .init_resource::<MenuCursor>()
            .init_resource::<MatchCharacters>()
            .init_resource::<Progress>()
            .insert_resource(GameSettings::default())
            .insert_resource(CharacterRoster(Vec::new()));
        app
    }

    /// Fire one menu action through the real `run_menu_action`.
    fn confirm(app: &mut App, action: MenuAction) {
        app.world_mut()
            .run_system_once(
                move |p: MenuActivateParams,
                      sfx: MessageWriter<PlaySfx>,
                      mut cursor: ResMut<MenuCursor>| {
                    run_menu_action(Some(action), true, 0, false, p, sfx, &mut cursor);
                },
            )
            .expect("the action system ran");
    }

    fn escape(app: &mut App) {
        app.world_mut()
            .run_system_once(
                |p: MenuActivateParams,
                 sfx: MessageWriter<PlaySfx>,
                 mut cursor: ResMut<MenuCursor>| {
                    run_menu_action(None, false, 0, true, p, sfx, &mut cursor);
                },
            )
            .expect("the back system ran");
    }

    fn pending(app: &App) -> Option<AppState> {
        match app.world().resource::<NextState<AppState>>() {
            NextState::Pending(state) => Some(*state),
            _ => None,
        }
    }

    #[test]
    fn the_menu_systems_can_all_initialize() {
        // A system that asks for the same resource twice fails when it is
        // *initialized*, which no amount of testing the functions
        // themselves will catch: `run_menu_action` picking up a
        // `MenuCursor` through `MenuActivateParams` while
        // `menu_keyboard_nav` already held one took the game down on
        // launch, and every other test in this file still passed.
        //
        // Initializing a system needs no resources to exist — only the
        // types — so this is the whole check and none of the setup.
        fn init<M, S: IntoSystem<(), (), M>>(system: S) {
            let mut world = World::new();
            IntoSystem::into_system(system).initialize(&mut world);
        }
        init(menu_keyboard_nav);
        init(menu_mouse);
        init(overlay_input);
        init(setup_character_select);
        init(refresh_character_footer);
        init(refresh_character_title);
        init(highlight_items);
        init(refresh_settings_labels);
    }

    #[test]
    fn every_mode_asks_who_you_are_first() {
        // The whole point of the screen: no mode may reach the match
        // without passing through it.
        for (screen, action, mode) in [
            (AppState::SoloSelect, MenuAction::Marathon, GameMode::Single),
            (AppState::SoloSelect, MenuAction::Sprint, GameMode::Sprint),
            (AppState::SoloSelect, MenuAction::Zen, GameMode::Zen),
            (
                AppState::StageSelect,
                MenuAction::Stage(1),
                GameMode::VsCpu { stage: 1 },
            ),
            (
                AppState::ZoneSelect,
                MenuAction::ZoneStage(1),
                GameMode::ZoneBattle { stage: 1 },
            ),
            (
                AppState::CustomSetup,
                MenuAction::CustomStart,
                GameMode::Custom,
            ),
        ] {
            let mut app = world_on(screen);
            confirm(&mut app, action);
            assert_eq!(
                pending(&app),
                Some(AppState::CharacterSelect),
                "{action:?} should detour through the picker"
            );
            // And the mode it is about to start is already decided.
            assert_eq!(*app.world().resource::<GameMode>(), mode);
            // ESC from the picker goes back where it came from.
            let mut app = escape_from_picker(app);
            assert_eq!(pending(&app), Some(screen), "ESC should return to {screen:?}");
            app.update();
        }
    }

    /// Move the app into the picker and press ESC there.
    fn escape_from_picker(mut app: App) -> App {
        let next = pending(&app).expect("a transition was queued");
        app.insert_state(next);
        escape(&mut app);
        app
    }

    #[test]
    fn a_locked_stage_never_reaches_the_picker() {
        // The unlock gate has to stay in front of the detour, or the
        // picker becomes a way around it.
        let mut app = world_on(AppState::StageSelect);
        confirm(&mut app, MenuAction::Stage(MAX_STAGE));
        assert_eq!(pending(&app), None, "a locked stage started anyway");
    }

    #[test]
    fn an_empty_roster_can_still_start_the_match() {
        // Characters are an extra; a build with none installed must not be
        // unable to play.
        let mut app = world_on(AppState::CharacterSelect);
        confirm(&mut app, MenuAction::CharacterNone);
        assert_eq!(pending(&app), Some(AppState::Playing));
        assert_eq!(
            app.world().resource::<MatchCharacters>().sides,
            [None, None]
        );
    }

    #[test]
    fn picking_a_character_starts_the_match_and_is_remembered() {
        let mut app = world_on(AppState::CharacterSelect);
        app.insert_resource(CharacterRoster(crate::character::test_roster(&[
            "alice", "bob",
        ])));
        confirm(&mut app, MenuAction::Character(1));
        assert_eq!(pending(&app), Some(AppState::Playing));
        assert_eq!(
            app.world().resource::<MatchCharacters>().sides[0],
            Some(1),
            "the pick has to survive into the match"
        );
        // The opponent is left for the match to resolve deterministically.
        assert_eq!(app.world().resource::<MatchCharacters>().sides[1], None);
        assert_eq!(app.world().resource::<GameSettings>().character, "bob");
    }

    #[test]
    fn a_custom_match_picks_both_sides() {
        let mut app = world_on(AppState::CharacterSelect);
        app.insert_resource(CharacterRoster(crate::character::test_roster(&[
            "alice", "bob",
        ])));
        *app.world_mut().resource_mut::<GameMode>() = GameMode::Custom;

        // First pass: the player's own side, and the screen stays put.
        confirm(&mut app, MenuAction::Character(0));
        assert_eq!(pending(&app), None, "it should ask for the other side");
        assert_eq!(
            app.world().resource::<MatchCharacters>().sides,
            [Some(0), None]
        );

        // ESC steps back one pass rather than leaving the screen.
        escape(&mut app);
        assert_eq!(pending(&app), None);
        assert_eq!(
            app.world().resource::<MatchCharacters>().sides,
            [None, None]
        );

        // Both passes through: now the match starts.
        confirm(&mut app, MenuAction::Character(0));
        confirm(&mut app, MenuAction::Character(1));
        assert_eq!(pending(&app), Some(AppState::Playing));
        assert_eq!(
            app.world().resource::<MatchCharacters>().sides,
            [Some(0), Some(1)]
        );
        // Only the player's own pick is remembered for next time.
        assert_eq!(app.world().resource::<GameSettings>().character, "alice");
    }

    #[test]
    fn only_a_custom_match_asks_twice() {
        // Everywhere else the opponent belongs to the stage.
        let mut app = world_on(AppState::CharacterSelect);
        app.insert_resource(CharacterRoster(crate::character::test_roster(&["alice"])));
        *app.world_mut().resource_mut::<GameMode>() = GameMode::VsCpu { stage: 3 };
        confirm(&mut app, MenuAction::Character(0));
        assert_eq!(pending(&app), Some(AppState::Playing));
        assert_eq!(
            app.world().resource::<MatchCharacters>().sides,
            [Some(0), None],
            "the opponent is resolved on the way into the match"
        );
    }

    #[test]
    fn a_short_roster_can_still_be_navigated() {
        // The grid step is a whole row wide, so a roster shorter than one
        // row used to underflow the wrap — a panic in a debug build, on the
        // very first Up press of a fresh install.
        for count in 1..=(CHARACTER_COLUMNS * 2 + 1) {
            let vstep = if count <= CHARACTER_COLUMNS {
                1
            } else {
                CHARACTER_COLUMNS
            };
            for cursor in 0..count {
                let up = (cursor as i64 - vstep as i64).rem_euclid(count as i64) as usize;
                let down = (cursor + vstep) % count;
                assert!(up < count, "up left the grid: {count} items, cursor {cursor}");
                assert!(down < count, "down left the grid");
            }
            // And with a single row, up and down still move somewhere.
            if count > 1 && count <= CHARACTER_COLUMNS {
                assert_ne!(vstep % count, 0, "a single row must still move");
            }
        }
    }

    #[test]
    fn a_tile_that_is_not_there_does_nothing() {
        // The roster is scanned once at startup, so this should be
        // impossible — which is exactly why it must not panic.
        let mut app = world_on(AppState::CharacterSelect);
        confirm(&mut app, MenuAction::Character(7));
        assert_eq!(pending(&app), None);
    }
}
