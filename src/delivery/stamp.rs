//! The version pin: what a host is, and what it refuses to talk to.
//!
//! PRD #855 asks for "bundle or version-pin client assets and reject
//! incompatible combinations clearly". A Phoenix host is identified by three
//! numbers, and all three already existed before this module — it names them
//! together rather than inventing a version scheme:
//!
//! * `protocol` — [`crate::messages::PROTOCOL_VERSION`], the revision of the
//!   `ClientMessage`/`ServerMessage` wire vocabulary. Compiled in, so a running
//!   client's protocol can only ever be learned at request time.
//! * `content_id` / `content_epoch` — the `[content]` block of the scenario
//!   manifest the host is serving, i.e. the identity the mod-pack contract
//!   (issue #986) already gates uploads on. Reused deliberately: a second
//!   content-version number would be a second, quietly disagreeing answer.
//!
//! There are two enforcement points, because the two halves are knowable at
//! different moments:
//!
//! 1. **Startup, against the bundle on disk.** A native host started with
//!    `--client-dir dist/` reads that bundle's own `assets/scenarios.toml` and
//!    refuses to start when its `[content]` disagrees with the manifest the
//!    host serves. This is the case that actually bites — pointing a native
//!    host at a demo bundle while serving the dev catalogue — and it is caught
//!    before a player ever connects.
//! 2. **Request time, against a running client.** `/host/manifest.json` takes
//!    the caller's stamp and answers `409` with a structured body naming both
//!    sides. This is where `protocol` is checked, because the bundle on disk
//!    cannot tell anyone which protocol its WASM was built against.
//!
//! Pure: no Bevy, no I/O, no target gates.

use crate::messages::PROTOCOL_VERSION;
use crate::world::manifest::{parse_content_identity, ContentIdentity};

/// Who a host (or a client) claims to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryStamp {
    /// Wire-vocabulary revision.
    pub protocol: u32,
    /// Content-set identity, from the scenario manifest's `[content] id`.
    pub content_id: String,
    /// Content revision, from the scenario manifest's `[content] epoch`.
    pub content_epoch: i64,
}

impl DeliveryStamp {
    /// The stamp of a host serving `manifest_toml`.
    ///
    /// A manifest declaring no `[content]` block yields the same empty identity
    /// the mod-pack contract uses (`""` / `0`), which no real client can match —
    /// so an unidentified content set refuses connections rather than silently
    /// accepting anything. That is the #986 default, on purpose.
    pub fn for_manifest(manifest_toml: &str) -> Self {
        let content = parse_content_identity(manifest_toml).unwrap_or_default();
        Self::from_content(&content)
    }

    /// The stamp for an already-parsed content identity.
    pub fn from_content(content: &ContentIdentity) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            content_id: content.id.clone(),
            content_epoch: content.epoch,
        }
    }

    /// Parse a stamp from the three request parameters, if all three are
    /// present and well-formed. A caller that supplies none of them is
    /// unstamped (`None`); a caller that supplies a malformed one is also
    /// `None`, and is rejected the same way — a garbled stamp is not evidence
    /// of compatibility.
    pub fn from_params(
        protocol: Option<&str>,
        content_id: Option<&str>,
        content_epoch: Option<&str>,
    ) -> Option<Self> {
        Some(Self {
            protocol: protocol?.trim().parse().ok()?,
            content_id: content_id?.trim().to_string(),
            content_epoch: content_epoch?.trim().parse().ok()?,
        })
    }
}

/// Why a client (or a bundle) was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StampMismatch {
    /// The caller sent no stamp at all.
    ClientStampMissing,
    /// The bundle on disk carries no scenario manifest, or one with no
    /// `[content]` block — so its content identity cannot be established.
    BundleContentMissing {
        path: String,
    },
    Protocol {
        host: u32,
        client: u32,
    },
    ContentId {
        host: String,
        client: String,
    },
    ContentEpoch {
        host: i64,
        client: i64,
    },
}

impl StampMismatch {
    /// Stable machine-readable reason code. Scripts and tests match on this;
    /// the prose in [`StampMismatch::detail`] is for the operator's terminal.
    pub fn code(&self) -> &'static str {
        match self {
            StampMismatch::ClientStampMissing => "client-stamp-missing",
            StampMismatch::BundleContentMissing { .. } => "bundle-content-missing",
            StampMismatch::Protocol { .. } => "protocol-mismatch",
            StampMismatch::ContentId { .. } => "content-id-mismatch",
            StampMismatch::ContentEpoch { .. } => "content-epoch-mismatch",
        }
    }

    /// One line naming BOTH sides and the fix. Operator/CLI diagnostics, in the
    /// same class as `phoenix-headless`'s exit summary — not player-visible
    /// text, so it is not a `strings.csv` id (AGENTS.md rule 11).
    pub fn detail(&self) -> String {
        match self {
            StampMismatch::ClientStampMissing => "client sent no version stamp: expected \
                 protocol, content_id and content_epoch (query parameters or the \
                 x-phoenix-client-stamp header). Refusing rather than guessing."
                .to_string(),
            StampMismatch::BundleContentMissing { path } => format!(
                "client bundle at {path} declares no [content] identity, so it cannot be \
                 matched against the host's. Point --client-dir at a built dist/ (its \
                 assets/scenarios.toml carries the identity), or pass --skip-bundle-check \
                 to serve it unverified."
            ),
            StampMismatch::Protocol { host, client } => format!(
                "protocol mismatch: host speaks {host}, client speaks {client}. Rebuild the \
                 client bundle from the same revision as this host."
            ),
            StampMismatch::ContentId { host, client } => format!(
                "content mismatch: host serves {host:?}, client was built for {client:?}. \
                 These are different content sets; serve the manifest the client expects."
            ),
            StampMismatch::ContentEpoch { host, client } => format!(
                "content epoch mismatch: host serves epoch {host}, client was built for \
                 epoch {client}. Shipped content changed incompatibly; rebuild the client \
                 bundle."
            ),
        }
    }
}

/// Check a running client's stamp against the host's.
///
/// Order matters and is deliberate: protocol first, because a protocol
/// mismatch makes every other field's *meaning* uncertain, and reporting a
/// content difference first would send the operator after the wrong thing.
pub fn check_client_stamp(
    host: &DeliveryStamp,
    client: Option<&DeliveryStamp>,
) -> Result<(), StampMismatch> {
    let Some(client) = client else {
        return Err(StampMismatch::ClientStampMissing);
    };
    if host.protocol != client.protocol {
        return Err(StampMismatch::Protocol {
            host: host.protocol,
            client: client.protocol,
        });
    }
    check_content(host, &client.content_id, client.content_epoch)
}

/// Check a client bundle's content identity against the host's.
///
/// The bundle half of the pin: `bundle` is the `[content]` block of the
/// scenario manifest found inside `--client-dir`. It carries no protocol — a
/// built bundle's protocol lives in its WASM — so only the content pair is
/// compared here, and the protocol half is enforced by
/// [`check_client_stamp`] at request time.
pub fn check_bundle_content(
    host: &DeliveryStamp,
    bundle_manifest_toml: Option<&str>,
    bundle_path: &str,
) -> Result<(), StampMismatch> {
    let content = bundle_manifest_toml
        .and_then(parse_content_identity)
        .filter(|c| !c.id.trim().is_empty());
    let Some(content) = content else {
        return Err(StampMismatch::BundleContentMissing {
            path: bundle_path.to_string(),
        });
    };
    check_content(host, &content.id, content.epoch)
}

fn check_content(host: &DeliveryStamp, id: &str, epoch: i64) -> Result<(), StampMismatch> {
    if host.content_id != id {
        return Err(StampMismatch::ContentId {
            host: host.content_id.clone(),
            client: id.to_string(),
        });
    }
    if host.content_epoch != epoch {
        return Err(StampMismatch::ContentEpoch {
            host: host.content_epoch,
            client: epoch,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "[content]\nid = \"phoenix-base\"\nepoch = 1\n";

    fn host() -> DeliveryStamp {
        DeliveryStamp::for_manifest(BASE)
    }

    #[test]
    fn a_host_stamp_pairs_the_compiled_protocol_with_the_served_content() {
        let s = host();
        assert_eq!(s.protocol, PROTOCOL_VERSION);
        assert_eq!(s.content_id, "phoenix-base");
        assert_eq!(s.content_epoch, 1);
    }

    #[test]
    fn a_manifest_without_a_content_block_stamps_an_identity_nothing_can_match() {
        let s = DeliveryStamp::for_manifest("[[scenario]]\nid = \"x\"\nworld = \"w.toml\"\n");
        assert_eq!(s.content_id, "");
        assert_eq!(s.content_epoch, 0);
        // And that identity really does refuse a real client.
        let client = DeliveryStamp {
            protocol: PROTOCOL_VERSION,
            content_id: "phoenix-base".into(),
            content_epoch: 1,
        };
        assert_eq!(
            check_client_stamp(&s, Some(&client)).unwrap_err().code(),
            "content-id-mismatch"
        );
    }

    #[test]
    fn a_matching_client_is_admitted() {
        let client = host();
        assert!(check_client_stamp(&host(), Some(&client)).is_ok());
    }

    #[test]
    fn an_unstamped_client_is_refused_rather_than_assumed_compatible() {
        assert_eq!(
            check_client_stamp(&host(), None).unwrap_err().code(),
            "client-stamp-missing"
        );
    }

    #[test]
    fn a_protocol_difference_is_reported_before_a_content_difference() {
        let client = DeliveryStamp {
            protocol: PROTOCOL_VERSION + 1,
            content_id: "something-else".into(),
            content_epoch: 99,
        };
        let err = check_client_stamp(&host(), Some(&client)).unwrap_err();
        assert_eq!(err.code(), "protocol-mismatch");
        assert!(err.detail().contains(&PROTOCOL_VERSION.to_string()));
        assert!(err.detail().contains(&(PROTOCOL_VERSION + 1).to_string()));
    }

    #[test]
    fn a_content_epoch_bump_is_reported_with_both_epochs() {
        let client = DeliveryStamp {
            protocol: PROTOCOL_VERSION,
            content_id: "phoenix-base".into(),
            content_epoch: 2,
        };
        let err = check_client_stamp(&host(), Some(&client)).unwrap_err();
        assert_eq!(err.code(), "content-epoch-mismatch");
        assert!(err.detail().contains('1'));
        assert!(err.detail().contains('2'));
    }

    #[test]
    fn a_bundle_serving_the_same_content_passes_the_startup_pin() {
        assert!(check_bundle_content(&host(), Some(BASE), "dist/assets/scenarios.toml").is_ok());
    }

    #[test]
    fn a_bundle_built_for_other_content_fails_the_startup_pin() {
        let other = "[content]\nid = \"other-game\"\nepoch = 1\n";
        let err =
            check_bundle_content(&host(), Some(other), "dist/assets/scenarios.toml").unwrap_err();
        assert_eq!(err.code(), "content-id-mismatch");
    }

    #[test]
    fn a_bundle_with_no_manifest_names_the_path_it_looked_at() {
        let err = check_bundle_content(&host(), None, "dist/assets/scenarios.toml").unwrap_err();
        assert_eq!(err.code(), "bundle-content-missing");
        assert!(err.detail().contains("dist/assets/scenarios.toml"));
    }

    #[test]
    fn a_bundle_manifest_with_no_content_block_is_missing_not_empty() {
        let err = check_bundle_content(
            &host(),
            Some("[[scenario]]\nid = \"x\"\nworld = \"w.toml\"\n"),
            "dist/assets/scenarios.toml",
        )
        .unwrap_err();
        assert_eq!(err.code(), "bundle-content-missing");
    }

    #[test]
    fn a_stamp_parses_from_all_three_params_and_from_nothing_less() {
        assert_eq!(
            DeliveryStamp::from_params(Some("1"), Some("phoenix-base"), Some("1")),
            Some(DeliveryStamp {
                protocol: 1,
                content_id: "phoenix-base".into(),
                content_epoch: 1,
            })
        );
        assert_eq!(
            DeliveryStamp::from_params(None, Some("phoenix-base"), Some("1")),
            None
        );
        assert_eq!(
            DeliveryStamp::from_params(Some("not-a-number"), Some("phoenix-base"), Some("1")),
            None
        );
    }
}
