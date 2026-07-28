//! bevytris: a guideline-flavored Tetris clone built on Bevy.

use bevy::camera::Hdr;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::PresentMode;

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
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "BEVYTRIS".into(),
                resolution: (1280, 720).into(),
                present_mode: PresentMode::AutoVsync,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.025, 0.06)))
        .insert_resource(config::load_settings())
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
        .add_systems(Startup, setup_camera)
        .run()
}

fn setup_camera(mut commands: Commands) {
    // HDR + bloom: anything drawn with color components pushed past 1.0
    // (active piece, frame glow, particles, shockwaves) blooms into neon.
    commands.spawn((
        Camera2d,
        Hdr,
        Bloom {
            intensity: 0.18,
            ..Bloom::NATURAL
        },
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
