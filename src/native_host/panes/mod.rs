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
//! feature **off**, which is how every CI job in this repository builds. That is
//! not an accident of packaging: it is where the acceptance criteria live. "A
//! pane cannot use the host operator's token", "a pane cannot read another
//! pane's projection", "a `Welcome` is not lost while the document is loading"
//! are all claims about [`identity`], [`routing`] and [`surface`], and a claim
//! that could only be checked on a Windows machine with a GPU is a claim nobody
//! checks.

pub mod document;
pub mod identity;
pub mod registry;
pub mod routing;
pub mod surface;
pub mod transport;

#[cfg(feature = "ultralight")]
pub mod ultralight;

use crate::delivery::serve::HostedDocuments;

use document::{build_pane_document, pane_document_path, DocumentError};

/// One pane the operator asked for, and the identity it was given.
#[derive(Clone, Debug)]
pub struct OpenedPane {
    pub id: PaneId,
    pub identity: PaneIdentity,
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
    /// `host:port` this process's own delivery server is bound to.
    pub host_addr: String,
}

impl LocalPanes {
    /// Open one pane per participant name, each with a freshly minted ordinary
    /// session token.
    ///
    /// Opening a pane creates no session and claims no station: a pane becomes a
    /// participant by sending `Identify` through the transport seam, like a
    /// phone, and takes a seat from inside its own console. That is the whole
    /// point — see [`identity`].
    pub fn open(names: &[String], host_addr: impl Into<String>) -> Self {
        let bus = PaneBus::default();
        let opened = names
            .iter()
            .map(|name| {
                let identity = PaneIdentity::mint(name.clone());
                let id = bus.open(identity.clone());
                OpenedPane { id, identity }
            })
            .collect();
        Self {
            bus,
            opened,
            host_addr: host_addr.into(),
        }
    }

    /// Every pane's handle, in order.
    pub fn ids(&self) -> Vec<PaneId> {
        self.opened.iter().map(|p| p.id).collect()
    }

    /// Publish each pane's document on the host's own HTTP surface.
    ///
    /// `client_index_html` is the served bundle's own `client/index.html`, read
    /// once: every pane document is that page plus three injected lines, so the
    /// consoles a pane shows are byte-for-byte the ones a phone loads.
    pub fn publish(
        &self,
        client_index_html: &str,
        documents: &HostedDocuments,
    ) -> Result<(), DocumentError> {
        for pane in &self.opened {
            documents.publish(
                pane_document_path(pane.id),
                build_pane_document(client_index_html, &pane.identity)?,
            );
        }
        Ok(())
    }

    /// Where each pane's view should navigate.
    pub fn urls(&self) -> Vec<String> {
        self.opened
            .iter()
            .map(|p| document::pane_url(&self.host_addr, p.id))
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
pub use registry::{PaneDispatch, PaneId, PaneLifecycle, PaneRegistry};
pub use routing::pane_receives;
pub use surface::{pump_pane, PanePumpReport, PaneSurface, PaneSurfaceError, RecordingSurface};
pub use transport::{PaneBus, PaneInputRefusal, PaneTransport};
