//! Headless verification of the state semantics the in-game restart flow
//! relies on (bevy_state 0.19).
//!
//! Regression context: restarting a finished match used to do
//! `NextState::set(AppState::Playing)` while already in `Playing`. That
//! identity transition re-runs the parent's OnExit/OnEnter, but the
//! `PlayState` SUB-state survives untouched — it stayed `Finished`, so the
//! result overlay (`DespawnOnExit(PlayState::Finished)`) never despawned and
//! every gameplay system (gated on `Running`/`Countdown`) stayed off.
//! The fix bounces through a dedicated `Restarting` state so both hops are
//! real transitions and the sub-state re-initializes to its default.

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;

#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum Parent {
    #[default]
    Title,
    Playing,
    Restarting,
}

#[derive(SubStates, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[source(Parent = Parent::Playing)]
enum Sub {
    #[default]
    Countdown,
    Finished,
}

fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    app.init_state::<Parent>();
    app.add_sub_state::<Sub>();
    // Mirror of MenuPlugin's bounce system.
    app.add_systems(
        OnEnter(Parent::Restarting),
        |mut next: ResMut<NextState<Parent>>| next.set(Parent::Playing),
    );
    app
}

fn parent(app: &App) -> Parent {
    *app.world().resource::<State<Parent>>().get()
}

fn sub(app: &App) -> Option<Sub> {
    app.world().get_resource::<State<Sub>>().map(|s| *s.get())
}

fn enter_finished_match(app: &mut App) {
    app.update();
    app.world_mut()
        .resource_mut::<NextState<Parent>>()
        .set(Parent::Playing);
    app.update();
    app.world_mut()
        .resource_mut::<NextState<Sub>>()
        .set(Sub::Finished);
    app.update();
    assert_eq!(parent(app), Parent::Playing);
    assert_eq!(sub(app), Some(Sub::Finished));
}

/// Documents the original bug: an identity transition of the parent state
/// does NOT reset the sub-state, so a "restart" this way leaves the result
/// overlay's state (`Finished`) in place.
#[test]
fn identity_transition_leaves_substate_stuck() {
    let mut app = test_app();
    enter_finished_match(&mut app);

    app.world_mut()
        .resource_mut::<NextState<Parent>>()
        .set(Parent::Playing); // identity
    app.update();
    app.update();

    assert_eq!(parent(&app), Parent::Playing);
    assert_eq!(
        sub(&app),
        Some(Sub::Finished),
        "sub-state survives a parent identity transition — restarts must not rely on it"
    );
}

/// The fix: bouncing through `Restarting` yields two real transitions and
/// the sub-state comes back at its default (`Countdown`).
#[test]
fn restart_bounce_resets_substate() {
    let mut app = test_app();
    enter_finished_match(&mut app);

    app.world_mut()
        .resource_mut::<NextState<Parent>>()
        .set(Parent::Restarting);
    app.update(); // Playing -> Restarting (OnEnter queues Playing)
    assert_eq!(sub(&app), None, "sub-state is gone outside Playing");
    app.update(); // Restarting -> Playing

    assert_eq!(parent(&app), Parent::Playing);
    assert_eq!(
        sub(&app),
        Some(Sub::Countdown),
        "re-entering Playing must reinitialize the sub-state to its default"
    );
}
