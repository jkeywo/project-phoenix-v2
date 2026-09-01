//! What the lobby surface's join panel is told, and where its QR points
//! (issue #1329).
//!
//! # The one thing that is genuinely different from the browser host
//!
//! A browser host builds its join URL from `location.href`: the operator is
//! standing on the page the client bundle sits beside, so "where I am, plus
//! `client/index.html`, plus the code" is exactly where a phone should go
//! (`gui/join-url.js`).
//!
//! The native host has no such page. Its lobby surface is an embedded view
//! loaded from **`http://127.0.0.1:<port>/host-lobby-<nonce>.html`** — the
//! loopback address [`connectable_host_addr`] normalises the documented
//! `0.0.0.0` bind to, because that is what the view itself has to dial. It is
//! also the one URL in the building a phone cannot open. A QR built from the
//! surface's own `location.href` would encode a code nobody in the room can
//! scan, and it would look completely correct on the viewscreen.
//!
//! So the *page base* is decided here instead, from the address the delivery
//! listener actually bound, and pushed to the surface with the code. The URL is
//! still assembled by `gui/join-url.js` — one join-URL implementation, shared
//! with the browser host — this module only answers "beside which address".
//!
//! [`connectable_host_addr`]: crate::native_host::panes::document::connectable_host_addr
//!
//! # `0.0.0.0` is not an address, so one has to be found
//!
//! The documented default bind is `0.0.0.0:8080` — every interface — and the
//! whole point of it is that phones on the LAN can reach the bundle. Nothing
//! can *dial* it, though, so the host has to name one of those interfaces in
//! the QR, and the honest one is whichever the machine would route out of. See
//! [`discover_lan_addr`] for how that is asked and what it costs (nothing: no
//! packet is sent, no name is resolved).
//!
//! A bind that names a specific address is left exactly alone: an operator who
//! passed `--addr 192.168.1.5:8080` has already answered this question, and an
//! operator who passed `--addr 127.0.0.1:8080` has said "loopback only", which
//! is a host with no crew rather than a discovery problem.

use std::net::{IpAddr, UdpSocket};

use serde::Serialize;

use crate::core::rendezvous::JoinCode;

/// What the surface should put in its join panel.
///
/// Two states, not three: there is deliberately no "waiting for a code". A
/// browser host in that window shows an empty framed panel — the markup with
/// nothing written into it yet — and the native surface does the same by
/// simply not being pushed anything until the service answers.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JoinInvite {
    /// Nobody can join this host at all: `--solo`, or no `--rendezvous`.
    ///
    /// The surface says so in words. A framed, empty QR would invite a crew to
    /// stand in front of the viewscreen scanning something that can never work,
    /// and "the code has not arrived yet" and "there will never be a code" look
    /// identical on a wall.
    Off,
    /// The service issued this host a crew code.
    Code {
        /// The five letters a guest types. `JoinCode::suffix`.
        code: String,
        /// The structured code a QR carries. `JoinCode::full`.
        full: String,
        /// The URL the client bundle sits beside — see the module note.
        page_base: String,
        /// The rendezvous service this host registered with, verbatim.
        ///
        /// Passed through rather than compared here: `gui/join-url.js` owns the
        /// rule that only a NON-default service has to appear in the URL, and it
        /// owns it for the browser host too. Duplicating the comparison in Rust
        /// would be a second answer to a question with one right answer, and the
        /// service's URL is deliberately ONE literal (a deploy-time sweep can
        /// only find a literal), which is in that module.
        ///
        /// **A non-default service and a LAN phone do not combine**, and that is
        /// #1112's decision rather than this one's: the `?rendezvous=` override
        /// the URL then carries is honoured only for a LOOPBACK page origin, so
        /// a phone opening `http://192.168.…/client/…?rendezvous=…` ignores it
        /// and dials the built-in service. A host on a local `wrangler dev` is
        /// therefore reachable by the machine it runs on and by nothing else,
        /// with or without this QR. Nothing here papers over that; the code is
        /// carried honestly and the constraint lives where the gate does.
        rendezvous: Option<String>,
    },
}

impl JoinInvite {
    /// The invitation the relay's issued code makes.
    pub fn from_code(
        code: &JoinCode,
        page_base: impl Into<String>,
        rendezvous: Option<&str>,
    ) -> Self {
        JoinInvite::Code {
            code: code.suffix.clone(),
            full: code.full.clone(),
            page_base: page_base.into(),
            rendezvous: rendezvous.map(str::to_string),
        }
    }

    /// Encode for the bridge.
    ///
    /// Infallible in practice — every field is a `String` — and an encoding
    /// failure would mean a broken serde, so it degrades to the honest thing a
    /// surface can render rather than panicking on the simulation's thread.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| r#"{"kind":"off"}"#.to_string())
    }
}

/// `http://<addr>/` — the base `gui/join-url.js` hangs `client/index.html` off.
///
/// The trailing slash is load-bearing: that function takes a page *href* and
/// strips back to the last `/`, so `http://host:8080` (no slash) would lose the
/// host and produce `http://client/index.html`.
pub fn join_page_base(addr: &str) -> String {
    format!("http://{addr}/")
}

/// The `host:port` a **phone** should be sent to, given what the listener bound.
///
/// Pure, and separate from [`discover_lan_addr`] for the usual reason: the
/// decision is worth testing and the syscall is not.
///
/// * a specific bind is kept — the operator already answered this;
/// * a wildcard bind takes `discovered`, when there is one and it is routable;
/// * anything else falls back to loopback, which at least *works*, from the
///   host machine, and is visibly wrong rather than silently wrong.
pub fn shareable_host_addr(bound: &str, discovered: Option<IpAddr>) -> String {
    let Some((host, port)) = bound.rsplit_once(':') else {
        return bound.to_string();
    };
    let wildcard = matches!(host.trim(), "0.0.0.0" | "::" | "[::]" | "");
    if !wildcard {
        return bound.to_string();
    }
    match discovered {
        // A discovered loopback is not an answer to this question — it is the
        // same non-answer the fallback gives, arrived at more slowly.
        Some(ip) if !ip.is_loopback() => match ip {
            IpAddr::V4(_) => format!("{ip}:{port}"),
            // A bare IPv6 literal in a URL authority has to be bracketed, or
            // every colon in it reads as the port separator.
            IpAddr::V6(_) => format!("[{ip}]:{port}"),
        },
        _ => crate::native_host::panes::document::connectable_host_addr(bound),
    }
}

/// Which of this machine's addresses a phone on the LAN could reach it at.
///
/// The standard trick, and worth spelling out because it looks like it does
/// something it does not: a UDP socket is *connected* to a documentation
/// address and its local address read back. UDP `connect` is a routing-table
/// lookup with no handshake — **no packet leaves the machine**, nothing is
/// resolved by DNS, and the peer does not have to exist — so this answers "if I
/// were to speak to the outside world, which interface would I speak from",
/// which is the same interface the phones in the room are on.
///
/// `None` when there is no route at all (a machine with every interface down,
/// or a sandbox with no network stack). The caller falls back to loopback and
/// the QR then points somewhere only the host machine can open, which is the
/// truthful answer for a host nothing can reach.
///
/// `203.0.113.1` is TEST-NET-3 (RFC 5737), reserved for documentation and
/// routable to nobody — chosen over a real public resolver precisely so that
/// nothing here can be mistaken for, or turn into, a call home.
pub fn discover_lan_addr() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("203.0.113.1:9").ok()?;
    socket
        .local_addr()
        .ok()
        .map(|addr| addr.ip())
        .filter(|ip| !ip.is_unspecified())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn code() -> JoinCode {
        JoinCode {
            full: "PHX-1-ABCDE".to_string(),
            suffix: "ABCDE".to_string(),
            project: "phx".to_string(),
            version: "1".to_string(),
            namespace: "client".to_string(),
        }
    }

    #[test]
    fn a_wildcard_bind_is_answered_with_the_interface_a_phone_can_reach() {
        // The defect this exists to prevent: `0.0.0.0` normalises to loopback
        // for the embedded view that has to dial it, and a QR built from THAT
        // encodes the one address in the building no phone can open — while
        // looking perfectly correct on the viewscreen.
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5));
        assert_eq!(
            shareable_host_addr("0.0.0.0:8080", Some(lan)),
            "192.168.1.5:8080"
        );
        assert_eq!(
            shareable_host_addr("[::]:8080", Some(lan)),
            "192.168.1.5:8080"
        );
    }

    #[test]
    fn a_specific_bind_is_the_operators_own_answer_and_is_left_alone() {
        let lan = IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5));
        assert_eq!(
            shareable_host_addr("10.0.0.9:8080", Some(lan)),
            "10.0.0.9:8080"
        );
        // Including the one that means "no crew": a host bound to loopback is
        // not a discovery problem to be solved behind the operator's back.
        assert_eq!(
            shareable_host_addr("127.0.0.1:8080", Some(lan)),
            "127.0.0.1:8080"
        );
    }

    #[test]
    fn no_route_falls_back_to_loopback_rather_than_to_a_wildcard() {
        // `http://0.0.0.0:8080/` in a QR is not a worse address than loopback,
        // it is not an address. Loopback at least opens, on the host machine,
        // which is where somebody will be standing when they wonder why.
        assert_eq!(shareable_host_addr("0.0.0.0:8080", None), "127.0.0.1:8080");
        assert_eq!(shareable_host_addr("[::]:8080", None), "[::1]:8080");
        assert_eq!(
            shareable_host_addr("0.0.0.0:8080", Some(IpAddr::V4(Ipv4Addr::LOCALHOST))),
            "127.0.0.1:8080",
            "a discovered loopback is the same non-answer, found more slowly"
        );
    }

    #[test]
    fn an_ipv6_address_is_bracketed_because_a_url_authority_needs_it() {
        let ip = IpAddr::V6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 5));
        assert_eq!(shareable_host_addr("[::]:8080", Some(ip)), "[fd00::5]:8080");
    }

    #[test]
    fn the_page_base_keeps_the_trailing_slash_the_url_builder_needs() {
        // gui/join-url.js takes a page HREF and strips back to the last `/`.
        // Without the slash the host itself is stripped, and the QR encodes
        // `http://client/index.html#…`.
        assert_eq!(
            join_page_base("192.168.1.5:8080"),
            "http://192.168.1.5:8080/"
        );
        assert!(join_page_base("192.168.1.5:8080").ends_with('/'));
    }

    #[test]
    fn an_invitation_carries_the_letters_the_code_and_where_to_go() {
        let invite = JoinInvite::from_code(&code(), "http://192.168.1.5:8080/", None);
        let json = invite.to_json();
        assert!(json.contains(r#""kind":"code""#));
        assert!(json.contains(r#""code":"ABCDE""#));
        assert!(json.contains(r#""full":"PHX-1-ABCDE""#));
        assert!(json.contains(r#""page_base":"http://192.168.1.5:8080/""#));
        assert!(json.contains(r#""rendezvous":null"#));
    }

    #[test]
    fn a_non_default_service_is_passed_through_for_the_url_builder_to_judge() {
        // Whether it belongs in the URL is `gui/join-url.js`'s rule, and it is
        // the browser host's rule too. Deciding it twice is how the two hosts
        // start disagreeing about what a code means.
        let invite = JoinInvite::from_code(
            &code(),
            "http://192.168.1.5:8080/",
            Some("http://127.0.0.1:8788"),
        );
        assert!(invite
            .to_json()
            .contains(r#""rendezvous":"http://127.0.0.1:8788""#));
    }

    #[test]
    fn a_host_nobody_can_join_says_so_rather_than_offering_an_empty_code() {
        assert_eq!(JoinInvite::Off.to_json(), r#"{"kind":"off"}"#);
    }

    #[test]
    fn discovery_never_leaves_the_machine_and_never_returns_a_wildcard() {
        // The syscall itself, run once. It may legitimately answer `None` (a
        // build machine with no route), so the assertion is about what it must
        // never say rather than about what it says.
        if let Some(ip) = discover_lan_addr() {
            assert!(
                !ip.is_unspecified(),
                "0.0.0.0 is not an address to send a phone to"
            );
        }
    }
}
