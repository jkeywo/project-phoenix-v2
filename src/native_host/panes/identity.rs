//! Who a pane is (issue #1122).
//!
//! A pane is **not** the host operator. That distinction is the whole of this
//! module, and it is the single most important design point in the issue.
//!
//! `console_bridge::LOCAL_CONSOLE_TOKEN` exists, it is reserved, and its doc
//! comment even names a native host as a future user of it — so reaching for it
//! is the obvious move and it is the wrong one. That token is a **privilege
//! bypass**: `command_admission::policy::is_command_authorized` accepts it with
//! no station-tenure check at all (branch 2, before the `holder_for_station`
//! comparison every ordinary participant is subject to), and
//! `lobby::handler` grants it `ReturnToLobbyAuthority::Host`, which can abort a
//! mission that is still in progress. It is the token the host's own viewscreen
//! controls ride under, and it stays that.
//!
//! Issue #1122's acceptance criterion is that in-process delivery "uses the same
//! logical-client, command admission and audience projection boundaries as a
//! network client and **cannot bypass them**". So a pane is a phone:
//!
//! * an ordinary UUIDv4 session token, minted the same shape
//!   `crypto.randomUUID()` mints in a browser tab,
//! * through `handle_identify` → `Welcome` → `SelectStation` → `SetReady`,
//! * subject to the station-tenure branch of `is_command_authorized`,
//! * resolvable by `Audience::Holding*` through
//!   `SessionManager::holder_for_station`, and by nothing else.
//!
//! # Two refusals, not one
//!
//! [`PaneIdentity::adopt`] refuses a reserved token at the point a pane is
//! *created*, and `native_host::transport::drain_native_inbound` refuses one
//! again at the point anything crosses the ingress. That is deliberate
//! duplication: the browser does the same (`server.html`'s `isPeerTokenAllowed`
//! at the PeerJS ingress *and* `handle_identify` server-side), and the reason is
//! that they close different holes. The first stops a pane being *configured*
//! with host authority; the second stops a page that talked its way into an
//! arbitrary token string from using one.

/// A pane's participant identity: the session token it presents at `Identify`,
/// and the name that token joins under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneIdentity {
    token: String,
    name: String,
    assigned_station: Option<String>,
}

/// Why an identity was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityRefusal {
    /// The token is one the lobby reserves — `__local_console__`, or an
    /// `ai:`-prefixed control-source label.
    Reserved(String),
    /// The token is empty, which no session manager can key on.
    Empty,
}

impl std::fmt::Display for IdentityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentityRefusal::Reserved(token) => write!(
                f,
                "{token:?} is reserved for the host operator or for AI control; a pane joins \
                 with an ordinary session token, like a phone"
            ),
            IdentityRefusal::Empty => write!(f, "a pane's session token cannot be empty"),
        }
    }
}

impl std::error::Error for IdentityRefusal {}

impl PaneIdentity {
    /// Mint a fresh identity for `name`.
    ///
    /// UUIDv4, the same shape and the same entropy source a browser tab's
    /// `crypto.randomUUID()` produces — because that is what the rest of the
    /// system already treats as an ordinary participant, and an identity that
    /// *looked* different would invite code that branched on it.
    ///
    /// Not drawn from `SimRng`, and not from `world_id::mint_id_with`. A session
    /// token is **transport identity, not simulation state**: it is minted
    /// before the participant is admitted, nothing folds it into the world
    /// digest, and taking it from the seeded generator would make two replays of
    /// one seed hand out the same tokens — a collision waiting for a second
    /// host, rather than a determinism win.
    ///
    /// Hence the scoped allow: `clippy.toml` bans `Uuid::new_v4` so a *world
    /// entity id* cannot come from the OS, and this is the other kind of
    /// identity — the same kind, and the same generator, a browser tab's
    /// `crypto.randomUUID()` produces for exactly this purpose.
    #[allow(clippy::disallowed_methods)]
    pub fn mint(name: impl Into<String>) -> Self {
        Self {
            token: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            assigned_station: None,
        }
    }

    /// Adopt an already-chosen token, refusing the reserved ones.
    ///
    /// The route a test or an operator takes when the token has to be
    /// predictable. Everything the lobby refuses at `Identify` is refused here
    /// too, so a pane cannot even be *built* around host authority.
    pub fn adopt(
        token: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, IdentityRefusal> {
        let token = token.into();
        if token.is_empty() {
            return Err(IdentityRefusal::Empty);
        }
        if crate::lobby::handler::is_reserved_token(&token) {
            return Err(IdentityRefusal::Reserved(token));
        }
        Ok(Self {
            token,
            name: name.into(),
            assigned_station: None,
        })
    }

    /// The session token this pane presents at `Identify`.
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn for_station(station: &str) -> Self {
        let mut identity = Self::mint(station);
        identity.assigned_station = Some(station.to_owned());
        identity
    }

    pub fn assigned_station(&self) -> Option<&str> {
        self.assigned_station.as_deref()
    }

    /// The participant name this pane joins under.
    ///
    /// Operator-supplied (`--pane <name>`), never a literal in this crate: a
    /// participant name is player-visible text, and this repository keeps
    /// player-visible text in `assets/strings/strings.csv` rather than in Rust.
    /// A crew member's own name is neither — it is input.
    pub fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
