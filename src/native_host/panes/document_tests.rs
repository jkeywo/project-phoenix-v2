use super::*;

/// The shape of the real page, reduced to the four things the assembly
/// depends on: a `<head>`, the PeerJS tag, the click-sound `<audio>`, and a
/// `</body>`.
///
/// The *real* page is asserted against too — see
/// [`a_pane_document_assembles_from_the_repositorys_own_client_page`], which
/// reads `client.html` off disk. This stub is for the cases that need a
/// document shaped a particular way (no `<head>`, no `</body>`) and for
/// asserting that nothing but those two elements is disturbed.
const CLIENT: &str = "<!doctype html>\n<html>\n<head>\n\
         <script src=\"gui/bg-raf-keepalive.js\"></script>\n\
         <script src=\"https://unpkg.com/peerjs@1.5.4/dist/peerjs.min.js\"></script>\n\
         <script type=\"module\" src=\"gui/connection-manager.js\"></script>\n\
         <script type=\"module\" src=\"gui/rendezvous-transport.js\"></script>\n\
         </head>\n<body>\n<div id=\"app\"></div>\n\
         <audio id=\"ui-click\" src=\"assets/sounds/ui_click.ogg\"></audio>\n\
         <script>var myName = 'x';</script>\n\
         </body>\n</html>\n";

/// The link script's own import, used as the marker for "the link is here".
/// The relative path only resolves because a pane document is published at
/// the client directory's own depth, which is the whole reason for it.
const PANE_LINK_IMPORT: &str = "from './gui/rendezvous-transport.js'";

fn identity() -> PaneIdentity {
    PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap()
}

/// How many `<script>` elements in `html` load PeerJS.
fn peerjs_script_tags(html: &str) -> usize {
    html.match_indices("<script")
        .filter(|(start, _)| {
            html[*start..]
                .find('>')
                .map(|end| {
                    html[*start..*start + end]
                        .to_ascii_lowercase()
                        .contains("peerjs")
                })
                .unwrap_or(false)
        })
        .count()
}

#[test]
fn the_boot_script_runs_before_the_pages_own_scripts() {
    // Two reasons, both load-bearing. It reads the session token and the
    // player name out of the fragment, which the page's inline script wants
    // at parse time — one script too late and the pane joins under a random
    // name on a token nothing else knows. And it replaces
    // `requestAnimationFrame`, which `gui/bg-raf-keepalive.js` — the very
    // first script the page loads — captures once and then delegates to.
    let html = build_pane_document(CLIENT).unwrap();
    let boot = html.find("__phoenixPane").unwrap();
    let raf = html.find("window.requestAnimationFrame =").unwrap();
    // The TAG, not the name: the boot script's own comment says why it has
    // to precede that module, so a bare name matches inside the prose.
    let first_page_script = html.find("src=\"gui/bg-raf-keepalive.js\"").unwrap();
    assert!(
        boot < first_page_script,
        "the boot script must be the first script in the document"
    );
    assert!(
        raf < first_page_script,
        "bg-raf-keepalive.js captures whatever rAF it finds; a pane's has to be there first"
    );
    assert!(html.find("<head>").unwrap() < boot);
}

#[test]
fn the_link_script_runs_after_the_pages_transport_module() {
    // Module scripts evaluate in document order, and the link IMPORTS the
    // channel labels from gui/rendezvous-transport.js — the same module
    // instance the page loads, which has to be in the graph first.
    let html = build_pane_document(CLIENT).unwrap();
    let transport = html.find("src=\"gui/rendezvous-transport.js\"").unwrap();
    let link = html.find(PANE_LINK_IMPORT).unwrap();
    assert!(transport < link);
    assert!(link < html.rfind("</body>").unwrap());
}

#[test]
fn the_link_script_installs_the_transport_factories_and_nothing_else() {
    // The seam it attaches through, named here so a rewrite that quietly
    // went back to publishing a global of its own fails. `currentLink()`
    // reads a closure `startPhoenixJoin` assigns, so a pane-owned link
    // object is unreachable without editing client.html; the factories are
    // the override the transport module documents for exactly this.
    let html = build_pane_document(CLIENT).unwrap();
    assert!(html.contains("window.PhoenixTransportFactories ="));
    assert!(
        !html.contains("window.connectionManager ="),
        "that global went with PeerJS in #1112; assigning it reaches nothing"
    );
}

#[test]
fn the_peerjs_tag_is_dropped_and_nothing_else_is() {
    // A pane never uses PeerJS, and a bridge machine may not be able to
    // reach a CDN at all — waiting for that fetch to time out is boot
    // latency for nothing.
    let html = build_pane_document(CLIENT).unwrap();
    assert_eq!(peerjs_script_tags(&html), 0);
    assert!(html.contains("gui/bg-raf-keepalive.js"));
    assert!(html.contains("gui/connection-manager.js"));
    assert!(html.contains("var myName = 'x';"));
    assert!(html.contains("<div id=\"app\"></div>"));
}

#[test]
fn the_click_sound_is_dropped_because_playing_it_would_eat_the_command() {
    // Not a nicety. Ultralight has no media backend — `el.play` is
    // undefined — and `client.html`'s `send()` calls `playClick()` BEFORE
    // `link.send(...)`. The TypeError propagates out of the
    // `console_action` listener and the command never reaches the
    // transport. `NO_CLICK` exempts SetThrust/SetSteering/Identify/SetName,
    // so a pane joins, renames itself and flies — and every deliberate
    // console command is silently dropped.
    //
    // With no element, `playClick`'s own `if (!el) return` is the path.
    let html = build_pane_document(CLIENT).unwrap();
    assert!(!html.contains("<audio"));
    assert!(!html.contains("ui_click.ogg"));
    assert!(html.contains("<div id=\"app\"></div>"));
    assert!(html.contains("var myName = 'x';"));
}

#[test]
fn a_peerjs_tag_from_anywhere_but_unpkg_is_dropped_too() {
    // The marker used to be the literal CDN host, so self-hosting the
    // library or moving CDN would have made this strip a silent no-op.
    let page = "<html><head>\
             <script SRC=\"gui/vendor/PeerJS.min.js\"></script>\
             </head><body></body></html>";
    assert_eq!(peerjs_script_tags(&build_pane_document(page).unwrap()), 0);
}

#[test]
fn a_panes_identity_never_appears_in_the_document_the_host_serves() {
    // The finding this whole arrangement answers: `phoenix-host` binds
    // 0.0.0.0 with no TLS and no auth, so a session token in a served body
    // is a seat on the bridge available to anyone on the LAN. It is in the
    // URL FRAGMENT, which a browser never transmits and this host never
    // writes.
    //
    // The name is deliberately hostile on two axes at once: `</script>`
    // would close the inline script element it used to be interpolated
    // into (no JavaScript string escape prevents that — an HTML script
    // element ends at the first `</script` regardless of quoting), and the
    // apostrophe would close the literal itself.
    let awkward = PaneIdentity::adopt("tok'en</script>", "</script><h1>O'Neil_x").unwrap();
    let html = build_pane_document(CLIENT).unwrap();
    assert!(!html.contains(awkward.token()));
    assert!(!html.contains(awkward.name()));
    assert!(!html.contains("<h1>"));
    assert!(
        !html.contains("__phoenixPaneIdentity"),
        "no identity is embedded at all, so there is nothing to escape"
    );

    // And it does reach the page — by the one route that costs nothing on
    // the wire.
    let url = pane_url("127.0.0.1:8080", PaneId(0), "n0nce", &awkward);
    let fragment = url.split_once('#').unwrap().1;
    assert!(fragment.contains(&fragment_encode(awkward.token())));
    assert!(fragment.contains(&fragment_encode(awkward.name())));
    assert!(!fragment.contains('<'));
    // The fragment is a `&`-separated list of `key=value` pairs, so a value
    // that carried either separator would swallow the field after it — or,
    // in the first field's case, look like the join code.
    assert_eq!(
        fragment.split('&').count(),
        3,
        "one code and two pairs, whatever the participant is called: {fragment}"
    );
    assert!(fragment.starts_with(&format!("{PANE_JOIN_CODE}&")));
}

#[test]
fn the_page_to_host_queue_is_installed_under_the_namespace_the_host_drains() {
    let html = build_pane_document(CLIENT).unwrap();
    assert!(html.contains("window.phoenixPaneOut.send"));
    assert!(html.contains("window.__phoenixPaneOutDrain"));
    assert_eq!(pane_drain_script(), "window.__phoenixPaneOutDrain()");
}

#[test]
fn a_pushed_message_is_escaped_into_the_apply_call() {
    assert_eq!(
        pane_apply_script(r#"{"type":"Welcome","data":{"name":"O'Neil"}}"#),
        r#"window.__phoenixPaneApply('{"type":"Welcome","data":{"name":"O\'Neil"}}')"#
    );
}

#[test]
fn a_document_with_nowhere_to_inject_is_refused_by_name() {
    assert_eq!(
        build_pane_document("<html><body></body></html>"),
        Err(DocumentError::NoHead)
    );
    assert_eq!(
        build_pane_document("<html><head></head></html>"),
        Err(DocumentError::NoBody)
    );
}

#[test]
fn a_panes_document_sits_at_the_client_directorys_own_depth() {
    // This is what makes every relative URL in the page — gui modules,
    // stylesheets, console iframes — resolve exactly as they do for a
    // phone, with no <base> tag and no rewriting.
    assert_eq!(
        pane_document_path(PaneId(0), "abcd"),
        "/client/pane-0-abcd.html"
    );
    assert_eq!(
        pane_url("127.0.0.1:8080", PaneId(3), "abcd", &identity()),
        "http://127.0.0.1:8080/client/pane-3-abcd.html\
             #PANES&token=3f1a6c2e-0a11-4b3c-9d55-000000000001&name=Ada"
    );
}

#[test]
fn two_panes_document_paths_cannot_be_guessed_from_each_other() {
    // `/client/pane-0.html` was one guess away from a live participant's
    // page. The nonce is per pane, so knowing one path tells you nothing
    // about the next.
    let a = mint_document_nonce();
    let b = mint_document_nonce();
    assert_ne!(a, b);
    assert!(a.len() >= 32, "a guessable nonce is not a nonce: {a}");
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    assert_ne!(
        pane_document_path(PaneId(0), &a),
        pane_document_path(PaneId(0), &b)
    );
}

#[test]
fn the_fragment_leads_with_a_join_code_the_page_can_actually_resolve() {
    // The two ways this page load can fail before a pane ever speaks:
    // an EMPTY fragment is `route: 'entry'`, which shows the join field and
    // stops; a fragment `parseJoinCode` refuses is `route: 'rendezvous'`
    // and then the same field with a reason on it. The code has to come
    // first, on its own, with no `=` in it — that is how PANE_BOOT_JS tells
    // it apart from the identity pairs, and it is what gets left in
    // `location.hash` for the page to read.
    let url = pane_url("127.0.0.1:8080", PaneId(0), "abcd", &identity());
    let fragment = url.split('#').nth(1).unwrap();
    assert!(!fragment.is_empty());
    let code = fragment.split('&').next().unwrap();
    assert_eq!(code, PANE_JOIN_CODE);
    assert!(!code.contains('='));
}

#[test]
fn the_join_code_is_one_the_authored_table_accepts() {
    // The Rust half of a claim `tests/client/pane-scripts.test.js` makes
    // against the real table: the authored length, all in the authored
    // alphabet,
    // none of them the confusables the canonicaliser rewrites. A code that
    // did not round-trip would compose into an identifier the page then
    // refuses, and the console would come up behind the join overlay.
    const ALPHABET: &str = "ABCDEFGHIJKMNOPQRSTUVWXYZ";
    assert_eq!(PANE_JOIN_CODE.len(), 5);
    assert!(
        PANE_JOIN_CODE.chars().all(|c| ALPHABET.contains(c)),
        "{PANE_JOIN_CODE} is not spelled in assets/join/join-codes.toml's alphabet"
    );
}

#[test]
fn a_wildcard_bind_becomes_a_loopback_url_because_nothing_can_dial_0_0_0_0() {
    // `--addr 0.0.0.0:8080` is the DOCUMENTED default, so this is the
    // ordinary invocation rather than an edge case: 0.0.0.0 means "every
    // interface" to `bind` and nothing at all to `connect`. A pane always
    // connects to its own process.
    assert_eq!(connectable_host_addr("0.0.0.0:8080"), "127.0.0.1:8080");
    assert_eq!(connectable_host_addr("[::]:8080"), "[::1]:8080");
    assert_eq!(connectable_host_addr(":::8080"), "[::1]:8080");
    // The port a `:0` bind was actually given is kept, which is the whole
    // point of taking the address from `local_addr()` rather than `--addr`.
    assert_eq!(connectable_host_addr("0.0.0.0:52341"), "127.0.0.1:52341");
    // A specific bind is left exactly alone.
    assert_eq!(
        connectable_host_addr("192.168.1.5:8080"),
        "192.168.1.5:8080"
    );
    assert_eq!(connectable_host_addr("127.0.0.1:8080"), "127.0.0.1:8080");
    assert_eq!(connectable_host_addr("[::1]:8080"), "[::1]:8080");
}

#[test]
fn the_fragment_encoding_escapes_everything_that_is_not_unreserved() {
    // The three that matter are the fragment's own grammar — `&` separates
    // fields, `=` separates a key from its value, `%` is the escape — and
    // escaping the whole of the rest is the rule with no edge cases.
    assert_eq!(fragment_encode("a&b=c%d"), "a%26b%3Dc%25d");
    assert_eq!(fragment_encode("O'Neil"), "O%27Neil");
    assert_eq!(fragment_encode("a b"), "a%20b");
    assert_eq!(fragment_encode("</script>"), "%3C%2Fscript%3E");
    // The whole unreserved set survives, the underscore now included: it
    // was escaped for a routing reason that no longer exists (see
    // `fragment_encode`), and `PANE_BOOT_JS` strips the identity out of the
    // fragment before the page's route is decided at all.
    assert_eq!(fragment_encode("ada_lovelace"), "ada_lovelace");
    assert_eq!(fragment_encode("Ada-1.0~x_y"), "Ada-1.0~x_y");
    // Non-ASCII goes out as UTF-8 bytes, which decodeURIComponent restores.
    assert_eq!(fragment_encode("é"), "%C3%A9");
}

#[test]
fn a_frame_interval_is_injected_ahead_of_the_boot_script_and_nowhere_by_default() {
    // The `raf33` frame experiment: the boot script reads
    // `window.PhoenixPaneFrameMs` once, when it installs its rAF timer, so
    // the assignment has to come first — and a document built with no
    // options is byte-for-byte the one `build_pane_document` builds.
    // `pane_boot.js` itself names the global it reads, so the absence being
    // asserted is the ASSIGNMENT, which only the host injects.
    let plain = build_pane_document(CLIENT).unwrap();
    assert!(!plain.contains("PhoenixPaneFrameMs = "));
    assert_eq!(
        plain,
        build_pane_document_with(CLIENT, &PaneDocumentOptions::default()).unwrap()
    );

    let slow =
        build_pane_document_with(CLIENT, &PaneDocumentOptions { frame_ms: Some(33) }).unwrap();
    let set = slow
        .find("window.PhoenixPaneFrameMs = 33;")
        .expect("the interval is set");
    let boot = slow.find("__phoenixPane").expect("the boot script follows");
    assert!(
        set < boot,
        "the interval must be set before pane_boot.js reads it"
    );
    assert_eq!(slow.matches("PhoenixPaneFrameMs = ").count(), 1);
}

#[test]
fn a_pane_document_assembles_from_the_repositorys_own_client_page() {
    // Every other test here runs against a ten-line stub, so nothing
    // checked the assembly against the page it actually operates on. The
    // repository's `client.html` is checked in, so this needs no build
    // step — and `scripts/build-client.mjs` copies it to
    // `dist/client/index.html` without touching its script tags.
    let client = std::fs::read_to_string("client.html")
        .expect("the repository's own client page is checked in");
    let html = build_pane_document(&client).expect("the real page becomes a pane document");
    // The strips run over the real page, and both look for an opening tag
    // and then its closing one. A page that put `<audio` or `<script` in a
    // comment or a string literal could make one of them swallow the span
    // between — so bound the damage before asserting anything finer.
    assert!(
        html.len() > client.len(),
        "the two injected scripts are far bigger than the two stripped elements; a \
             shorter document means a strip ran away"
    );

    let boot = html
        .find("__phoenixPane")
        .expect("the boot script is injected");
    let first_gui_script = html
        .find("src=\"gui/")
        .expect("the real page loads gui/ modules");
    assert!(
        boot < first_gui_script,
        "the boot script must precede every gui/ module the page loads"
    );

    let transport = html
        .find("src=\"gui/rendezvous-transport.js\"")
        .expect("the real page loads the crew transport");
    let link = html
        .find(PANE_LINK_IMPORT)
        .expect("the link script is injected");
    assert!(
        transport < link,
        "the link imports the transport's channel labels; it must not run first"
    );
    assert!(link < html.rfind("</body>").unwrap());
}

#[test]
fn a_pane_document_of_the_repositorys_own_client_page_carries_no_peerjs_script() {
    // Belt and braces, and honest in both states: today `client.html`
    // carries the CDN tag and this strips it; issue #1112 deletes the tag
    // outright, at which point this strips nothing and the assertion is
    // still exactly the claim being made — a pane never loads PeerJS.
    let client = std::fs::read_to_string("client.html").unwrap();
    let html = build_pane_document(&client).unwrap();
    assert_eq!(
        peerjs_script_tags(&html),
        0,
        "no <script> element in a pane document may load PeerJS"
    );
    // The page's own prose and its inline fallback still MENTION PeerJS,
    // and must: this strips a script element, it does not rewrite the
    // client.
    assert!(html.contains("gui/connection-manager.js"));
}

#[test]
fn the_repositorys_client_uses_the_explicit_native_private_audio_provider() {
    let client = std::fs::read_to_string("client.html").unwrap();
    let html = build_pane_document(&client).unwrap();
    let provider = "root.PhoenixPrivateAudioProvider = function";
    let module = "src=\"gui/private-audio.js\"";
    let mount = "window.PrivateAudio.createPrivateAudio(";
    assert!(!client.contains(provider));
    assert_eq!(html.matches(provider).count(), 1);
    assert_eq!(client.matches(module).count(), 1);
    assert_eq!(html.matches(module).count(), 1);
    assert_eq!(client.matches(mount).count(), 1);
    assert_eq!(html.matches(mount).count(), 1);
    assert!(html.find(provider).unwrap() < html.find(module).unwrap());
    assert!(html.find("surface: 'native-pane'").unwrap() < html.find(module).unwrap());
    assert!(!html.contains("<audio"));
    assert!(!html.contains("gui/host-audio.js"));
}

#[test]
fn a_native_pane_declares_profile_capabilities_before_the_shared_adapter() {
    // #1280 does not create a native profile, input sampler or
    // Accessibility apply path. The first injected script declares only
    // capability facts; the repository's ordinary profile and surface
    // adapter then consume them. Keyboard still arrives through #1124 and
    // OS defaults are still injected separately by #1127 below.
    let client = std::fs::read_to_string("client.html").unwrap();
    let html = build_pane_document(&client).unwrap();
    let declaration = html
        .find("window.PhoenixOperatorCapabilities =")
        .expect("pane boot declares its operator capabilities");
    let profile = html
        .find("src=\"gui/operator-profile.js\"")
        .expect("the ordinary versioned profile remains the schema owner");
    let adapter = html
        .find("src=\"gui/operator-surface-adapter.js\"")
        .expect("the shared surface adapter remains in the pane document");
    assert!(declaration < profile);
    assert!(profile < adapter);
    assert!(html.contains("surface: 'native-pane'"));
    // The native pane now DOES support a gamepad: the host feeds one from
    // gilrs and the boot script installs a `navigator.getGamepads()` shim, so
    // the ordinary client runtime samples it. Vibration still has no native
    // backend and stays off.
    assert!(html.contains("gamepad: true"));
    assert!(
        html.contains("navigator.getGamepads"),
        "the native pane installs the host-fed gamepad shim so the runtime can sample it"
    );
    assert!(html.contains("vibration: false"));
    assert_eq!(
        html.matches("src=\"gui/operator-profile.js\"").count(),
        1,
        "a pane must not gain a competing native profile"
    );
}

// ── OS accessibility default injection (issue #1127) ─────────────────────

#[test]
fn os_accessibility_defaults_are_injected_before_the_pages_own_scripts() {
    use super::super::os_prefs::OsAccessibilityPrefs;
    // The page reads them at resolve time through `osAccessibilityDefaults`,
    // which runs inside the boot the first `gui/` module drives — so the
    // assignment has to be in <head>, before any gui/ script.
    let html = build_pane_document(CLIENT).unwrap();
    let seeded = inject_os_accessibility_defaults(
        &html,
        &OsAccessibilityPrefs {
            reduced_motion: true,
            high_contrast: true,
            text_scale: 1.25,
            ..Default::default()
        },
    );
    let assign = seeded
        .find("window.PhoenixOsAccessibilityDefaults =")
        .expect("the OS default layer is injected");
    let first_gui = seeded
        .find("src=\"gui/")
        .expect("the page still loads its gui/ modules");
    assert!(
        assign < first_gui,
        "the OS defaults must be seeded before any gui/ module reads them"
    );
    assert!(seeded.find("<head>").unwrap() < assign);
    // The exact values the page overlays: reducedMotion + contrast (from
    // high-contrast) true, and the imported text scale.
    assert!(seeded.contains("\"reducedMotion\":true"));
    assert!(seeded.contains("\"contrast\":true"));
    assert!(seeded.contains("\"textScale\":1.25"));
    // Injection adds only the one script; nothing the page owns is lost.
    assert!(seeded.contains("<div id=\"app\"></div>"));
    assert!(seeded.contains("var myName = 'x';"));
}

#[test]
fn os_accessibility_defaults_of_a_quiet_machine_match_a_silent_browser() {
    use super::super::os_prefs::OsAccessibilityPrefs;
    // A machine with nothing set injects the SAME default layer a browser
    // computes from a silent matchMedia, so a pane and a phone on that
    // machine start from the identical baseline (AC2/AC5 equivalence).
    let seeded = inject_os_accessibility_defaults(CLIENT, &OsAccessibilityPrefs::default());
    assert!(seeded.contains("\"reducedMotion\":false"));
    assert!(seeded.contains("\"contrast\":false"));
    assert!(seeded.contains("\"textScale\":1"));
}

#[test]
fn a_document_with_no_head_keeps_its_os_layer_absent_rather_than_failing() {
    use super::super::os_prefs::OsAccessibilityPrefs;
    // No <head> means no OS layer, which is exactly a browser with no
    // matchMedia — the page falls back to "no preference", not an error.
    let no_head = "<html><body></body></html>";
    assert_eq!(
        inject_os_accessibility_defaults(no_head, &OsAccessibilityPrefs::default()),
        no_head
    );
}

#[test]
fn the_injected_os_layer_never_carries_a_setting_the_seam_could_leak() {
    use super::super::os_prefs::OsAccessibilityPrefs;
    // AC4 at the document: the injected layer is machine OS defaults, not a
    // player's private profile. It names the three matchMedia-shaped keys
    // and nothing resembling a stored setting, diagnosis or assistance
    // request — the page keeps the profile client-local and only the
    // anonymous ineligible-station set ever crosses the seam.
    let seeded = inject_os_accessibility_defaults(
        CLIENT,
        &OsAccessibilityPrefs {
            reduced_motion: true,
            high_contrast: true,
            text_scale: 1.5,
            ..Default::default()
        },
    );
    let line = seeded
        .lines()
        .find(|l| l.contains("PhoenixOsAccessibilityDefaults"))
        .unwrap();
    for forbidden in ["assistance", "diagnos", "presentation", "request", "token"] {
        assert!(
            !line.contains(forbidden),
            "the OS default layer must not carry `{forbidden}`: {line}"
        );
    }
}
