//! `EffectQueue<T>`: a per-owner, transient, per-tick effect queue (issue #1223,
//! Track 3 step C11).
//!
//! # Why this exists
//!
//! Several subsystems resolve a scripted or console effect at one site — where
//! `WorldContentRuntime::name_to_uuid` is in scope — but must APPLY it at
//! another, because the one system that owns a threshold edge, a compliance
//! machine or an entity query is the only place the effect can land without a
//! divergence. The resolved-but-not-yet-applied command rides a queue between the
//! two.
//!
//! Those queues used to be `pending_*` fields on the `WorldContentRuntime`
//! god-resource, in part so the authoritative-state census (issue #894) "saw no
//! new registration". Issues #1220–#1222 gave the census a real declaration
//! registry ([`crate::authoritative::StateCensus`]) with per-type declaration
//! sites, so that reason is gone: each queue is now its OWN resource, registered
//! and drained by the plugin that owns it, and declared into the census at that
//! plugin's `build()`.
//!
//! # Census classification
//!
//! Every `EffectQueue<T>` is a transient inter-system queue: an owning system
//! drains it in full every tick (`std::mem::take`), so it is structurally empty
//! at the `RenderInterp` fold point on every correctly-running instance. That is
//! exactly the [`crate::authoritative::StateClass::ClearedAtFold`] class — never
//! folded (`src/sim_digest.rs` walks the COMPONENTS a drain writes, not the
//! queue), never snapshotted (empty at every capture boundary). Each distinct
//! instantiation keys distinctly in the census by its full type path
//! (`EffectQueue<A>` and `EffectQueue<B>` are two entries), the same shape
//! `BroadcastRegistry<M>` already relies on.

use bevy::prelude::Resource;

/// A transient per-tick effect queue owned by exactly one plugin.
///
/// Effect appliers PUSH resolved commands onto it (`queue.0.push(..)` /
/// `.extend(..)`); the OWNING system DRAINS it every tick
/// (`std::mem::take(&mut queue.0)`). It is empty at every tick boundary, so it is
/// classified [`crate::authoritative::StateClass::ClearedAtFold`] and is inert to
/// both the authoritative-state digest and the `#863` snapshot.
///
/// One instantiation per payload type `T`; each keys distinctly in the
/// authoritative-state census by its full type path.
#[derive(Resource)]
pub struct EffectQueue<T: Send + Sync + 'static>(pub Vec<T>);

// Hand-written rather than `#[derive(Default)]` so the impl does NOT demand
// `T: Default`: an empty queue is `Vec::new()` whatever `T` is, and the payloads
// (`ConditionAdjustment`, a `(String, bool)` weapons hold, …) are not all
// `Default`.
impl<T: Send + Sync + 'static> Default for EffectQueue<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
