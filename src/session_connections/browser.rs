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
mod tests {
    use super::*;

    #[test]
    fn ffi_is_usable_before_an_app_and_keeps_handles_exact_past_javascript_integer_precision() {
        let mut host = BrowserConnections::new();
        host.registry.next_incarnation = 9_007_199_254_740_992;
        let old = host.open();
        let current = host.open();
        assert_eq!(old, "9007199254740992");
        assert_eq!(current, "9007199254740993");
        assert_eq!(host.bind(&old, "crew"), r#"{"ok":true,"previous":null}"#);
        assert_eq!(
            host.bind(&current, "crew"),
            r#"{"ok":true,"previous":"9007199254740992"}"#
        );
        assert_eq!(host.sender(&old), None);
        assert_eq!(host.close(&old), None);
        assert_eq!(host.sender(&current).as_deref(), Some("crew"));
        assert_eq!(host.recipients("token:crew"), r#"["9007199254740993"]"#);
        assert_eq!(host.recipients("except:crew"), "[]");
        assert_eq!(host.recipients("unknown"), "[]");
        assert_eq!(
            host.bind(&current, "other"),
            r#"{"code":"invalid-token","ok":false}"#
        );
        assert_eq!(host.close(&current).as_deref(), Some("crew"));
        assert_eq!(host.recipients("all"), "[]");
    }

    #[test]
    fn ffi_does_not_accept_aliased_or_invalid_handles() {
        let mut host = BrowserConnections::new();
        let handle = host.open();
        assert_eq!(handle, "0");
        for alias in ["00", "+0", "-0", "", "not-a-handle", "18446744073709551616"] {
            assert_eq!(
                host.bind(alias, "crew"),
                r#"{"code":"invalid-token","ok":false}"#
            );
            assert_eq!(host.sender(alias), None);
            assert_eq!(host.close(alias), None);
        }
        assert_eq!(host.bind(&handle, "crew"), r#"{"ok":true,"previous":null}"#);
        assert_eq!(host.sender(&handle).as_deref(), Some("crew"));
    }
}
