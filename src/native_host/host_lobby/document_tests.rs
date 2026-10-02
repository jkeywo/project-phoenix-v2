use super::*;

/// The shape of the real host page, reduced to what the assembly depends
/// on: a nested `#lobby-panel` with the AI-launch button inside it, a
/// `#landing-panel` with the fullscreen control this surface cannot answer
/// and the `#landing-ship` hull column it cannot reach, and enough
/// surrounding page to prove nothing else is taken.
///
/// The hull column is `<section>`-wrapped exactly as the page's is, because
/// that is the whole reason [`extract_tagged`] exists: a `<div>`-counting
/// removal would have stopped at the column's inner panel and left a stray
/// `</section>` behind. A stub that flattened it to a `<div>` would pass
/// while the real page failed.
///
/// The *real* page is asserted against too — see
/// [`a_lobby_document_assembles_from_the_repositorys_own_host_page`], which
/// reads `server.html` off disk.
const HOST_PAGE: &str = "<!doctype html>\n<html>\n<head>\n\
         <link rel=\"stylesheet\" href=\"gui/tokens.css\" />\n\
         </head>\n<body>\n\
         <div id=\"scenario-panel\">\n\
         <div id=\"world-list\">\n\
         <div id=\"world-list-label\" data-i18n=\"server.select_world\">Select a world</div>\n\
         <div id=\"scenario-loading\">Loading…</div>\n\
         <div id=\"mod-pack-upload\">\n\
         <input id=\"mod-pack-file\" type=\"file\">\n\
         <button id=\"mod-pack-btn\" class=\"world-btn\">Upload mod pack</button>\n\
         <div id=\"mod-pack-status\"></div>\n\
         </div>\n\
         <div id=\"snapshot-import\">\n\
         <input id=\"snapshot-import-file\" type=\"file\">\n\
         <button id=\"snapshot-import-btn\" class=\"world-btn\">Import saved game</button>\n\
         </div>\n\
         </div>\n\
         </div>\n\
         <aside id=\"landing-join-panel\">\n\
         <span id=\"landing-join-role\"></span><span id=\"landing-join-blurb\"></span>\n\
         <label id=\"landing-join-label\" for=\"landing-join-code\"></label>\n\
         <input id=\"landing-join-code\" type=\"text\" />\n\
         <p id=\"landing-join-hint\"></p><p id=\"landing-join-error\" role=\"alert\"></p>\n\
         <button id=\"landing-join-submit\" type=\"button\"></button>\n\
         </aside>\n\
         <div id=\"landing-panel\" class=\"is-idle\" data-landing-stage=\"idle\">\n\
         <aside class=\"landing-rail\">\n\
         <span id=\"landing-rail-stamp\" data-i18n=\"server.landing.rail_stamp\">PHX</span>\n\
         </aside>\n\
         <button id=\"landing-fullscreen-btn\" class=\"landing-icon-btn\" type=\"button\" \
         data-i18n-attr=\"title:client.fullscreen_tip\">&#9974;</button>\n\
         <div class=\"landing-stage\">\n\
         <div class=\"landing-stage-inner\">\n\
         <div class=\"landing-track\">\n\
         <section class=\"landing-col landing-col-menu\">\n\
         <div class=\"landing-lockup\">\n\
         <span id=\"landing-logo\" role=\"img\"></span>\n\
         <h1 id=\"landing-title\" data-i18n=\"server.landing.title\">Project Phoenix</h1>\n\
         <span id=\"landing-tagline\"></span><span id=\"landing-platform\"></span>\n\
         </div>\n\
         <nav id=\"landing-menu\"></nav>\n\
         </section>\n\
         <section id=\"landing-mid\" class=\"landing-col landing-col-mid\"></section>\n\
         <section id=\"landing-ship\" class=\"landing-col landing-col-ship\">\n\
         <div class=\"landing-ship-panel\">\n\
         <div class=\"landing-ship-head\">\n\
         <h2 id=\"ship-list-label\" data-i18n=\"server.select_ship\">Select a ship</h2>\n\
         </div>\n\
         <div id=\"ship-list\" class=\"landing-ship-body\"></div>\n\
         </div>\n\
         </section>\n\
         </div>\n\
         </div>\n\
         </div>\n\
         <footer class=\"landing-statusbar\">\n\
         <span id=\"landing-status-platform\"></span>\n\
         <span id=\"landing-status-session\"></span>\n\
         <span id=\"landing-status-build\"></span>\n\
         </footer>\n\
         </div>\n\
         <div id=\"lobby-panel\" class=\"lobby-panel\" style=\"display:none;\">\n\
         <div class=\"lobby-bg\"></div>\n\
         <!-- AI-only launch -->\n\
         <button id=\"ai-launch-btn\" style=\"display:none;\" \
         data-i18n=\"server.launch_ai_ship\">Launch AI Ship</button>\n\
         <div class=\"lobby-panel-wrap\">\n\
         <div id=\"station-grid\" class=\"lobby-grid\"></div>\n\
         <aside class=\"lobby-rail\"><div id=\"lobby-status-hint\"></div></aside>\n\
         </div>\n\
         </div>\n\
         <div id=\"overlay\">\n\
         <div id=\"qr-panel\">\n\
         <div id=\"qr-caption\" data-i18n=\"server.qr_caption\"></div>\n\
         <a id=\"qr-link\"><canvas id=\"qr\"></canvas></a>\n\
         <div id=\"qr-url-row\"><span id=\"qr-url\"></span></div>\n\
         <div id=\"join-code-row\" style=\"display:none\"><span id=\"join-code\"></span></div>\n\
         </div>\n\
         <div id=\"fleet-panel\"><div id=\"fleet-slots\"></div></div>\n\
         </div>\n\
         <canvas id=\"canvas\"></canvas>\n\
         </body>\n</html>\n";

#[test]
fn the_lobby_markup_is_the_host_pages_own_nest_and_stops_where_it_stops() {
    // The claim that makes "one lobby" true: the document carries the
    // page's own element ids, and nothing that is not the lobby.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("id=\"lobby-panel\""));
    assert!(html.contains("id=\"station-grid\""));
    assert!(html.contains("id=\"lobby-status-hint\""));
    assert!(html.contains("class=\"lobby-panel-wrap\""));
    assert!(
        !html.contains("id=\"canvas\""),
        "the depth count must stop at the panel's own closing tag"
    );
    // The `<canvas>` that IS here is the join panel's own (issue #1329) —
    // the QR is drawn into it — and it comes from the page's `#qr-panel`,
    // never from a runaway count over the viewscreen's.
    assert!(html.contains("id=\"qr\""));
}

#[test]
fn the_ai_launch_button_is_kept_because_the_surface_can_now_answer_it() {
    // #1325 stripped it: a read-only surface with a control that silently
    // does nothing is worse than one with no control. #1328 removed the
    // reason — `server::bridge::apply_force_start` is no longer wasm-only —
    // and this is the one way a crew who are all on phones can be launched
    // from the viewscreen they are standing in front of.
    //
    // The monitor row (issue #1330) is not a second exception: its buttons
    // are BUILT by the shared renderer from a row the host pushes, so they
    // are not in this markup at all and exist exactly when something is
    // wired behind them.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("id=\"ai-launch-btn\""));
    // It carries the page's own string ids, so `applyToDom` names it in
    // whatever language the room is in — the English in the markup is
    // server.html's fallback, not this document's (AGENTS.md rule 11).
    assert!(html.contains("data-i18n=\"server.launch_ai_ship\""));
    // …and keeping it took nothing else with it.
    assert!(html.contains("class=\"lobby-bg\""));
    assert!(html.contains("id=\"station-grid\""));

    // The guard #1325 had as `!html.contains("<button")`, restored as an
    // ALLOWLIST rather than dropped with the blanket strip. What the blanket
    // assertion was really protecting is still true and still worth pinning:
    // no control reaches this document that nothing behind it answers. Now
    // that one control IS answered, the claim becomes a named set — so a
    // third control arriving inside a borrowed subtree (the host page grows
    // buttons for its own reasons, and both `#lobby-panel` and
    // `#scenario-panel` are taken from it wholesale) fails here instead of
    // appearing on a viewscreen as a dead button.
    assert_eq!(
        button_ids(&html),
        vec![
            "ai-launch-btn",
            "host-lobby-qr-toggle",
            "landing-fullscreen-btn",
            "landing-join-submit"
        ],
        "every control on the assembled document has something wired behind \
             it: the AI launch (issue #1328), the QR toggle (issue #1329) and \
             the landing's fullscreen control (issue #1367), whose press \
             crosses the queue as `HostLobbyRecord::ToggleFullscreen` and moves \
             this window's mode"
    );
}

/// Every button-shaped control in `html`, by id, sorted.
///
/// Both spellings, because the document uses both: `<button>` for the
/// markup it borrows from the host page, and `role="button"` for the one
/// control it supplies itself ([`QR_TOGGLE_MARKUP`]). Matching only the
/// element name would let a hand-written `role="button"` in here unseen,
/// which is exactly the shape the surface's own additions take.
fn button_ids(html: &str) -> Vec<String> {
    let mut ids: Vec<String> = html
        .split('<')
        .filter_map(|tag| {
            let open = tag.split_once('>')?.0;
            let is_button = open.starts_with("button") || open.contains("role=\"button\"");
            if !is_button {
                return None;
            }
            Some(
                open.split_once("id=\"")
                    .map(|(_, rest)| rest.split('"').next().unwrap_or("").to_string())
                    .unwrap_or_else(|| format!("<unnamed: {open}>")),
            )
        })
        .collect();
    ids.sort();
    ids
}

#[test]
fn the_scenario_picker_is_the_host_pages_own_and_starts_hidden() {
    // Issue #1328. Same rule as the lobby and the join panel: the page's
    // own markup, so `gui/host-scenario-render.js` writes into the ids it
    // expects. Hidden to start because a `--world` host has nothing to pick
    // and never pushes, and a picker covering the lobby of such a host
    // would be a viewscreen that never moves.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("<div id=\"scenario-panel\" style=\"display:none\">"));
    assert!(html.contains("id=\"world-list\""));
    assert!(html.contains("id=\"world-list-label\""));
    assert!(html.contains("id=\"scenario-loading\""));
    assert!(html.contains("href=\"gui/host-scenarios.css\""));
}

#[test]
fn the_landing_is_the_host_pages_own_and_starts_hidden() {
    // Issue #1361, the FOURTH extraction, by the rule the three before it
    // set: the page's own markup, so `gui/host-landing-render.js` writes
    // into the ids it expects and a row added to the web landing is a row
    // on this one. Hidden to start because only a world-less host pushes a
    // landing, and a `--world` host must not be shown a menu asking a
    // question it was answered at the prompt.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("<div id=\"landing-panel\" style=\"display:none\""));
    for id in [
        "landing-menu",
        "landing-mid",
        "landing-title",
        "landing-tagline",
        "landing-platform",
        "landing-logo",
        "landing-status-platform",
        "landing-status-session",
        "landing-status-build",
    ] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "the lobby document must carry #{id}, which the shared landing renderer \
                 writes into"
        );
    }
    // Its stylesheet comes with it, or the landing would be an unstyled
    // column of controls over the viewscreen.
    assert!(html.contains("href=\"gui/host-landing.css\""));
    // …and the extraction stopped where the panel does.
    assert!(!html.contains("id=\"canvas\""));
}

#[test]
fn the_landings_hull_column_is_not_carried_because_this_surface_never_stages_it() {
    // Issue #1362's staged rung, and the one part of the landing this
    // document does NOT take whole. See `LANDING_HULL_COLUMN_MARKER` for
    // the three things that make the rung reachable and which this surface
    // has none of; what this pins is the consequence.
    //
    // The failure it guards is silent rather than loud: `#ship-list` is
    // where `gui/host-scenario-render.js` mounts `ph-ship-picker` when a
    // document carries it, and `.landing-col-ship` is
    // `opacity: 0; visibility: hidden` until the root is `is-deep`, which
    // this surface never makes it. Carried, the hull picker would mount
    // into an invisible column and a multi-hull World would simply be
    // unpickable on the viewscreen, with a clean log and no error.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(
        HOST_PAGE.contains("id=\"landing-ship\""),
        "the stub must carry the column, or this test proves nothing"
    );
    assert!(HOST_PAGE.contains("id=\"ship-list\""));
    assert!(
        !html.contains("id=\"landing-ship\""),
        "the staged hull column must not reach the viewscreen"
    );
    assert!(
        !html.contains("id=\"ship-list\""),
        "with #ship-list absent the shared picker draws the hulls in the World \
             column, which is the only rendering this document can show"
    );
    // The `<section>` went whole. A `<div>`-counting removal would have
    // stopped at the column's inner panel and left this behind, and a
    // landing with an unopened `</section>` in it is a layout nobody
    // authored.
    assert!(!html.contains("landing-ship-panel"));
    assert!(!html.contains("ship-list-label"));
    assert_eq!(
        html.matches("<section").count(),
        html.matches("</section>").count(),
        "the removal left an unbalanced <section> in the document"
    );
    // The two columns that DO belong here are untouched, so this is a
    // removal and not a landing that lost its middle.
    assert!(html.contains("id=\"landing-mid\""));
    assert!(html.contains("id=\"landing-menu\""));
    // …and the panel still closes where it did: the removal ran inside the
    // extracted landing, so nothing after it was swallowed.
    assert!(html.contains("id=\"landing-status-build\""));
    assert!(!html.contains("id=\"canvas\""));
}

#[test]
fn the_landing_menu_is_empty_markup_because_its_entries_are_data() {
    // The claim that makes "one landing" true rather than aspirational:
    // this document carries the NEST and none of the rows. Which entries
    // exist — and which platform is offered which — is
    // `gui/host-landing-view.js`'s table, so the two surfaces differ
    // exactly where a row says they differ. If the menu's controls were
    // markup, the native one would have to be edited every time the web one
    // grew an entry, and Connect to Host could only be kept off this
    // surface by an edit here.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    let start = html
        .find("<nav id=\"landing-menu\"")
        .expect("the menu is carried");
    let nav = &html[start..][..html[start..].find("</nav>").expect("the menu closes")];
    assert!(
        !nav.contains("<button"),
        "the menu ships empty and the shared renderer fills it: {nav}"
    );
    for id in [
        "new_game",
        "load_game",
        "host_gm",
        "join_peer",
        "connect_host",
        "load_mod_pack",
        "exit_desktop",
    ] {
        assert!(
            !nav.contains(id),
            "{id} is a row in gui/host-landing-view.js, never markup in this document"
        );
    }
}

#[test]
fn the_landings_fullscreen_control_is_kept_because_this_surface_can_now_answer_it() {
    // The doctrine turned around, the same way the AI launch's was in
    // #1328. #1361 stripped this control: a browser host's forwards to
    // `gui/page-chrome.js`'s `initFullscreen`, which asks a BROWSER to fill
    // a screen, and this is an embedded view with no browser chrome. #1367
    // removed the reason — the press crosses the page->host queue as
    // `HostLobbyRecord::ToggleFullscreen`, and `fullscreen`'s applier sets
    // the primary window's mode the way the display-assignment law already
    // does — so the control comes back by the rule that took it away.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("id=\"landing-fullscreen-btn\""));
    // It carries the page's own string id, so `applyToDom` names it in
    // whatever language the room is in (AGENTS.md rule 11).
    assert!(html.contains("client.fullscreen_tip"));
    // …and keeping it took nothing else with it: the rail it sits beside
    // and the stage it sits above are untouched.
    assert!(html.contains("id=\"landing-rail-stamp\""));
    assert!(html.contains("class=\"landing-stage\""));
    assert!(html.contains("id=\"landing-menu\""));
    assert_eq!(
        button_ids(&html),
        vec![
            "ai-launch-btn",
            "host-lobby-qr-toggle",
            "landing-fullscreen-btn",
            "landing-join-submit"
        ]
    );
}

#[test]
fn the_native_settings_overlay_brings_its_own_sheet() {
    // Issue #1367. The markup this document borrows comes from the host
    // page, but `server.html`'s settings CSS is an inline `<style>` block
    // in that page and there is nothing to slice — so the surface that
    // mounts the shared overlay kit links a token-only sheet of its own.
    // The kit, the tab list and the controls' behaviour stay shared; only
    // the paint is here.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("href=\"gui/native-settings.css\""));
    // Beside the sheets it already had, not instead of one of them.
    assert!(html.contains("href=\"gui/host-landing.css\""));
    assert!(html.contains("href=\"gui/tokens.css\""));
    // The cog and the panel themselves are NOT markup here: the kit
    // find-or-creates both (`gui/settings-overlay-kit.js`), so they exist
    // exactly when `host_lobby_link.js` has mounted them and never as a
    // control the assembly could leave dead.
    assert!(!html.contains("id=\"native-settings-btn\""));
    assert!(!html.contains("id=\"native-settings-overlay\""));
}

#[test]
fn this_surface_scales_its_own_chrome_with_the_saved_text_size() {
    // Issue #1427. The lobby chrome, the picker and the landing are all
    // written in `--text-*` rungs, which are `rem` against a root this
    // document never set — so before this rule the Display tab's text size
    // reached nothing on the surface it is a setting FOR. The rule is the
    // one `server.html` carries, over the token that names the 16px both
    // documents rode by default.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains(
        "html { font-size: calc(var(--root-size-viewscreen) * var(--a11y-text-scale, 1)); }"
    ));
    // …and it is the SCENE that must not move with it. The viewscreen this
    // surface floats over is drawn by the host process into the window, not
    // by a `rem`-sized element, so there is nothing here for the scale to
    // reach — which is why this document sizes its own chrome and stops.
    assert!(html.contains("href=\"gui/tokens.css\""));
}

#[test]
fn the_saved_display_settings_are_seeded_before_any_module_runs() {
    // The seed is what makes the setting survive a restart on native: the
    // page's own storage session is ephemeral, so a document without this
    // assignment comes up at the default however many times the room turned
    // it up. `publish` composes the two; this asserts the composition puts
    // the assignment in `<head>`, ahead of the module island at the end of
    // `<body>` that reads it.
    let seeded = crate::native_host::panes::document::inject_head_script(
            &build_host_lobby_document(HOST_PAGE).unwrap(),
            &crate::native_host::viewscreen_presentation::presentation_script(
                &crate::native_host::viewscreen_presentation::ViewscreenPresentation {
                    text_scale_percent: Some(150),
                    contrast: Some(true),
                    shake_percent: Some(0),
                    ..crate::native_host::viewscreen_presentation::ViewscreenPresentation::following_system()
                },
            ),
        );
    let at = seeded
        .find("window.PhoenixViewscreenPresentation")
        .expect("the saved settings are seeded");
    assert!(seeded[at..].contains("\"textScale\":1.5"));
    assert!(seeded[at..].contains("\"contrast\":true"));
    // Issue #1428: the three effects are seeded on the same assignment, so
    // a display that was left with its shake off comes up with it off
    // rather than shaking once before the settings mount.
    assert!(seeded[at..].contains("\"shake\":0"));
    assert!(seeded[at..].contains("\"flash\":null"));
    assert!(at < seeded.find("</head>").expect("a head"));
}

#[test]
fn the_entries_this_surface_cannot_answer_are_absent_from_the_shipped_table() {
    // The other half of the doctrine, and it is deliberately checked
    // against `gui/host-landing-view.js` rather than against this document:
    // the menu is DATA, so "Connect to Host is not on native" is a field on
    // a row and not an edit to the markup. A native host is always a host
    // and has no join leg at all, so nothing is behind that entry here.
    //
    // Exit to Desktop is the mirror image — issue #1365's native-only row,
    // because a browser tab cannot quit an application. Both claims read
    // the same file, because "a control exists exactly when something
    // behind it answers it" is one rule, not two, and both are settled by a
    // FIELD ON A ROW rather than by an edit to this document.
    let view = std::fs::read_to_string("gui/host-landing-view.js")
        .expect("the shared landing view model is checked in");
    let row = view
        .find("id: 'connect_host'")
        .expect("the shipped table still has a Connect to Host row");
    let tail = &view[row..];
    let end = tail.find("},").expect("the row closes");
    assert!(
        tail[..end].contains("platforms: ['web']"),
        "Connect to Host must be web-only: a native host has no join leg \
             (issues #1361, #1364)"
    );
    let row = view
        .find("id: 'exit_desktop'")
        .expect("the shipped table has an Exit to Desktop row (issue #1365)");
    let tail = &view[row..];
    let end = tail
        .find("confirm:")
        .expect("the Exit to Desktop row declares its confirmation");
    assert!(
        tail[..end].contains("platforms: ['native']"),
        "Exit to Desktop must be native-only: a browser tab has no application to quit"
    );
    assert!(
        tail[..end].contains("stage: 'exit-confirm'"),
        "Exit to Desktop opens the stage that ASKS; the press itself must not quit"
    );

    // Load Game is the CONTRAST, and the reason the table carries two
    // fields rather than one. That route belongs on this surface — issue
    // #1363's AC5 asks for a native host that can resume a save without a
    // startup flag — so the row IS offered here. What is missing is the
    // stage behind it: this document carries no `#save-slots-panel` (the
    // catalogue became a body-level sibling of the picker in #1363, so it
    // is no longer swept up with `#scenario-panel`), `HostLobbyBridge` has
    // no save-catalogue channel to fill one from, and native resume is
    // startup-only by design. That is recorded as `stagePlatforms`, which
    // leaves the row rendered and inert; recording it as `platforms` would
    // have dropped the row and turned an unfinished AC into a claim that
    // native hosts do not load games.
    let load = view
        .find("id: 'load_game'")
        .expect("the shipped table still has a Load Game row");
    let load_row = &view[load..][..view[load..].find("},").expect("the row closes")];
    assert!(
        load_row.contains("platforms: ['web', 'native'],"),
        "Load Game is OFFERED on both surfaces (issue #1363 AC5)"
    );
    assert!(
            load_row.contains("stagePlatforms: ['web'],"),
            "…and only the web host can open its stage yet, which is the gap              this surface still has to close (issue #1363 AC5)"
        );
    assert!(
            !build_host_lobby_document(HOST_PAGE)
                .unwrap()
                .contains("id=\"save-slots-panel\""),
            "the panel that stage would borrow is genuinely not in this              document — the row's comment says so, and this is the check"
        );
}

#[test]
fn a_page_with_no_landing_is_refused_by_its_own_name() {
    // A world-less host whose viewscreen cannot show the front door opens
    // on nothing. Being told so at the prompt beats discovering it in a
    // room full of people.
    let page = "<body><div id=\"lobby-panel\">x</div><div id=\"qr-panel\">y</div>\
                    <div id=\"scenario-panel\"><div id=\"world-list\"></div></div></body>";
    assert_eq!(
        build_host_lobby_document(page),
        Err(HostLobbyDocumentError::NoLandingPanel)
    );
}

#[test]
fn a_landing_panel_that_never_closes_is_refused_by_its_own_name_too() {
    // Its three siblings each have this case for the same reason: an
    // operator is told WHICH panel of the bundle is malformed, and an error
    // variant nothing constructs in a test is a name that can quietly stop
    // being reachable.
    let page = "<body><div id=\"lobby-panel\">x</div><div id=\"qr-panel\">y</div>\
                    <div id=\"scenario-panel\"><div id=\"world-list\"></div></div>\
                    <div id=\"landing-panel\"><div class=\"landing-stage\"></div></body>";
    assert_eq!(
        build_host_lobby_document(page),
        Err(HostLobbyDocumentError::UnbalancedLandingPanel)
    );
}

#[test]
fn a_pushed_landing_state_is_escaped_into_its_own_call() {
    // Its own entry point, not a field of the scenario payload beside it:
    // that payload is precisely `scenarioCatalogView`'s three arguments,
    // and it carries three and nothing else on purpose.
    assert_eq!(
        host_lobby_landing_script(r#"{"build":"0.1.0","dismissed":false}"#),
        r#"window.__phoenixHostLobbyLanding('{"build":"0.1.0","dismissed":false}')"#
    );
}

#[test]
fn the_landings_two_menu_records_are_in_the_one_vocabulary_the_host_drains() {
    // The page→host half (issue #1361). Not a second record type and not a
    // second queue: `take_records` is a drain with one reader, so a new
    // control is a variant. The tags are what `host_lobby_link.js` writes
    // by hand — it has no serde — so they are pinned here rather than left
    // to be discovered on a viewscreen.
    use super::super::HostLobbyRecord;
    assert_eq!(
        HostLobbyRecord::decode(r#"{"kind":"landing_open","entry":"new_game"}"#),
        Some(HostLobbyRecord::LandingOpen {
            entry: "new_game".into()
        })
    );
    assert_eq!(
        HostLobbyRecord::decode(r#"{"kind":"landing_close"}"#),
        Some(HostLobbyRecord::LandingClose)
    );
    // The document's own client half is what sends them, and it sends them
    // through the queue this document installs rather than by touching the
    // namespace directly.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("kind: 'landing_open'"));
    assert!(html.contains("kind: 'landing_close'"));
    assert!(html.contains("window.phoenixHostLobbyOut.send"));
}

#[test]
fn the_landing_renders_from_the_shared_modules_and_decides_nothing_itself() {
    // The rule `host_lobby_link.js`'s own header states: a render decision
    // appearing in that file has escaped the shared path. So the document's
    // client half imports the #1360 pair and calls them, and what it tells
    // the renderer are facts about this surface rather than judgements
    // about the landing.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("gui/host-landing-view.js"));
    assert!(html.contains("gui/host-landing-render.js"));
    assert!(html.contains("landingViewModel"));
    assert!(html.contains("nextOpenEntry"));
    assert!(html.contains("platform: 'native'"));
    assert!(html.contains("ownPanelVisibility: true"));
    // The fullscreen hook IS handed over since issue #1367, and what it
    // does is a record rather than a call: a browser host's control
    // forwards to `gui/page-chrome.js`'s `initFullscreen`, and there is no
    // browser here to ask. So the press crosses the same queue as every
    // other thing this surface cannot do itself, and the verb is pinned
    // here for the reason the two menu records above are — the client half
    // writes it by hand and has no serde to keep it honest.
    assert!(html.contains("toggleFullscreen:"));
    assert!(html.contains("kind: 'toggle_fullscreen'"));
    assert_eq!(
        super::super::HostLobbyRecord::decode(r#"{"kind":"toggle_fullscreen"}"#),
        Some(super::super::HostLobbyRecord::ToggleFullscreen)
    );
}

#[test]
fn the_settings_cog_is_mounted_from_the_shared_kit_and_not_rebuilt_here() {
    // Issue #1367's other half, and the claim the PRD makes in one line:
    // settings are not rebuilt. So the document's client half imports
    // `gui/native-settings.js`, which takes its shell from
    // `gui/settings-overlay-kit.js` and its tabs from
    // `gui/settings-tabs.js` — the same two the host page's cog and the
    // phone's use — and this surface supplies only the hook that says what
    // a pressed row's verb means here.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("gui/native-settings.js"));
    assert!(html.contains("mountNativeSettings"));
    // Its two verbs, pinned for the reason every other hand-written tag on
    // this bridge is: one is answered in the page and one crosses the
    // queue, and neither is spelled anywhere serde can check.
    assert!(html.contains("toggle_qr:"));
    assert!(html.contains("kind: 'toggle_fullscreen'"));
    // …and no second settings implementation rides along: nothing here
    // reaches for the host PAGE's cog, which is wired to `wasm_*` bindings
    // this process does not publish to a document.
    assert!(!html.contains("server-settings"));
}

#[test]
fn a_pushed_mod_pack_shelf_is_escaped_into_its_own_call() {
    // Issue #1366, and its own entry point rather than a field of the
    // landing payload beside it: that payload carries the two facts the page
    // cannot know about the PROCESS, and this carries a whole panel.
    assert_eq!(
        host_lobby_packs_script(r#"{"dir":"mods","offered":[]}"#),
        r#"window.__phoenixHostLobbyPacks('{"dir":"mods","offered":[]}')"#
    );
}

#[test]
fn the_mod_pack_hooks_are_handed_over_and_forward_the_rows_own_verb() {
    // The same claim the confirm hook's case below makes, for issue #1366's
    // two: `landing-packs-cta` and `landing-packs-cancel` are on this
    // document's button allowlist because something on this surface is
    // behind them, and the whole of that something is these two lines.
    // Delete either and the buttons still render, are still allowlisted, and
    // do nothing.
    assert!(
        HOST_LOBBY_LINK_JS
            .contains("installPack: (action, file) => send({ kind: action, pack: file })"),
        "the shelf's install control must reach the host: `renderHostLanding` hangs its \
             handler on this hook, and `apply_mod_pack_choice` is what answers the verb it sends"
    );
    assert!(
        HOST_LOBBY_LINK_JS.contains("pickPack:"),
        "highlighting a row is this surface's own memory, and the renderer reports it \
             through a hook rather than holding it"
    );
    // Verbatim, which is the point of the hook being one line: the verb is
    // the OPEN ROW's, so a second shelf-shaped route costs a row and not a
    // branch here. `install_mod_pack` appearing in this file would be the
    // mapping table `gui/host-landing-view.js`'s `action` field replaces.
    assert!(
        !HOST_LOBBY_LINK_JS.contains("install_mod_pack"),
        "the verb travels as the record's `kind`; naming it here is the mapping table \
             the row's `action` field replaces"
    );
    // And the surface says what it can ANSWER rather than checking a flag:
    // a host started without --mod-pack-dir never pushes a shelf, so
    // `landingProvides()` is empty and the row stays inert with no
    // build check, no CLI check and no id read by name on this side.
    assert!(
        HOST_LOBBY_LINK_JS.contains("provides: landingProvides()"),
        "which routes this surface can answer is data it declares, not a branch"
    );
}

#[test]
fn the_confirm_hook_is_handed_over_and_forwards_the_rows_own_verb() {
    // The positive twin of the `toggleFullscreen:` assertion above, and the
    // half the allowlist below TAKES ON TRUST: `landing-confirm-cta` is on
    // that list because "something on this surface is behind it", and the
    // whole of that something is one line in `host_lobby_link.js`. Delete
    // it and the button still renders, is still allowlisted, and does
    // nothing — the dead button the allowlist exists to forbid, with every
    // case in this file green. So the hook is pinned here, the way
    // `the_entries_this_surface_cannot_answer_are_absent_from_the_shipped_table`
    // pins `platforms: ['native']` in the file that declares it.
    assert!(
            HOST_LOBBY_LINK_JS.contains("confirm: (action) => send({ kind: action })"),
            "the landing's confirm control must reach the host: `renderHostLanding`              hangs the confirmation's handler on this hook, and `drain_surface_records`              is what answers the verb it sends"
        );
    // Verbatim, which is the point of the hook being one line: the verb is
    // the OPEN ROW's, so a second confirming route costs a row and not a
    // branch here. A mapping table in this file would show up as the verb's
    // own name — and `exit_desktop` appearing here would be this link
    // knowing which route it is, which the menu-is-data case above already
    // forbids for the markup.
    assert!(
            !HOST_LOBBY_LINK_JS.contains("exit_desktop"),
            "the verb travels as the record's `kind`; naming a route here is the              mapping table `gui/host-landing-view.js`'s `confirm` block replaces"
        );
}

/// Every value `prop` takes in the rules whose selector is exactly
/// `selector`, in source order, as the leading integer of the declaration.
///
/// Read out of the real stylesheets rather than restated as constants here:
/// the point of the two assertions below is that a number moving in
/// `gui/host-landing.css` breaks a test in THIS file, and a copy of that
/// number kept here would move with it and prove nothing.
fn declared(sheet: &str, selector: &str, prop: &str) -> Vec<u32> {
    /// The ancestor guard the host sheets put on their portrait rules so a
    /// phone locked to landscape never takes them (server.html sets the
    /// attribute; this document does not, so the rules DO apply here).
    const LANDSCAPE_LOCK_GUARD: &str = "html:not([data-phx-force-landscape])";
    let opener = format!("{selector} {{");
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(found) = sheet[at..].find(&opener) {
        let start = at + found;
        let body = start + opener.len();
        let end = body + sheet[body..].find('}').expect("unterminated CSS rule");
        at = end;
        // The selector must OPEN its line, or `.landing-statusbar {` would
        // also match `.landing-statusbar .landing-sep {`'s tail. The one
        // ancestor allowed before it is the phone landscape lock's guard:
        // the portrait rules are written `html:not([data-phx-force-landscape])
        // .landing-statusbar { … }` so they stay inert on server.html, and
        // the number in such a rule is still the number this file keeps
        // the ground in step with.
        let line_start = sheet[..start].rfind('\n').map_or(0, |i| i + 1);
        let before = sheet[line_start..start].trim();
        if !(before.is_empty() || before == LANDSCAPE_LOCK_GUARD) {
            continue;
        }
        for decl in sheet[body..end].split(';') {
            if let Some(value) = decl.trim().strip_prefix(&format!("{prop}:")) {
                let digits: String = value
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(n) = digits.parse::<u32>() {
                    out.push(n);
                }
                break;
            }
        }
    }
    out
}

#[test]
fn the_join_code_stays_above_the_landing_and_the_picker_that_would_cover_it() {
    // Crew must see the join QR above both selection panels, with its
    // toggle above the QR. The numeric layers may change together.
    let qr = std::fs::read_to_string("gui/host-qr.css").unwrap();
    let picker = std::fs::read_to_string("gui/host-scenarios.css").unwrap();
    let landing = std::fs::read_to_string("gui/host-landing.css").unwrap();
    let base = declared(&qr, "#overlay", "z-index");
    let over_picker = declared(&picker, "#scenario-panel", "z-index");
    let over_landing = declared(&landing, "#landing-panel", "z-index");
    let lift = declared(GROUND_CSS, "#overlay", "z-index");
    let toggle = declared(GROUND_CSS, "#host-lobby-qr-toggle", "z-index");
    assert!(
        base[0] < over_picker[0]
            && over_picker[0] < over_landing[0]
            && over_landing[0] < lift[0]
            && lift[0] < toggle[0],
        "the join panel belongs above BOTH panels this document carries, \
             and its toggle above the panel"
    );
    // …and the lift is written AFTER the linked sheets, so the later rule
    // wins on equal specificity rather than relying on source order in one
    // file.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    let sheet = html.find("href=\"gui/host-qr.css\"").unwrap();
    let ground = html.find(GROUND_CSS).unwrap();
    assert!(sheet < ground);
}

#[test]
fn the_lifted_join_panel_clears_the_landings_status_bar_at_both_breakpoints() {
    // What the lift buys: the join panel now floats over the landing, and
    // the landing owns the corner it floats in. `.landing-statusbar` is
    // anchored `right: 0; bottom: 0`, and the row under it carries the
    // build stamp, so a panel left at `bottom: 1rem` paints on top of it.
    // The ground raises it by the bar's height plus the inset it would
    // otherwise have had, at both of the landing sheet's heights and inside
    // the landing sheet's own breakpoint, so the two cannot drift apart.
    const OWN_INSET_PX: u32 = 16; // #overlay's `bottom: 1rem`, gui/host-qr.css
    const NARROW: &str = "@media (max-width: 999px), (orientation: portrait)";
    let landing = std::fs::read_to_string("gui/host-landing.css").unwrap();
    let bars = declared(&landing, ".landing-statusbar", "height");
    assert_eq!(bars, [56, 44], "the wide bar then the narrow one");
    assert_eq!(
        declared(GROUND_CSS, "#overlay", "bottom"),
        [bars[0] + OWN_INSET_PX, bars[1] + OWN_INSET_PX],
        "the join panel would paint over the landing's build stamp"
    );
    assert!(
        landing.contains(NARROW),
        "the sheet's own narrow breakpoint"
    );
    assert!(GROUND_CSS.contains(NARROW), "and the ground matches it");
}

#[test]
fn the_pickers_host_tooling_is_removed_rather_than_hidden() {
    // The mod-pack upload and the save importer are file inputs with
    // page-lifetime handlers this document does not carry, and a demo build
    // removes them outright for its own reasons. A `display: none` control
    // is still in the DOM to be reached.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    for id in [
        "mod-pack-upload",
        "mod-pack-file",
        "mod-pack-btn",
        "mod-pack-status",
        "snapshot-import",
        "snapshot-import-file",
        "snapshot-import-btn",
    ] {
        assert!(
            !html.contains(&format!("id=\"{id}\"")),
            "#{id} is host tooling and must not reach the viewscreen"
        );
    }
    // The removal is depth-counted: everything around the two blocks is
    // still there, and the column did not lose its tail.
    assert!(html.contains("id=\"world-list-label\""));
    assert!(html.contains("id=\"scenario-loading\""));
    assert!(html.contains("id=\"lobby-panel\""));
}

#[test]
fn a_page_with_no_scenario_panel_is_refused_by_its_own_name() {
    // A `--lobby` host whose viewscreen cannot show the picker is a host
    // that can never start a mission.
    let page = "<body><div id=\"lobby-panel\">x</div><div id=\"qr-panel\">y</div></body>";
    assert_eq!(
        build_host_lobby_document(page),
        Err(HostLobbyDocumentError::NoScenarioPanel)
    );
}

#[test]
fn a_scenario_panel_that_never_closes_is_refused_by_its_own_name_too() {
    let page = "<body><div id=\"lobby-panel\">x</div><div id=\"qr-panel\">y</div>\
                    <div id=\"scenario-panel\"><div class=\"z\"></div></body>";
    assert_eq!(
        build_host_lobby_document(page),
        Err(HostLobbyDocumentError::UnbalancedScenarioPanel)
    );
}

#[test]
fn the_boot_script_runs_before_the_module_island_that_needs_it() {
    // `host_lobby_link.js` assigns `window.__phoenixHostLobby.render`, so
    // the object the boot script publishes has to exist first — and the
    // host starts pushing the moment the document reports itself loaded,
    // which is before either has necessarily run.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    let boot = html.find("window.__phoenixHostLobbyApply").unwrap();
    let link = html.find("host-lobby-render.js").unwrap();
    let panel = html.find("id=\"lobby-panel\"").unwrap();
    assert!(
        boot < panel,
        "the bridge is installed before the markup parses"
    );
    assert!(
        panel < link,
        "the renderer runs after the markup it writes into"
    );
    assert!(html.find("<head>").unwrap() < boot);
    assert!(link < html.rfind("</body>").unwrap());
}

#[test]
fn the_page_to_host_queue_is_installed_under_the_namespace_the_host_drains() {
    // Nothing rides it yet. It is installed now so the slices that will —
    // the QR overlay, settings, layout rows — extend one bridge instead of
    // opening a second.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("window.phoenixHostLobbyOut.send"));
    assert!(html.contains("window.__phoenixHostLobbyOutDrain"));
    assert_eq!(
        host_lobby_drain_script(),
        "window.__phoenixHostLobbyOutDrain()"
    );
}

#[test]
fn the_document_loads_the_same_stylesheets_the_web_lobby_does() {
    // gui/host-lobby.css and gui/host-qr.css exist BECAUSE of this surface
    // (issues #1325 and #1329 lifted them out of server.html's inline
    // <style>), so a document that did not link them would have re-created
    // the duplication the extractions removed.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("href=\"gui/tokens.css\""));
    assert!(html.contains("href=\"gui/host-lobby.css\""));
    assert!(html.contains("href=\"gui/host-qr.css\""));
    assert!(html.contains("href=\"gui/host-scenarios.css\""));
    // Relative, not absolute: the path is what makes the served depth do the
    // resolving, exactly as it does for a pane document.
    assert!(!html.contains("href=\"/gui/"));
}

#[test]
fn the_join_panel_comes_with_the_lobby_because_a_crew_has_to_get_in() {
    // Issue #1329. The page's OWN join markup, for the same reason the
    // lobby's is the page's own: a hand-written copy here would render
    // nowhere the first time somebody added a row to the web panel.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    for id in [
        "overlay",
        "qr-panel",
        "qr-caption",
        "qr-link",
        "qr",
        "qr-url",
        "join-code",
    ] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "the lobby document must carry #{id}, which gui/host-qr.js writes into"
        );
    }
}

#[test]
fn the_overlay_carries_the_join_panel_and_nothing_else_the_page_puts_there() {
    // #overlay is this document's own one-line wrapper, not the page's: the
    // page's also holds the fleet panel (which admits other ship HOSTS,
    // issue #1114) and the connection diagnostics, and a viewscreen has
    // nothing to say about either.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(!html.contains("id=\"fleet-panel\""));
    assert!(!html.contains("id=\"fleet-slots\""));
}

#[test]
fn a_pushed_scenario_state_is_escaped_into_its_own_call() {
    assert_eq!(
        host_lobby_scenario_script(r#"{"scenarios":[],"locked":false}"#),
        r#"window.__phoenixHostLobbyScenario('{"scenarios":[],"locked":false}')"#
    );
}

#[test]
fn the_encoder_is_the_local_copy_because_a_bridge_has_no_internet() {
    // The whole point of vendoring it (issue #1329): the host serves this
    // document AND the encoder it loads, so a room with no network still
    // gets a join code. A CDN here would have been a lobby that shows a
    // crew everything except how to join.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("<script src=\"gui/vendor/qrcode.js\"></script>"));
    assert!(!html.contains("http://cdn."));
    assert!(!html.contains("https://"));
}

#[test]
fn the_surfaces_own_qr_control_carries_a_string_id_and_no_english() {
    // A host page toggles the QR from its settings cog; this window has no
    // cog. The control's text is resolved by `applyToDom` from the same
    // string id the page's own menu uses — prose in this file would be
    // player-visible English authored in a Rust source (AGENTS.md rule 11).
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("id=\"host-lobby-qr-toggle\""));
    assert!(html.contains("data-i18n=\"settings.toggle_qr\""));
    assert!(html.contains("role=\"button\" tabindex=\"0\""));
}

#[test]
fn a_page_with_no_join_panel_is_refused_by_name_too() {
    // A lobby nobody can be shown how to join is not a lobby to composite
    // silently — it is a bundle that is not what the operator thinks.
    let page = "<body><div id=\"lobby-panel\"><div id=\"station-grid\"></div></div></body>";
    assert_eq!(
        build_host_lobby_document(page),
        Err(HostLobbyDocumentError::NoJoinPanel)
    );
}

#[test]
fn the_cog_keep_out_clears_the_corner_its_cog_occupies() {
    // `.lobby-panel-wrap` floors its left padding at the settings cog's
    // corner and `#world-list` floors its TOP padding at the same token,
    // both naming it with a fallback so the property may be absent. It was
    // absent here through #1325-#1366, because there was no cog. #1367
    // mounts one from the shared kit, pinned into the landing rail's top —
    // so the token is defined and the two floors hold a real corner rather
    // than an imported one.
    //
    // ONE scalar, TWO axes, and that is the whole difficulty. The host
    // page's cog is a 34px square at top/left 10, so 56px clears it either
    // way and `tests/client/server-settings.test.js` asserts exactly this
    // pair of inequalities over there. THIS cog is not square-cornered:
    // `top: 34px; left: 9px` on the wide breakpoint, because 9px centres a
    // 46px button in the 64px rail and 34px sits it level with
    // `#landing-fullscreen-btn` in the opposite corner. So the honest
    // scalar is the tallest of the four extents, and every number below is
    // read out of the two sheets rather than restated here — a cog that
    // moves has to break this test rather than the first screen the
    // operator reads.
    let settings = std::fs::read_to_string("gui/native-settings.css").unwrap();
    let landing = std::fs::read_to_string("gui/host-landing.css").unwrap();
    let size = declared(&landing, ".landing-icon-btn", "height");
    assert_eq!(size, [46], "the chamfered square the cog borrows");
    let keepout = declared(GROUND_CSS, ":root", "--settings-cog-keepout");
    assert_eq!(keepout.len(), 1, "one token, declared once");
    let insets: Vec<u32> = declared(&settings, ".native-settings-btn", "top")
        .into_iter()
        .chain(declared(&settings, ".native-settings-btn", "left"))
        .collect();
    assert_eq!(insets.len(), 4, "a top and a left at each breakpoint");
    for inset in insets {
        assert!(
            keepout[0] >= inset + size[0],
            "the keep-out is {}px and the cog reaches {}px; an undocked \
                 picker would print its label under the cog",
            keepout[0],
            inset + size[0]
        );
    }
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains(&format!("--settings-cog-keepout: {}px", keepout[0])));
}

#[test]
fn the_body_is_painted_because_its_pixels_are_copied_into_an_opaque_texture() {
    // Before the first lobby push the page has nothing to show, and the
    // host still copies its frame. An unpainted body would composite as a
    // white sheet over the viewscreen for those frames.
    let html = build_host_lobby_document(HOST_PAGE).unwrap();
    assert!(html.contains("background: #000"));
}

#[test]
fn a_page_with_no_lobby_is_refused_by_name_rather_than_shown_empty() {
    assert_eq!(
        build_host_lobby_document("<html><body><canvas></canvas></body></html>"),
        Err(HostLobbyDocumentError::NoLobbyPanel)
    );
}

#[test]
fn a_lobby_panel_that_never_closes_is_refused_rather_than_truncated() {
    let unbalanced = "<body><div id=\"lobby-panel\"><div class=\"x\"></div></body>";
    assert_eq!(
        build_host_lobby_document(unbalanced),
        Err(HostLobbyDocumentError::UnbalancedLobbyPanel)
    );
}

#[test]
fn a_comment_mentioning_a_div_does_not_unbalance_the_count() {
    // The real page's lobby markup is full of comments; one of them saying
    // `</div>` would truncate the panel at a place that still looked like
    // valid markup.
    let page = "<div id=\"lobby-panel\"><!-- a </div> in prose --><div>x</div></div>\
                    <div id=\"qr-panel\"></div>\
                    <div id=\"scenario-panel\"><div id=\"world-list\"></div></div>\
                    <aside id=\"landing-join-panel\"></aside>\
                    <div id=\"landing-panel\"><nav id=\"landing-menu\"></nav></div>\
                    <canvas id=\"trailing\"></canvas>";
    let html = build_host_lobby_document(page).unwrap();
    assert!(html.contains("<div>x</div>"));
    assert!(!html.contains("trailing"));
}

#[test]
fn the_document_path_sits_at_the_host_pages_own_depth() {
    // This is what makes `gui/host-lobby-render.js` and
    // `../assets/strings/strings.csv` resolve as they do for index.html.
    // A pane's `/client/` depth is the same rule applied to the other page.
    assert_eq!(host_lobby_document_path("abcd"), "/host-lobby-abcd.html");
    assert_eq!(
        host_lobby_url("127.0.0.1:8080", "abcd"),
        "http://127.0.0.1:8080/host-lobby-abcd.html"
    );
}

#[test]
fn the_lobby_url_carries_no_fragment_because_there_is_no_identity_to_carry() {
    // A pane's URL puts its session token in the fragment. This surface is
    // not a participant: no token, no Identify, no station.
    let url = host_lobby_url("127.0.0.1:8080", "abcd");
    assert!(!url.contains('#'));
    assert!(!url.contains("token"));
}

#[test]
fn a_pushed_payload_is_escaped_into_the_apply_call() {
    assert_eq!(
        host_lobby_apply_script(r#"{"phase":"Lobby","scenario_title":"O'Neil"}"#),
        r#"window.__phoenixHostLobbyApply('{"phase":"Lobby","scenario_title":"O\'Neil"}')"#
    );
}

#[test]
fn a_pushed_monitor_row_is_escaped_into_its_own_call() {
    // Its own entry point, not a field of the lobby payload: that payload
    // is the shared `LobbyStatePayload` the browser host reads too, and a
    // browser host has no monitors.
    assert_eq!(
        host_lobby_layout_script(r#"{"monitors":[{"name":"O'Neil's TV"}]}"#),
        r#"window.__phoenixHostLobbyLayout('{"monitors":[{"name":"O\'Neil\'s TV"}]}')"#
    );
}

#[test]
fn the_reveal_flag_crosses_as_a_string_because_that_is_the_bridges_one_shape() {
    assert_eq!(
        host_lobby_reveal_script(true),
        "window.__phoenixHostLobbyReveal('true')"
    );
    assert_eq!(
        host_lobby_reveal_script(false),
        "window.__phoenixHostLobbyReveal('false')"
    );
}

#[test]
fn a_lobby_document_assembles_from_the_repositorys_own_host_page() {
    // Every other test here runs against a stub, so nothing checked the
    // assembly against the page it actually operates on. The repository's
    // `server.html` is checked in, so this needs no build step — and
    // `trunk build` copies its lobby markup into `dist/index.html`
    // untouched (it rewrites only the `data-trunk` links and appends the
    // wasm bootstrap).
    let page = std::fs::read_to_string("server.html")
        .expect("the repository's own host page is checked in");
    let html = build_host_lobby_document(&page).expect("the real page becomes a lobby document");

    // The ids `gui/host-lobby-render.js` writes into. If the web lobby ever
    // renames one, this fails here rather than rendering nothing on the
    // viewscreen with a clean log.
    for id in [
        "lobby-panel",
        "lobby-title",
        "lobby-subtitle",
        "lobby-crew-count",
        "lobby-crew-dots",
        "lobby-spectator-tag",
        "lobby-ready-badge",
        "lobby-countdown",
        "station-grid",
        "reserved-aggregate",
        "lobby-spectator-list",
        "lobby-status-hint",
    ] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "the lobby document must carry #{id}, which the shared renderer writes into"
        );
    }

    // The join panel's own ids, which gui/host-qr.js writes into (issue
    // #1329) — taken out of the same page, by the same rule.
    for id in ["qr-panel", "qr", "qr-url", "join-code", "join-code-row"] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "the lobby document must carry #{id}, which the shared join panel writes into"
        );
    }

    // The picker's own ids, which gui/host-scenario-render.js writes into
    // (issue #1328) — taken out of the same page, by the same rule.
    for id in ["scenario-panel", "world-list", "world-list-label"] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "the lobby document must carry #{id}, which the shared picker writes into"
        );
    }

    // The landing's own ids, which gui/host-landing-render.js writes into
    // (issue #1361) — the fourth extraction, by the same rule again. The
    // menu is `#landing-menu` and nothing inside it: its entries are data,
    // which is what lets this surface offer a different set from the web
    // one without a second copy of the markup.
    for id in [
        "landing-panel",
        "landing-menu",
        "landing-mid",
        "landing-title",
        "landing-tagline",
        "landing-platform",
        "landing-logo",
        "landing-status-platform",
        "landing-status-session",
        "landing-status-build",
        // The confirmation stage's own (issue #1365). Markup, unlike the
        // menu's entries, because it is one panel whose WORDS are data
        // rather than a list whose length is: the shared renderer writes
        // the open row's `confirm` block into these ids on both surfaces.
        "landing-confirm",
        "landing-confirm-title",
        "landing-confirm-eyebrow",
        "landing-confirm-lead",
        "landing-confirm-note",
        "landing-confirm-cancel",
        "landing-confirm-cta",
        // The mod-pack shelf's own (issue #1366). Markup for the same
        // reason the confirmation's is: one panel whose CONTENTS are data.
        // Its list and its report column ship empty, like `#landing-menu`,
        // because what is in the folder is the host's answer.
        "landing-packs",
        "landing-packs-title",
        "landing-packs-folder",
        "landing-packs-empty",
        "landing-packs-list",
        "landing-packs-notes",
        "landing-packs-cancel",
        "landing-packs-cta",
    ] {
        assert!(
            html.contains(&format!("id=\"{id}\"")),
            "the lobby document must carry #{id}, which the shared landing writes into"
        );
    }
    // …including the fullscreen control, which since issue #1367 has a host
    // verb behind it on this surface as well as on the page.
    assert!(page.contains("id=\"landing-fullscreen-btn\""));
    assert!(html.contains("id=\"landing-fullscreen-btn\""));
    // …minus the staged hull column, which the real page really does carry
    // and this surface cannot reach (issue #1362; see
    // `LANDING_HULL_COLUMN_MARKER`). This is the assertion the stub above
    // cannot make on its own: `#landing-ship` arrived here by ACCIDENT, as
    // a child of the whole-panel extraction, and nothing said so until the
    // hull picker was mounting into a permanently invisible column.
    assert!(page.contains("id=\"landing-ship\""));
    assert!(page.contains("id=\"ship-list\""));
    assert!(
        !html.contains("id=\"landing-ship\""),
        "the staged hull column reached the viewscreen, where `is-deep` is \
             never set and it can therefore never be seen"
    );
    assert!(
        !html.contains("id=\"ship-list\""),
        "with #ship-list present the shared picker mounts ph-ship-picker into \
             the hidden column and a multi-hull World becomes unpickable here"
    );
    assert!(!html.contains("id=\"ship-list-label\""));
    // …and the column went whole. A `<div>`-counting removal would have
    // stopped at its inner panel and left a `</section>` nothing opened.
    assert_eq!(
        html.matches("<section").count(),
        html.matches("</section>").count(),
        "the hull-column removal left an unbalanced <section> in the document"
    );
    // …minus the host tooling, which the real page really does carry.
    assert!(page.contains("id=\"mod-pack-upload\""));
    assert!(!html.contains("id=\"mod-pack-upload\""));
    assert!(page.contains("id=\"snapshot-import\""));
    assert!(!html.contains("id=\"snapshot-import\""));
    // …and minus the lobby rail's GM start controls (issue #1300's Game
    // Master): wired only by the page's GM session script, which this
    // surface never runs. The `<section>` shell stays; its buttons go.
    assert!(page.contains("id=\"gm-ready-btn\""));
    assert!(page.contains("id=\"gm-force-start-btn\""));
    assert!(!html.contains("id=\"gm-ready-btn\""));
    assert!(!html.contains("id=\"gm-force-start-btn\""));
    assert!(html.contains("id=\"gm-start-controls\""));
    // The only control the surface carries out of the lobby markup is the
    // AI launch, which is now wired (issue #1328). Asserted as the whole
    // allowlist and not just those three ids, because THIS is the page that
    // can drift: the stub above is authored beside the test that reads it,
    // and `server.html` grows controls for its own reasons inside the two
    // subtrees this document borrows wholesale.
    assert_eq!(
        button_ids(&html),
        vec![
            "ai-launch-btn",
            "host-lobby-qr-toggle",
            // The confirmation stage's own two (issue #1365). They are on
            // the allowlist because they ARE wired: the shared renderer
            // hangs its handlers on them from `renderHostLanding`, and the
            // verb the confirm control carries is answered here by
            // `drain_surface_records` with an `AppExit`.
            "landing-confirm-cancel",
            "landing-confirm-cta",
            // The landing's fullscreen control (issue #1367), on the
            // allowlist for the same reason and by the same rule: the
            // shared renderer hangs `toggleFullscreen` on it,
            // `host_lobby_link.js` supplies a hook that sends
            // `toggle_fullscreen`, and `fullscreen::apply_window_mode_toggle`
            // moves this window's mode. Until it had that, #1361 stripped
            // it from this document rather than ship a dead corner.
            "landing-fullscreen-btn",
            // Join as Peer's submit control is wired to the typed native
            // fleet-member request through host_lobby_link.js.
            "landing-join-submit",
            // The mod-pack shelf's own two (issue #1366), on the allowlist
            // by the same rule and for the same reason: `renderHostLanding`
            // hangs `pick` and `installPack` on them, `host_lobby_link.js`
            // supplies both, and the verb the install control carries is
            // answered here by `drain_surface_records` and
            // `apply_mod_pack_choice`. On a host started without
            // `--mod-pack-dir` the panel never opens at all — the row is
            // inert because this surface never says it provides a shelf —
            // so the buttons are unreachable rather than dead.
            "landing-packs-cancel",
            "landing-packs-cta"
        ],
        "a control on the viewscreen with nothing wired behind it is a dead \
             button the operator will press"
    );
    assert!(!html.contains("id=\"mod-pack-btn\""));
    assert!(!html.contains("id=\"snapshot-import-btn\""));

    // …and stops at each panel. `#hud-overlay` is the next full-screen
    // surface in the page and a runaway count would swallow it; the fleet
    // panel is #overlay's other child and is not this surface's business.
    assert!(!html.contains("id=\"hud-overlay\""));
    assert!(!html.contains("id=\"canvas\""));
    assert!(!html.contains("id=\"fleet-panel\""));

    // Nothing in either borrowed subtree reaches the internet. This is the
    // half `the_encoder_is_the_local_copy_because_a_bridge_has_no_internet`
    // cannot hold: that one runs against the stub above, which has no CDN
    // link in it to find, so it proves the assembly rather than the page. A
    // bridge machine is not assumed to have a network, so a `<script>`, an
    // `<img>` or a webfont added inside #lobby-panel or #qr-panel would
    // arrive here silently and be a lobby that could show a crew everything
    // except how to join. Scoped to the subtrees, because this document's
    // own head is written above and asserted for elsewhere.
    for (marker, missing, unbalanced) in [
        (
            LOBBY_PANEL_MARKER,
            HostLobbyDocumentError::NoLobbyPanel,
            HostLobbyDocumentError::UnbalancedLobbyPanel,
        ),
        (
            JOIN_PANEL_MARKER,
            HostLobbyDocumentError::NoJoinPanel,
            HostLobbyDocumentError::UnbalancedJoinPanel,
        ),
        (
            SCENARIO_PANEL_MARKER,
            HostLobbyDocumentError::NoScenarioPanel,
            HostLobbyDocumentError::UnbalancedScenarioPanel,
        ),
        (
            LANDING_PANEL_MARKER,
            HostLobbyDocumentError::NoLandingPanel,
            HostLobbyDocumentError::UnbalancedLandingPanel,
        ),
    ] {
        let subtree = extract_element(&page, marker, missing, unbalanced).unwrap();
        assert!(
            !subtree.contains("https://"),
            "{marker} carries an off-machine URL, which a native host cannot fetch"
        );
        assert!(!subtree.contains("http://"));
    }
}

#[test]
fn a_join_panel_that_never_closes_is_refused_by_its_own_name() {
    // The lobby's twin, and it needs its own case: both unbalanced errors
    // exist so an operator is told WHICH panel of the bundle is malformed,
    // and an error variant nothing constructs in a test is a name that can
    // quietly stop being reachable.
    let unbalanced = "<body><div id=\"lobby-panel\">x</div>\
                          <div id=\"qr-panel\"><div class=\"y\"></div></body>";
    assert_eq!(
        build_host_lobby_document(unbalanced),
        Err(HostLobbyDocumentError::UnbalancedJoinPanel)
    );
}

#[test]
fn the_repositorys_own_host_page_links_the_stylesheet_this_document_links() {
    // The other half of the extraction: if server.html went back to an
    // inline lobby block, this document would be styling itself from a file
    // nothing else used — the duplication, restored quietly.
    let page = std::fs::read_to_string("server.html").unwrap();
    assert!(
        page.contains("href=\"gui/host-lobby.css\""),
        "server.html must link the shared lobby stylesheet this document also links"
    );
    assert!(
        !page.contains(".lobby-panel-wrap {"),
        "the lobby's rules belong in gui/host-lobby.css, not back in server.html"
    );
    assert!(
        page.contains("href=\"gui/host-qr.css\""),
        "server.html must link the shared join-panel stylesheet this document also links"
    );
    assert!(
        !page.contains("#qr-panel {"),
        "the join panel's rules belong in gui/host-qr.css, not back in server.html"
    );
    assert!(
        page.contains("src=\"gui/vendor/qrcode.js\""),
        "both surfaces load the vendored encoder from the host's own server (issue #1329)"
    );
    assert!(
        page.contains("href=\"gui/host-scenarios.css\""),
        "server.html must link the shared picker stylesheet this document also links"
    );
    assert!(
        !page.contains("#scenario-panel {"),
        "the picker's rules belong in gui/host-scenarios.css, not back in server.html"
    );
    assert!(
        page.contains("href=\"gui/host-landing.css\""),
        "server.html must link the shared landing stylesheet this document also links"
    );
    assert!(
        !page.contains("#landing-panel {"),
        "the landing's rules belong in gui/host-landing.css, not back in server.html"
    );
}
