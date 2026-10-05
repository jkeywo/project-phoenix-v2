//! Registry sharing between independently polled transport legs.
use crate::connections::{ConnectionRegistry, IdentityPolicy, OpaqueIdentity};
use std::sync::{Arc, Mutex, MutexGuard};

pub struct SharedConnections<P: IdentityPolicy = OpaqueIdentity>(Arc<Mutex<ConnectionRegistry<P>>>);
impl<P: IdentityPolicy> Clone for SharedConnections<P> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<P: IdentityPolicy> Default for SharedConnections<P> {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(ConnectionRegistry::default())))
    }
}
impl<P: IdentityPolicy> SharedConnections<P> {
    pub fn lock(&self) -> MutexGuard<'_, ConnectionRegistry<P>> {
        self.0.lock().expect("connection registry poisoned")
    }
}
pub struct ConnectionLeg<P: IdentityPolicy = OpaqueIdentity> {
    pub shared: SharedConnections<P>,
    pub id: u64,
}
impl<P: IdentityPolicy> ConnectionLeg<P> {
    pub fn new(shared: SharedConnections<P>) -> Self {
        let id = shared.lock().new_leg();
        Self { shared, id }
    }
}
impl<P: IdentityPolicy> Default for ConnectionLeg<P> {
    fn default() -> Self {
        Self::new(SharedConnections::default())
    }
}
