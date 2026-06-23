use crate::stations_config::StationAssignments;

// reassign_on_join, reassign_on_leave, and all per-count cascade helpers
// removed in B3 (issue #533). Station layout is now a fixed flat list
// populated from ShipConfigResource.
// default_complexity_presets and complexity_presets field removed in B4 (issue #534).

/// Maps session token → station name.  A token absent from this map is a spectator.
pub type StationAssignmentsAlias = StationAssignments;
