//! Gamepad input layer: merges every connected pad's buttons and left
//! stick into the states the game and menus read alongside the keyboard.
//!
//! Two kinds of state live here. Menu navigation is FIXED (D-pad/stick
//! moves, A confirms, B backs out) so menus always work no matter how the
//! game buttons are configured. In-game actions are REBINDABLE — each
//! [`Action`] maps to the gamepad button chosen in the settings — with
//! one hardwired convenience: the left stick always moves the piece
//! (left/right/soft drop, up = hard drop), mirroring the D-pad defaults.

use bevy::prelude::*;
use std::collections::HashSet;

use crate::config::{Action, GameSettings, bindable_pad_buttons};

/// Fixed navigation actions (menus, overlays).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PadAction {
    Left,
    Right,
    Up,
    Down,
    Confirm,
    Back,
}

/// One connected gamepad's in-game action state, kept apart from the
/// merged view so local versus can hand each player their own pad.
#[derive(Default)]
struct PadSlot {
    act_pressed: HashSet<Action>,
    act_just: HashSet<Action>,
}

/// Merged digital state of all connected gamepads, rebuilt every frame in
/// `PreUpdate`. Stick deflections are digitized here so callers get the
/// same pressed/just_pressed edges as for real buttons.
///
/// Menus and single-board play read the merged state, so any pad works
/// without asking which. Local versus reads [`PadInput::slot_pressed`]
/// instead, which keeps pad 0 and pad 1 on separate boards.
#[derive(Resource, Default)]
pub struct PadInput {
    pressed: HashSet<PadAction>,
    just_pressed: HashSet<PadAction>,
    act_pressed: HashSet<Action>,
    act_just: HashSet<Action>,
    raw_pressed: HashSet<GamepadButton>,
    raw_just: Vec<GamepadButton>,
    /// Per-pad state, ordered by gamepad entity so a given controller
    /// keeps its slot for as long as it stays connected.
    slots: Vec<PadSlot>,
}

impl PadInput {
    pub fn just_pressed(&self, action: PadAction) -> bool {
        self.just_pressed.contains(&action)
    }

    /// Rebindable in-game action state (bound button or stick).
    pub fn action_pressed(&self, action: Action) -> bool {
        self.act_pressed.contains(&action)
    }

    pub fn action_just_pressed(&self, action: Action) -> bool {
        self.act_just.contains(&action)
    }

    /// Action state of one pad only. `slot` past the number of connected
    /// pads reads as "nothing pressed", so a versus match with a single
    /// controller simply leaves player 2 on the keyboard.
    pub fn slot_pressed(&self, slot: usize, action: Action) -> bool {
        self.slots
            .get(slot)
            .is_some_and(|s| s.act_pressed.contains(&action))
    }

    pub fn slot_just_pressed(&self, slot: usize, action: Action) -> bool {
        self.slots
            .get(slot)
            .is_some_and(|s| s.act_just.contains(&action))
    }

    /// A bindable button that went down this frame (for the rebind UI).
    pub fn raw_just_pressed(&self) -> Option<GamepadButton> {
        self.raw_just.first().copied()
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

fn poll_pads(
    mut state: ResMut<PadInput>,
    settings: Res<GameSettings>,
    pads: Query<(Entity, &Gamepad)>,
) {
    let mut nav: HashSet<PadAction> = HashSet::new();
    let mut act: HashSet<Action> = HashSet::new();
    let mut raw: HashSet<GamepadButton> = HashSet::new();
    // Sorted so slot assignment is stable frame to frame: query order is
    // an archetype detail, and a pad that swapped slots mid-match would
    // hand the wrong board to the wrong player.
    let mut ordered: Vec<(Entity, &Gamepad)> = pads.iter().collect();
    ordered.sort_by_key(|(entity, _)| *entity);
    let mut per_pad: Vec<HashSet<Action>> = Vec::with_capacity(ordered.len());

    for (_, pad) in ordered {
        let mut mine: HashSet<Action> = HashSet::new();
        let x = pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0);
        let y = pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0);

        // Fixed navigation: D-pad/stick directions, A confirm, B back.
        let mut set = |cond: bool, action: PadAction| {
            if cond {
                nav.insert(action);
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
        set(pad.pressed(GamepadButton::South), PadAction::Confirm);
        set(pad.pressed(GamepadButton::East), PadAction::Back);

        // Rebindable in-game actions.
        for action in Action::ALL {
            if pad.pressed(settings.pad_for(action)) {
                mine.insert(action);
            }
        }
        // The stick always moves the piece, whatever the buttons say.
        if x < -STICK_THRESHOLD {
            mine.insert(Action::MoveLeft);
        }
        if x > STICK_THRESHOLD {
            mine.insert(Action::MoveRight);
        }
        if y < -STICK_THRESHOLD {
            mine.insert(Action::SoftDrop);
        }
        if y > STICK_THRESHOLD {
            mine.insert(Action::HardDrop);
        }

        // Raw buttons for the rebind capture UI.
        for button in bindable_pad_buttons() {
            if pad.pressed(button) {
                raw.insert(button);
            }
        }
        act.extend(mine.iter().copied());
        per_pad.push(mine);
    }

    // Per-pad edges, computed against the same pad's previous frame.
    state.slots.resize_with(per_pad.len(), PadSlot::default);
    for (slot, pressed) in state.slots.iter_mut().zip(per_pad) {
        slot.act_just = pressed.difference(&slot.act_pressed).copied().collect();
        slot.act_pressed = pressed;
    }

    state.just_pressed = nav.difference(&state.pressed).copied().collect();
    state.pressed = nav;
    state.act_just = act.difference(&state.act_pressed).copied().collect();
    state.act_pressed = act;
    // Deterministic order (the bindable list) in case two buttons land on
    // the same frame during a rebind.
    state.raw_just = bindable_pad_buttons()
        .into_iter()
        .filter(|b| raw.contains(b) && !state.raw_pressed.contains(b))
        .collect();
    state.raw_pressed = raw;
}
