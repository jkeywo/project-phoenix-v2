//! Local bridge stations as isolated Ultralight panes (issue #1122).
//!
//! A pane is **one logical client running inside the host process**. It has its
//! own document, its own minted session token, its own input queue and its own
//! lifecycle, and it joins, claims a Station, readies and plays through exactly
//! the contracts a phone does — because in-process delivery is allowed to skip
//! network serialisation and nothing else (PRD #1093: *"In-process delivery may
//! avoid network serialisation but cannot bypass command admission or projection
//! boundaries"*).
//!
//! # The shape, in one pass
//!
//! ```text
//! Ultralight View  ── the ordinary client page, from this host's own HTTP server
//!   │  window.phoenixPaneOut.send(json)          [document]
//!   ▼
//! UltralightPaneSurface::drain                   [ultralight, feature-gated]
//!   ▼
//! PaneBus::submit_json → core::codec             [transport]
//!   ▼
//! PaneTransport::poll → TransportEvent::Received [transport]
//!   ▼
//! native_host::transport::drain_native_inbound   ← the #1121 seam, reserved-token gate
//!   ▼
//! Messages<InboundMessage> → lobby::handler / command_admission
//!
//! …and back:
//!
//! Messages<OutboundMessage>  (Target already resolved by core::broadcast::audience)
//!   ▼
//! flush_native_outbound → PaneTransport::dispatch [transport]
//!   ▼
//! routing::pane_receives                          ← the projection boundary
//!   ▼
//! Pane's outbound queue → pane_apply_script       [registry, document]
//!   ▼
//! window.__phoenixPaneApply(json) → the page's own handleMessage
//! ```
//!
//! # What is here, and what is deliberately in vellum
//!
//! Issue #1122 asked for an audit of void-and-thunder's working Ultralight
//! integration, recording which pieces are project-specific and which suit a
//! shared crate. That audit's conclusions are recorded in three places — this
//! module tree, `vellum`'s `docs/handbook/dependencies.md`, and
//! `wiki/concepts/native-host.md` — and its verdict is:
//!
//! | piece | where it went | why |
//! |---|---|---|
//! | SDK acquisition, linking, DLL/`resources` staging | **vellum** (`vellum_ultralight::staging`) | identical for any consumer, and *neither* game had automated it |
//! | `Renderer`/`View` → surface → straight-alpha RGBA, dirty-rect gated | **vellum** (`surface`, `runtime`) | pure plumbing with no product in it |
//! | the `evaluate_script` push/poll transport and its queue shim | **vellum** (`bridge`) | both games built the same thing around the one primitive Ultralight offers |
//! | pane registry, lifecycle, the outbound cap policy | **phoenix** ([`registry`]) | void-and-thunder has exactly one view and no lifecycle at all |
//! | session identity and its refusals | **phoenix** ([`identity`]) | there is no such concept in a single-player HUD |
//! | audience projection | **phoenix** ([`routing`]) | ditto |
//! | which of the host's inputs reach a page | **phoenix** ([`ultralight`]) | void-and-thunder deliberately withholds the keyboard; a console has form fields |
//! | the document, and what is injected into it | **phoenix** ([`document`]) | void-and-thunder `include_str!`s its own page; a pane loads the shipped client |
//!
//! # What is testable without an SDK, and why that matters
//!
//! Every module here except [`ultralight`] compiles and is tested with the
//! feature **off**, which is how every CI job in this repository builds — and
//! that is an arrangement rather than a fact: `--all-features` turns the feature
//! on, so `ci.yml`'s clippy step names its features explicitly. See
//! `Cargo.toml`'s `[features]`.
//!
//! The split is not an accident of packaging either: it is where the acceptance
//! criteria live. "A pane cannot use the host operator's token", "a pane cannot
//! read another pane's projection", "a `Welcome` is not lost while the document
//! is loading", "a pane's identity never appears in a served body" are all
//! claims about [`identity`], [`routing`], [`surface`] and [`document`], and a
//! claim that could only be checked on a Windows machine with a GPU is a claim
//! nobody checks.

pub mod document;
pub mod identity;
pub mod os_prefs;
pub mod recovery;
pub mod registry;
pub mod routing;
pub mod surface;
pub mod transport;

#[cfg(feature = "ultralight")]
pub mod ultralight;

use crate::delivery::serve::HostedDocuments;

use document::{
    build_pane_document, connectable_host_addr, mint_document_nonce, pane_document_path,
    DocumentError,
};

/// One pane the operator asked for, the identity it was given, and the
/// unguessable path segment its document is published under.
#[derive(Clone, Debug)]
pub struct OpenedPane {
    pub id: PaneId,
    pub identity: PaneIdentity,
    /// Per-pane, per-run, from [`mint_document_nonce`]. Known only to this
    /// process and to the URL its own view navigates to — see
    /// [`document::pane_document_path`].
    pub nonce: String,
}

/// The local Station panes one host process owns.
///
/// Assembled by `phoenix-host` from its `--pane <NAME>` flags **after** the
/// delivery listener has bound, because a pane's document is published at that
/// listener's own address and a `:0` bind does not know its port until then.
#[derive(Clone)]
pub struct LocalPanes {
    /// The shared registry every pane's traffic goes through.
    pub bus: PaneBus,
    /// The panes, in the order the operator named them.
    pub opened: Vec<OpenedPane>,
    /// `host:port` a pane's view should **connect** to.
    ///
    /// Normalised from the listener's bind address by
    /// [`document::connectable_host_addr`], because the documented default bind
    /// is `0.0.0.0:8080` and nothing can dial that.
    pub host_addr: String,
}

impl LocalPanes {
    /// Open one pane per participant name, each with a freshly minted ordinary
    /// session token and its own document nonce.
    ///
    /// Opening a pane creates no session and claims no station: a pane becomes a
    /// participant by sending `Identify` through the transport seam, like a
    /// phone, and takes a seat from inside its own console. That is the whole
    /// point — see [`identity`].
    ///
    /// `host_addr` is the listener's **bind** address, and is normalised here:
    /// see [`LocalPanes::host_addr`].
    pub fn open(names: &[String], host_addr: impl AsRef<str>) -> Self {
        let bus = PaneBus::default();
        let opened = names
            .iter()
            .map(|name| {
                let identity = PaneIdentity::mint(name.clone());
                let id = bus.open(identity.clone());
                OpenedPane {
                    id,
                    identity,
                    nonce: mint_document_nonce(),
                }
            })
            .collect();
        Self {
            bus,
            opened,
            host_addr: connectable_host_addr(host_addr.as_ref()),
        }
    }

    /// Every pane's handle, in order.
    pub fn ids(&self) -> Vec<PaneId> {
        self.opened.iter().map(|p| p.id).collect()
    }

    /// Every pane's handle paired with the URL its view should navigate to.
    ///
    /// The pairing exists because the URL now carries the pane's identity and
    /// its document nonce, so an id alone is no longer enough to build one —
    /// which is the point: nothing downstream can reconstruct a pane's URL
    /// without having been handed it.
    pub fn views(&self) -> Vec<(PaneId, String)> {
        self.opened
            .iter()
            .map(|p| {
                (
                    p.id,
                    document::pane_url(&self.host_addr, p.id, &p.nonce, &p.identity),
                )
            })
            .collect()
    }

    /// Publish each pane's document on the host's own HTTP surface.
    ///
    /// `client_index_html` is the served bundle's own `client/index.html`, read
    /// once: every pane document is that page plus two injected scripts, so the
    /// consoles a pane shows are byte-for-byte the ones a phone loads.
    ///
    /// The documents are **identical** for every pane — nothing about who a pane
    /// is lives in the body any more (see [`document`]) — and are still
    /// published one per pane, at one nonce'd path each, because a path is what
    /// gets withdrawn when a pane closes. That withdrawal is why the handle is
    /// handed to the bus here rather than kept: [`PaneBus::close`] is the one
    /// place that knows a pane has gone, whether it was closed by the operator,
    /// by a fault, or at shutdown.
    pub fn publish(
        &self,
        client_index_html: &str,
        documents: &HostedDocuments,
    ) -> Result<(), DocumentError> {
        // Read the host machine's OS accessibility preferences ONCE and seed
        // every pane's document with them (issue #1127), so a pane's private
        // profile initialises from the OS exactly as a browser's does from
        // matchMedia. The read is best-effort and machine-wide, not per-pane;
        // an explicit player choice in a pane still overrides it, and it never
        // rides the transport seam. Baked into `body` here so both the published
        // documents AND the recreation arming below carry it — a pane brought
        // back after a crash (#1125) sees the same OS layer it first loaded.
        let os_prefs = os_prefs::query_os_accessibility_prefs();
        let body = document::inject_os_accessibility_defaults(
            &build_pane_document(client_index_html)?,
            &os_prefs,
        );
        self.bus.attach_documents(documents.clone());
        // Arm recreation with the same body and host address every pane loaded
        // from, so a pane brought back after a crash (issue #1125) rebuilds an
        // identical document at a fresh nonce — the in-process analogue of a
        // reconnecting phone reloading the same client page.
        self.bus
            .arm_recreation(self.host_addr.clone(), body.clone());
        for pane in &self.opened {
            self.bus.publish_document(
                pane.id,
                pane_document_path(pane.id, &pane.nonce),
                body.clone(),
            );
        }
        Ok(())
    }

    /// Where each pane's view should navigate.
    pub fn urls(&self) -> Vec<String> {
        self.views().into_iter().map(|(_, url)| url).collect()
    }

    /// Every pane's handle, the URL its view navigates to, and the participant
    /// **label** it was opened under (the `--pane <NAME>` name).
    ///
    /// The label is what ties a pane to a bridge profile's Station pane slot,
    /// whose `label` is that same participant name (issue #1124): the pane host
    /// composites a `--pane Ada` onto whichever Station monitor the profile gives
    /// a pane labelled `Ada`. It is deliberately kept out of the URL — the URL
    /// carries the session token — so the association is made in-process, never
    /// served.
    pub fn display_entries(&self) -> Vec<(PaneId, String, String)> {
        self.opened
            .iter()
            .map(|p| {
                (
                    p.id,
                    document::pane_url(&self.host_addr, p.id, &p.nonce, &p.identity),
                    p.identity.name().to_string(),
                )
            })
            .collect()
    }
}

/// The pane bus, as a Bevy resource.
///
/// A newtype rather than `impl Resource for PaneBus`, so [`transport`] stays a
/// plain module a unit test can build without an `App`.
#[derive(bevy::prelude::Resource, Clone)]
pub struct PaneBusResource(pub PaneBus);

pub use identity::{IdentityRefusal, PaneIdentity};
pub use recovery::{service_faults, FaultOutcome, PaneFault};
pub use registry::{PaneDispatch, PaneId, PaneLifecycle, PaneRegistry};
pub use routing::pane_receives;
pub use surface::{pump_pane, PanePumpReport, PaneSurface, PaneSurfaceError, RecordingSurface};
pub use transport::{PaneBus, PaneInputRefusal, PaneTransport};
