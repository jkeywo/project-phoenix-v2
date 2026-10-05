//! The STANDALONE game master's bound identity.
//!
//! The landing's `host_gm` route opens a session in which the game master IS
//! the session: one browser peer, one simulation, the hull its operator picked
//! flown by AI backfill, and nobody else on the wire. PRD #930 calls that peer a
//! full roster member and the `gm-milestone-peer-tracer` exit criterion is "a
//! GM-only deterministic session runs the world".
//!
//! It did not. Privileged GM admission binds the acting operator from the
//! frozen private slot binding — `FleetRoster::gm_operator(local)` in
//! [`crate::gm_action::submit_local`] — and a fleetless peer had no binding at
//! all, so every typed action was refused `NotInFleet`/`NotGameMaster` and the
//! page, which gates each control on the same identity, disabled the whole desk
//! with no sentence anywhere saying why.
//!
//! This module is the missing half: one place that binds the identity into the
//! two authoritative resources admission already reads, and one read-back the
//! page uses so the page and the reducer cannot disagree about who is acting.
//! There is deliberately no second admission rule, no bypass and no token — the
//! bound peer goes through exactly the path a fleet GM goes through.

use bevy::prelude::World;

use crate::gm_roster::{GmOperator, GmRoster};
use crate::lockstep::FleetRoster;

/// The public operator identity a standalone game master binds.
///
/// The same id `gui/host-mesh.js` mints for the FIRST game master of a fleet
/// (`gmId(1)`, and a fleet a game master opens makes itself `gm-1`). Chosen so a
/// session that starts standalone and later opens a fleet keeps the identity its
/// journal already attributes actions to, rather than changing names underneath
/// a durable log.
pub const SOLO_GM_OPERATOR_ID: &str = "gm-1";

/// Bind this peer as the standalone game master of its own session.
///
/// Installs the one-peer roster that names the operator and adds the
/// crew-public roster row admission checks for presence. Returns the bound row,
/// or `None` when this App is not a standalone session after all.
///
/// Fails closed rather than overwriting: a peer that already carries a fleet
/// (an adopted roster, or a lockstep wait-set) is a fleet member whatever route
/// booted it, and the mesh is then the only authority over who may act.
///
/// The row is `connected` because this operator is present by construction —
/// it is the peer — and NOT `ready`, for the reason the page's own start
/// controls give: readiness is a collective answer and there is nobody here to
/// be ready for. That also keeps the legacy countdown dormant
/// (`enforce_fleet_managed_countdown`) exactly as the native local GM does, so
/// the session starts when this operator presses Start and not before.
pub fn bind_standalone_game_master(world: &mut World) -> Option<GmOperator> {
    let scenario_only = world.contains_resource::<crate::gm_projection::GameMasterPeer>();
    Some(prepare_standalone_game_master(world, scenario_only)?.install(world))
}

/// The admission binding, checked before a deferred World is materialized.
pub struct PreparedStandaloneGameMaster {
    roster: FleetRoster,
    replacement: GmRoster,
    operator: GmOperator,
}

impl PreparedStandaloneGameMaster {
    pub fn install(self, world: &mut World) -> GmOperator {
        world.insert_resource(self.roster);
        world.insert_resource(self.replacement);
        self.operator
    }
}

pub fn prepare_standalone_game_master(
    world: &World,
    scenario_only: bool,
) -> Option<PreparedStandaloneGameMaster> {
    if world.contains_resource::<crate::lockstep::FleetLockstep>()
        || world
            .get_resource::<FleetRoster>()
            .is_some_and(|roster| roster != &FleetRoster::default())
    {
        return None;
    }
    // A scenario-only GM has no locally owned hull. Its unclaimed launch slots
    // supply AI ships separately; do not inherit the legacy solo ship row.
    let roster = if scenario_only {
        let slot = crate::command_admission::HostSlot::SOLO;
        FleetRoster::with_participants_and_gms(
            Vec::new(),
            vec![slot],
            vec![crate::lockstep::FleetGm {
                host: slot,
                operator_id: SOLO_GM_OPERATOR_ID.into(),
            }],
            slot,
            slot,
        )?
    } else {
        FleetRoster::solo_game_master(SOLO_GM_OPERATOR_ID)?
    };
    let operator = GmOperator {
        id: SOLO_GM_OPERATOR_ID.to_string(),
        // The operator never typed one: this route asks for a scenario and a
        // hull, not a name. Empty is the value `GmRoster` documents for exactly
        // that, and the surfaces that show an operator already fall back.
        name: String::new(),
        connected: true,
        ready: false,
    };
    let mut rows: Vec<GmOperator> = world
        .get_resource::<GmRoster>()
        .map(|roster| roster.projection())
        .unwrap_or_default()
        .into_iter()
        .filter(|row| row.id != operator.id)
        .collect();
    rows.push(operator.clone());
    let replacement = GmRoster::try_new(rows).ok()?;
    Some(PreparedStandaloneGameMaster {
        roster,
        replacement,
        operator,
    })
}

/// The public operator row this peer's own privileged actions are attributed
/// to, or `None` when this peer may not act as a game master.
///
/// The single read-back the host page reads its admission from. It answers from
/// the SAME two resources [`crate::gm_action::submit_local`] binds against — the
/// frozen slot binding for identity, the crew-public roster for presence — so a
/// control this says is live is a control whose action the reducer will accept,
/// and a control it says is dead is one the reducer would refuse. A standalone
/// session and a fleet session both answer here; neither gets a special case.
pub fn local_gm_operator(world: &World) -> Option<GmOperator> {
    resolve_local_gm_operator(
        world.get_resource::<FleetRoster>(),
        world.get_resource::<GmRoster>(),
    )
}

/// Keep a standalone game master's own bound presence across one host-page
/// replacement of the crew-public roster, or `None` to leave it exactly alone.
///
/// The host page owns that roster as the projection of a FLEET's operators, and
/// publishes a complete array every time. A session with no fleet therefore
/// publishes the EMPTY array — at boot, and again whenever a fleet closes — and
/// that used to land on top of the binding [`bind_standalone_game_master`] made,
/// so the desk was admitted by identity and then refused for absence, which is
/// the same dead console by a different door.
///
/// Narrow on purpose. A peer with more than one participant is a fleet peer, and
/// there the page's projection IS the roster — including this peer's own row and
/// its honest connected state, which the simulation must not overrule. So only a
/// solo roster is considered, and only when the replacement does not mention this
/// peer's operator at all; anything the page actually says about that operator
/// wins.
pub fn preserved_standalone_presence(
    fleet: &FleetRoster,
    current: Option<&GmRoster>,
    replacement: &GmRoster,
) -> Option<GmRoster> {
    if !fleet.is_solo() {
        return None;
    }
    let id = fleet.gm_operator(fleet.local())?;
    if replacement.operators().iter().any(|row| row.id == id) {
        return None;
    }
    let mut rows = replacement.projection();
    rows.push(
        current
            .and_then(|roster| roster.operators().iter().find(|row| row.id == id).cloned())
            .unwrap_or(GmOperator {
                id: id.to_string(),
                name: String::new(),
                connected: true,
                ready: false,
            }),
    );
    GmRoster::try_new(rows).ok()
}

/// [`local_gm_operator`] over the two resources directly, for the frame system
/// that mirrors it to the page and for tests that hold no `World`.
pub fn resolve_local_gm_operator(
    fleet: Option<&FleetRoster>,
    gms: Option<&GmRoster>,
) -> Option<GmOperator> {
    let fleet = fleet?;
    let id = fleet.gm_operator(fleet.local())?;
    gms?.operators()
        .iter()
        .find(|row| row.id == id && row.connected)
        .cloned()
}

#[cfg(test)]
#[path = "gm_solo_tests.rs"]
mod tests;
