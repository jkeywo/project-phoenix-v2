//! Shared action/Comms recipient vocabulary (issue #1533).
//!
//! An absent selection retains a caller's legacy behaviour. An explicit empty
//! selection is a valid empty set. Callers must never turn either a resolution
//! error or an empty result into an unaddressed action.

use std::collections::BTreeSet;

use crate::objective_instances::{ObjectiveInstanceManager, PlayerShipMembership};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecipientDiagnostic {
    pub tick: u64,
    pub source: Option<String>,
    pub line: Option<usize>,
    pub action: String,
    pub message: String,
}

/// Bounded observation only: never snapshotted, replayed, or used by gameplay.
#[derive(bevy::prelude::Resource, Default)]
pub struct RecipientDiagnostics(pub std::collections::VecDeque<RecipientDiagnostic>);

pub(crate) fn report(world: &mut bevy::prelude::World, diagnostic: RecipientDiagnostic) {
    if let Some(mut trace) = world.get_resource_mut::<crate::workshop::test_trace::TestTrace>() {
        trace.push(
            diagnostic.tick,
            diagnostic.source.as_deref(),
            diagnostic.line,
            crate::workshop::test_protocol::TestTraceKind::RecipientDiagnostic {
                action: diagnostic.action.clone(),
                message: diagnostic.message.clone(),
            },
        );
    }
    let mut diagnostics = world.get_resource_or_insert_with(RecipientDiagnostics::default);
    // Authoring feedback must remain bounded even for a repeat trigger that
    // continues selecting an absent slot indefinitely.
    const CAPACITY: usize = 64;
    if diagnostics.0.len() == CAPACITY {
        diagnostics.0.pop_front();
    }
    diagnostics.0.push_back(diagnostic);
}

pub(crate) fn queue_report(
    commands: &mut bevy::prelude::Commands,
    tick: u64,
    source: &str,
    line: Option<usize>,
    action: &str,
    message: String,
) {
    let diagnostic = RecipientDiagnostic {
        tick,
        source: Some(source.into()),
        line,
        action: action.into(),
        message,
    };
    commands.queue(move |world: &mut bevy::prelude::World| report(world, diagnostic));
}

/// Read the same authored slots and player-ship identity in every execution
/// adapter. Factions are borrowed from the caller because action/Comms systems
/// also mutate that registry; taking another resource borrow would conflict.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct RecipientSources<'w, 's> {
    config: Option<bevy::prelude::Res<'w, crate::world::config::WorldConfig>>,
    ships: bevy::prelude::Query<
        'w,
        's,
        (
            &'static crate::entities::spawner::EntityUuid,
            &'static crate::ship_slots::AuthoredShipSlotId,
            Option<&'static crate::entities::spawner::FactionComponent>,
        ),
        bevy::prelude::With<crate::server_app::Ship>,
    >,
}

/// Ephemeral inputs, rebuilt at execution. Hull/endpoint eligibility belongs to
/// the delivery adapter, not Objective assignment or its frozen crew history.
pub(crate) struct PreparedRecipients {
    pub fleet: Vec<PlayerShipMembership>,
    pub catalog: RecipientCatalog,
}

impl RecipientSources<'_, '_> {
    pub(crate) fn prepare(
        &self,
        registry: Option<&crate::entities::config_cache::FactionRegistryResource>,
    ) -> PreparedRecipients {
        let mut fleet: Vec<_> = self
            .ships
            .iter()
            .map(|(uuid, slot, faction)| PlayerShipMembership {
                ship_id: uuid.0.clone(),
                slot_id: slot.0.clone(),
                faction: faction
                    .and_then(|id| registry.and_then(|registry| registry.get(&id.0)))
                    .map(|faction| faction.name.clone())
                    .unwrap_or_default(),
            })
            .collect();
        fleet.sort_by(|a, b| a.ship_id.cmp(&b.ship_id).then(a.slot_id.cmp(&b.slot_id)));
        PreparedRecipients {
            fleet,
            catalog: RecipientCatalog {
                ship_slots: self
                    .config
                    .as_deref()
                    .map(|config| {
                        config
                            .effective_ship_slots()
                            .into_iter()
                            .map(|slot| slot.id)
                            .collect()
                    })
                    .unwrap_or_default(),
                factions: registry
                    .map(|registry| {
                        registry
                            .iter()
                            .map(|faction| faction.name.clone())
                            .collect()
                    })
                    .unwrap_or_default(),
                objective_instances: BTreeSet::new(),
            },
        }
    }
}

impl PreparedRecipients {
    /// Loaded declarations and current records augment the catalogue at the
    /// actual resolution point. Never re-read raw authored scripts or cached
    /// last-known crew views to infer current instance membership.
    pub(crate) fn resolve(
        mut self,
        selection: &RecipientSelection,
        script: Option<&crate::world::server::WorldScriptRuntime>,
        instances: &ObjectiveInstanceManager,
    ) -> Result<Vec<String>, RecipientRefusal> {
        if let Some(script) = script {
            self.catalog
                .objective_instances
                .extend(script.recipient_declarations.iter().cloned());
        }
        self.catalog
            .objective_instances
            .extend(instances.records().iter().map(|row| row.spec.key.clone()));
        selection.resolve(&self.catalog, &self.fleet, instances)
    }
}

/// Exclusive World adapter for deferred actions and script Objective commands.
/// Uses the same source query as ordinary systems; no catalogue is stored.
pub(crate) fn prepare_in_world(world: &mut bevy::prelude::World) -> PreparedRecipients {
    let mut sources = bevy::ecs::system::SystemState::<RecipientSources>::new(world);
    sources
        .get(world)
        .prepare(world.get_resource::<crate::entities::config_cache::FactionRegistryResource>())
}

/// Resolve at the deferred application boundary, after earlier Objective
/// commands have landed. Authored declarations augment the live registry.
pub(crate) fn resolve_in_world(
    world: &mut bevy::prelude::World,
    selection: &RecipientSelection,
) -> Result<Vec<String>, RecipientRefusal> {
    let mut prepared = prepare_in_world(world);
    // Lethal damage retains crew hull identity and Objective history. Delivery
    // excludes those hulls without changing their assignment or frozen view.
    let mut hulls = world.query::<(
        &crate::entities::spawner::EntityUuid,
        &crate::entities::spawner::EntitySystemHull,
    )>();
    let destroyed: BTreeSet<_> = hulls
        .iter(world)
        .filter(|(_, hull)| hull.0.total_current() <= 0.0)
        .map(|(uuid, _)| uuid.0.clone())
        .collect();
    prepared
        .fleet
        .retain(|ship| !destroyed.contains(&ship.ship_id));
    let empty = ObjectiveInstanceManager::default();
    let instances = world
        .get_resource::<crate::world::server::ObjectiveInstanceManagerRes>()
        .map(|manager| &manager.0)
        .unwrap_or(&empty);
    prepared.resolve(
        selection,
        world.get_resource::<crate::world::server::WorldScriptRuntime>(),
        instances,
    )
}

#[cfg(test)]
#[path = "recipients_tests.rs"]
mod tests;

pub use phoenix_sim_world::recipients::*;
