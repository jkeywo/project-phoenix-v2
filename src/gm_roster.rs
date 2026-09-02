//! Crew-public Game Master roster (issue #1289).
//!
//! A GM is a host-class peer, not a [`crate::core::messages::Player`], a
//! [`crate::lobby::Sessions`] row, or a [`crate::lockstep::FleetShip`].  This
//! resource is therefore deliberately separate from all three.  It is the
//! public projection only: reconnect credentials, rendezvous peer ids, mesh
//! slots and any owner/leader concept never enter this type.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// Resource ceiling for one rendezvous roster.
///
/// This is a protocol/memory bound, not a gameplay value.  It matches the
/// host-mesh rendezvous record ceiling and prevents an untrusted host-page
/// projection from growing an unbounded authoritative resource.
pub const MAX_GM_OPERATORS: usize = 32;

/// Stable public operator ids follow the existing session-token wire bound.
pub const MAX_GM_OPERATOR_ID_CHARS: usize = 64;

/// Host-mesh display names use the rendezvous `max_slot_name_length` default.
/// Empty names are valid while an operator has not chosen a display name.
pub const MAX_GM_OPERATOR_NAME_CHARS: usize = 48;

/// One crew-visible GM identity.
///
/// `deny_unknown_fields` is intentional: the decoder must reject a host-page
/// row that accidentally includes a peer id, reconnect credential, owner flag
/// or any other private/authority-bearing field instead of silently stripping
/// it only after it crossed the Rust boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GmOperator {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub ready: bool,
}

impl GmOperator {
    /// Create a newly admitted operator. Readiness is deliberately never
    /// inherited from admission or reconnect state.
    pub fn new(id: String, name: String, connected: bool) -> Self {
        Self {
            id,
            name,
            connected,
            ready: false,
        }
    }
}

/// Why a host-page GM roster was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GmRosterError {
    TooManyOperators,
    EmptyId,
    IdTooLong,
    NameTooLong,
    DuplicateId,
}

/// Full replacement roster of equal GM operators.
///
/// Rows are kept in public-id order, making equality and every projection a
/// deterministic function of the same input set rather than of rendezvous
/// arrival order.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct GmRoster {
    operators: Vec<GmOperator>,
}

impl GmRoster {
    /// Validate and canonicalise one full host-page replacement.
    pub fn try_new(mut operators: Vec<GmOperator>) -> Result<Self, GmRosterError> {
        if operators.len() > MAX_GM_OPERATORS {
            return Err(GmRosterError::TooManyOperators);
        }

        for operator in &mut operators {
            let id_len = operator.id.chars().count();
            if id_len == 0 {
                return Err(GmRosterError::EmptyId);
            }
            if id_len > MAX_GM_OPERATOR_ID_CHARS {
                return Err(GmRosterError::IdTooLong);
            }
            if operator.name.chars().count() > MAX_GM_OPERATOR_NAME_CHARS {
                return Err(GmRosterError::NameTooLong);
            }
            // A disconnected operator cannot carry readiness into a later
            // reconnect. Canonicalise at the Rust boundary even when a stale
            // browser projection accidentally says otherwise.
            if !operator.connected {
                operator.ready = false;
            }
        }

        operators.sort_by(|left, right| left.id.cmp(&right.id));
        if operators.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(GmRosterError::DuplicateId);
        }

        Ok(Self { operators })
    }

    /// Crew-public rows, in stable public-id order.
    pub fn operators(&self) -> &[GmOperator] {
        &self.operators
    }

    /// Clone the public projection for a wire snapshot.
    pub fn projection(&self) -> Vec<GmOperator> {
        self.operators.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.operators.is_empty()
    }

    /// Reconcile a full replacement with the previous public presence. A
    /// disconnected -> connected transition is a reconnect and always starts
    /// unready, even if a stale page projection retained the old flag.
    pub fn clear_reconnected_readiness(&mut self, previous: &Self) {
        for operator in &mut self.operators {
            if previous
                .operators
                .iter()
                .any(|old| old.id == operator.id && !old.connected)
            {
                operator.ready = false;
            }
        }
    }

    pub fn readiness_tally(&self) -> crate::lobby::start_policy::ReadinessTally {
        let connected = self
            .operators
            .iter()
            .filter(|operator| operator.connected)
            .count() as u32;
        let ready = self
            .operators
            .iter()
            .filter(|operator| operator.connected && operator.ready)
            .count() as u32;
        crate::lobby::start_policy::ReadinessTally { connected, ready }
    }

    pub fn is_connected(&self, id: &str) -> bool {
        self.operators
            .iter()
            .any(|operator| operator.id == id && operator.connected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operator(id: &str, name: &str, connected: bool) -> GmOperator {
        GmOperator {
            id: id.into(),
            name: name.into(),
            connected,
            ready: false,
        }
    }

    #[test]
    fn roster_order_is_canonical_and_empty_display_names_are_valid() {
        let roster = GmRoster::try_new(vec![
            operator("gm-2", "", false),
            operator("gm-1", "Morgan", true),
        ])
        .unwrap();

        assert_eq!(
            roster
                .operators()
                .iter()
                .map(|operator| operator.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gm-1", "gm-2"]
        );
    }

    #[test]
    fn duplicate_and_unbounded_rows_are_refused() {
        assert_eq!(
            GmRoster::try_new(vec![
                operator("gm-1", "One", true),
                operator("gm-1", "Two", false),
            ]),
            Err(GmRosterError::DuplicateId)
        );
        assert_eq!(
            GmRoster::try_new(
                (0..=MAX_GM_OPERATORS)
                    .map(|index| operator(&format!("gm-{index}"), "", true))
                    .collect()
            ),
            Err(GmRosterError::TooManyOperators)
        );
    }

    #[test]
    fn ids_and_names_are_character_bounded() {
        assert_eq!(
            GmRoster::try_new(vec![operator("", "No id", true)]),
            Err(GmRosterError::EmptyId)
        );
        assert_eq!(
            GmRoster::try_new(vec![operator(
                &"x".repeat(MAX_GM_OPERATOR_ID_CHARS + 1),
                "",
                true,
            )]),
            Err(GmRosterError::IdTooLong)
        );
        assert_eq!(
            GmRoster::try_new(vec![operator(
                "gm-1",
                &"x".repeat(MAX_GM_OPERATOR_NAME_CHARS + 1),
                true,
            )]),
            Err(GmRosterError::NameTooLong)
        );
    }

    #[test]
    fn gm_rows_consume_neither_player_readiness_nor_a_ship_slot() {
        let mut sessions = crate::lobby::session::SessionManager::new();
        sessions.register("crew-1".into(), "Alice".into()).unwrap();
        sessions.set_ready("crew-1", true);
        let fleet = crate::lockstep::FleetRoster::default();

        let gms = GmRoster::try_new(vec![operator("gm-1", "Morgan", true)]).unwrap();

        assert_eq!(gms.operators().len(), 1);
        assert!(sessions.all_ready());
        assert_eq!(fleet.len(), 1);
        assert!(fleet.is_solo());
    }

    #[test]
    fn disconnect_and_reconnect_clear_readiness_without_changing_identity() {
        let connected_ready = GmRoster::try_new(vec![GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: true,
            ready: true,
        }])
        .unwrap();
        let disconnected = GmRoster::try_new(vec![GmOperator {
            id: "gm-1".into(),
            name: "Morgan".into(),
            connected: false,
            ready: true,
        }])
        .unwrap();
        assert!(!disconnected.operators()[0].ready);

        let mut reconnected = connected_ready.clone();
        reconnected.clear_reconnected_readiness(&disconnected);
        assert_eq!(reconnected.operators()[0].id, "gm-1");
        assert!(!reconnected.operators()[0].ready);
    }

    #[test]
    fn gm_readiness_counts_only_connected_rows() {
        let roster = GmRoster::try_new(vec![
            GmOperator {
                ready: true,
                ..operator("gm-1", "One", true)
            },
            GmOperator {
                ready: true,
                ..operator("gm-2", "Two", false)
            },
        ])
        .unwrap();
        assert_eq!(
            roster.readiness_tally(),
            crate::lobby::start_policy::ReadinessTally {
                connected: 1,
                ready: 1
            }
        );
    }
}
