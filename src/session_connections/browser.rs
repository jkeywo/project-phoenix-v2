//! Browser-host FFI for the shared connection owner. Constructed when the WASM
//! module loads, before any ECS App or World exists. Strings keep incarnation
//! handles exact across JavaScript's number boundary; they are opaque to JS.

use super::{BindRefusal, ConnectionId, ConnectionRegistry};
use crate::core::codec::{encode_connection_binding, encode_connection_recipients};
use crate::lobby::handler::Target;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct BrowserConnections {
    registry: ConnectionRegistry,
    leg: u64,
}

impl Default for BrowserConnections {
    fn default() -> Self {
        let mut registry = ConnectionRegistry::default();
        let leg = registry.new_leg();
        Self { registry, leg }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl BrowserConnections {
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen(constructor))]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self) -> String {
        self.registry.open(self.leg).incarnation.to_string()
    }

    /// Local FFI result, not a game message. The adapter sends an ordinary
    /// JoinRefused for a refusal and closes `previous` only after this returns.
    pub fn bind(&mut self, handle: &str, token: &str) -> String {
        let result = self
            .id(handle)
            .ok_or(BindRefusal::StaleConnection)
            .and_then(|id| self.registry.bind(id, token));
        encode_connection_binding(result)
    }

    pub fn sender(&self, handle: &str) -> Option<String> {
        self.registry.sender(self.id(handle)?).map(str::to_owned)
    }

    pub fn close(&mut self, handle: &str) -> Option<String> {
        self.registry.close(self.id(handle)?)
    }

    /// Decode the existing outbound callback's Target spelling. Recipient
    /// selection itself belongs entirely to ConnectionRegistry.
    pub fn recipients(&self, target: &str) -> String {
        let target = if target == "all" {
            Some(Target::All)
        } else if let Some(token) = target.strip_prefix("token:") {
            Some(Target::Token(token.to_owned()))
        } else {
            target
                .strip_prefix("except:")
                .map(|token| Target::AllExcept(token.to_owned()))
        };
        let recipients = target
            .map(|target| self.registry.recipients(&target))
            .unwrap_or_default();
        encode_connection_recipients(&recipients)
    }
}

impl BrowserConnections {
    fn id(&self, handle: &str) -> Option<ConnectionId> {
        let incarnation = handle.parse::<u64>().ok()?;
        // Reject aliases such as `00` or `+1`; one physical handle has one key.
        (incarnation.to_string() == handle).then_some(ConnectionId {
            leg: self.leg,
            incarnation,
        })
    }
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
