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
        })
    }

    /// The session token this pane presents at `Identify`.
    pub fn token(&self) -> &str {
        &self.token
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
mod tests {
    use super::*;

    #[test]
    fn a_minted_token_is_a_uuid_and_two_panes_never_share_one() {
        let a = PaneIdentity::mint("Ada");
        let b = PaneIdentity::mint("Grace");
        assert_ne!(a.token(), b.token());
        assert_eq!(a.token().len(), 36, "UUIDv4 hyphenated: {}", a.token());
        assert_eq!(a.name(), "Ada");
        // And it is admissible by the lobby's own test, which is the claim that
        // matters: the seam refuses reserved tokens, so a minted one that
        // happened to look reserved would produce a pane that silently never
        // joined.
        assert!(!crate::lobby::handler::is_reserved_token(a.token()));
    }

    #[test]
    fn the_host_operators_token_cannot_become_a_panes_identity() {
        // `LOCAL_CONSOLE_TOKEN` skips the station-tenure branch of
        // `is_command_authorized` entirely and carries mission-abort authority.
        // A pane built around it would satisfy every other acceptance criterion
        // and violate the one that matters.
        let err = PaneIdentity::adopt(crate::console_bridge::LOCAL_CONSOLE_TOKEN, "impostor")
            .expect_err("the host operator's token is not a participant identity");
        assert!(matches!(err, IdentityRefusal::Reserved(_)));
        assert!(
            err.to_string().contains("ordinary session token"),
            "the refusal must say what to do instead: {err}"
        );
    }

    #[test]
    fn an_ai_control_source_label_cannot_become_a_panes_identity() {
        // `ai:`-prefixed tokens take the `policy.operate_ai` branch, which is
        // the other half of the same hole.
        assert!(matches!(
            PaneIdentity::adopt("ai:helm", "impostor"),
            Err(IdentityRefusal::Reserved(_))
        ));
    }

    #[test]
    fn an_empty_token_is_refused_rather_than_producing_an_unkeyable_session() {
        assert_eq!(PaneIdentity::adopt("", "Ada"), Err(IdentityRefusal::Empty));
    }

    #[test]
    fn an_ordinary_token_is_adopted_unchanged() {
        let identity = PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap();
        assert_eq!(identity.token(), "3f1a6c2e-0a11-4b3c-9d55-000000000001");
    }
}
