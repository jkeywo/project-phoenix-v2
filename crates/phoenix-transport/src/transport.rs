#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    All,
    Token(String),
    AllExcept(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeliveryClass {
    Reliable,
    Snapshot,
}

/// The game supplies its decoded vocabulary and shared connection policy.
pub trait Profile: 'static {
    type Inbound;
    type Outbound;
    type Connections: Clone + Default;
    /// Opt-in transient inputs retain the actual adapter-owned incarnation.
    fn scoped_ingress_enabled() -> bool {
        false
    }
    fn scoped_ingress(_message: &Self::Inbound) -> bool {
        false
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Event<M> {
    Received {
        token: String,
        msg: M,
    },
    ReceivedScoped {
        token: String,
        connection: u64,
        msg: M,
    },
    Disconnected {
        token: String,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct SnapshotBounds {
    pub max_bytes: usize,
    pub max_buffered_bytes: usize,
}

pub struct Dispatch<'a, M> {
    pub target: &'a Target,
    pub msg: &'a M,
    pub delivery: DeliveryClass,
}

/// Polling and dispatch must not block the application's frame or tick loop.
pub trait Transport<P: Profile>: Send + Sync + 'static {
    fn share_connections(&mut self, _connections: P::Connections) {}
    fn poll(&mut self) -> Vec<Event<P::Inbound>>;
    /// None means this adapter cannot attest a current physical owner.
    fn current_connection(&self, _token: &str) -> Option<u64> {
        None
    }
    fn dispatch(&mut self, dispatch: Dispatch<'_, P::Outbound>);
    /// Explicit lossy refusal, never ordinary-send fallback.
    fn dispatch_snapshot_only(
        &mut self,
        _target: &Target,
        _message: &P::Outbound,
        _bounds: SnapshotBounds,
    ) -> bool {
        false
    }
    fn name(&self) -> &'static str {
        "native"
    }
}

impl<P: Profile, T: Transport<P> + ?Sized> Transport<P> for Box<T> {
    fn share_connections(&mut self, connections: P::Connections) {
        (**self).share_connections(connections);
    }
    fn poll(&mut self) -> Vec<Event<P::Inbound>> {
        (**self).poll()
    }
    fn current_connection(&self, token: &str) -> Option<u64> {
        (**self).current_connection(token)
    }
    fn dispatch(&mut self, dispatch: Dispatch<'_, P::Outbound>) {
        (**self).dispatch(dispatch);
    }
    fn dispatch_snapshot_only(
        &mut self,
        target: &Target,
        message: &P::Outbound,
        bounds: SnapshotBounds,
    ) -> bool {
        (**self).dispatch_snapshot_only(target, message, bounds)
    }
    fn name(&self) -> &'static str {
        (**self).name()
    }
}

/// Ordered composition with one identity owner across every physical leg.
pub struct PairedTransport<P: Profile, A: Transport<P>, B: Transport<P>> {
    first: A,
    second: B,
    profile: std::marker::PhantomData<fn() -> P>,
}
impl<P: Profile, A: Transport<P>, B: Transport<P>> PairedTransport<P, A, B> {
    pub fn new(mut first: A, mut second: B) -> Self {
        let connections = P::Connections::default();
        first.share_connections(connections.clone());
        second.share_connections(connections);
        Self {
            first,
            second,
            profile: std::marker::PhantomData,
        }
    }
}
impl<P: Profile, A: Transport<P>, B: Transport<P>> Transport<P> for PairedTransport<P, A, B> {
    fn share_connections(&mut self, connections: P::Connections) {
        self.first.share_connections(connections.clone());
        self.second.share_connections(connections);
    }
    fn poll(&mut self) -> Vec<Event<P::Inbound>> {
        let mut events = self.first.poll();
        events.extend(self.second.poll());
        events
    }
    fn current_connection(&self, token: &str) -> Option<u64> {
        self.first
            .current_connection(token)
            .or_else(|| self.second.current_connection(token))
    }
    fn dispatch(&mut self, dispatch: Dispatch<'_, P::Outbound>) {
        self.first.dispatch(Dispatch {
            target: dispatch.target,
            msg: dispatch.msg,
            delivery: dispatch.delivery,
        });
        self.second.dispatch(dispatch);
    }
    fn dispatch_snapshot_only(
        &mut self,
        target: &Target,
        message: &P::Outbound,
        bounds: SnapshotBounds,
    ) -> bool {
        let first = self.first.dispatch_snapshot_only(target, message, bounds);
        let second = self.second.dispatch_snapshot_only(target, message, bounds);
        first || second
    }
    fn name(&self) -> &'static str {
        "paired"
    }
}
