//! Grid-specific vocabulary and compatibility; the relay knows none of these variants.
use crate::{codec, Grid, GridState, Move};
use phoenix_transport::relay::{CompatibilityRefusal, RelayProtocol};
use phoenix_transport::{
    connections::OpaqueIdentity, shared_connections::SharedConnections, Profile,
};
use serde::{Deserialize, Serialize};
pub const STAMP: &str = "grid/1";
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Input {
    Identify { token: String, name: String },
    Move(Move),
    Recover,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Output {
    State { state: GridState, digest: String },
    Recovery { checkpoint: String },
}
impl Output {
    pub fn state(grid: &Grid) -> Self {
        Self::State {
            state: grid.state.clone(),
            digest: grid.digest(),
        }
    }
}
pub struct GridProtocol;
impl Profile for GridProtocol {
    type Inbound = Input;
    type Outbound = Output;
    type Connections = SharedConnections;
}
impl RelayProtocol for GridProtocol {
    type Identity = OpaqueIdentity;
    type Stamp = ();
    fn check_stamp(_: &(), peer: Option<&str>) -> Result<(), CompatibilityRefusal> {
        if peer == Some(STAMP) {
            Ok(())
        } else {
            Err(CompatibilityRefusal {
                code: "version-mismatch".into(),
                detail: "This host runs Grid 1".into(),
            })
        }
    }
    fn decode_client(payload: &str) -> Result<Input, String> {
        codec::decode(payload)
    }
    fn identity(message: &Input) -> Option<&str> {
        if let Input::Identify { token, .. } = message {
            Some(token)
        } else {
            None
        }
    }
    fn encode_server(message: &Output) -> Result<String, String> {
        codec::encode(message)
    }
}

/// Shared application decision after physical identity admission.
pub fn apply(grid: &mut Grid, input: Input) -> Result<Option<Output>, String> {
    match input {
        Input::Move(movement) => {
            grid.submit(movement);
            Ok(None)
        }
        Input::Identify { .. } | Input::Recover => Ok(Some(Output::Recovery {
            checkpoint: grid.checkpoint()?,
        })),
    }
}

#[derive(Serialize)]
pub struct BrowserReply {
    pub previous: Option<String>,
    pub output: Option<Output>,
}

/// Browser physical handles use the same ownership policy as native relay.
/// JavaScript only holds sockets; decoding, binding and recipient admission live here.
pub struct BrowserAdmission {
    connections: phoenix_transport::connections::ConnectionRegistry,
    handles: std::collections::BTreeMap<String, phoenix_transport::connections::ConnectionId>,
}
impl Default for BrowserAdmission {
    fn default() -> Self {
        Self {
            connections: Default::default(),
            handles: Default::default(),
        }
    }
}
impl BrowserAdmission {
    pub fn open(&mut self) -> String {
        let id = self.connections.open(0);
        let handle = id.incarnation.to_string();
        self.handles.insert(handle.clone(), id);
        handle
    }
    pub fn close(&mut self, handle: &str) {
        if let Some(id) = self.handles.remove(handle) {
            self.connections.close(id);
        }
    }
    pub fn receive(
        &mut self,
        grid: &mut Grid,
        handle: &str,
        raw: &str,
    ) -> Result<BrowserReply, String> {
        let input = GridProtocol::decode_client(raw)?;
        let id = *self.handles.get(handle).ok_or("stale-connection")?;
        let previous = if let Some(token) = GridProtocol::identity(&input) {
            self.connections
                .bind(id, token)
                .map_err(|reason| format!("{reason:?}"))?
                .map(|old| old.incarnation.to_string())
        } else {
            None
        };
        let output = if self.connections.sender(id).is_some() {
            apply(grid, input)?
        } else {
            None
        };
        Ok(BrowserReply { previous, output })
    }
    pub fn recipients(&self) -> Vec<String> {
        self.connections
            .recipients(&phoenix_transport::Target::All)
            .iter()
            .map(|id| id.incarnation.to_string())
            .collect()
    }
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
