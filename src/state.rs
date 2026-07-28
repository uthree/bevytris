//! Global app states and mode selection resources.

use bevy::prelude::*;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    Title,
    Settings,
    Playing,
    /// One-frame bounce state used to restart a match. A Playing→Playing
    /// identity transition would leave the `PlayState` sub-state untouched
    /// (stuck at `Finished`), so restarts route through here to get real
    /// OnExit/OnEnter transitions and a freshly initialized sub-state.
    Restarting,
}

/// Sub-state active only while in `AppState::Playing`.
#[derive(SubStates, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[source(AppState = AppState::Playing)]
pub enum PlayState {
    /// 3-2-1 countdown before control is handed to the players.
    #[default]
    Countdown,
    Running,
    Paused,
    /// Somebody topped out (or won); result overlay is showing.
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuDifficulty {
    Easy,
    Normal,
    Hard,
}

impl CpuDifficulty {
    pub fn label(self) -> &'static str {
        match self {
            CpuDifficulty::Easy => "EASY",
            CpuDifficulty::Normal => "NORMAL",
            CpuDifficulty::Hard => "HARD",
        }
    }
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Single,
    VsCpu(CpuDifficulty),
}

impl Default for GameMode {
    fn default() -> Self {
        GameMode::Single
    }
}
