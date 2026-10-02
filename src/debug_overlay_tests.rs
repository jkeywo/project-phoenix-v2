use super::*;

#[test]
fn debug_regions_disabled_by_default() {
    let plugin = DebugOverlayPlugin { enabled: false };
    let mut app = App::new();
    plugin.build(&mut app);
    let enabled = app.world().resource::<DebugRegionsEnabled>();
    assert!(!enabled.0, "default should be disabled");
}

#[test]
fn debug_regions_enabled_when_flag_set() {
    let plugin = DebugOverlayPlugin { enabled: true };
    let mut app = App::new();
    plugin.build(&mut app);
    let enabled = app.world().resource::<DebugRegionsEnabled>();
    assert!(enabled.0, "should be enabled when flag is set");
}

/// Toggling the resource from false → true should flip DebugRegionsEnabled.
#[test]
fn toggle_debug_regions_false_to_true() {
    let plugin = DebugOverlayPlugin { enabled: false };
    let mut app = App::new();
    plugin.build(&mut app);
    // Simulate what the catalogue adapter does: flip the resource.
    app.world_mut().resource_mut::<DebugRegionsEnabled>().0 = true;
    let enabled = app.world().resource::<DebugRegionsEnabled>();
    assert!(enabled.0, "resource should be true after toggle");
}

/// Toggling the resource from true → false should flip DebugRegionsEnabled.
#[test]
fn toggle_debug_regions_true_to_false() {
    let plugin = DebugOverlayPlugin { enabled: true };
    let mut app = App::new();
    plugin.build(&mut app);
    app.world_mut().resource_mut::<DebugRegionsEnabled>().0 = false;
    let enabled = app.world().resource::<DebugRegionsEnabled>();
    assert!(!enabled.0, "resource should be false after toggle");
}

// ── DebugOverlayEnabled tests ─────────────────────────────────────────

#[test]
fn debug_overlay_disabled_by_default() {
    let plugin = DebugOverlayPlugin { enabled: false };
    let mut app = App::new();
    plugin.build(&mut app);
    let enabled = app.world().resource::<DebugOverlayEnabled>();
    assert!(!enabled.0, "overlay should be disabled by default");
}

#[test]
fn toggle_debug_overlay_false_to_true() {
    let plugin = DebugOverlayPlugin { enabled: false };
    let mut app = App::new();
    plugin.build(&mut app);
    app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = true;
    let enabled = app.world().resource::<DebugOverlayEnabled>();
    assert!(enabled.0, "overlay should be enabled after toggle");
}

#[test]
fn toggle_debug_overlay_true_to_false() {
    let plugin = DebugOverlayPlugin { enabled: false };
    let mut app = App::new();
    plugin.build(&mut app);
    app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = true;
    app.world_mut().resource_mut::<DebugOverlayEnabled>().0 = false;
    let enabled = app.world().resource::<DebugOverlayEnabled>();
    assert!(!enabled.0, "overlay should be disabled after second toggle");
}

// ── DamageLog tests ───────────────────────────────────────────────────

fn entry(source: &str, arc: Option<&str>, amount: f32) -> DamageLogEntry {
    DamageLogEntry {
        source: source.to_string(),
        shield_arc: arc.map(|s| s.to_string()),
        amount,
    }
}

#[test]
fn damage_log_starts_empty() {
    let log = DamageLog::default();
    assert!(log.entries.is_empty());
}

#[test]
fn damage_log_pushes_newest_to_front() {
    let mut log = DamageLog::default();
    log.push(entry("a", Some("Fore"), 1.0));
    log.push(entry("b", Some("Port"), 2.0));
    assert_eq!(log.entries.len(), 2);
    assert_eq!(log.entries[0].source, "b");
    assert_eq!(log.entries[1].source, "a");
}

#[test]
fn damage_log_caps_at_capacity() {
    let mut log = DamageLog::default();
    for i in 0..(DAMAGE_LOG_CAPACITY + 5) {
        log.push(entry(&format!("s{}", i), None, i as f32));
    }
    assert_eq!(log.entries.len(), DAMAGE_LOG_CAPACITY);
    // Newest at front
    assert_eq!(
        log.entries[0].source,
        format!("s{}", DAMAGE_LOG_CAPACITY + 4)
    );
    // Oldest retained is the one DAMAGE_LOG_CAPACITY back from newest
    assert_eq!(log.entries[DAMAGE_LOG_CAPACITY - 1].source, "s5");
}

// `damage_log_format_includes_source_arc_and_amount` retired with
// `DamageLog::format` (issue #1150). The same facts — source, arc and
// amount surviving into the surface, newest first — are now pinned on the
// structured projection by
// `crate::debug::damage::tests::projection_preserves_newest_first_order_and_facts`.

#[test]
fn debug_damage_disabled_by_default() {
    let plugin = DebugOverlayPlugin { enabled: false };
    let mut app = App::new();
    plugin.build(&mut app);
    let enabled = app.world().resource::<DebugDamageEnabled>();
    assert!(!enabled.0, "damage overlay should be disabled by default");
}

// ── The phone client's settings routes (issue #940) ─────────────────────
//
// Gated as a whole: in a demo build neither drain exists, so there is
// nothing here to test. That the routes are ABSENT there is pinned from
// both builds by `codec`'s two
// `*_route_is_absent_from_a_demo_build` tests, which ask the question
// through the wire rather than through a predicate this module owns.

#[cfg(not(phoenix_demo_build))]
mod client_route {
    use super::*;
    use crate::core::debug_surface::DebugSurface;

    /// Only tokens in this list count as connected players.
    fn connected<'a>(known: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
        move |token| known.contains(&token)
    }

    /// The happy path: a connected player's flags reach the pending set as
    /// the canonical surface identities, in submission order.
    #[test]
    fn a_connected_players_flag_is_admitted() {
        let kinds = admitted_flag_toggles(
            [
                ("phone", DebugSurface::Regions),
                ("phone", DebugSurface::Damage),
            ],
            connected(&["phone"]),
        );
        assert_eq!(kinds, vec![DebugSurface::Regions, DebugSurface::Damage]);
    }

    /// A token nobody registered is refused. The route widens *who* may
    /// flip a debug flag, not *whether* a sender has to exist.
    #[test]
    fn an_unregistered_token_is_refused() {
        assert!(
            admitted_flag_toggles([("ghost", DebugSurface::Regions)], connected(&["phone"]),)
                .is_empty()
        );
    }

    // `an_admitted_batch_flips_only_the_flags_it_names` — the round-trip that
    // feeds `admitted_flag_toggles`' output into the catalogue applier —
    // moved to `server::bridge::tests` with the drain and the marshalling it
    // exercises (issue #1193). The two tests above keep the sim-side authority
    // filter (`admitted_flag_toggles`) covered here.

    /// Pause is a toggle, so an even number of admitted taps in one frame
    /// is a no-op and an odd number is one flip. Collapsing the batch to
    /// "someone asked" would turn a double-tap into a pause.
    #[test]
    fn pause_taps_are_counted_not_collapsed() {
        let known = ["phone", "other"];
        assert_eq!(admitted_pause_toggles(["phone"], connected(&known)), 1);
        assert_eq!(
            admitted_pause_toggles(["phone", "other"], connected(&known)),
            2
        );
    }

    /// The same identity rule the flags use: a token nobody registered
    /// cannot stop the clock, even in a dev build.
    #[test]
    fn an_unregistered_token_cannot_pause() {
        assert_eq!(
            admitted_pause_toggles(["ghost"], connected(&["phone"])),
            0,
            "an unidentified sender must not reach the simulation clock"
        );
    }
}
