//! The Bevy adapter for [`super::layout_store`] (issue #1334): **remember the
//! bridge**.
//!
//! Two systems and one resource. The pure half — the class key, the file, the
//! atomic write, the re-validation — is the sibling module, which has no Bevy
//! import; this is the part that knows *when*.
//!
//! ```text
//!   the hull becomes known ──▶ adopt_remembered_layout  (once per ship class)
//!                                 │  reconcile onto the hull's roster
//!                                 │  load + adopt this class's saved file
//!                                 ▼
//!   an accepted lobby press ──▶ remember_bridge_layout  (on every change)
//!                                 │  compare against what the file holds
//!                                 ▼
//!                              layout_store::save
//! ```
//!
//! # When a layout can be pre-applied, and why it is not at boot
//!
//! A saved layout is filed **per ship class**, so it cannot be applied until
//! the hull is known — and the two world-arrival paths learn that at different
//! moments. A `--world` host resolves its hull in `app::install_world_selection`
//! before the `App` runs; a `--lobby` host (issue #1326) resolves it at the
//! *pick*, minutes later, on a running world. [`adopt_remembered_layout`] is
//! written against the state rather than against either path: it fires the frame
//! [`SelectedShipResource`](crate::lobby::SelectedShipResource) first names a
//! class this host is not already remembering, which is the same line for both.
//!
//! It does two things in that one moment, and the first is not optional:
//!
//!  1. **Reconcile onto the hull's roster.** The live layout was seeded by
//!     `bridge_display::apply_bridge_profile` from whatever roster existed then
//!     — the hull's on a `--world` boot, and *nothing at all* on a `--lobby`
//!     one, which has no `PendingShipConfig` until the pick. Without this a
//!     world-less host would carry an empty roster for the rest of the run, and
//!     the lobby's per-station screen rows (a `map` over
//!     [`BridgeLayout::eligibility`](crate::native_host::bridge_layout::BridgeLayout::eligibility))
//!     would be empty with it.
//!  2. **Adopt the saved file** onto that reconciled bridge, through
//!     [`BridgeLayout::adopt_profile`](crate::native_host::bridge_layout::BridgeLayout::adopt_profile)
//!     — the same door a hand-authored `--profile` comes through, so a saved
//!     layout gets no privileges a written one does not have. A monitor that is
//!     no longer plugged in is a `SeatRefused` note and an unassigned station,
//!     never an error; a saved viewscreen whose screen is gone leaves the
//!     bridge on the one it booted with. That is issue #1334's changed-monitor
//!     criterion, and it is the law's answer rather than a second one here.
//!
//! Notes from both are **logged, not pushed to the lobby's notice row**, for the
//! reason
//! [`BridgeLayoutResource::notices`](super::bridge_display::BridgeLayoutResource)
//! already gives about boot-time adoption: this is a bridge arriving, not an
//! answer to a press, and a wall of notes would bury the one that answers one.
//!
//! # `--profile` wins, and never writes (the data-loss guard)
//!
//! Both systems are gated on the run *not* being authored
//! ([`BridgeDisplayConfig::authored`](super::bridge_display::BridgeDisplayConfig)),
//! so a host launched with `--profile` uses that profile verbatim, skips the
//! pre-apply, and files nothing. That is issue #1334's precedence rule, and it
//! is also a **data-loss guard**, which is why it is stated twice and tested
//! directly.
//!
//! [`BridgeLayout::to_validated_profile`](crate::native_host::bridge_layout::BridgeLayout::to_validated_profile)
//! emits seats and only seats: the authored `--pane` participant surfaces a
//! layout merely *knows about*
//! ([`reserved_on`](crate::native_host::bridge_layout::BridgeLayout::reserved_on))
//! do not come back out. So persisting a `--profile`-seeded layout would
//! silently drop the operator's `[[display.pane]]` entries from the file, and
//! the next boot would read those screens as free and move the viewscreen on top
//! of a crew member's live console — the exact failure `reserved` exists to
//! prevent, re-introduced by the save. The never-write-on-authored rule makes it
//! unreachable, and [`super::layout_store::LayoutStore::save`] refuses a
//! reservation-carrying layout outright so a *second* caller cannot re-open it.
//!
//! **A saved layout is therefore only ever a lobby-built one, and lobby-built
//! layouts are reservation-free by construction**: nothing but `adopt_profile`
//! populates `reserved`, and nothing but an authored `--profile` reaches it.

use bevy::prelude::*;

use crate::logging::{LogCat, LogFilterConfig};

use super::bridge_display::{BridgeDisplayConfig, BridgeLayoutResource};
use super::bridge_layout::BridgeLayout;
use super::layout_store::{LayoutStore, ShipClassKey};

/// Where this host files its per-ship-class layouts, and what it currently
/// believes is on disk.
///
/// **Its presence is the switch.** `app::build_native_host_app` inserts one for
/// a host that has somewhere to save to, and does not for a `--profile` run or
/// for a machine with no resolvable home directory — so a host that cannot (or
/// must not) remember its bridge carries no resource, and both systems below are
/// inert through an ordinary `resource_exists` rather than through a flag each
/// of them has to remember to read.
#[derive(Resource, Clone, Debug)]
pub struct BridgeLayoutStore {
    /// The directory. Injected — the tests point it at a scratch directory, and
    /// [`LayoutStore::user`] is the only thing that ever resolves the real one.
    pub store: LayoutStore,
    /// The class this host is remembering, and the arrangement its file holds.
    /// `None` until the hull is known.
    ///
    /// It carries the *saved* layout rather than a "dirty" flag so the writer
    /// can answer "has anything actually changed" by value. A `ResMut` deref
    /// that wrote the same arrangement back marks the law's resource changed
    /// without changing it, and a writer keyed on change detection alone would
    /// rewrite the file — and log a line — for a bridge nobody touched.
    pub remembered: Option<Remembered>,
}

/// One ship class's remembered bridge: the key it is filed under, the
/// arrangement the file holds, and the bridge that arrangement was agreed on.
#[derive(Clone, Debug)]
pub struct Remembered {
    pub class: ShipClassKey,
    /// What [`super::layout_store::LayoutStore::save`] last wrote — or, before
    /// the first write, the arrangement this host started the class with, so
    /// that a class nobody has rearranged yet creates **no file**. "Created on
    /// first write" is the acceptance criterion, and this is where it is true.
    pub saved: BridgeLayout,
    /// The monitors the layout had when [`saved`](Self::saved) was agreed.
    ///
    /// This is how the writer tells **an operator changing their bridge** from
    /// **their bridge changing under them** — see
    /// [`remember_bridge_layout`]'s note on the cable. Identities rather than
    /// [`DiscoveredMonitor`](super::bridge_profile::DiscoveredMonitor)s,
    /// because a monitor's geometry is not part of its identity and
    /// `bridge_display::reconcile_layout` deliberately rebuilds nothing when a
    /// display merely moves or renegotiates its mode: a television waking up
    /// must not count as the bridge changing.
    pub bridge: Vec<super::bridge_profile::MonitorIdentity>,
}

/// Installs the two systems. Both are inert without a [`BridgeLayoutStore`].
pub struct BridgeLayoutStorePlugin;

impl Plugin for BridgeLayoutStorePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (adopt_remembered_layout, remember_bridge_layout)
                .chain()
                // Both need the store, and one of them is the whole reason it
                // exists — see the resource's note on presence being the switch.
                .run_if(resource_exists::<BridgeLayoutStore>)
                // And nothing to read or write until winit has reported its
                // displays and `apply_bridge_profile` has seeded the law.
                .run_if(resource_exists::<BridgeLayoutResource>)
                .run_if(lobby_built)
                // AFTER THE WHOLE DISPLAY ADAPTER, not merely after the system
                // that seeds the layout. `adopt_remembered_layout` takes
                // `ResMut<BridgeLayoutResource>`, which `follow_layout_stations`
                // and `watch_runtime_displays` also want — so with only an
                // `.after(apply_bridge_profile)` the executor would serialise
                // this against them in an order that is arbitrary but silent,
                // and the consoles a remembered layout seats would open on the
                // seed frame or the one after it depending on how the run went.
                //
                // Ordered after the set, it is always the frame after: this
                // adopts in the frame the layout is first seeded (the resource
                // exists by then — `apply_bridge_profile` is exclusive and
                // inserts it directly), and `follow_layout_stations` opens the
                // consoles on the next pass. That is the same one-frame settle a
                // lobby press already takes.
                .after(super::bridge_display::BridgeDisplaySet),
        );
    }
}

/// Whether this host's arrangement is the lobby's to remember.
///
/// `false` for a run an operator gave `--profile` — that profile is used
/// verbatim, nothing is pre-applied over it, and nothing is written back. See
/// the [module note](self#--profile-wins-and-never-writes-the-data-loss-guard).
///
/// `false` too before `apply_bridge_profile` has run at all, which costs
/// nothing: the layout it seeds does not exist yet either, and the run condition
/// above has already stopped on that.
fn lobby_built(config: Option<Res<BridgeDisplayConfig>>) -> bool {
    config.is_some_and(|c| !c.authored)
}

/// Apply this ship class's remembered bridge, the moment the hull is known.
///
/// Runs at most once per class — see the
/// [module note](self#when-a-layout-can-be-pre-applied-and-why-it-is-not-at-boot)
/// for why that moment is `SelectedShipResource` rather than boot, and why the
/// roster reconcile happens here too.
///
/// A hull whose path yields no class key, and a store that cannot be read, are
/// both **warnings**: this host then runs on the bridge it already has, which is
/// exactly today's behaviour, and the operator arranges it by hand. Nothing here
/// can fail a boot.
fn adopt_remembered_layout(
    mut store: ResMut<BridgeLayoutStore>,
    mut layout: ResMut<BridgeLayoutResource>,
    ship: Option<Res<crate::lobby::SelectedShipResource>>,
    hull: Option<Res<crate::ship::components::PendingShipConfig>>,
    log: Option<Res<LogFilterConfig>>,
) {
    // The hull-known moment: both resources land together, in
    // `install_world_selection`, on either world-arrival path.
    let (Some(ship), Some(hull)) = (ship, hull) else {
        return;
    };
    let Some(class) = ShipClassKey::from_template_path(&ship.0) else {
        // Reached only through a hull path with no usable file stem, which
        // `install_world_selection`'s own template-cache gate would already have
        // refused. Warned rather than ignored because the consequence is silent:
        // the bridge simply never remembers anything.
        crate::pwarn!(
            log,
            LogCat::Lobby,
            "bridge layouts: the hull path {:?} yields no ship-class name, so this bridge's \
             arrangement cannot be remembered",
            ship.0
        );
        return;
    };
    if store.remembered.as_ref().is_some_and(|r| r.class == class) {
        return;
    }

    // 1. The roster. A `--world` host was seeded with this exact list and the
    //    reconcile is a no-op; a `--lobby` host was seeded with none, and this
    //    is where its station rows come from.
    let roster: Vec<crate::core::messages::StationId> =
        hull.0.stations.iter().map(|s| s.id.clone()).collect();
    let monitors = layout.monitors.clone();
    let (reconciled, notes) = layout.layout.reconcile(&monitors, roster);
    for note in &notes {
        crate::pwarn!(log, LogCat::Lobby, "bridge layouts: {note}");
    }

    // 2. The saved file, through the law's own door.
    let next = match store.store.load(&class) {
        Ok(Some(profile)) => {
            let (adopted, notes) = reconciled.adopt_profile(&profile);
            for note in &notes {
                crate::pwarn!(log, LogCat::Lobby, "bridge layouts: {note}");
            }
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "bridge layouts: {} remembers a bridge — the viewscreen on {}, {} console(s) \
                 reopening on the screens they were left on",
                class,
                adopted.viewscreen(),
                adopted
                    .roster()
                    .iter()
                    .filter(|s| adopted.monitor_of(s).is_some())
                    .count()
            );
            adopted
        }
        Ok(None) => {
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "bridge layouts: nothing saved for {class} yet; the first arrangement made from \
                 the lobby writes {}",
                store.store.path_for(&class).display()
            );
            reconciled
        }
        Err(e) => {
            // Acceptance criterion: IGNORED with a warning, never fatal. The
            // file stays where it is — an operator who hand-edited it wants to
            // see what they wrote, not to find it deleted — and this session
            // runs on the bridge it booted with.
            crate::pwarn!(
                log,
                LogCat::Lobby,
                "bridge layouts: {e} — it is ignored for this run and the bridge keeps the \
                 arrangement it booted with"
            );
            reconciled
        }
    };

    // Written through a value compare so the frame this runs on a `--world`
    // host, where the reconcile is a no-op and there is no saved file, does not
    // mark the law's resource changed and republish an identical row.
    if layout.layout != next {
        layout.layout = next.clone();
    }
    store.remembered = Some(Remembered {
        class,
        bridge: next.monitors().to_vec(),
        saved: next,
    });
}

/// File the live arrangement whenever the **operator** has moved it away from
/// what this class's saved layout holds.
///
/// # Write on every accepted change
///
/// [ai] Not debounced, and not deferred to shutdown. A saved layout is a handful
/// of `[[display]]` tables — hundreds of bytes — written through one atomic
/// rename, so the cost of writing it on each accepted press is a rounding error
/// beside the frame that press already caused. What the alternatives cost is
/// real: a debounce needs a timer, a flush on the last edit, and an answer for a
/// host killed inside the window (the arrangement the operator just made is the
/// one they lose); a write on exit needs a shutdown hook a windowed process is
/// not guaranteed to reach — a `phoenix-host` closed from the taskbar, or a
/// display driver taking the process with it, would lose the whole session's
/// work. The simplest trigger is also the only one with no lossy state.
///
/// The `saved` value compare is what keeps that honest: this system is on the
/// law's change detection, and change detection answers "did somebody hold a
/// `ResMut`", not "is the bridge different". A reconcile that rebuilt an
/// identical layout, or a press the no-op doctrine accepted without changing
/// anything, marks the resource and must not rewrite the file.
///
/// # A cable coming out is not the operator changing their mind
///
/// [ai] The saved layout is the operator's **intent**, and the law's resource
/// has a second writer that is not them: `bridge_display`'s reconcile, which
/// degrades a station whose monitor has gone to unassigned. Writing *that* would
/// mean a screen blipping through a dock permanently forgets where two consoles
/// were, on a bridge nobody touched — and it is the same silent loss issue
/// #1123's never-re-home doctrine exists to refuse, arriving through the file
/// instead of through a window.
///
/// So the trigger is "the arrangement changed while **the bridge did not**",
/// which is what [`Remembered::bridge`] is for. A frame that changed the monitor
/// set **re-baselines and writes nothing**: the file keeps the fuller
/// arrangement, and the next press — the operator putting that console
/// somewhere real, which is exactly how #1334's changed-monitor criterion says
/// it comes back — writes the updated layout, because by then the bridge and the
/// baseline agree again. It is also what makes the *cross-session* case right:
/// launching on a laptop with two of four screens adopts a degraded layout and
/// writes nothing, so the full arrangement is still there next time the bridge
/// is whole.
///
/// Two honest edges. A press landing in the same frame as an unplug is
/// re-baselined with it, so that one press reaches the file only on the next
/// one — a rare race, with the bridge on screen correct throughout.  And
/// `reconcile_seated_consoles` surrendering a seat it could not build a console
/// for (`ConsoleCouldNotOpen`) changes no monitor, so it *is* filed; that is a
/// real state the operator is told about, and re-pressing writes it back.
///
/// A failed write leaves `saved` alone, so the next accepted change tries again
/// rather than the host giving up on the file for the rest of the run.
fn remember_bridge_layout(
    mut store: ResMut<BridgeLayoutStore>,
    layout: Res<BridgeLayoutResource>,
    log: Option<Res<LogFilterConfig>>,
) {
    // Nothing to file until a class is known — which is also what stops the boot
    // seed being written over a saved layout that has not been read yet.
    let Some(remembered) = store.remembered.as_ref() else {
        return;
    };
    if remembered.saved == layout.layout {
        return;
    }
    let class = remembered.class.clone();
    if remembered.bridge != layout.layout.monitors() {
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "bridge layouts: the bridge changed under this host, so {class}'s saved layout is \
             left as it is rather than overwritten with the degraded arrangement; the next \
             change made from the lobby files the new one"
        );
        if let Some(remembered) = store.remembered.as_mut() {
            remembered.saved = layout.layout.clone();
            remembered.bridge = layout.layout.monitors().to_vec();
        }
        return;
    }
    match store.store.save(&class, &layout.layout) {
        Ok(path) => {
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "bridge layouts: {class}'s bridge saved to {}",
                path.display()
            );
            if let Some(remembered) = store.remembered.as_mut() {
                remembered.saved = layout.layout.clone();
            }
        }
        Err(e) => crate::pwarn!(
            log,
            LogCat::Lobby,
            "bridge layouts: {e}. The bridge on screen is unaffected; the next change tries again"
        ),
    }
}

#[cfg(test)]
#[path = "layout_store_systems_tests.rs"]
mod tests;
