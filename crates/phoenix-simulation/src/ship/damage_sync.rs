use bevy::prelude::*;

use crate::core::messages::CoordinationPayload;
use crate::ship::components::{
    CoordinationEnqueue, LastSystemTiers, RepairHumanAlerted, ShipConfigComponent,
    ShipSystemControlSources,
};
use crate::ship::damage::DamageTier;

// ── Damage-tier → control gate sync ──────────────────────────────────────────

/// Synchronise only physical operating loss into the existing damage gate.
/// Intake/cooling faults leave a charged working capability available. GM state
/// remains independent in the resolver and cannot be cleared by restoration.
pub fn sync_console_damage_tiers(
    mut ships: Query<(
        Option<&crate::entities::spawner::EntitySystemHull>,
        Option<&crate::entities::spawner::EntityShipArcHull>,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        &mut ShipSystemControlSources,
    )>,
) {
    for (hull, arcs, damage, mut sources) in &mut ships {
        sources
            .0
            .set_destroyed(crate::ship::compartment_damage::is_catastrophic(
                damage, hull,
            ));
        if let Some(damage) = damage {
            for system in damage.definition.interior.placements.keys() {
                sources
                    .0
                    .set_offline(system.clone(), !damage.operational(system));
            }
            continue;
        }
        // Legacy prop/test owners are explicit; equipped ships have no HP map.
        if let Some(hull) = hull {
            for (system, _, _) in hull.0.entries() {
                sources.0.set_offline(
                    system.clone(),
                    matches!(
                        hull.0.tier_for(system),
                        DamageTier::Disabled | DamageTier::Destroyed
                    ),
                );
            }
        }
        if let Some(arcs) = arcs {
            for (id, _) in arcs.0.iter() {
                if let Some(system) = crate::ship::system_registry::shield_arc_system_id(id) {
                    sources.0.set_offline(
                        system,
                        matches!(
                            arcs.0.tier_for(id),
                            DamageTier::Disabled | DamageTier::Destroyed
                        ),
                    );
                }
            }
        }
    }
}

/// Detect damage-tier crossings and emit `CoordinationEnqueue::RepairRequest`
/// when a system drops to a worse tier (issue #682).
///
/// Runs in `SimSet::Damage` (after hull damage is applied). For each ship
/// with both `EntitySystemHull` and `LastSystemTiers`, compares the current
/// tier (via `tier_for`) against the last-seen tier.  On a crossing to a
/// *worse* tier, enqueues a `RepairRequest` for the system's owning station
/// (or `"core"` for ownerless systems).
///
/// A crossing INTO `Destroyed` files a `RepairRequest` like any other, and
/// additionally raises the captain Alert. Until issue #1013 it filed nothing —
/// a destroyed system was unrepairable, so the alert was all there was to say —
/// which meant a system knocked from `Operational` straight to `Destroyed` by
/// one hit was never reported to Repair at all and no team was ever sent. The
/// sweep repairs destroyed systems now, so the request is the one that matters.
pub fn detect_damage_tier_crossings(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut ships: Query<(
        Entity,
        Option<&crate::entities::spawner::EntitySystemHull>,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        &mut LastSystemTiers,
        &ShipConfigComponent,
        &ShipSystemControlSources,
        Option<&mut RepairHumanAlerted>,
        Option<&crate::entities::spawner::EntityUuid>,
        // Issue #893: the ship's own standing Tactical target lock. `Option`
        // because not every bare-`App` fixture in this crate spawns one.
        Option<&mut crate::console::weapons::TacticalRadarSelection>,
        // Out of red alert, ANY fresh damage is reported (not just a tier
        // crossing) — see the `current_tier == prev_tier` branch below.
        // `Option` because not every bare-`App` fixture spawns one; absent
        // reads as "not at red alert" (report freely).
        Option<&crate::ship::state::ShipRedAlert>,
    )>,
    mut coord_writer: MessageWriter<CoordinationEnqueue>,
    // Balance telemetry. `Option<ResMut<Messages<_>>>` so bare-`App` fixtures
    // that never registered the message still pass parameter validation.
    mut balance_events: Option<
        ResMut<bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
    >,
) {
    for (
        entity,
        hull_comp,
        damage,
        mut last_tiers,
        config,
        sources,
        mut alerted,
        ship_uuid,
        mut tactical_lock,
        red_alert,
    ) in &mut ships
    {
        let red_alert = red_alert.map(|ra| ra.0).unwrap_or(false);
        if let Some(damage) = damage {
            report_physical_crossings(
                entity,
                damage,
                &config.0,
                sources,
                &mut last_tiers,
                alerted.as_deref_mut(),
                ship_uuid,
                tactical_lock.as_deref_mut(),
                tick.as_ref().map(|tick| tick.0),
                &mut coord_writer,
                balance_events.as_deref_mut(),
            );
            continue;
        }
        let Some(hull_comp) = hull_comp else { continue };
        let hull = &hull_comp.0;
        for (system_id, cur, _max) in hull.entries() {
            let current_tier = hull.tier_for(system_id);
            let prev_tier = last_tiers
                .tiers
                .get(system_id)
                .copied()
                .unwrap_or(DamageTier::Operational);
            let prev_hp = last_tiers.hp.get(system_id).copied();

            // Balance tracer: report every tier crossing (either direction),
            // on every ship. A crossing to Disabled/Destroyed is the knockout
            // the ledger timestamps. Emitted unconditionally, alongside the
            // coordination traffic below. Skipped for a ship with no uuid —
            // there is no identity to key a ledger on.
            if current_tier != prev_tier {
                if let (Some(ref mut msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
                    msgs.write(crate::core::balance::BalanceEvent::SystemTierCrossed {
                        ship: uuid.0.clone(),
                        system_id: system_id.0.clone(),
                        from_tier: format!("{prev_tier:?}"),
                        to_tier: format!("{current_tier:?}"),
                    });
                }
            }

            let worsened_tier = current_tier > prev_tier;
            // Out of red alert, any fresh damage is worth a report even
            // within the same tier — combat noise justified batching by tier
            // crossing, but a scratch taken at peace should never sit
            // unreported just because it didn't cross a threshold.
            let any_damage_off_alert =
                !worsened_tier && !red_alert && prev_hp.is_some_and(|p| cur < p);

            if worsened_tier || any_damage_off_alert {
                let entry = hull.get(system_id).expect("just iterated entry");
                if worsened_tier && current_tier == DamageTier::Destroyed {
                    // Issue #893: a tactical radar reaching Destroyed clears
                    // the ship's standing target lock. Keyed on the SYSTEM
                    // crossing tiers, not on who set the lock, so a human's
                    // lock and an AI's lock clear the identical way — no
                    // origin branch (AGENTS.md #6). The existing #887
                    // admission gate (`sync_console_damage_tiers` marks
                    // `tactical-radar` offline on Disabled/Destroyed, which
                    // refuses a NEW `SetTarget` from either origin) is
                    // untouched; this is the companion half for the lock the
                    // ship already held when the radar went dark, which that
                    // gate never revisited.
                    if system_id.0 == crate::ship::system_registry::TACTICAL_RADAR_SYSTEM_ID {
                        if let Some(lock) = tactical_lock.as_deref_mut() {
                            lock.0 = None;
                        }
                    }

                    let sender_origin = sources.0.source_for(system_id);
                    let captain_system = crate::ship::system_registry::captain_system_id();
                    if let Some(address) =
                        crate::ship::coordination::address_for_system(&config.0, &captain_system)
                    {
                        let presentation = crate::core::messages::CoordinationPresentation::new(
                            "coordination.system_destroyed.title",
                            "coordination.system_destroyed.body",
                        )
                        .with_title_param("label", entry.display_name.clone())
                        .with_body_param("label", entry.display_name.clone());
                        coord_writer.write(CoordinationEnqueue {
                            source_entity: entity,
                            sender_origin,
                            address,
                            payload: CoordinationPayload::Alert {
                                title: "coordination.system_destroyed.title".to_string(),
                                body: "coordination.system_destroyed.body".to_string(),
                            },
                            presentation,
                            sender_label: system_id.0.clone(),
                            sender_system: system_id.clone(),
                        });
                    }
                    // NO `continue` here (issue #1013). The Alert is an
                    // addition to the RepairRequest below, not a replacement
                    // for it: a system that crosses straight from Operational
                    // to Destroyed in one hit passes through no intermediate
                    // tier, so this is its ONLY chance to be reported to
                    // Repair, and skipping it left the station unrepairable in
                    // practice however capable the sweep was.
                    //
                    // A consequence, deliberate and bounded: a system a team
                    // keeps un-latching under sustained fire re-crosses INTO
                    // Destroyed each cycle and so re-raises this Alert each
                    // cycle, where pre-#1013 the crossing could only happen
                    // once. That is the truthful report — it really was
                    // destroyed again — and the repeat traffic is bounded
                    // because `push_or_merge` merges the accompanying
                    // RepairRequest rather than growing the queue.
                }

                let group = crate::modifiers::repair_teams::repair_group(system_id, &config.0);
                let station_label =
                    crate::console::repair::server::repair_group_label(&config.0, &group);
                let deficit = entry.max - entry.current;
                let sender_origin = sources.0.source_for(system_id);

                let repair_system = crate::ship::system_registry::repair_system_id();
                if let Some(address) =
                    crate::ship::coordination::address_for_system(&config.0, &repair_system)
                {
                    coord_writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin,
                        address,
                        payload: CoordinationPayload::RepairRequest {
                            observation: Some(phoenix_model::observation::ObservationMetadata {
                                subject: phoenix_model::observation::ObservationSubject::System {
                                    id: system_id.clone(),
                                },
                                provenance: phoenix_model::observation::ObservationProvenance {
                                    kind:
                                        phoenix_model::observation::ObservationSource::SystemReport,
                                    system: Some(system_id.clone()),
                                    label: String::new(),
                                },
                                observed_tick: tick.as_ref().map(|tick| tick.0),
                                received_tick: None,
                                uncertainty:
                                    phoenix_model::observation::ObservationUncertainty::Qualitative,
                            }),
                            system_id: system_id.clone(),
                            group,
                            station_label: station_label.clone(),
                            tier: current_tier,
                            // Exact on the host-internal enqueue — the AI repair
                            // queue sorts by it. Coarsened to `None` on the way out
                            // to a human console unless the recipient is entitled
                            // to exact detail for this system (issue #737).
                            deficit: Some(deficit),
                        },
                        // Deliberately names only the coarse Station bucket.
                        // Exact `deficit` remains semantic payload data behind
                        // the #737 per-recipient coarsening boundary.
                        presentation: crate::core::messages::CoordinationPresentation::titled(
                            "coordination.repair.title",
                        )
                        .with_title_param("label", station_label),
                        sender_label: system_id.0.clone(),
                        sender_system: system_id.clone(),
                    });
                }
            } else if current_tier == DamageTier::Operational && prev_tier > DamageTier::Operational
            {
                let group = crate::modifiers::repair_teams::repair_group(system_id, &config.0);
                if let Some(ref mut a) = alerted {
                    if crate::console::repair::server::all_systems_in_group_are_operational(
                        &group, hull, &config.0,
                    ) {
                        a.0.remove(&group);
                    }
                }
            }
        }
        // Disarmed detection (issue #841): a ship is disarmed when every
        // weapon-*emitter* system (phaser bank, torpedo tube, blaster bank) is
        // non-operational — it can no longer put a shot downrange. Emitted once
        // on the transition into fully-disarmed, using the pre-update
        // `last_tiers` for the "before" state. Reported, never terminal.
        //
        // Enabling systems (phaser control, torpedo magazine) are deliberately
        // *not* in the emitter set: a live control panel over dead banks is
        // still a ship that cannot fire, so keying disarm off the emitters
        // reports the true "can't attack" moment.
        if let (Some(ref mut msgs), Some(uuid)) = (balance_events.as_mut(), ship_uuid) {
            let emitters: Vec<&crate::core::messages::SystemId> = config
                .0
                .systems
                .iter()
                .filter(|s| is_weapon_emitter_kind(&s.kind))
                .map(|s| &s.id)
                .collect();
            if !emitters.is_empty() {
                let nonoperational =
                    |tier: DamageTier| matches!(tier, DamageTier::Disabled | DamageTier::Destroyed);
                let now_disarmed = emitters
                    .iter()
                    .all(|sid| nonoperational(hull.tier_for(sid)));
                let prev_disarmed = emitters.iter().all(|sid| {
                    nonoperational(
                        last_tiers
                            .tiers
                            .get(sid)
                            .copied()
                            .unwrap_or(DamageTier::Operational),
                    )
                });
                if now_disarmed && !prev_disarmed {
                    msgs.write(crate::core::balance::BalanceEvent::Disarmed {
                        ship: uuid.0.clone(),
                    });
                }
            }
        }

        for (system_id, cur, _max) in hull.entries() {
            last_tiers
                .tiers
                .insert(system_id.clone(), hull.tier_for(system_id));
            last_tiers.hp.insert(system_id.clone(), cur);
        }
    }
}

/// Physical reports expose operating symptoms, not private component locations or
/// a synthetic percentage-HP deficit. Diagnosis/remedies remain Repair work.
#[allow(clippy::too_many_arguments)]
fn report_physical_crossings(
    entity: Entity,
    damage: &crate::ship::compartment_damage::ShipDamage,
    config: &crate::ship::config::ShipConfig,
    sources: &ShipSystemControlSources,
    last: &mut LastSystemTiers,
    _alerted: Option<&mut RepairHumanAlerted>,
    uuid: Option<&crate::entities::spawner::EntityUuid>,
    mut tactical: Option<&mut crate::console::weapons::TacticalRadarSelection>,
    tick: Option<u64>,
    writer: &mut MessageWriter<CoordinationEnqueue>,
    mut balance: Option<&mut bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>,
) {
    let prior_disarmed = config
        .systems
        .iter()
        .filter(|system| is_weapon_emitter_kind(&system.kind))
        .all(|system| {
            matches!(
                last.tiers.get(&system.id),
                Some(DamageTier::Disabled | DamageTier::Destroyed)
            )
        });
    for system in &config.systems {
        let tier = damage.operating_tier(&system.id);
        let previous = last
            .tiers
            .get(&system.id)
            .copied()
            .unwrap_or(DamageTier::Operational);
        if tier != previous {
            if let (Some(events), Some(uuid)) = (balance.as_deref_mut(), uuid) {
                events.write(crate::core::balance::BalanceEvent::SystemTierCrossed {
                    ship: uuid.0.clone(),
                    system_id: system.id.0.clone(),
                    from_tier: format!("{previous:?}"),
                    to_tier: format!("{tier:?}"),
                });
            }
        }
        if tier != previous {
            if !damage.operational(&system.id)
                && system.kind == crate::ship::system_registry::TACTICAL_RADAR_KIND
            {
                if let Some(lock) = tactical.as_deref_mut() {
                    lock.0 = None;
                }
            }
            if tier > previous && tier == DamageTier::Destroyed {
                if let Some(address) = crate::ship::coordination::address_for_system(
                    config,
                    &crate::ship::system_registry::captain_system_id(),
                ) {
                    writer.write(CoordinationEnqueue {
                        source_entity: entity,
                        sender_origin: sources.0.source_for(&system.id),
                        address,
                        payload: CoordinationPayload::Alert {
                            title: "coordination.system_destroyed.title".into(),
                            body: "coordination.system_destroyed.body".into(),
                        },
                        presentation: crate::core::messages::CoordinationPresentation::new(
                            "coordination.system_destroyed.title",
                            "coordination.system_destroyed.body",
                        )
                        .with_title_param("label", system.id.0.clone())
                        .with_body_param("label", system.id.0.clone()),
                        sender_label: system.id.0.clone(),
                        sender_system: system.id.clone(),
                    });
                }
            }
            let group = crate::modifiers::repair_teams::repair_group(&system.id, config);
            let label = crate::console::repair::server::repair_group_label(config, &group);
            if let Some(address) = crate::ship::coordination::address_for_system(
                config,
                &crate::ship::system_registry::repair_system_id(),
            ) {
                writer.write(CoordinationEnqueue {
                    source_entity: entity,
                    sender_origin: sources.0.source_for(&system.id),
                    address,
                    payload: CoordinationPayload::RepairRequest {
                        observation: Some(phoenix_model::observation::ObservationMetadata {
                            subject: phoenix_model::observation::ObservationSubject::System {
                                id: system.id.clone(),
                            },
                            provenance: phoenix_model::observation::ObservationProvenance {
                                kind: phoenix_model::observation::ObservationSource::SystemReport,
                                system: Some(system.id.clone()),
                                label: String::new(),
                            },
                            observed_tick: tick,
                            received_tick: None,
                            uncertainty:
                                phoenix_model::observation::ObservationUncertainty::Qualitative,
                        }),
                        system_id: system.id.clone(),
                        group,
                        station_label: label.clone(),
                        tier,
                        deficit: None,
                    },
                    presentation: crate::core::messages::CoordinationPresentation::titled(
                        "coordination.repair.title",
                    )
                    .with_title_param("label", label),
                    sender_label: system.id.0.clone(),
                    sender_system: system.id.clone(),
                });
            }
        }
        last.tiers.insert(system.id.clone(), tier);
        last.hp.remove(&system.id);
    }
    let emitters: Vec<_> = config
        .systems
        .iter()
        .filter(|system| is_weapon_emitter_kind(&system.kind))
        .collect();
    if !emitters.is_empty()
        && !prior_disarmed
        && emitters
            .iter()
            .all(|system| !damage.operational(&system.id))
    {
        if let (Some(events), Some(uuid)) = (balance, uuid) {
            events.write(crate::core::balance::BalanceEvent::Disarmed {
                ship: uuid.0.clone(),
            });
        }
    }
}

/// Whether a system `kind` is a weapon *emitter* — a system that itself puts a
/// shot downrange (a phaser bank, torpedo tube, or blaster bank), as opposed to
/// an enabling system (phaser control, torpedo magazine). Used by the
/// `Disarmed` detector to decide when a ship can no longer attack.
fn is_weapon_emitter_kind(kind: &str) -> bool {
    kind == crate::ship::system_registry::PHASER_BANK_KIND
        || kind == crate::ship::system_registry::TORPEDO_TUBE_KIND
        || kind == crate::ship::system_registry::BLASTER_BANK_KIND
}

#[cfg(test)]
#[path = "damage_sync_tests.rs"]
mod tests;
