//! The bounded inverse of one allowed GM removal (issue #1444).
//!
//! Every other reversible GM family (#1442) restores a FIELD: a doctrine id, a
//! hostility bit. A removal is different in kind — the thing that changed is
//! that an entity stopped existing — so the inverse cannot be "the same action
//! with the old value". It has to hold enough of the removed entity to build it
//! again, and it has to be honest about the parts of the world that moved on
//! while it was gone.
//!
//! # What is captured, and why it is the snapshot's own row
//!
//! [`GmRemovalCapture::state`] is a [`crate::snapshot::EntityState`] — the exact
//! per-entity row a save writes and a resume applies — taken through the exact
//! function a save takes it with. That is deliberate and it is the whole design:
//! a despawn undo is a one-entity restore, and a second definition of "the
//! complete state of an entity" would be a second set of answers to the question
//! [`crate::snapshot`] already answers for every resumed run. The restore below
//! likewise goes through [`crate::snapshot::spawn_from_origin`] and
//! [`crate::snapshot::apply_entity_state`], not through a private rebuild.
//!
//! The row carries [`crate::world::spawn_origin::SpawnOrigin`], the recipe a
//! runtime spawn recorded. An entity that has none — an authored `[[entity]]`
//! block, which a fresh boot of the same world re-creates from the file rather
//! than from a record — cannot be rebuilt from a capture at all, and the capture
//! says so ([`GmRemovalCapture::state`] is `None`) rather than fabricating a
//! recipe from components the merge already consumed. `undo_precheck`'s caller
//! then refuses with [`crate::gm_action::GmActionRefusalReason::InverseUnsupported`].
//!
//! `mesh_crew` is stripped from every capture. Live crew authority is not state
//! of the entity in the sense an inverse restores — PRD #1420 keeps *current*
//! participants and assignments across even a full recovery restore — and it is
//! the one row field that is about who is sitting at a console rather than about
//! the world.
//!
//! # What the cleanup releases, and what a capture is allowed to hold
//!
//! [`crate::gm_despawn::remove_entity`] clears a long list of live references
//! held by OTHER entities: radar selections, weapon locks, tractor and dock
//! couplings, transports, dispatched repair crews, anchored waypoints, Comms
//! presence, task activations. None of them is put back, and none of them is
//! CAPTURED either. Those are two separate decisions with two separate reasons.
//!
//! Not put back, because a crew watched their lock drop and their tractor
//! release. Re-coupling a beam a Tactical operator has since re-aimed would be
//! overwriting a newer decision, which is exactly what PRD #1420 forbids ("never
//! overwrite newer changes"), and doing it silently would be worse. What is
//! genuinely derived comes back on its own: Comms contacts and range flags are
//! recomputed from live positions, and a restored hull re-enters sensor range.
//!
//! Not captured, because a capture is folded into the authoritative digest and
//! written into every save, and half of what that cleanup touches is state this
//! repo keeps OUTSIDE the #894 digest boundary on purpose. A task activation
//! lives in [`crate::core::task_lifecycle::TaskLifecycles`], which is
//! presentation-class and is not snapshotted — a peer that RESUMED holds no
//! activations while a peer that ran through holds them. Comms presence lives in
//! `CommsRuntime`'s `contacts` and `range_flags`, which
//! `update_comms_range_flags` rebuilds from a [`crate::server_app::LocalShip`]
//! query — this host's own screen, which a shipless GM peer never runs at all.
//! A list derived from either would make two peers that agree about the WORLD
//! disagree about a removal's inverse data, and a spurious `DigestMismatch` that
//! stops a live event is a worse failure than anything such a list would buy.
//! It would buy nothing: the preview names the released links in the same words
//! for every removal (`server.gm.inverse.witnessed_removal`), because the
//! sentence a GM needs is that locks and tows do not come back, not which ones
//! this particular hull happened to hold.
//!
//! [`GmRemovalReference`] — an attributed GM contact-knowledge override — is
//! therefore the ONLY cleared reference a capture carries, and it clears both
//! bars. It is not a crew observation but an attributed GM decision made through
//! [`crate::gm_action::GmAction::SetContactOverride`], so dropping it on the
//! floor would silently discard one GM action while claiming to reverse another;
//! and its source, `WorldContentRuntime::contact_overrides`, is already folded
//! into the digest and already written into every save, so capturing rows of it
//! asks two peers to agree on nothing they did not already agree on. It is
//! restored, and a NEWER override on the same observer/target pair refuses the
//! whole undo rather than being overwritten.
//!
//! # What is bounded
//!
//! [`MAX_GM_DESPAWN_CAPTURES`]. The captures ride the snapshot and the
//! deterministic digest, and one of them is a whole entity row; keeping 4096 of
//! them (the journal's own cap) would put megabytes of removed hulls in every
//! save. The oldest is evicted first, deterministically, and an evicted capture
//! is reported to the page as such — [`crate::gm_journal::GmJournalEntry::capture_lost`]
//! — so the Undo control disappears instead of offering an inverse the reducer
//! would only refuse.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::gm_action::{GmActionId, GmActionRefusalReason, GmUndoReference};
use crate::gm_contact::ContactMode;
use crate::snapshot::EntityState;

/// How many removal captures one run retains. See the module docs.
pub const MAX_GM_DESPAWN_CAPTURES: usize = 16;

/// One attributed GM contact-knowledge override
/// [`crate::gm_despawn::remove_entity`] cleared, and an undo puts back.
///
/// The only class of cleared reference a capture carries — the module docs say
/// what else that cleanup releases, and why none of THAT may ride a capture.
///
/// Recorded by the removal itself rather than by a read-only mirror of it, so
/// the list cannot drift out of step with the cleanup it describes: the one
/// walk that clears them is the one walk that reports them.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GmRemovalReference {
    /// The observing ship whose knowledge this row belongs to.
    pub observer: String,
    /// The entity the row is about — the removed entity when it was an
    /// observer's row, or the removed entity's own observations.
    pub target: String,
    /// The knowledge the GM had set for that pair.
    pub mode: ContactMode,
}

/// The complete inverse data for one allowed GM removal.
///
/// Bounded by construction: one entity row, plus the references that entity's
/// cleanup cleared. Nothing about the rest of the world is in here, which is
/// what makes an undo a one-entity restore rather than a rewind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GmRemovalCapture {
    /// The public identity of the [`crate::gm_action::GmAction::DespawnEntity`]
    /// this reverses — the same three fields every other inverse names an
    /// original by (see [`GmUndoReference`]).
    pub action: GmUndoReference,
    /// The removed entity's stable uuid, which the restore reuses.
    pub entity: String,
    /// Every authored scenario name that resolved to it when it was removed.
    ///
    /// The name→uuid table deliberately SURVIVES a removal (destruction
    /// predicates read it as history), so these are not restored. They are the
    /// identity an undo checks for occupancy: if a name now resolves to a
    /// different live entity, the world has re-used the identity and the
    /// restore is refused rather than producing two claimants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// The simulation tick the removal applied at.
    pub tick: u64,
    /// The complete restorable state, or `None` when this entity cannot be
    /// rebuilt at all. See the module docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Box<EntityState>>,
    /// The attributed contact overrides the cleanup cleared, sorted. An undo
    /// puts every one of them back; see the module docs for what that same
    /// cleanup released and deliberately did not hand to this capture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<GmRemovalReference>,
}

impl GmRemovalCapture {
    /// Whether this capture can rebuild its entity at all.
    pub fn is_restorable(&self) -> bool {
        self.state.is_some()
    }
}

/// One accepted-but-not-yet-executed removal, waiting for the fixed pipeline to
/// destroy the entity so the capture can be taken from the world one instant
/// before it stops existing.
///
/// Cross-tick for [`crate::world::server::WorldContentRuntime::pending_gm_despawns`]'
/// reason — armed in `PreUpdate`, consumed in `FixedUpdate`, which a paused
/// session never reaches — so it travels in the snapshot and the digest with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmedGmRemoval {
    pub action: GmUndoReference,
    pub entity: String,
    pub tick: u64,
}

/// One accepted-but-not-yet-executed inverse. Same cross-tick reason: the
/// reducer decides in `PreUpdate` and the rebuild needs exclusive world access,
/// which only the fixed pipeline's command queue has.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArmedGmRestore {
    /// The identity of the removal being reversed, which selects the capture.
    pub original: GmUndoReference,
}

/// Every retained removal capture in this run, plus the two cross-tick arms.
///
/// A `Vec` rather than a map keyed by the action identity, for
/// [`crate::gm_faction::GmFactionOverrides`]' reason: the same value is
/// serialized into the snapshot, folded into the digest and read by the
/// projection, and a three-field tuple key is not expressible in all of those.
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GmDespawnCaptures {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    armed: Vec<ArmedGmRemoval>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    restores: Vec<ArmedGmRestore>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    entries: Vec<GmRemovalCapture>,
}

impl GmDespawnCaptures {
    /// Nothing to fold. An absent store and an untouched one are the same fact,
    /// so a run that never removes anything keeps the digest it had before this
    /// module existed.
    pub fn is_empty(&self) -> bool {
        self.armed.is_empty() && self.restores.is_empty() && self.entries.is_empty()
    }

    /// The half of this store the deterministic digest folds.
    ///
    /// The captured entity ROW is deliberately NOT in here, and the omission is
    /// the point rather than an economy. Everything a restore puts back becomes
    /// live components the ordinary per-entity fold already covers, so two peers
    /// that rebuilt different hulls are caught at the very next comparison —
    /// which is the same tick. Folding the row itself would add nothing to that
    /// detection while making every field the capture happens to carry into a
    /// cross-peer agreement, including the ones `snapshot` documents as
    /// continuation readings of a local clock. A spurious mismatch that stops a
    /// live event is a worse failure than the one it would be guarding against.
    ///
    /// What IS folded is what the peers must agree on BEFORE any restore: which
    /// removals are armed, which captures are retained under which public action
    /// identity, whether each can be rebuilt at all, and the attributed contact
    /// overrides an inverse would put back.
    ///
    /// Those overrides are the only cleared references a capture holds, and that
    /// is what makes folding them safe: they are rows of
    /// `WorldContentRuntime::contact_overrides`, which this same digest already
    /// folds, so no peer is being asked to agree about anything it was not
    /// already agreeing about. Anything derived from the presentation-class or
    /// `LocalShip`-gated state the same cleanup touches would break that — see
    /// the module docs — which is why no such thing reaches a capture at all.
    pub fn digest_facts(&self) -> impl Serialize + '_ {
        (
            &self.armed,
            &self.restores,
            self.entries
                .iter()
                .map(|capture| {
                    (
                        &capture.action,
                        capture.entity.as_str(),
                        &capture.names,
                        capture.tick,
                        capture.state.is_some(),
                        &capture.references,
                    )
                })
                .collect::<Vec<_>>(),
        )
    }

    /// Record that an allowed removal is about to happen, so the destruction
    /// path knows to capture rather than merely clean up.
    ///
    /// Bounded on the retained captures' own rule: an arm whose removal the
    /// pipeline never performs — the entity was already gone by the time the
    /// fixed step ran — has nothing to consume it, and an unbounded list of
    /// those would ride every subsequent save.
    pub fn arm_removal(&mut self, armed: ArmedGmRemoval) {
        self.armed
            .retain(|existing| existing.entity != armed.entity);
        if self.armed.len() >= MAX_GM_DESPAWN_CAPTURES {
            let overflow = self.armed.len() + 1 - MAX_GM_DESPAWN_CAPTURES;
            self.armed.drain(0..overflow);
        }
        self.armed.push(armed);
    }

    /// The armed removal for one entity, consumed by the destruction path.
    pub fn take_armed(&mut self, entity: &str) -> Option<ArmedGmRemoval> {
        let index = self.armed.iter().position(|armed| armed.entity == entity)?;
        Some(self.armed.remove(index))
    }

    /// Retain one capture, evicting the oldest once the bound is reached.
    pub fn record(&mut self, capture: GmRemovalCapture) {
        self.entries
            .retain(|existing| existing.action != capture.action);
        if self.entries.len() >= MAX_GM_DESPAWN_CAPTURES {
            let overflow = self.entries.len() + 1 - MAX_GM_DESPAWN_CAPTURES;
            self.entries.drain(0..overflow);
        }
        self.entries.push(capture);
    }

    /// The capture for one original removal, by its public action identity.
    pub fn capture_for(
        &self,
        operator_id: &str,
        correlation: &GmActionId,
        sequence: u64,
    ) -> Option<&GmRemovalCapture> {
        self.entries.iter().find(|capture| {
            capture.action.operator_id == operator_id
                && capture.action.correlation == *correlation
                && capture.action.sequence == sequence
        })
    }

    /// Whether a retained, rebuildable capture exists for one original removal.
    pub fn is_restorable(
        &self,
        operator_id: &str,
        correlation: &GmActionId,
        sequence: u64,
    ) -> bool {
        self.capture_for(operator_id, correlation, sequence)
            .is_some_and(GmRemovalCapture::is_restorable)
    }

    /// Whether an accepted inverse is waiting for the fixed pipeline.
    ///
    /// Distinct from [`Self::is_empty`], which is the digest's question:
    /// RETAINING captures is not work, and waking the trigger pipeline on every
    /// tick after the session's first GM removal would be a real cost for none.
    pub fn has_armed_restores(&self) -> bool {
        !self.restores.is_empty()
    }

    /// Arm one accepted inverse for the fixed pipeline to execute.
    pub fn arm_restore(&mut self, original: GmUndoReference) {
        self.restores.push(ArmedGmRestore { original });
    }

    /// Take every armed inverse, in acceptance order.
    pub fn take_restores(&mut self) -> Vec<ArmedGmRestore> {
        std::mem::take(&mut self.restores)
    }

    /// Drop the capture an executed inverse consumed.
    pub fn consume(&mut self, original: &GmUndoReference) -> Option<GmRemovalCapture> {
        let index = self
            .entries
            .iter()
            .position(|capture| capture.action == *original)?;
        Some(self.entries.remove(index))
    }

    #[cfg(test)]
    pub fn entries(&self) -> &[GmRemovalCapture] {
        &self.entries
    }
}

/// Why a proposed inverse of one removal cannot run, checked against LIVE state
/// at the canonical apply tick.
///
/// Pure and total: it reads, it never writes, and it answers with exactly one
/// named thing that is wrong. That is what makes the restore atomic — every
/// fallible question is asked here, before the first component is written, so
/// there is no half-restored entity to unwind.
pub fn restore_precheck(
    capture: &GmRemovalCapture,
    live_uuids: &std::collections::BTreeSet<String>,
    name_to_uuid: &std::collections::HashMap<String, String>,
    contact_overrides: &crate::gm_contact::ContactOverrides,
    template_resolves: impl FnOnce(&crate::world::spawn_origin::SpawnOrigin) -> bool,
) -> Result<(), GmActionRefusalReason> {
    // Nothing to rebuild from: an authored `[[entity]]` block, or a capture the
    // bound has already evicted. Both are the same answer to a GM.
    let Some(state) = capture.state.as_deref() else {
        return Err(GmActionRefusalReason::InverseUnsupported);
    };
    let Some(origin) = state.spawn.as_ref() else {
        return Err(GmActionRefusalReason::InverseUnsupported);
    };
    // The identity itself. A live entity already answering to the captured uuid
    // means the world has re-used it, and a restore would produce two
    // claimants — so it is refused, never overwritten.
    if live_uuids.contains(&capture.entity) {
        return Err(GmActionRefusalReason::RestoreIdentityOccupied);
    }
    // The authored names the removed entity answered to. `name_to_uuid` keeps a
    // removed entity's own mapping — destruction predicates, comms routes and
    // `AiDirective::Destroy` all read it as history — so a name still pointing
    // at THIS uuid is the expected case. A name the world has since re-bound is
    // a NEWER reference to something else, and putting the entity back under it
    // would leave the name and the entity disagreeing about who is who.
    for name in &capture.names {
        match name_to_uuid.get(name) {
            Some(uuid) if uuid != &capture.entity => {
                return Err(GmActionRefusalReason::RestoreReferenceConflict);
            }
            _ => {}
        }
    }
    // The references an undo would put back. A pair that now holds ANY override
    // is a newer decision by someone — possibly another GM — and this refuses
    // rather than overwriting it. Unrelated pairs, and every other change to the
    // same observer, are deliberately not consulted.
    //
    // Not reachable through today's ordinary systems, and kept anyway:
    // `gm_contact::prune` retains an override only while its target is LIVE, so
    // no row about a removed entity can exist for the window an inverse spans.
    // That is a property of a sibling system's policy, not of this one, and a
    // restore that silently overwrote a GM decision the day that policy changed
    // is not a failure worth inviting.
    for reference in &capture.references {
        if contact_overrides
            .get(&reference.observer)
            .and_then(|rows| rows.get(&reference.target))
            .is_some()
        {
            return Err(GmActionRefusalReason::RestoreReferenceConflict);
        }
    }
    // Last, because it is the only question with a cost: the template the
    // rebuild reads. Asked HERE rather than discovered by a half-run restore,
    // so a withdrawn content pack refuses instead of leaving a gap.
    if !template_resolves(origin) {
        return Err(GmActionRefusalReason::InverseUnsupported);
    }
    Ok(())
}

/// Take the capture for an armed removal, one instant before the entity stops
/// existing.
///
/// Returns `None` when nothing armed this entity — every ordinary scripted or
/// combat destruction, which must not pay for a capture walk it will never use.
pub fn capture_before_removal(world: &mut World, entity: Entity) -> Option<GmRemovalCapture> {
    let uuid = world
        .get::<crate::entities::spawner::EntityUuid>(entity)?
        .0
        .clone();
    let armed = world
        .get_resource_mut::<crate::world::server::WorldContentRuntime>()?
        .gm_despawn_captures
        .take_armed(&uuid)?;
    // The one canonical definition of "the complete state of an entity", taken
    // through the one function a save takes it with. See the module docs.
    let mut state = crate::snapshot::capture_entity_state(world, &uuid);
    if let Some(state) = state.as_mut() {
        // Live crew authority is not entity state an inverse restores; a restore
        // preserves CURRENT seating (PRD #1420).
        state.mesh_crew = None;
        if state.spawn.is_none() {
            // An authored `[[entity]]` block. The capture is still recorded —
            // the references it cleared are real and the preview names them —
            // but it cannot be rebuilt, and says so rather than guessing a
            // recipe out of components the template merge already consumed.
            return Some(GmRemovalCapture {
                action: armed.action,
                entity: uuid.clone(),
                names: authored_names(world, &uuid),
                tick: armed.tick,
                state: None,
                references: Vec::new(),
            });
        }
    }
    Some(GmRemovalCapture {
        action: armed.action,
        entity: uuid.clone(),
        names: authored_names(world, &uuid),
        tick: armed.tick,
        state: state.map(Box::new),
        references: Vec::new(),
    })
}

/// Every authored scenario name that currently resolves to one uuid, sorted.
fn authored_names(world: &World, uuid: &str) -> Vec<String> {
    world
        .get_resource::<crate::world::server::WorldContentRuntime>()
        .map(|runtime| {
            let mut names: Vec<String> = runtime
                .name_to_uuid
                .iter()
                .filter(|(_, id)| id.as_str() == uuid)
                .map(|(name, _)| name.clone())
                .collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

/// Finish a capture with the references the cleanup actually cleared, and retain
/// it under the run's bound.
pub fn record_capture(
    world: &mut World,
    capture: Option<GmRemovalCapture>,
    cleared: Vec<GmRemovalReference>,
) {
    let Some(mut capture) = capture else {
        return;
    };
    // An unrestorable capture keeps no reference list: nothing will ever put
    // them back and the preview says only that the removal cannot be reversed.
    if capture.state.is_some() {
        capture.references = cleared;
    }
    if let Some(mut runtime) = world.get_resource_mut::<crate::world::server::WorldContentRuntime>()
    {
        runtime.gm_despawn_captures.record(capture);
    }
}

/// Execute every armed inverse. Exclusive-world, because a rebuild is a spawn.
///
/// Each restore is atomic in the only sense that matters: [`restore_precheck`]
/// has already asked every fallible question against the same tick's state, and
/// the first write here is the spawn itself. A spawn that nonetheless fails
/// leaves the world exactly as it was and reports it, rather than applying the
/// entity row to nothing.
pub fn apply_armed_restores(world: &mut World) {
    let armed = match world.get_resource_mut::<crate::world::server::WorldContentRuntime>() {
        Some(mut runtime) => runtime.gm_despawn_captures.take_restores(),
        None => return,
    };
    for restore in armed {
        let capture = world
            .get_resource_mut::<crate::world::server::WorldContentRuntime>()
            .and_then(|mut runtime| runtime.gm_despawn_captures.consume(&restore.original));
        let Some(capture) = capture else {
            bevy::log::warn!(
                target: crate::logging::LogCat::World.target(),
                "GM despawn undo: no retained capture for {}/{} — nothing restored",
                restore.original.operator_id,
                restore.original.correlation.as_str()
            );
            continue;
        };
        let Some(state) = capture.state.as_deref() else {
            continue;
        };
        let Some(origin) = state.spawn.clone() else {
            continue;
        };
        let tick = world
            .get_resource::<crate::sim_tick::SimTick>()
            .map_or(0, |tick| tick.0);
        let Some(entity) = crate::snapshot::spawn_from_origin(world, state, &origin) else {
            bevy::log::warn!(
                target: crate::logging::LogCat::World.target(),
                "GM despawn undo: template '{}' no longer resolves — {} not restored",
                origin.template_path,
                capture.entity
            );
            continue;
        };
        // The same per-entity apply a resumed save performs, at the CURRENT
        // tick: no absent time is simulated and no global state is rewound.
        crate::snapshot::apply_entity_state(world, entity, state, tick);
        // The attributed GM knowledge overrides the cleanup cleared. Every
        // conflicting pair was already refused by `restore_precheck`.
        if let Some(mut runtime) =
            world.get_resource_mut::<crate::world::server::WorldContentRuntime>()
        {
            for reference in &capture.references {
                runtime
                    .contact_overrides
                    .entry(reference.observer.clone())
                    .or_default()
                    .insert(reference.target.clone(), reference.mode);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(uuid: &str) -> GmUndoReference {
        GmUndoReference {
            operator_id: "gm-1".into(),
            correlation: GmActionId::new(uuid.to_string()).unwrap(),
            sequence: 1,
        }
    }

    fn capture(correlation: &str, entity: &str, restorable: bool) -> GmRemovalCapture {
        GmRemovalCapture {
            action: reference(correlation),
            entity: entity.into(),
            names: Vec::new(),
            tick: 10,
            state: restorable.then(|| {
                Box::new(EntityState {
                    uuid: entity.into(),
                    spawn: Some(crate::world::spawn_origin::SpawnOrigin {
                        template_path: "assets/entities/raider.toml".into(),
                        name: entity.into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            }),
            references: Vec::new(),
        }
    }

    #[test]
    fn evicts_the_oldest_capture_once_the_run_bound_is_reached() {
        let mut store = GmDespawnCaptures::default();
        for index in 0..MAX_GM_DESPAWN_CAPTURES + 3 {
            store.record(capture(
                &format!("corr-{index}"),
                &format!("e-{index}"),
                true,
            ));
        }
        assert_eq!(store.entries().len(), MAX_GM_DESPAWN_CAPTURES);
        // The three oldest are gone; the newest are the ones a live GM is
        // actually about to reverse.
        assert!(store
            .capture_for("gm-1", &GmActionId::new("corr-0".to_string()).unwrap(), 1)
            .is_none());
        assert!(store
            .capture_for(
                "gm-1",
                &GmActionId::new(format!("corr-{}", MAX_GM_DESPAWN_CAPTURES + 2)).unwrap(),
                1
            )
            .is_some());
    }

    #[test]
    fn an_unrestorable_capture_is_retained_but_never_reported_restorable() {
        let mut store = GmDespawnCaptures::default();
        store.record(capture("corr-a", "authored-1", false));
        assert!(store
            .capture_for("gm-1", &GmActionId::new("corr-a".to_string()).unwrap(), 1)
            .is_some());
        assert!(!store.is_restorable("gm-1", &GmActionId::new("corr-a".to_string()).unwrap(), 1));
    }

    #[test]
    fn refuses_a_restore_whose_identity_is_live_again() {
        let capture = capture("corr-a", "raider-1", true);
        let live: std::collections::BTreeSet<String> = ["raider-1".to_string()].into();
        assert_eq!(
            restore_precheck(
                &capture,
                &live,
                &Default::default(),
                &Default::default(),
                |_| true
            ),
            Err(GmActionRefusalReason::RestoreIdentityOccupied)
        );
    }

    #[test]
    fn refuses_a_restore_whose_authored_name_the_world_has_re_bound() {
        let mut capture = capture("corr-a", "raider-1", true);
        capture.names = vec!["raider".into()];
        let live: std::collections::BTreeSet<String> = ["raider-2".to_string()].into();
        let mut names = std::collections::HashMap::new();
        names.insert("raider".to_string(), "raider-2".to_string());
        assert_eq!(
            restore_precheck(&capture, &live, &names, &Default::default(), |_| true),
            Err(GmActionRefusalReason::RestoreReferenceConflict)
        );
    }

    #[test]
    fn allows_a_restore_whose_authored_name_still_names_its_own_removed_uuid() {
        let mut capture = capture("corr-a", "raider-1", true);
        capture.names = vec!["raider".into()];
        let mut names = std::collections::HashMap::new();
        names.insert("raider".to_string(), "raider-1".to_string());
        assert_eq!(
            restore_precheck(
                &capture,
                &Default::default(),
                &names,
                &Default::default(),
                |_| true
            ),
            Ok(())
        );
    }

    #[test]
    fn refuses_a_restore_whose_contact_override_pair_has_a_newer_decision() {
        let mut capture = capture("corr-a", "raider-1", true);
        capture.references = vec![GmRemovalReference {
            observer: "player-1".into(),
            target: "raider-1".into(),
            mode: ContactMode::Reveal,
        }];
        let mut overrides = crate::gm_contact::ContactOverrides::new();
        overrides
            .entry("player-1".into())
            .or_default()
            .insert("raider-1".into(), ContactMode::Conceal);
        assert_eq!(
            restore_precheck(
                &capture,
                &Default::default(),
                &Default::default(),
                &overrides,
                |_| true
            ),
            Err(GmActionRefusalReason::RestoreReferenceConflict)
        );
    }

    #[test]
    fn allows_a_restore_when_only_unrelated_overrides_changed() {
        let mut capture = capture("corr-a", "raider-1", true);
        capture.references = vec![GmRemovalReference {
            observer: "player-1".into(),
            target: "raider-1".into(),
            mode: ContactMode::Reveal,
        }];
        let mut overrides = crate::gm_contact::ContactOverrides::new();
        overrides
            .entry("player-1".into())
            .or_default()
            .insert("freighter-9".into(), ContactMode::Conceal);
        assert_eq!(
            restore_precheck(
                &capture,
                &Default::default(),
                &Default::default(),
                &overrides,
                |_| true
            ),
            Ok(())
        );
    }

    #[test]
    fn refuses_a_restore_whose_template_no_longer_resolves() {
        let capture = capture("corr-a", "raider-1", true);
        assert_eq!(
            restore_precheck(
                &capture,
                &Default::default(),
                &Default::default(),
                &Default::default(),
                |_| false
            ),
            Err(GmActionRefusalReason::InverseUnsupported)
        );
    }
}
