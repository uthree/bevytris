//! Gamepad input layer: merges every connected pad's buttons and left
//! stick into semantic actions that the game and menus read alongside the
//! keyboard. The mapping is fixed, guideline-style: D-pad/stick moves
//! (up = hard drop), A/Y rotate CW, B/X rotate CCW, bumpers hold,
//! triggers fire the zone, Start pauses; menus use D-pad/stick plus
//! A confirm / B back.

use bevy::prelude::*;
use std::collections::HashSet;

/// Semantic pad actions. Game and menu actions overlap on purpose (South
/// is both RotateCw and Confirm) — they are read in different app states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadAction {
    Left,
    Right,
    Up,
    Down,
    RotateCw,
    RotateCcw,
    Hold,
    /// Zone super move (zone battle mode).
    Zone,
    Pause,
    Confirm,
    Back,
}

/// Merged digital state of all connected gamepads, rebuilt every frame in
/// `PreUpdate`. Stick deflections are digitized here so callers get the
/// same pressed/just_pressed edges as for real buttons.
#[derive(Resource, Default)]
pub struct PadInput {
    pressed: HashSet<PadAction>,
    just_pressed: HashSet<PadAction>,
}

impl PadInput {
    pub fn pressed(&self, action: PadAction) -> bool {
        self.pressed.contains(&action)
    }

    pub fn just_pressed(&self, action: PadAction) -> bool {
        self.just_pressed.contains(&action)
    }
}

/// Stick deflection treated as a digital press.
const STICK_THRESHOLD: f32 = 0.5;

pub struct PadInputPlugin;

impl Plugin for PadInputPlugin {
    fn build(&self, app: &mut App) {
        // After bevy's gamepad processing, before the Update systems that
        // read PadInput — no one-frame lag.
        app.init_resource::<PadInput>()
            .add_systems(PreUpdate, poll_pads.after(bevy::input::InputSystems));
    }
}

fn poll_pads(mut state: ResMut<PadInput>, pads: Query<&Gamepad>) {
    let mut now: HashSet<PadAction> = HashSet::new();
    for pad in &pads {
        let x = pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0);
        let y = pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0);
        let mut set = |cond: bool, action: PadAction| {
            if cond {
                now.insert(action);
            }
        };
        set(
            pad.pressed(GamepadButton::DPadLeft) || x < -STICK_THRESHOLD,
            PadAction::Left,
        );
        set(
            pad.pressed(GamepadButton::DPadRight) || x > STICK_THRESHOLD,
            PadAction::Right,
        );
        set(
            pad.pressed(GamepadButton::DPadDown) || y < -STICK_THRESHOLD,
            PadAction::Down,
        );
        set(
            pad.pressed(GamepadButton::DPadUp) || y > STICK_THRESHOLD,
            PadAction::Up,
        );
        set(
            pad.pressed(GamepadButton::South) || pad.pressed(GamepadButton::North),
            PadAction::RotateCw,
        );
        set(
            pad.pressed(GamepadButton::East) || pad.pressed(GamepadButton::West),
            PadAction::RotateCcw,
        );
        set(
            pad.pressed(GamepadButton::LeftTrigger) || pad.pressed(GamepadButton::RightTrigger),
            PadAction::Hold,
        );
        set(
            pad.pressed(GamepadButton::LeftTrigger2) || pad.pressed(GamepadButton::RightTrigger2),
            PadAction::Zone,
        );
        set(pad.pressed(GamepadButton::Start), PadAction::Pause);
        set(pad.pressed(GamepadButton::South), PadAction::Confirm);
        set(pad.pressed(GamepadButton::East), PadAction::Back);
    }
    state.just_pressed = now.difference(&state.pressed).copied().collect();
    state.pressed = now;
}
