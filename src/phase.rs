//! The console app's UI state machine: which app is showing, and which phase
//! it is in.
//!
//! Moved off `main.rs` for the cutover (U8 step 1), verbatim — the same move
//! `view`, `gamestate` and `symbology` already are. Each cluster moves to the
//! module that owns its rules, and its tests go with it. `Phase` gates
//! authority and `AppMode` names the showing app; the GPUI shell has neither
//! (it holds one exercise, live), so this module documents the legacy app's
//! rules until that app is gone rather than serving a second caller.

/// UI state machine (state-machine grill, #26): phase gates authority,
/// the arm flag gates the engine. Orders flow only in Live + armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Setup,
    Live,
    Closed,
}

/// Top-level mode (task #39): a session exists only in Simulation;
/// Presentation renders live backend data through the same Registry.
/// Switching clears the *view* either way — wire ships and sim ships
/// never share a screen — while the simulation underneath (session,
/// roster, journal, owned units) survives for the return trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Presentation,
    Simulation,
}

/// Whether a map click with a fleet pick in hand may place: a pick, a
/// phase that accepts work, an armed engine, and no assumed identity.
pub fn placement_click_allowed(
    phase: Phase,
    pick: Option<&str>,
    armed: bool,
    acting_as: Option<&str>,
) -> bool {
    pick.is_some()
        && (phase == Phase::Setup || phase == Phase::Live)
        && armed
        && acting_as.is_none()
}

/// Simulation keeps local fixes visible during Setup so placed units appear
/// immediately. Presentation remains wire-first even when its local phase is
/// not Live; a closed Simulation is frozen and accepts no movement.
pub fn accepts_movement_fixes(app_mode: AppMode, phase: Phase) -> bool {
    app_mode == AppMode::Presentation
        || (app_mode == AppMode::Simulation
            && (phase == Phase::Setup || phase == Phase::Live))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Planning must show a local unit as soon as it is placed; the Sim filter
    /// then keeps wire traffic out until the session is live.
    #[test]
    fn simulation_planning_accepts_local_movement_fixes() {
        assert!(accepts_movement_fixes(AppMode::Simulation, Phase::Setup));
        assert!(accepts_movement_fixes(AppMode::Simulation, Phase::Live));
        assert!(!accepts_movement_fixes(AppMode::Simulation, Phase::Closed));
        assert!(accepts_movement_fixes(AppMode::Presentation, Phase::Closed));
    }

    /// A selected fleet pick remains a valid map-placement intent even when
    /// another UI state transition resets the visual tool hint.
    #[test]
    fn selected_fleet_pick_stays_map_clickable_after_tool_reset() {
        assert!(placement_click_allowed(Phase::Setup, Some("unit-1"), true, None));
        assert!(placement_click_allowed(Phase::Live, Some("unit-1"), true, None));
        assert!(!placement_click_allowed(Phase::Setup, None, true, None));
        assert!(!placement_click_allowed(Phase::Closed, Some("unit-1"), true, None));
        assert!(!placement_click_allowed(Phase::Setup, Some("unit-1"), false, None));
        assert!(!placement_click_allowed(Phase::Setup, Some("unit-1"), true, Some("observer")));
    }
}
