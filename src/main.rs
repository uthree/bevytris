//! bevytris: a guideline-flavored Tetris clone built on Bevy.

use bevy::prelude::*;
use bevy::window::PresentMode;

mod audio;
mod config;
mod effects;
mod menu;
mod render;
mod session;
mod state;

/// The engine-independent rules crate, exposed as `crate::core`.
pub(crate) mod core {
    pub use bevytris_core::*;
}

use state::{AppState, CpuDifficulty, GameMode, PlayState};

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
        // Dev helpers: BEVYTRIS_SCREEN=settings|playing skips the title menu,
        // BEVYTRIS_MODE=vs-easy|vs-normal|vs-hard preselects the game mode.
        .insert_state(match std::env::var("BEVYTRIS_SCREEN").as_deref() {
            Ok("settings") => AppState::Settings,
            Ok("playing") => AppState::Playing,
            _ => AppState::Title,
        })
        .insert_resource(match std::env::var("BEVYTRIS_MODE").as_deref() {
            Ok("vs-easy") => GameMode::VsCpu(CpuDifficulty::Easy),
            Ok("vs-normal") => GameMode::VsCpu(CpuDifficulty::Normal),
            Ok("vs-hard") => GameMode::VsCpu(CpuDifficulty::Hard),
            _ => GameMode::Single,
        })
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
    commands.spawn(Camera2d);
}
