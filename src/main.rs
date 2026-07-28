//! bevytris: a guideline-flavored Tetris clone built on Bevy.

use bevy::camera::{Hdr, ScalingMode};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowMode};

mod audio;
mod config;
mod effects;
mod menu;
mod progress;
mod render;
mod session;
mod state;

/// The engine-independent rules crate, exposed as `crate::core`.
pub(crate) mod core {
    pub use bevytris_core::*;
}

use state::{AppState, GameMode, PlayState};

fn main() -> AppExit {
    let settings = config::load_settings();
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "BEVYTRIS".into(),
                resolution: (1280, 720).into(),
                present_mode: present_mode(settings.vsync),
                mode: window_mode(settings.fullscreen),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.025, 0.06)))
        .insert_resource(settings)
        // Dev helpers: BEVYTRIS_SCREEN=settings|stages|playing skips the
        // title menu, BEVYTRIS_MODE=vs-stage-N|vs-easy|vs-normal|vs-hard
        // preselects the game mode, BEVYTRIS_UNLOCK_ALL=1 opens all stages.
        .insert_state(match std::env::var("BEVYTRIS_SCREEN").as_deref() {
            Ok("settings") => AppState::Settings,
            Ok("stages") => AppState::StageSelect,
            Ok("playing") => AppState::Playing,
            _ => AppState::Title,
        })
        .insert_resource(match std::env::var("BEVYTRIS_MODE").as_deref() {
            Ok("vs-easy") => GameMode::VsCpu { stage: 5 },
            Ok("vs-normal") => GameMode::VsCpu { stage: 15 },
            Ok("vs-hard") => GameMode::VsCpu { stage: 25 },
            Ok(mode) => match mode.strip_prefix("vs-stage-").and_then(|n| n.parse().ok()) {
                Some(stage) => GameMode::VsCpu { stage },
                None => GameMode::Single,
            },
            _ => GameMode::Single,
        })
        .insert_resource(progress::load_progress_with_env())
        .add_sub_state::<PlayState>()
        .add_plugins((
            audio::GameAudioPlugin,
            session::SessionPlugin,
            render::RenderPlugin,
            effects::EffectsPlugin,
            menu::MenuPlugin,
        ))
        .add_systems(PreStartup, use_misaki_font)
        .add_systems(Startup, setup_camera)
        .add_systems(
            Update,
            (fullscreen_hotkey, apply_window_settings, sync_ui_scale),
        )
        .run()
}

/// Replace Bevy's bundled default font with Misaki Gothic 2nd — an 8x8
/// Japanese pixel font (free license, see assets/CREDITS.md) — so every
/// `TextFont::default()` in the game renders pixel-style.
fn use_misaki_font(mut fonts: ResMut<Assets<Font>>) {
    let bytes = include_bytes!("../assets/fonts/misaki_gothic_2nd.ttf");
    fonts.insert(
        Handle::<Font>::default().id(),
        Font::from_bytes(bytes.to_vec()),
    );
}

fn present_mode(vsync: bool) -> PresentMode {
    if vsync {
        PresentMode::AutoVsync
    } else {
        // Lowest input latency; macOS/Windows compositors prevent tearing.
        PresentMode::AutoNoVsync
    }
}

fn window_mode(fullscreen: bool) -> WindowMode {
    if fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    }
}

/// F11 toggles fullscreen from anywhere (persisted like the settings row).
fn fullscreen_hotkey(
    keys: Res<ButtonInput<KeyCode>>,
    mut settings: ResMut<config::GameSettings>,
) {
    if keys.just_pressed(KeyCode::F11) {
        settings.fullscreen = !settings.fullscreen;
        config::save_settings(&settings);
    }
}

/// Live-apply the VSync / Fullscreen toggles.
fn apply_window_settings(
    settings: Res<config::GameSettings>,
    mut windows: Query<&mut Window>,
) {
    if !settings.is_changed() {
        return;
    }
    for mut window in &mut windows {
        let wanted_present = present_mode(settings.vsync);
        if window.present_mode != wanted_present {
            window.present_mode = wanted_present;
        }
        let wanted_mode = window_mode(settings.fullscreen);
        if window.mode != wanted_mode {
            window.mode = wanted_mode;
        }
    }
}

/// Keep UI proportional to the world camera's AutoMin scaling so menus and
/// HUD grow with the window instead of shrinking relative to the boards.
fn sync_ui_scale(windows: Query<&Window>, mut ui_scale: ResMut<UiScale>) {
    let Ok(window) = windows.single() else { return };
    let scale = (window.width() / 1280.0)
        .min(window.height() / 720.0)
        .max(0.5);
    if (ui_scale.0 - scale).abs() > 0.001 {
        ui_scale.0 = scale;
    }
}

fn setup_camera(mut commands: Commands) {
    // HDR + bloom: anything drawn with color components pushed past 1.0
    // (active piece, frame glow, particles, shockwaves) blooms into neon.
    // AutoMin keeps the 1280x720 composition fully visible and scales it
    // up on larger/fullscreen displays.
    commands.spawn((
        Camera2d,
        Hdr,
        Bloom {
            intensity: 0.18,
            ..Bloom::NATURAL
        },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::AutoMin {
                min_width: 1280.0,
                min_height: 720.0,
            },
            ..OrthographicProjection::default_2d()
        }),
    ));
}

/// Boost a color into HDR range so it blooms (factor 1.0 = no glow).
pub fn emissive(color: Color, boost: f32) -> Color {
    let l = color.to_linear();
    Color::LinearRgba(bevy::color::LinearRgba {
        red: l.red * boost,
        green: l.green * boost,
        blue: l.blue * boost,
        alpha: l.alpha,
    })
}
