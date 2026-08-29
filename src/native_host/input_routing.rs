//! Routing native input to bridge panes (issue #1124) — **pure, Bevy-free**.
//!
//! [`super::bridge_profile`] says which monitor is which and where each console
//! pane sits on it; [`super::bridge_display`] opens the windows. This module is
//! the third pure piece: given a physical pointer or touch coordinate and the
//! resolved layout, *which pane does it belong to, and where inside that pane*.
//! It is the whole of the input-routing logic — the coordinate transforms, the
//! pane-boundary hit test, the keyboard-focus order, and the touch
//! contact-capture map — and it has no Bevy, no winit and no window in it, which
//! is the point: every acceptance criterion with arithmetic or a rule in it is
//! decided here and checked by the ordinary `cargo test` CI runs. The winit/Bevy
//! adapter that reads real `CursorMoved`/`MouseButton`/`KeyboardInput`/`TouchInput`
//! per window and injects the resolved event into an Ultralight view is the thin
//! feature-gated layer in [`super::panes::ultralight`], provable only under the
//! `#[ignore]`d integration test on a machine with the displays and the SDK.
//!
//! # The three models
//!
//! * [`PaneRouter`] — the spatial map. A flat list of [`PanePlacement`]s (each a
//!   pane, the window it is composited on, its rectangle in that window's
//!   physical pixels, the window origin on the virtual desktop, and the scale
//!   factor) plus two resolutions: [`PaneRouter::resolve_in_window`] for an event
//!   a window delivered in its own coordinates (a mouse over that window, a touch
//!   on that screen), and [`PaneRouter::resolve_desktop`] for a device-global
//!   physical point (a touchscreen mapped to a monitor by the profile). Both
//!   answer with the pane and the pointer position in that pane's **own logical
//!   pixels**, which is what an Ultralight view's `mouse_move` wants.
//! * [`FocusRing`] — the keyboard-focus order over the panes and which one holds
//!   focus. Traversal cycles; a closed pane is reconciled out without silently
//!   handing focus to whoever inherits its slot.
//! * [`ContactCaptureMap`] — the per-contact pin. A touch is resolved to a pane
//!   at the instant it *starts* and stays captured by that pane until it lifts,
//!   however far the finger then drifts; contacts on different screens are
//!   independent because each is one entry keyed by its own id.
//!
//! # Coordinate spaces, stated once
//!
//! Everything spatial here is in **physical pixels** on the input side and
//! **logical (page CSS) pixels** on the output side, because those are the two
//! the rest of the stack already speaks: [`super::bridge_profile::PaneRect`] and
//! [`super::bridge_profile::MonitorGeometry`] are physical, and a pane's
//! Ultralight view is created with a device scale equal to its window's, so a
//! logical coordinate handed to `mouse_move` lands on the same CSS pixel a phone
//! would touch. A pane-local logical coordinate is therefore `(physical −
//! pane_origin) / scale`, and that one division is the whole of "display scaling"
//! in acceptance criterion 3.

use std::collections::BTreeMap;

use super::bridge_profile::PaneRect;
use super::panes::registry::PaneId;

/// Which OS window (and so which monitor) a pane is composited on.
///
/// Opaque to this module — it only ever compares two of them for equality. The
/// Bevy adapter fills it with something stable per window (an `Entity`'s bits, or
/// a monitor index for the single-window host), so an event a window delivers can
/// be routed only among the panes on *that* window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowKey(pub u64);

/// Where one pane lives, for the purpose of routing input to it.
///
/// The rectangle is in the window's own **physical** pixels with the origin at
/// the window's top-left; the window origin locates that window on the virtual
/// desktop (also physical), so a device-global point can be mapped to a window
/// and then to a pane. `scale_factor` is the window/monitor scale, and the only
/// thing that turns a physical hit into the logical coordinate an Ultralight view
/// wants.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanePlacement {
    /// The pane this rectangle belongs to.
    pub pane: PaneId,
    /// The window it is composited on — a mouse or touch that window delivers is
    /// routed only among placements sharing this key.
    pub window: WindowKey,
    /// The window's top-left on the virtual desktop, physical pixels. `(0, 0)`
    /// for the single-window host, whose events already arrive window-local.
    pub window_origin_x: i32,
    pub window_origin_y: i32,
    /// The pane's rectangle within its window, physical pixels, origin at the
    /// window top-left.
    pub rect: PaneRect,
    /// The window/monitor scale factor. A pane-local logical coordinate is the
    /// physical offset into [`rect`](Self::rect) divided by this.
    pub scale_factor: f64,
}

impl PanePlacement {
    /// Whether a **window-local** physical point falls inside this pane.
    ///
    /// Low edge inclusive, high edge exclusive, so the shared boundary between
    /// two tiled panes belongs to the right/bottom one and no point is claimed by
    /// two panes at once.
    fn window_contains(&self, wx: f64, wy: f64) -> bool {
        let x0 = self.rect.x as f64;
        let y0 = self.rect.y as f64;
        wx >= x0
            && wx < x0 + self.rect.width as f64
            && wy >= y0
            && wy < y0 + self.rect.height as f64
    }

    /// Whether a **desktop-global** physical point falls inside this pane.
    fn desktop_contains(&self, x: f64, y: f64) -> bool {
        self.window_contains(
            x - self.window_origin_x as f64,
            y - self.window_origin_y as f64,
        )
    }

    /// A window-local physical point turned into this pane's own logical
    /// coordinate. Assumes the point is inside [`rect`](Self::rect).
    fn local_of_window(&self, wx: f64, wy: f64) -> PaneHit {
        let scale = if self.scale_factor > 0.0 {
            self.scale_factor
        } else {
            1.0
        };
        PaneHit {
            pane: self.pane,
            local_x: ((wx - self.rect.x as f64) / scale) as i32,
            local_y: ((wy - self.rect.y as f64) / scale) as i32,
        }
    }
}

/// A resolved hit: which pane, and the pointer position in that pane's **own
/// logical (page CSS) pixels** — exactly what an Ultralight view's `mouse_move`,
/// `mouse_down` and `mouse_up` take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneHit {
    pub pane: PaneId,
    pub local_x: i32,
    pub local_y: i32,
}

/// The spatial map from a physical coordinate to the pane under it.
///
/// Built fresh by the adapter whenever the layout changes (a pane opens or
/// closes, a profile applies). Placement order is the operator's order — window
/// by window, panes left-to-right within a window — and is also the keyboard
/// focus order [`focus_order`](Self::focus_order) hands to a [`FocusRing`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaneRouter {
    placements: Vec<PanePlacement>,
}

impl PaneRouter {
    /// A router over these placements, in order.
    pub fn new(placements: Vec<PanePlacement>) -> Self {
        Self { placements }
    }

    /// Whether any pane is placed at all.
    pub fn is_empty(&self) -> bool {
        self.placements.is_empty()
    }

    /// How many panes are placed.
    pub fn len(&self) -> usize {
        self.placements.len()
    }

    /// Every placement, in order.
    pub fn placements(&self) -> &[PanePlacement] {
        &self.placements
    }

    /// The panes in traversal order — the order they were placed.
    pub fn focus_order(&self) -> Vec<PaneId> {
        self.placements.iter().map(|p| p.pane).collect()
    }

    /// One pane's placement, if it is placed.
    pub fn placement(&self, pane: PaneId) -> Option<&PanePlacement> {
        self.placements.iter().find(|p| p.pane == pane)
    }

    /// Route a **window-local** physical point delivered to `window`.
    ///
    /// The hot path for both mouse and touch: winit reports each event against
    /// the window it happened on, so a coordinate is resolved among that window's
    /// panes alone — which is what makes a mouse on the right half of Station 2
    /// reach the pane there and nothing on Station 1.
    pub fn resolve_in_window(&self, window: WindowKey, wx: f64, wy: f64) -> Option<PaneHit> {
        self.placements
            .iter()
            .find(|p| p.window == window && p.window_contains(wx, wy))
            .map(|p| p.local_of_window(wx, wy))
    }

    /// Route a **desktop-global** physical point.
    ///
    /// For an input source that reports in virtual-desktop coordinates rather
    /// than against a window — a touchscreen the profile maps to a monitor by
    /// physical position. Chains the monitor hit (window origin) and the pane hit
    /// in one pass.
    pub fn resolve_desktop(&self, x: f64, y: f64) -> Option<PaneHit> {
        self.placements
            .iter()
            .find(|p| p.desktop_contains(x, y))
            .map(|p| p.local_of_window(x - p.window_origin_x as f64, y - p.window_origin_y as f64))
    }
}

/// The keyboard-focus order over the panes, and which one currently has focus.
///
/// Exactly one pane holds keyboard focus at a time (Ultralight drops input into
/// an unfocused view, so there is no such thing as two focused panes), and focus
/// moves either by pointing at a pane or by cycling with the keyboard. This model
/// owns *which* pane that is; drawing the visible non-colour indicator over it is
/// the adapter's job.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusRing {
    order: Vec<PaneId>,
    focused: Option<PaneId>,
}

impl FocusRing {
    /// An empty ring — no panes, nothing focused.
    pub fn new() -> Self {
        Self::default()
    }

    /// A ring over `order` with nothing focused yet.
    pub fn from_order(order: Vec<PaneId>) -> Self {
        Self {
            order,
            focused: None,
        }
    }

    /// The pane that currently holds keyboard focus, if any.
    pub fn focused(&self) -> Option<PaneId> {
        self.focused
    }

    /// The traversal order.
    pub fn order(&self) -> &[PaneId] {
        &self.order
    }

    /// Reconcile the order with the panes that are actually placed now.
    ///
    /// Called whenever the layout changes. The focused pane is **kept** if it is
    /// still in the new order and **cleared** if it has gone — never silently
    /// carried onto whatever pane now occupies its old position, which would hand
    /// one participant's keystrokes to another.
    pub fn sync_order(&mut self, order: Vec<PaneId>) {
        if let Some(focused) = self.focused {
            if !order.contains(&focused) {
                self.focused = None;
            }
        }
        self.order = order;
    }

    /// Give focus to a specific pane, if it is in the order. Returns whether
    /// focus actually moved (so the adapter can skip re-focusing the view and
    /// moving the indicator when the pointer merely stays on the same pane).
    pub fn focus(&mut self, pane: PaneId) -> bool {
        if !self.order.contains(&pane) || self.focused == Some(pane) {
            return false;
        }
        self.focused = Some(pane);
        true
    }

    /// Drop focus entirely (the pointer left every pane, or the focused pane
    /// closed).
    pub fn clear(&mut self) {
        self.focused = None;
    }

    /// Move focus to the next pane in order, cycling. With nothing focused this
    /// focuses the first pane; on an empty ring it is a no-op. Returns the new
    /// focus.
    pub fn focus_next(&mut self) -> Option<PaneId> {
        self.step(1)
    }

    /// Move focus to the previous pane in order, cycling. With nothing focused
    /// this focuses the last pane. Returns the new focus.
    pub fn focus_prev(&mut self) -> Option<PaneId> {
        self.step(-1)
    }

    fn step(&mut self, direction: isize) -> Option<PaneId> {
        if self.order.is_empty() {
            self.focused = None;
            return None;
        }
        let len = self.order.len() as isize;
        let next_index = match self
            .focused
            .and_then(|f| self.order.iter().position(|p| *p == f))
        {
            Some(current) => (current as isize + direction).rem_euclid(len),
            // Nothing focused: forward starts at the first pane, backward at the
            // last, so a single press always lands somewhere predictable.
            None => {
                if direction >= 0 {
                    0
                } else {
                    len - 1
                }
            }
        };
        self.focused = Some(self.order[next_index as usize]);
        self.focused
    }
}

/// The per-contact touch capture map (acceptance criterion 4).
///
/// A touch contact is pinned to the pane its *first* point resolved to, and every
/// subsequent move and its final lift are routed to that pane no matter where the
/// finger has since drifted — off the pane, off the screen, onto another pane. A
/// gesture that begins on the helm pane stays the helm's even if it slides onto
/// the pane beside it, which is what makes a drag or a swipe behave.
///
/// Simultaneous contacts are independent by construction: each is one entry keyed
/// by its own contact id, so two fingers on two different screens (or two panes)
/// never interact.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactCaptureMap {
    pinned: BTreeMap<u64, PaneId>,
}

impl ContactCaptureMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any contact is currently captured.
    pub fn is_empty(&self) -> bool {
        self.pinned.is_empty()
    }

    /// How many contacts are captured.
    pub fn len(&self) -> usize {
        self.pinned.len()
    }

    /// Pin a newly-started contact to the pane it resolved to.
    ///
    /// A contact id already pinned is left on its original pane and this returns
    /// `false`: a second `Started` for a live id is a driver quirk, not a reason
    /// to re-home a gesture midway.
    pub fn start(&mut self, contact: u64, pane: PaneId) -> bool {
        if self.pinned.contains_key(&contact) {
            return false;
        }
        self.pinned.insert(contact, pane);
        true
    }

    /// The pane a live contact is pinned to, for routing a move.
    pub fn pane_for(&self, contact: u64) -> Option<PaneId> {
        self.pinned.get(&contact).copied()
    }

    /// Release a contact on lift, returning the pane it was pinned to so its
    /// final `mouse_up` can be routed there.
    pub fn end(&mut self, contact: u64) -> Option<PaneId> {
        self.pinned.remove(&contact)
    }

    /// Every live contact and the pane it is captured by, in contact-id order.
    pub fn active(&self) -> impl Iterator<Item = (u64, PaneId)> + '_ {
        self.pinned.iter().map(|(id, pane)| (*id, *pane))
    }

    /// Drop every contact captured by a pane that has gone, returning their ids
    /// so the adapter can synthesise a release. A pane closing under a finger
    /// must not leave that finger pinned to a view that no longer exists.
    pub fn release_pane(&mut self, pane: PaneId) -> Vec<u64> {
        let released: Vec<u64> = self
            .pinned
            .iter()
            .filter(|(_, p)| **p == pane)
            .map(|(id, _)| *id)
            .collect();
        for id in &released {
            self.pinned.remove(id);
        }
        released
    }
}

#[cfg(test)]
#[path = "input_routing_tests.rs"]
mod tests;
