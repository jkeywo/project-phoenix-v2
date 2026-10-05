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
