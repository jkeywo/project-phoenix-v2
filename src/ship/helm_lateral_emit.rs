//! Single-owner command-admission seam for the per-axis Helm lateral-thrust AI
//! operator (issue #745).
//!
//! This module is the single file named by the PASM entity
//! `helm-lateral-thrust-ai-operator`'s dependency on the host command-admission
//! seam. It exists as its own module so that dependency is a real, observed code
//! edge (the `use crate::command_admission::…;` below) rather than a claim made
//! only in prose — the same reason `src/console/repair/dispatch.rs` exists for
//! the Repair router (issue #736).
//!
//! `ai_helm_lateral_thrust` emits its dodge through this wrapper, which forwards
//! to the shared `command_admission::ai_emit::emit_ai_command` arbiter (issue
//! #738) — the same `validate_and_admit` seam a network `ControlSystem` message
//! passes through. The command is checked against the emitting ship's own
//! `ControlSourceResolver`, so the AI and human paths converge on one admission
//! gate (AGENTS.md rule 6: nothing downstream branches on controller identity).

use crate::command_admission::ai_emit::emit_ai_command;

/// Emit the per-axis Helm lateral-thrust AI decision into the emitting ship's
/// own `AdmittedCommands`, through the shared [`emit_ai_command`] arbiter.
///
/// A thin forwarding wrapper: it exists so this operator's dependency on the
/// command-admission seam is observable per-entity, not to change any admission
/// policy. The token shape, fallback config, and validation call are all
/// `emit_ai_command`'s (unchanged since issue #738).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_helm_lateral_command(
    entity_uuid: Option<&crate::entity_spawner::EntityUuid>,
    target: crate::messages::SystemId,
    payload: crate::messages::SystemControlPayload,
    sources: &crate::ship_plugin::ShipSystemControlSources,
    sessions: &crate::lobby::Sessions,
    ship_config: Option<&crate::ship_plugin::ShipConfigComponent>,
    admitted: &mut crate::messages::AdmittedCommands,
) -> bool {
    emit_ai_command(
        entity_uuid,
        target,
        payload,
        sources,
        sessions,
        ship_config,
        admitted,
    )
}
