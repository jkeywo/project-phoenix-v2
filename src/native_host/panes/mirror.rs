//! What the main thread still knows about a pane once the views live elsewhere
//! (issue #1404, slice 3).
//!
//! # Why there is a mirror at all
//!
//! A pane is two things at once. On the renderer's side it is a live document —
//! a `View`, its load state, the buffer it paints into. On Bevy's side it is a
//! texture, a UI node, a camera and a rectangle on a monitor. Slice 3 splits
//! those: [`PaneLoop`](super::pane_thread::PaneLoop) owns the first, and this
//! owns the second.
//!
//! The split is what makes the *questions* the main thread has to answer about a
//! frame answerable at all once a frame can arrive from a thread that has since
//! moved on:
//!
//! - **Is this frame still about a surface that exists?** A resize mints a new
//!   texture and a new [`epoch`](MirrorPane::epoch); a frame copied against the
//!   old one describes pixels of a surface that is gone.
//! - **Has this pane's view died?** A single failed copy is a transient; a run
//!   of [`VIEW_CRASH_COPY_FAILURES`] is the honest crash signal — for a console,
//!   which has a station to hand back to Backfill. A permanent surface has
//!   nowhere to fall back to and is never faulted.
//! - **What happens if the renderer stops entirely?** Every console faults;
//!   every permanent surface is simply dropped, leaving its last frame on screen
//!   rather than a fault with nowhere to go.
//!
//! Generic over the payload `T` so the type itself is **Bevy-free**: the
//! per-pane Bevy state (the `Image` handle, the canvas entity, the window, the
//! rectangle) is `T`, and every rule above is checked by the ordinary `cargo
//! test` CI runs rather than only on a Windows machine with an SDK. The
//! `ultralight` adapter instantiates it with its own payload; the tests below
//! instantiate it with `()`.
//!
//! The adapter instantiates this with its canvas payload; the views and their
//! pools are owned by the pane thread.

use super::pane_thread::PaneKind;
use super::recovery::PaneFault;
use super::registry::PaneId;

/// How many consecutive frames a pane's frame copy may fail before the view is
/// treated as crashed (issue #1125).
///
/// A `copy_frame` error is ordinarily transient — a repaint mid-flight, a buffer
/// not ready — so a single one is not a crash. A view that has genuinely died
/// (a lost surface, a renderer that stopped answering) fails *every* frame, so a
/// short run of consecutive failures is the honest crash signal. Sized to about
/// half a second at 60 fps: long enough not to fire on a blip, short enough that
/// a dead console flips its station to Backfill promptly.
pub const VIEW_CRASH_COPY_FAILURES: u32 = 30;

/// One pane, as the side that does *not* own its view knows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MirrorPane<T> {
    pub id: PaneId,
    /// What the surface is — and so whether it can be faulted at all. See
    /// [`PaneKind::permanent`].
    pub kind: PaneKind,
    /// This texture's generation, bumped by every resize. A frame carrying an
    /// older one describes a surface that no longer exists.
    pub epoch: u64,
    /// Consecutive frames whose copy failed, as last reported. Reset by any
    /// success.
    pub copy_failures: u32,
    /// Whether the surface is drawn. A hidden surface is still pumped — so a
    /// reveal is instant — but is not copied.
    pub visible: bool,
    /// Everything the owning side knows that this module must not: the Bevy
    /// texture, node, window and rectangle in the adapter; `()` in the tests.
    pub payload: T,
}

impl<T> std::ops::Deref for MirrorPane<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.payload
    }
}
impl<T> std::ops::DerefMut for MirrorPane<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.payload
    }
}

/// Every pane the main thread knows about.
///
/// A `Vec` rather than a map, and deliberately: pane order is the order they
/// were opened in, which is the order the loop drives them in and the order the
/// input router places them in. A map would make "the same order on both sides"
/// an accident.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneMirror<T> {
    panes: Vec<MirrorPane<T>>,
}

impl<T> std::ops::Index<usize> for PaneMirror<T> {
    type Output = MirrorPane<T>;
    fn index(&self, index: usize) -> &Self::Output {
        &self.panes[index]
    }
}

/// What a renderer that stopped means for the panes it was drawing.
///
/// Two lists rather than one, because the two halves are answered differently:
/// a console has a station and rides the fault path to Backfill, and a
/// permanent surface has neither and is simply taken off the books.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThreadDeath {
    /// Consoles to fault, so each station falls back to AI control.
    pub fault: Vec<PaneId>,
    /// Permanent surfaces (the host lobby, the viewscreen HUD) to drop.
    pub drop_permanent: Vec<PaneId>,
}

impl<T> PaneMirror<T> {
    /// An empty mirror.
    pub fn new() -> Self {
        Self { panes: Vec::new() }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, MirrorPane<T>> {
        self.panes.iter()
    }
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, MirrorPane<T>> {
        self.panes.iter_mut()
    }
    pub fn retain(&mut self, keep: impl FnMut(&MirrorPane<T>) -> bool) {
        self.panes.retain(keep);
    }

    /// Record a pane that has just been opened, at generation zero.
    pub fn insert(&mut self, id: PaneId, kind: PaneKind, visible: bool, payload: T) {
        self.panes.push(MirrorPane {
            id,
            kind,
            epoch: 0,
            copy_failures: 0,
            visible,
            payload,
        });
    }

    /// Forget a pane, handing back what was known about it — its payload is the
    /// caller's to tear down.
    pub fn remove(&mut self, id: PaneId) -> Option<MirrorPane<T>> {
        let index = self.panes.iter().position(|p| p.id == id)?;
        Some(self.panes.remove(index))
    }

    pub fn get(&self, id: PaneId) -> Option<&MirrorPane<T>> {
        self.panes.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: PaneId) -> Option<&mut MirrorPane<T>> {
        self.panes.iter_mut().find(|p| p.id == id)
    }

    /// Every pane, in the order they were opened.
    pub fn ids(&self) -> Vec<PaneId> {
        self.panes.iter().map(|p| p.id).collect()
    }

    /// How many panes are known.
    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    /// Start a new texture generation for a pane, and say which it is — the
    /// number the resize command carries to the side that owns the view, so both
    /// halves agree on which frames belong to which texture.
    pub fn bump_epoch(&mut self, id: PaneId) -> Option<u64> {
        let pane = self.get_mut(id)?;
        pane.epoch += 1;
        Some(pane.epoch)
    }

    /// Whether a frame at `epoch` describes the surface this pane has **now**.
    ///
    /// A pane that has closed accepts nothing: its frames are about a texture
    /// nothing draws any more, and a late one would otherwise be uploaded into
    /// whatever asset id happened to be reused.
    pub fn accepts_frame(&self, id: PaneId, epoch: u64) -> bool {
        self.get(id).is_some_and(|p| p.epoch == epoch)
    }

    /// Record that a pane's copy has now failed `consecutive` times in a row,
    /// and say whether that is a crash.
    ///
    /// The count is the *producer's*, not one kept here: the side that made the
    /// copies is the side that knows how many in a row failed, and two counters
    /// that could disagree would be a bug waiting for a dropped event.
    ///
    /// A permanent surface never faults, whatever the count. It holds no station
    /// to fall back to AI control, and closing it would remove the one surface
    /// the operator drives the host from — a dead permanent view is a warning
    /// and a blank rectangle, honest and recoverable by restarting the host.
    pub fn record_copy_failure(&mut self, id: PaneId, consecutive: u32) -> Option<PaneFault> {
        let pane = self.get_mut(id)?;
        pane.copy_failures = consecutive;
        (consecutive >= VIEW_CRASH_COPY_FAILURES && !pane.kind.permanent())
            .then_some(PaneFault::ViewCrashed)
    }

    /// Record that a pane's copy succeeded, ending any run of failures.
    pub fn record_copy_ok(&mut self, id: PaneId) {
        if let Some(pane) = self.get_mut(id) {
            pane.copy_failures = 0;
        }
    }

    /// The renderer has stopped and no further frame will arrive: which panes
    /// fault, and which are simply dropped.
    pub fn thread_death(&self) -> ThreadDeath {
        let mut death = ThreadDeath::default();
        for pane in &self.panes {
            if pane.kind.permanent() {
                death.drop_permanent.push(pane.id);
            } else {
                death.fault.push(pane.id);
            }
        }
        death
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONSOLE: PaneId = PaneId(1);
    const OTHER: PaneId = PaneId(2);
    const LOBBY: PaneId = PaneId(900);
    const HUD: PaneId = PaneId(901);

    fn mirror() -> PaneMirror<()> {
        let mut mirror = PaneMirror::new();
        mirror.insert(CONSOLE, PaneKind::Console, true, ());
        mirror.insert(LOBBY, PaneKind::Lobby, true, ());
        mirror.insert(HUD, PaneKind::Hud, false, ());
        mirror
    }

    #[test]
    fn a_frame_from_the_generation_before_a_resize_is_not_accepted() {
        // The whole reason an epoch exists: a resize mints a NEW texture, and a
        // frame still in flight against the old one is the right pixels for the
        // wrong surface. Uploading it would either be refused by the layout
        // check or, worse, land a partial rectangle in a texture that has never
        // held anything but its fill.
        let mut mirror = mirror();
        assert!(mirror.accepts_frame(CONSOLE, 0));

        assert_eq!(mirror.bump_epoch(CONSOLE), Some(1));
        assert!(!mirror.accepts_frame(CONSOLE, 0), "the frame is stale");
        assert!(mirror.accepts_frame(CONSOLE, 1));

        // And the bump is per pane: the surfaces that did not resize are
        // untouched.
        assert!(mirror.accepts_frame(LOBBY, 0));
        assert_eq!(mirror.bump_epoch(OTHER), None, "no such pane");
    }

    #[test]
    fn a_closed_pane_accepts_nothing_that_arrives_after_it() {
        // A pane closes while the side that owns its view is mid-copy. The
        // frame is honest about the surface it came from and completely wrong
        // about what to do with it: nothing draws that texture now.
        let mut mirror = mirror();
        assert!(mirror.accepts_frame(CONSOLE, 0));
        let removed = mirror.remove(CONSOLE).expect("it was open");
        assert_eq!(removed.id, CONSOLE);
        assert!(!mirror.accepts_frame(CONSOLE, 0));
        assert_eq!(
            mirror.ids(),
            vec![LOBBY, HUD],
            "and the rest keep their order"
        );
        assert!(mirror.remove(CONSOLE).is_none(), "closed once");
    }

    #[test]
    fn a_console_faults_at_the_threshold_and_a_permanent_surface_never_does() {
        let mut mirror = mirror();
        for n in 1..VIEW_CRASH_COPY_FAILURES {
            assert_eq!(
                mirror.record_copy_failure(CONSOLE, n),
                None,
                "a run shorter than the threshold is a transient, not a crash"
            );
        }
        assert_eq!(
            mirror.record_copy_failure(CONSOLE, VIEW_CRASH_COPY_FAILURES),
            Some(PaneFault::ViewCrashed)
        );
        assert_eq!(mirror.get(CONSOLE).map(|p| p.copy_failures), Some(30));

        // The lobby and the HUD hold no station, so there is no Backfill to
        // fall back to: a dead permanent view is a blank rectangle, not a fault
        // aimed at nothing.
        for surface in [LOBBY, HUD] {
            assert_eq!(
                mirror.record_copy_failure(surface, VIEW_CRASH_COPY_FAILURES * 10),
                None
            );
        }
        assert_eq!(
            mirror.record_copy_failure(OTHER, 1000),
            None,
            "no such pane"
        );
    }

    #[test]
    fn one_success_ends_a_run_of_failures() {
        let mut mirror = mirror();
        assert_eq!(
            mirror.record_copy_failure(CONSOLE, VIEW_CRASH_COPY_FAILURES - 1),
            None
        );
        mirror.record_copy_ok(CONSOLE);
        assert_eq!(mirror.get(CONSOLE).map(|p| p.copy_failures), Some(0));
        // The count the producer sends is the producer's own run, so a fresh
        // run starts at one rather than resuming where the old one stopped.
        assert_eq!(mirror.record_copy_failure(CONSOLE, 1), None);
    }

    #[test]
    fn a_dead_renderer_faults_the_consoles_and_merely_drops_the_permanent_surfaces() {
        let mut mirror = mirror();
        mirror.insert(OTHER, PaneKind::Console, true, ());
        let death = mirror.thread_death();
        assert_eq!(death.fault, vec![CONSOLE, OTHER]);
        assert_eq!(death.drop_permanent, vec![LOBBY, HUD]);
        assert_eq!(
            mirror.len(),
            4,
            "the partition is a question, not the teardown itself"
        );
    }

    #[test]
    fn the_payload_is_the_owners_and_the_mirror_only_carries_it() {
        // Generic over the payload for one reason: everything above is checked
        // by the ordinary `cargo test`, with no Bevy and no SDK in scope.
        let mut mirror: PaneMirror<String> = PaneMirror::new();
        mirror.insert(CONSOLE, PaneKind::Console, true, "helm".to_string());
        assert_eq!(
            mirror.get(CONSOLE).map(|p| p.payload.as_str()),
            Some("helm")
        );
        mirror
            .get_mut(CONSOLE)
            .unwrap()
            .payload
            .push_str(" console");
        assert_eq!(
            mirror.remove(CONSOLE).map(|p| p.payload),
            Some("helm console".to_string())
        );
        assert!(mirror.is_empty());
    }
}
