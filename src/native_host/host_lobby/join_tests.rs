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
