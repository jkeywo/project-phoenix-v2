//! Native sharing of the pure connection registry across transport adapters.

use std::sync::{Arc, Mutex, MutexGuard};

use crate::session_connections::ConnectionRegistry;

/// One host's connection policy, shared by every composed transport leg.
#[derive(Clone, Default)]
pub struct SharedConnections(Arc<Mutex<ConnectionRegistry>>);

impl SharedConnections {
    pub(crate) fn lock(&self) -> MutexGuard<'_, ConnectionRegistry> {
        self.0.lock().expect("connection registry poisoned")
    }
}

pub(crate) struct ConnectionLeg {
    pub shared: SharedConnections,
    pub id: u64,
}

impl ConnectionLeg {
    pub fn new(shared: SharedConnections) -> Self {
        let id = shared.lock().new_leg();
        Self { shared, id }
    }
}

impl Default for ConnectionLeg {
    fn default() -> Self {
        Self::new(SharedConnections::default())
    }
}
