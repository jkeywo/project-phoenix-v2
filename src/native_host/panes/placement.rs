//! Where a pane's view is built — and the one place it must never be built
//! (issue #1333).
//!
//! A pane's view is created once, at one size, on one window, so *rebuilding* it
//! after a crash (issue #1125) or a move (issue #1331) is the moment the host
//! decides which screen a console lives on. Before this module that decision was
//! six lines inside `super::ultralight::open_pending_views` — which is behind
//! `--features ultralight`, so no CI job in this repository compiled it, let
//! alone ran it. The decision is policy, and policy nothing checks is exactly
//! the arrangement [`super::surface`] and [`super::recovery`] exist to avoid: it
//! lives here instead, pure and always compiled, and the adapter is left with
//! the Ultralight calls.
//!
//! # A console that has a screen is never rebuilt over the viewscreen
//!
//! That is the whole rule, and it is stated rather than emergent. The order is:
//!
//! 1. **The live Station slot wins.** [`BridgeStationSurfaces::slot_for`] is
//!    asked first, so a console the operator put on a wall monitor — whether the
//!    lobby's screen row seated it or a `--profile` authored it — is rebuilt on
//!    that monitor's own window, at the rectangle the layout gives it now.
//! 2. **A console the LAW seats has no other home.** If the layout seats a
//!    station of this name but the adapter has no slot for it this frame — a
//!    monitor between hot-plug frames, a Station window not yet rebuilt — the
//!    answer is [`NoHome::SeatedButUnplaced`] and *nothing is built*. Tiling it
//!    on the primary window would put a console the operator assigned to a wall
//!    screen straight over the shared view, which is the one failure this
//!    module's issue exists to make impossible. `bridge_display`'s
//!    `reconcile_seated_consoles` is what then repairs or honestly surrenders
//!    that seat, boundedly and with a notice on the row.
//! 3. **Otherwise the stored primary tile**, which is the legacy
//!    `--pane`-without-`--profile` host: panes tile left-to-right across the
//!    viewscreen window, and a crashed one is rebuilt on its own tile exactly as
//!    issue #1125 shipped it.
//! 4. **Otherwise nowhere**, with a reason, rather than a guess.
//!
//! # Only a TILED pane gets a stored tile
//!
//! [`PaneTile`] is recorded at `init_pane_host` for a pane that is genuinely
//! tiled on the primary window, and for no other. A `--pane Ada` the profile
//! seated on a Station window used to record one too — carrying that Station
//! monitor's *rectangle*, which on the primary window is an arbitrary offset
//! over the viewscreen. Rule 2 above already refuses to reach for a tile that a
//! seated station has, but a rectangle measured on one screen and stored as a
//! home on another is a trap with no reader left, so it is not recorded at all.

use bevy::prelude::Entity;

use crate::core::messages::StationId;
use crate::native_host::bridge_display::BridgeStationSurfaces;
use crate::native_host::bridge_layout::BridgeLayout;

/// Where a pane **tiled on the primary (viewscreen) window** sits, kept so a
/// pane recreated after a crash rebuilds in the same place (issue #1125).
///
/// Recorded per participant *name*, because that is what survives the [`PaneId`]
/// change a recreation makes. Only ever recorded for a pane that really is
/// tiled — see the [module note](self#only-a-tiled-pane-gets-a-stored-tile).
///
/// [`PaneId`]: super::registry::PaneId
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneTile {
    /// The participant name this tile belongs to.
    pub name: String,
    /// Top-left corner in the primary window's physical pixels.
    pub origin: (u32, u32),
    pub size: (u32, u32),
}

/// Why a pane's view is not built at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoHome {
    /// The live bridge layout **seats** a station of this pane's name, but no
    /// Station surface carries a slot for it right now.
    ///
    /// The honest answer, and never a primary tile: a console with a screen of
    /// its own does not fall back onto the shared view. The seat is
    /// `bridge_display::reconcile_seated_consoles`' to repair or give back.
    SeatedButUnplaced,
    /// Neither a Station slot nor a stored primary tile — nothing knows where
    /// this pane goes, so nothing guesses.
    Unplaced,
}

impl NoHome {
    /// A one-line reason for the operator log.
    ///
    /// Operator diagnostics, on the same footing as the rest of the pane host's
    /// log lines (inline English, not `strings.csv`): a file, a scrollback and a
    /// screenshot for whoever is running the bridge, never player-visible
    /// console text.
    pub fn reason(&self) -> &'static str {
        match self {
            NoHome::SeatedButUnplaced => {
                "its screen has no slot for it at the moment, and a console with a screen of its \
                 own is never rebuilt over the viewscreen; its seat is reconciled or given back"
            }
            NoHome::Unplaced => "it has no Station slot and no stored tile",
        }
    }
}

/// Where one pane's view is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaneHome {
    /// Composited into a slot on a Station window, on that window's own 2-D
    /// camera.
    Station {
        /// The Station window entity to render on.
        window: Entity,
        /// Top-left corner of the slot, in that window's physical pixels.
        origin: (u32, u32),
        size: (u32, u32),
        /// The monitor's device scale — the divisor from physical to page
        /// logical.
        scale: f64,
        /// The window's top-left on the virtual desktop, physical pixels, so a
        /// click resolves in the monitor's own coordinate space.
        window_origin: (i32, i32),
    },
    /// Tiled on the primary (viewscreen) window at the pane's stored tile, on
    /// the game's default UI camera — the legacy `--pane`-without-`--profile`
    /// host.
    PrimaryTile {
        origin: (u32, u32),
        size: (u32, u32),
    },
    /// Not built, for a stated reason.
    Nowhere(NoHome),
}

impl PaneHome {
    /// Whether this home is on a Station window rather than the viewscreen.
    pub fn is_station(&self) -> bool {
        matches!(self, PaneHome::Station { .. })
    }
}

/// Decide where the pane joining under `name` has its view built.
///
/// The four-step order in the [module note](self#a-console-that-has-a-screen-is-never-rebuilt-over-the-viewscreen),
/// and the whole of it: the caller supplies the live Station surfaces, the live
/// bridge layout and the stored primary tiles, and does nothing else with them.
///
/// `surfaces` is `None` on a host with no display adapter at all (a
/// `NativeRenderSurface::Contract` composition), and `layout` is `None` on the
/// same host and before the boot seed — in both cases there is no seating for a
/// console to be judged against, so a genuinely tiled pane still tiles.
pub fn home_for_pane(
    name: &str,
    surfaces: Option<&BridgeStationSurfaces>,
    layout: Option<&BridgeLayout>,
    tiles: &[PaneTile],
) -> PaneHome {
    // 1. The live Station slot wins. Resolved before anything else, because a
    //    console the operator put on a wall monitor must not be built on the
    //    viewscreen window instead.
    if let Some((surface, slot)) = surfaces.and_then(|s| s.slot_for(name)) {
        return PaneHome::Station {
            window: surface.window,
            origin: (slot.rect.x, slot.rect.y),
            // A zero-width slot would make Ultralight refuse the view; the
            // layout does not produce one, and clamping is cheaper than a
            // failure whose cause is a rounding rule three files away.
            size: (slot.rect.width.max(1), slot.rect.height.max(1)),
            scale: surface.geometry.scale_factor.max(0.1),
            window_origin: (surface.geometry.position_x, surface.geometry.position_y),
        };
    }
    // 2. A console the LAW seats has a screen of its own, so it has no business
    //    on the viewscreen even when that screen is momentarily unusable.
    if layout.is_some_and(|l| l.monitor_of(&StationId(name.to_string())).is_some()) {
        return PaneHome::Nowhere(NoHome::SeatedButUnplaced);
    }
    // 3. The legacy tile, unchanged since issue #1125. 4. Or nowhere.
    match tiles.iter().find(|t| t.name == name) {
        Some(tile) => PaneHome::PrimaryTile {
            origin: tile.origin,
            size: tile.size,
        },
        None => PaneHome::Nowhere(NoHome::Unplaced),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_host::bridge_display::{BridgeStationSurface, StationPane};
    use crate::native_host::bridge_profile::{MonitorGeometry, PaneRect};

    const BENQ: &str = "BenQ EX@1920x1080";

    fn station(id: &str) -> StationId {
        StationId(id.to_string())
    }

    fn geometry() -> MonitorGeometry {
        MonitorGeometry {
            physical_width: 1920,
            physical_height: 1080,
            position_x: 3840,
            position_y: 0,
            scale_factor: 1.5,
        }
    }

    /// A Station surface on the BenQ carrying one slot for `label`, seated for
    /// `station` when the layout placed it.
    fn surfaces_with(label: &str, station_of: Option<StationId>) -> BridgeStationSurfaces {
        BridgeStationSurfaces(vec![BridgeStationSurface {
            identity: BENQ.to_string(),
            window: Entity::from_raw_u32(7).expect("a test window handle"),
            monitor: Entity::PLACEHOLDER,
            geometry: geometry(),
            panes: vec![StationPane {
                label: label.to_string(),
                rect: PaneRect {
                    x: 960,
                    y: 0,
                    width: 960,
                    height: 1080,
                },
                station: station_of,
            }],
        }])
    }

    /// A layout over one screen with `helm` seated on it.
    fn layout_seating_helm() -> BridgeLayout {
        use crate::native_host::bridge_profile::{DiscoveredMonitor, MonitorIdentity};
        let discovered = vec![
            DiscoveredMonitor {
                identity: MonitorIdentity::new("DELL U2720Q@3840x2160"),
                geometry: MonitorGeometry {
                    physical_width: 3840,
                    physical_height: 2160,
                    position_x: 0,
                    position_y: 0,
                    scale_factor: 1.0,
                },
                name: Some("DELL U2720Q".to_string()),
                primary: true,
            },
            DiscoveredMonitor {
                identity: MonitorIdentity::new(BENQ),
                geometry: geometry(),
                name: Some("BenQ EX".to_string()),
                primary: false,
            },
        ];
        let seeded = BridgeLayout::from_discovered(&discovered, [station("helm")])
            .expect("two screens and a one-station roster seed a layout");
        seeded
            .apply(
                &crate::native_host::bridge_layout::LayoutAction::AssignStation {
                    station: station("helm"),
                    monitor: MonitorIdentity::new(BENQ),
                },
            )
            .expect("seating helm on the BenQ is lawful")
    }

    fn helm_tile() -> Vec<PaneTile> {
        vec![PaneTile {
            name: "helm".to_string(),
            origin: (0, 0),
            size: (1280, 720),
        }]
    }

    #[test]
    fn a_console_with_a_live_station_slot_is_built_on_that_station_window() {
        // Rule 1: the slot wins, and it carries the whole seat — the window, the
        // rectangle the layout gives it NOW, the monitor's scale, and the
        // window's own corner on the virtual desktop.
        let surfaces = surfaces_with("helm", Some(station("helm")));
        let home = home_for_pane(
            "helm",
            Some(&surfaces),
            Some(&layout_seating_helm()),
            &helm_tile(),
        );
        assert_eq!(
            home,
            PaneHome::Station {
                window: Entity::from_raw_u32(7).unwrap(),
                origin: (960, 0),
                size: (960, 1080),
                scale: 1.5,
                window_origin: (3840, 0),
            },
            "even with a stored tile sitting right there, the seat wins"
        );
    }

    #[test]
    fn a_seated_console_with_no_slot_is_left_unbuilt_rather_than_tiled_on_the_viewscreen() {
        // Rule 2, and the whole point of issue #1333. The law still seats helm —
        // the operator's row says so — but the adapter has no slot for it this
        // frame (a monitor between hot-plug frames, a Station window not yet
        // rebuilt). A tile is deliberately present and deliberately NOT used:
        // rebuilding a wall console over the shared view is the failure this
        // rule exists to make impossible.
        let home = home_for_pane(
            "helm",
            Some(&BridgeStationSurfaces::default()),
            Some(&layout_seating_helm()),
            &helm_tile(),
        );
        assert_eq!(home, PaneHome::Nowhere(NoHome::SeatedButUnplaced));
        assert!(
            !matches!(home, PaneHome::PrimaryTile { .. }),
            "never the viewscreen"
        );
    }

    #[test]
    fn a_legacy_tiled_pane_still_rebuilds_on_its_own_tile() {
        // Rule 3, untouched since issue #1125: a `--pane` on a host with no
        // `--profile` has no Station window anywhere, is seated by no law, and
        // rebuilds exactly where it was.
        assert_eq!(
            home_for_pane(
                "Ada",
                Some(&BridgeStationSurfaces::default()),
                Some(&layout_seating_helm()),
                &[PaneTile {
                    name: "Ada".to_string(),
                    origin: (960, 0),
                    size: (960, 1080),
                }],
            ),
            PaneHome::PrimaryTile {
                origin: (960, 0),
                size: (960, 1080)
            }
        );
    }

    #[test]
    fn a_host_with_no_display_adapter_at_all_still_tiles_its_panes() {
        // The `NativeRenderSurface::Contract` composition: no surfaces, no
        // layout, and the pre-#1123 single-window tiling is all there is.
        assert_eq!(
            home_for_pane("Ada", None, None, &helm_tile()),
            PaneHome::Nowhere(NoHome::Unplaced),
            "a name with no tile of its own is still nowhere"
        );
        assert_eq!(
            home_for_pane(
                "Ada",
                None,
                None,
                &[PaneTile {
                    name: "Ada".to_string(),
                    origin: (0, 0),
                    size: (1280, 720),
                }],
            ),
            PaneHome::PrimaryTile {
                origin: (0, 0),
                size: (1280, 720)
            }
        );
    }

    #[test]
    fn an_authored_participants_slot_is_a_station_home_like_any_other() {
        // A `--profile` `[[display.pane]]` with a label and no station key is a
        // participant's pane on a Station window (`StationPane::station` is
        // `None`). It is still SEATED — it has a screen of its own — so it is
        // built there, not tiled.
        let surfaces = surfaces_with("Ada", None);
        assert!(
            home_for_pane("Ada", Some(&surfaces), Some(&layout_seating_helm()), &[]).is_station(),
            "an authored participant's console lives on its Station window too"
        );
    }

    #[test]
    fn a_name_nothing_knows_is_refused_rather_than_guessed_at() {
        assert_eq!(
            home_for_pane(
                "Grace",
                Some(&BridgeStationSurfaces::default()),
                Some(&layout_seating_helm()),
                &helm_tile(),
            ),
            PaneHome::Nowhere(NoHome::Unplaced)
        );
    }

    #[test]
    fn a_console_the_law_stopped_seating_may_take_a_tile_again() {
        // The rule is about what the LAW says now, not about what a name once
        // was: a station whose seat was surrendered is no longer a wall console,
        // so nothing is being kept off the viewscreen any more.
        let free = layout_seating_helm()
            .apply(
                &crate::native_host::bridge_layout::LayoutAction::UnassignStation {
                    station: station("helm"),
                },
            )
            .expect("giving a seat back is lawful");
        assert_eq!(
            home_for_pane(
                "helm",
                Some(&BridgeStationSurfaces::default()),
                Some(&free),
                &helm_tile(),
            ),
            PaneHome::PrimaryTile {
                origin: (0, 0),
                size: (1280, 720)
            }
        );
    }
}
