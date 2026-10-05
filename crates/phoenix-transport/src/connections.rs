//! Physical connection ownership, separate from authoritative Session state.
//!
//! Adapters admit a build/role first, then bind its connection here. Handles
//! carry a transport leg and a never-reused incarnation. Replacement changes
//! ownership before the adapter closes the old link; its late input and close
//! can therefore neither command nor disconnect the replacement Session.

use std::collections::BTreeMap;

use crate::Target;
use std::marker::PhantomData;

/// Application-owned reserved identities. Ordinary transport does not invent roles.
pub trait IdentityPolicy {
    fn is_reserved(token: &str) -> bool;
}
pub struct OpaqueIdentity;
impl IdentityPolicy for OpaqueIdentity {
    fn is_reserved(_: &str) -> bool {
        false
    }
}

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
pub fn validate_token<P: IdentityPolicy>(token: &str) -> Result<(), BindRefusal> {
    if token.is_empty()
        || token.chars().count() > MAX_TOKEN_CHARS
        || token.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(BindRefusal::InvalidToken);
    }
    if P::is_reserved(token) {
        return Err(BindRefusal::ReservedToken);
    }
    Ok(())
}

/// Pure host-local policy. No sockets, queues, ECS resources or Session mutation.
pub struct ConnectionRegistry<P: IdentityPolicy = OpaqueIdentity> {
    policy: PhantomData<fn() -> P>,
    next_leg: u64,
    next_incarnation: u64,
    bindings: BTreeMap<ConnectionId, Option<String>>,
    owners: BTreeMap<String, ConnectionId>,
}

impl<P: IdentityPolicy> Default for ConnectionRegistry<P> {
    fn default() -> Self {
        Self {
            policy: PhantomData,
            next_leg: 0,
            next_incarnation: 0,
            bindings: BTreeMap::new(),
            owners: BTreeMap::new(),
        }
    }
}
impl<P: IdentityPolicy> ConnectionRegistry<P> {
    #[cfg(feature = "test-support")]
    pub fn seed_next_incarnation_for_test(&mut self, next: u64) {
        self.next_incarnation = next;
    }

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
        validate_token::<P>(token)?;
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
