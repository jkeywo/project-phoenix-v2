//! Prepare one finite batch of pending console views before the renderer runs.
//!
//! This SDK-free lifecycle owns retirement filtering and placement faults.
//! Recovery is serviced later in the frame, so retries cannot enter this batch.

use crate::native_host::bridge_display::BridgeStationSurfaces;
use crate::native_host::bridge_layout::BridgeLayout;

use super::placement::{home_for_pane, NoHome, PaneHome, PaneTile};
use super::{PaneBus, PaneFault, PaneId};

/// The outcome of preparing one queued view. Build homes are always Station
/// or PrimaryTile; an absent home is represented by Unplaced instead.
#[derive(Clone, Debug, PartialEq)]
pub enum PendingViewDisposition {
    Build {
        id: PaneId,
        name: String,
        url: String,
        home: PaneHome,
    },
    Retired {
        id: PaneId,
    },
    Unplaced {
        id: PaneId,
        name: String,
        reason: NoHome,
    },
}

/// Drain once, resolve the live home, and enqueue faults for retryable absence.
/// This eagerly finishes the whole batch before the adapter can render any of
/// it. It never services recovery or drains newly queued replacement views.
pub fn prepare_pending_views(
    bus: &PaneBus,
    surfaces: Option<&BridgeStationSurfaces>,
    layout: Option<&BridgeLayout>,
    tiles: &[PaneTile],
) -> Vec<PendingViewDisposition> {
    bus.take_pending_views()
        .into_iter()
        .map(|(id, url)| {
            if !bus.is_open(id) || bus.is_superseded(id) {
                return PendingViewDisposition::Retired { id };
            }
            let Some(name) = bus.name_of(id) else {
                return PendingViewDisposition::Retired { id };
            };
            let home = home_for_pane(&name, surfaces, layout, tiles);
            if let PaneHome::Nowhere(reason) = home {
                if reason.should_retry() {
                    bus.fault(id, PaneFault::ViewCrashed);
                }
                PendingViewDisposition::Unplaced { id, name, reason }
            } else {
                PendingViewDisposition::Build {
                    id,
                    name,
                    url,
                    home,
                }
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "pending_views_tests.rs"]
mod tests;
