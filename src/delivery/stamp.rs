//! The version pin: what a host is, and what it refuses to talk to.
//!
//! PRD #855 asks for "bundle or version-pin client assets and reject
//! incompatible combinations clearly". A Phoenix host is identified by three
//! numbers, and all three already existed before this module — it names them
//! together rather than inventing a version scheme:
//!
//! * `protocol` — [`crate::core::messages::PROTOCOL_VERSION`], the revision of the
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

use crate::core::messages::PROTOCOL_VERSION;
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
    /// Canonical field shared by browser and native fleet handshakes.
    pub fn to_field(&self) -> String {
        format!(
            "{}/{}/{}",
            self.protocol, self.content_id, self.content_epoch
        )
    }

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
#[path = "stamp_tests.rs"]
mod tests;
