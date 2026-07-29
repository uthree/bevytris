//! Global app states and mode selection resources.

use bevy::prelude::*;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    Title,
    Settings,
    /// Mode picker for the solo modes (marathon / sprint / dig).
    SoloSelect,
    /// Stage picker for VS CPU mode.
    StageSelect,
    /// Difficulty picker for zone battle mode.
    ZoneSelect,
    /// Rule sheet editor for a custom match.
    CustomSetup,
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
    /// A round ended but the match is still open (first-to-n).
    RoundOver,
    /// The match is decided (or the marathon ended); result overlay shows.
    Finished,
}

#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Single,
    /// Race: clear 40 lines as fast as possible.
    Sprint,
    /// Race: dig through a pre-stacked field of holed garbage rows.
    Dig,
    VsCpu { stage: u32 },
    /// VS with the zone super move; `stage` picks the CPU profile but the
    /// match never touches campaign progress.
    ZoneBattle { stage: u32 },
    /// VS under the user-authored rule sheet (`GameSettings::custom`);
    /// never touches campaign progress.
    Custom,
}

impl Default for GameMode {
    fn default() -> Self {
        GameMode::Single
    }
}
