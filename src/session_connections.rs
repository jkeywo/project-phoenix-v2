//! Physical connection ownership, separate from authoritative Session state.
//!
//! Adapters admit a build/role first, then bind its connection here. Handles
//! carry a transport leg and a never-reused incarnation. Replacement changes
//! ownership before the adapter closes the old link; its late input and close
//! can therefore neither command nor disconnect the replacement Session.

use std::collections::BTreeMap;

use crate::lobby::handler::Target;

pub mod browser;

/// The existing Session-token bound, measured in Unicode characters.
pub const MAX_TOKEN_CHARS: usize = 64;

/// A physical connection, including its transport namespace and incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnectionId {
    pub leg: u64,
    pub incarnation: u64,
}

/// Why a connection cannot claim an identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindRefusal {
    InvalidToken,
    ReservedToken,
    ChangedIdentity,
    StaleConnection,
}

/// Validate without normalising: routing and Session policy must see the same key.
/// Bounded opaque tokens remain supported, including browser hex and native UUIDs.
pub fn validate_token(token: &str) -> Result<(), BindRefusal> {
    if token.is_empty()
        || token.chars().count() > MAX_TOKEN_CHARS
        || token.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(BindRefusal::InvalidToken);
    }
    if crate::lobby::handler::is_reserved_token(token) {
        return Err(BindRefusal::ReservedToken);
    }
    Ok(())
}

/// Pure host-local policy. No sockets, queues, ECS resources or Session mutation.
#[derive(Default)]
pub struct ConnectionRegistry {
    next_leg: u64,
    next_incarnation: u64,
    bindings: BTreeMap<ConnectionId, Option<String>>,
    owners: BTreeMap<String, ConnectionId>,
}

impl ConnectionRegistry {
    pub fn new_leg(&mut self) -> u64 {
        let leg = self.next_leg;
        self.next_leg = self
            .next_leg
            .checked_add(1)
            .expect("connection leg exhausted");
        leg
    }

    pub fn open(&mut self, leg: u64) -> ConnectionId {
        let id = ConnectionId {
            leg,
            incarnation: self.next_incarnation,
        };
        self.next_incarnation = self
            .next_incarnation
            .checked_add(1)
            .expect("connection incarnation exhausted");
        self.bindings.insert(id, None);
        id
    }

    /// Bind once. Returns the superseded owner for adapter teardown, if any.
    /// Repeating the same identity on the current connection is idempotent;
    /// a superseded connection may never reclaim ownership by re-Identifying.
    pub fn bind(
        &mut self,
        id: ConnectionId,
        token: &str,
    ) -> Result<Option<ConnectionId>, BindRefusal> {
        validate_token(token)?;
        let Some(binding) = self.bindings.get_mut(&id) else {
            return Err(BindRefusal::StaleConnection);
        };
        if let Some(bound) = binding {
            if bound.as_str() != token {
                return Err(BindRefusal::ChangedIdentity);
            }
            return if self.owners.get(token) == Some(&id) {
                Ok(None)
            } else {
                Err(BindRefusal::StaleConnection)
            };
        }
        *binding = Some(token.to_string());
        Ok(self.owners.insert(token.to_string(), id))
    }

    /// Only the current owner supplies an authenticated sender token.
    pub fn sender(&self, id: ConnectionId) -> Option<&str> {
        let token = self.bindings.get(&id)?.as_deref()?;
        (self.owners.get(token) == Some(&id)).then_some(token)
    }

    pub fn is_superseded(&self, id: ConnectionId) -> bool {
        self.bindings.get(&id).is_some_and(|token| {
            token
                .as_ref()
                .is_some_and(|token| self.owners.get(token) != Some(&id))
        })
    }

    /// Forget one physical link. Only its current owner owes a Session departure.
    pub fn close(&mut self, id: ConnectionId) -> Option<String> {
        let token = self.bindings.remove(&id)??;
        if self.owners.get(&token) != Some(&id) {
            return None;
        }
        self.owners.remove(&token);
        Some(token)
    }

    /// One current connection per recipient, independent of delivery class.
    /// Adapters choose that connection's ready channel and backpressure policy.
    pub fn recipients(&self, target: &Target) -> Vec<ConnectionId> {
        self.owners
            .iter()
            .filter(|(token, _)| match target {
                Target::All => true,
                Target::Token(wanted) => *token == wanted,
                Target::AllExcept(excluded) => *token != excluded,
            })
            .map(|(_, id)| *id)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_owns_both_routing_directions_before_old_close() {
        let mut registry = ConnectionRegistry::default();
        let lan = registry.new_leg();
        let cloud = registry.new_leg();
        let old = registry.open(lan);
        let current = registry.open(cloud);
        assert_eq!(registry.sender(old), None);
        assert_eq!(registry.bind(old, "opaque:crew-A"), Ok(None));
        assert_eq!(registry.bind(current, "opaque:crew-A"), Ok(Some(old)));
        assert_eq!(registry.sender(old), None);
        assert_eq!(registry.sender(current), Some("opaque:crew-A"));
        assert_eq!(
            registry.bind(old, "opaque:crew-A"),
            Err(BindRefusal::StaleConnection)
        );
        assert_eq!(registry.recipients(&Target::All), vec![current]);
        assert_eq!(registry.close(old), None);
        assert_eq!(registry.close(current), Some("opaque:crew-A".into()));
        assert_eq!(registry.close(current), None);
        let fresh = registry.open(cloud);
        assert_ne!(fresh, current);
        assert_eq!(
            registry.bind(current, "opaque:crew-A"),
            Err(BindRefusal::StaleConnection)
        );
    }

    #[test]
    fn same_link_cannot_rename_itself_or_change_recipient_selection() {
        let mut registry = ConnectionRegistry::default();
        let leg = registry.new_leg();
        let a = registry.open(leg);
        let b = registry.open(leg);
        registry.bind(a, "A").unwrap();
        registry.bind(b, "B").unwrap();
        assert_eq!(registry.bind(a, "A"), Ok(None));
        assert_eq!(registry.bind(a, "B"), Err(BindRefusal::ChangedIdentity));
        assert_eq!(registry.recipients(&Target::Token("A".into())), vec![a]);
        assert_eq!(registry.recipients(&Target::AllExcept("A".into())), vec![b]);
        assert_eq!(registry.sender(a), Some("A"));
        assert_eq!(registry.sender(b), Some("B"));
    }
}
