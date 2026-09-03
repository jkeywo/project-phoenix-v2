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
    /// Nobody can join this host at all.
    ///
    /// Since issue #1353 that is a narrow condition rather than the default: a
    /// host serving a client bundle accepts LAN joins on its own port, so this
    /// is `--solo` (nobody is meant to join), or a host serving no bundle and
    /// given no `--rendezvous` (a phone would have no page to load and so no
    /// origin to dial).
    ///
    /// The surface says so in words. A framed, empty QR would invite a crew to
    /// stand in front of the viewscreen scanning something that can never work,
    /// and "the code has not arrived yet" and "there will never be a code" look
    /// identical on a wall.
    Off,
    /// The service issued this host a crew code.
    Code {
        /// The code a guest types. `JoinCode::suffix`.
        code: String,
        /// The structured code a QR carries. `JoinCode::full`.
        full: String,
        /// The URL the client bundle sits beside — see the module note.
        page_base: String,
        /// The rendezvous service this host registered with, verbatim — and
        /// `None` for the case that is now ordinary.
        ///
        /// **Issue #1353 made this field the exception rather than the rule.**
        /// A host that accepts LAN joins itself sends `None`, because the
        /// client's rule is that a page dials the origin that served it
        /// (`gui/join-url.js`'s `rendezvousBaseForOrigin`) — so the QR carries
        /// no service at all, which is both a shorter QR and no parameter for a
        /// link to point somewhere else. Everything below is the cloud-only
        /// host's remaining story.
        ///
        /// Passed through rather than compared here: `gui/join-url.js` owns the
        /// rule that only a NON-default service has to appear in the URL, and it
        /// owns it for the browser host too. Duplicating the comparison in Rust
        /// would be a second answer to a question with one right answer, and the
        /// service's URL is deliberately ONE literal (a deploy-time sweep can
        /// only find a literal), which is in that module.
        ///
        /// **A non-default service and a LAN phone do not combine**, and that
        /// is #1112's decision rather than this one's. The gate reads the
        /// PARAMETER's own host, not the page's origin
        /// (`rendezvousBaseFromLocation` in `gui/rendezvous-transport.js`), so
        /// it splits into two cases and only one of them looks like a failure:
        ///
        /// * a **loopback value** — `http://127.0.0.1:8788`, a `wrangler dev` —
        ///   IS honoured, from any page, including one served over the LAN. It
        ///   then resolves on the *phone*, where nothing is listening. Such a
        ///   host is reachable from the machine it runs on and from nowhere
        ///   else, with or without this QR.
        /// * a **non-loopback value** — a deployed worker, a staging URL — is
        ///   silently replaced with the client bundle's built-in service
        ///   ([`CLIENT_DEFAULT_RENDEZVOUS`]). The phone joins *something*; just
        ///   not the service the host that printed the code registered with,
        ///   and nothing on the wall says so.
        ///
        /// Nothing here papers over either: the code is carried honestly, the
        /// gate stays where it is, and [`phone_rendezvous`] is what lets
        /// `phoenix-host` say at the prompt which of the two an operator has
        /// walked into. Changing the gate is a security-posture call that
        /// belongs to #1112 and to a follow-up issue, not to this module.
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

/// What a phone in the room can do with the address the QR names.
///
/// `discover_lan_addr` answers "which interface would this machine route out
/// of", and on a laptop that is not always an interface the room is on: a VPN
/// (Tailscale hands out `100.64.0.0/10`), a network with no DHCP server
/// (`169.254.0.0/16`), or a machine with a public address on the default route
/// all produce a QR that is perfectly well-formed and unreachable from every
/// phone standing in front of it. Loopback was the only one `phoenix-host`
/// warned about; the rest looked exactly like success.
///
/// Only [`JoinAddrReach::Lan`] means "a phone on the room's Wi-Fi can open
/// this". Everything else has [`JoinAddrReach::unreachable_reason`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinAddrReach {
    /// A private address (RFC 1918, or an IPv6 unique-local `fc00::/7`) — or a
    /// name, which is the operator's own answer and not this module's to
    /// second-guess. The ordinary, working case.
    Lan,
    /// `127.0.0.0/8` or `::1`: opens on this machine and nowhere else. What a
    /// `--addr 127.0.0.1` bind asks for, and where a machine with no route
    /// falls back to.
    Loopback,
    /// `100.64.0.0/10`. Carrier-grade NAT by RFC 6598, and in a room almost
    /// always a VPN interface — Tailscale's `100.x` is the common one — which
    /// the default route prefers precisely because it is a tunnel.
    CarrierGrade,
    /// `169.254.0.0/16` or `fe80::/10`: link-local, which means nothing
    /// answered DHCP. There is a network, and it is one nobody else is on.
    LinkLocal,
    /// Routable and not private: a public address, or some other interface a
    /// phone on the room's Wi-Fi has no path to.
    NotPrivate,
}

impl JoinAddrReach {
    /// Why no phone in the room can open this address, or `None` if one can.
    ///
    /// The words rather than the format string, so the reason is unit-testable
    /// here and the recourse sentence is written once, at the prompt that adds
    /// it (`src/bin/phoenix_host.rs`). Operator text, not player text.
    pub fn unreachable_reason(self) -> Option<&'static str> {
        match self {
            JoinAddrReach::Lan => None,
            JoinAddrReach::Loopback => {
                Some("which is a loopback address, so no phone can open it.")
            }
            JoinAddrReach::CarrierGrade => Some(
                "which is a carrier-grade NAT address (100.64.0.0/10) — usually a VPN interface \
                 such as Tailscale — so a phone on the room's Wi-Fi has no path to it.",
            ),
            JoinAddrReach::LinkLocal => Some(
                "which is a link-local address (169.254.0.0/16 or fe80::/10), so nothing answered \
                 DHCP and no phone in the room is on that network.",
            ),
            JoinAddrReach::NotPrivate => Some(
                "which is not a private LAN address, so it is probably an interface the phones in \
                 the room are not on.",
            ),
        }
    }
}

/// Classify the base URL the QR is built from — see [`JoinAddrReach`].
///
/// Takes the *join base* rather than an `IpAddr` because that is what the
/// binary is holding by the time it can warn (`LocalHostLobby::join_base`), and
/// because the string is where a bracketed IPv6 authority has to be undone.
/// A base whose host is not an IP literal is [`JoinAddrReach::Lan`]: an
/// operator who bound a name has answered this question themselves.
pub fn join_addr_reach(join_base: &str) -> JoinAddrReach {
    let Some(host) = url_host(join_base) else {
        return JoinAddrReach::Lan;
    };
    let Ok(ip) = host.trim_start_matches('[').trim_end_matches(']').parse() else {
        return JoinAddrReach::Lan;
    };
    ip_reach(ip)
}

/// The classification itself, over a parsed address.
///
/// Written out rather than leaning on `std`, because the two ranges that matter
/// most here are the two `std` will not answer for on stable: `Ipv4Addr::is_shared`
/// (100.64/10) and `Ipv6Addr::is_unique_local` are both unstable.
fn ip_reach(ip: IpAddr) -> JoinAddrReach {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            if v4.is_loopback() {
                JoinAddrReach::Loopback
            } else if v4.is_link_local() {
                JoinAddrReach::LinkLocal
            } else if a == 100 && (64..=127).contains(&b) {
                JoinAddrReach::CarrierGrade
            } else if v4.is_private() {
                JoinAddrReach::Lan
            } else {
                JoinAddrReach::NotPrivate
            }
        }
        IpAddr::V6(v6) => {
            let head = v6.segments()[0];
            if v6.is_loopback() {
                JoinAddrReach::Loopback
            } else if head & 0xffc0 == 0xfe80 {
                JoinAddrReach::LinkLocal
            } else if head & 0xfe00 == 0xfc00 {
                JoinAddrReach::Lan
            } else {
                JoinAddrReach::NotPrivate
            }
        }
    }
}

/// The rendezvous service a client page dials when its URL names none.
///
/// A **mirror** of `DEV_RENDEZVOUS_URL` in `gui/join-url.js`, and deliberately
/// not a second source of truth: that module owns the literal, because a
/// deploy-time sweep can only find a literal and it rewrites `dist/` rather
/// than this checkout. The mirror exists for one reason — Rust cannot otherwise
/// tell an operator whether the `--rendezvous` they passed is one the phones
/// will honour — and `the_built_in_service_is_the_one_the_client_bundle_dials`
/// below reads the JS off disk, so the two cannot drift without failing here.
pub const CLIENT_DEFAULT_RENDEZVOUS: &str =
    "https://phoenix-rendezvous.project-phoenix.workers.dev";

/// What a scanning phone actually does with the `?rendezvous=` a QR carries.
///
/// See [`JoinInvite`]'s `rendezvous` field for the gate this reads off. Three
/// outcomes, and the operator can act on only the third — which is the one
/// with nothing on screen to show for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneRendezvous {
    /// The phone lands on the service this host registered with. Either
    /// `gui/join-url.js` put no override in the URL (the exact default), or it
    /// put one in that the gate discards in favour of the same service (the
    /// default spelled with a trailing slash). Both end in the right place.
    BuiltIn,
    /// A loopback service. The override IS honoured — the gate reads the
    /// value's host, not the page's — and resolves on the phone, where nothing
    /// is listening. A dev lever behaving exactly as documented.
    HonouredLoopback,
    /// Non-default and non-loopback: the override rides in the URL and the
    /// phone silently swaps it for [`CLIENT_DEFAULT_RENDEZVOUS`]. Every phone
    /// scanning the wall dials a service this host never registered with.
    SilentlyIgnored,
}

/// Classify a `--rendezvous` value — see [`PhoneRendezvous`].
///
/// A value that is not an `http(s)` URL is [`PhoneRendezvous::SilentlyIgnored`]
/// for the same reason a public one is: the client's gate falls back to the
/// built-in service for anything it cannot parse, so the phone ends up
/// somewhere other than where the host is. (Such a host does not usually get
/// this far — `relay_socket::host_socket_url` refuses it at the prompt — but
/// this classifies the value, not the process.)
pub fn phone_rendezvous(rendezvous: &str) -> PhoneRendezvous {
    let value = rendezvous.trim();
    if value.trim_end_matches('/') == CLIENT_DEFAULT_RENDEZVOUS.trim_end_matches('/') {
        return PhoneRendezvous::BuiltIn;
    }
    let scheme_ok = value.starts_with("http://") || value.starts_with("https://");
    match url_host(value) {
        Some(host) if scheme_ok && is_loopback_host(host) => PhoneRendezvous::HonouredLoopback,
        _ => PhoneRendezvous::SilentlyIgnored,
    }
}

/// The hostnames `isLoopbackHost` in `gui/rendezvous-transport.js` accepts.
///
/// Mirrored spelling for spelling, brackets included: `new URL('http://[::1]')`
/// keeps them in `hostname`, so both forms are listed there and both here.
fn is_loopback_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "localhost" || h == "127.0.0.1" || h == "[::1]" || h == "::1" || h.ends_with(".localhost")
}

/// The host of a `scheme://host[:port]/…` URL, port and brackets kept.
///
/// Enough of a URL parser for the two questions above and no more — this crate
/// has no `url` dependency, and `relay_socket::host_socket_url` splits on
/// `://` for the same reason.
fn url_host(url: &str) -> Option<&str> {
    let rest = url.trim().split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    // A bracketed IPv6 authority's colons are part of the address; only the
    // port after the closing bracket may be stripped.
    let host = match authority.rsplit_once(']') {
        Some((head, _)) => &authority[..head.len() + 1],
        None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
    };
    (!host.is_empty()).then_some(host)
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
///
/// # Two edges, both deliberate, both reported rather than solved
///
/// * **The probe is IPv4-only.** It binds `0.0.0.0:0`, so it can only ever name
///   an IPv4 interface. A host bound to `[::]` therefore gets an IPv4 answer
///   for a v6 listener — usually right, because such a bind is dual-stack, but
///   not on a platform where `IPV6_V6ONLY` defaults on — and a host with only
///   IPv6 routes gets `None` and falls back to `[::1]`.
/// * **The default route is not always the room.** A VPN, a link-local
///   interface or a public address answers here exactly as a LAN address does.
///   [`join_addr_reach`] classifies what came back and `phoenix-host` says so
///   at the prompt, with `--addr` as the recourse in every case.
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
    fn only_a_private_address_is_one_a_phone_in_the_room_can_open() {
        // The classification the boot warning branches on. Before this, only
        // loopback was warned about — a Tailscale `100.x`, an address from a
        // network with no DHCP, or a public interface on the default route all
        // produced a QR that looked exactly like success and could not be
        // scanned by anybody standing in front of it.
        for lan in [
            "http://192.168.1.5:8080/",
            "http://10.0.0.9:8080/",
            "http://172.16.4.4:8080/",
            "http://[fd00::5]:8080/",
        ] {
            assert_eq!(join_addr_reach(lan), JoinAddrReach::Lan, "{lan}");
            assert_eq!(join_addr_reach(lan).unreachable_reason(), None, "{lan}");
        }
        for (base, expected) in [
            ("http://127.0.0.1:8080/", JoinAddrReach::Loopback),
            ("http://[::1]:8080/", JoinAddrReach::Loopback),
            ("http://100.101.102.103:8080/", JoinAddrReach::CarrierGrade),
            ("http://100.64.0.1:8080/", JoinAddrReach::CarrierGrade),
            ("http://169.254.13.9:8080/", JoinAddrReach::LinkLocal),
            ("http://[fe80::1]:8080/", JoinAddrReach::LinkLocal),
            ("http://203.0.113.7:8080/", JoinAddrReach::NotPrivate),
            ("http://[2001:db8::1]:8080/", JoinAddrReach::NotPrivate),
        ] {
            assert_eq!(join_addr_reach(base), expected, "{base}");
            assert!(
                join_addr_reach(base).unreachable_reason().is_some(),
                "{base} must be warned about at the prompt"
            );
        }
        // 100.128.x is OUTSIDE 100.64/10 and is ordinary public space; a /10
        // read as a /8 would have swallowed it.
        assert_eq!(
            join_addr_reach("http://100.128.0.1:8080/"),
            JoinAddrReach::NotPrivate
        );
        // A name is the operator's own answer to "which interface", and this
        // module does not second-guess one it cannot classify.
        assert_eq!(
            join_addr_reach("http://host.local:8080/"),
            JoinAddrReach::Lan
        );
        assert_eq!(join_addr_reach("not a url"), JoinAddrReach::Lan);
    }

    #[test]
    fn a_deployed_service_in_the_qr_is_the_one_a_phone_silently_discards() {
        // Issue #1329's F1. The `?rendezvous=` gate reads the PARAMETER's host,
        // not the page's origin, so a non-loopback override rides in the QR and
        // is swapped for the built-in service on arrival — every scanned phone
        // dials a service this host never registered with, with nothing on the
        // wall saying so. This is the predicate `phoenix-host` warns on.
        assert_eq!(
            phone_rendezvous("https://phoenix-rendezvous-demo.example.workers.dev"),
            PhoneRendezvous::SilentlyIgnored
        );
        assert_eq!(
            phone_rendezvous("https://staging.kiwigamedesign.co.uk"),
            PhoneRendezvous::SilentlyIgnored
        );
        // `localhost.attacker.example` is not a loopback host, and the mirrored
        // spelling of the client's own check is what keeps that true here.
        assert_eq!(
            phone_rendezvous("https://localhost.attacker.example"),
            PhoneRendezvous::SilentlyIgnored
        );

        // …and the two the warning must stay quiet about.
        assert_eq!(
            phone_rendezvous(CLIENT_DEFAULT_RENDEZVOUS),
            PhoneRendezvous::BuiltIn
        );
        assert_eq!(
            phone_rendezvous(&format!("{CLIENT_DEFAULT_RENDEZVOUS}/")),
            PhoneRendezvous::BuiltIn,
            "a trailing slash is the same service, not a second one"
        );
        for dev in [
            "http://127.0.0.1:8788",
            "http://localhost:8787/",
            "http://[::1]:8787",
            "http://phoenix.localhost:8787",
        ] {
            assert_eq!(
                phone_rendezvous(dev),
                PhoneRendezvous::HonouredLoopback,
                "{dev}"
            );
        }
    }

    #[test]
    fn the_built_in_service_is_the_one_the_client_bundle_dials() {
        // The mirror's only justification: `gui/join-url.js` owns the literal,
        // and this const is worth having ONLY while it says the same thing. A
        // deploy sweep rewrites `dist/`, never this checkout, so a difference
        // here is drift rather than a deployment.
        let js = std::fs::read_to_string("gui/join-url.js")
            .expect("the client's join-URL module is checked in");
        assert!(
            js.contains(&format!(
                "export const DEV_RENDEZVOUS_URL = '{CLIENT_DEFAULT_RENDEZVOUS}'"
            )),
            "CLIENT_DEFAULT_RENDEZVOUS must mirror DEV_RENDEZVOUS_URL in gui/join-url.js"
        );
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
