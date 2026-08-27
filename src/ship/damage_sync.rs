use bevy::prelude::*;

use crate::core::messages::CoordinationPayload;
use crate::ship::components::{
    CoordinationEnqueue, LastSystemTiers, RepairHumanAlerted, ShipConfigComponent,
    ShipSystemControlSources,
};
use crate::ship::damage::DamageTier;

// ── Damage-tier → control gate sync ──────────────────────────────────────────

/// Bevy system that synchronises `ControlSourceResolver.offline_systems` with
/// the current damage tiers of each system in the ship hull.
///
/// Runs in `SimSet::Damage` (after hull damage is applied). For every ship that
/// carries both an [`EntitySystemHull`](crate::entities::spawner::EntitySystemHull)
/// (wrapping [`SystemHull`]) and `ShipSystemControlSources`:
///
/// - Systems in `Disabled` or `Destroyed` tier: their corresponding `SystemId`
///   is added to `offline_systems`.
/// - Systems in `Operational` or `Damaged` tier: their corresponding
///   `SystemId` is removed from `offline_systems` (restoring normal gating).
///
/// The `SystemId` for each entry is the key of the [`SystemHull`] map
/// directly — no `Console` → `SystemId` translation is needed.
///
/// Post-#514: also iterates the ship's `ShipArcHull` (when present) and flips
/// each arc's fine `SystemId("shield-arc-<id>")` in/out of `offline_systems`
/// using the same tier-derivation policy. Ships without a `ShipArcHull` (NPCs,
/// legacy fixtures) are unaffected.
///
/// Fix to issue #617: earlier this system iterated BOTH `EntityConsoleHull`
/// AND `EntitySystemHull` in parallel. In production only one of the two was
/// mutated by damage code, so the second (unmodified) iteration silently
/// cleared `offline_systems` entries that the first iteration correctly
/// inserted. The reviewer caught this and the fix drops the duplicate
/// iteration and picks `EntitySystemHull` as the single source of truth.
pub fn sync_console_damage_tiers(
    mut ships: Query<(
        &crate::entities::spawner::EntitySystemHull,
        Option<&crate::entities::spawner::EntityShipArcHull>,
        &mut ShipSystemControlSources,
    )>,
) {
    for (system_hull_component, arc_hull_opt, mut control_sources) in ships.iter_mut() {
        let hull = &system_hull_component.0;
        for (sid, _cur, _max) in hull.entries() {
            let tier = hull.tier_for(sid);
            match tier {
                DamageTier::Disabled | DamageTier::Destroyed => {
                    control_sources.0.set_offline(sid.clone(), true);
                }
                DamageTier::Operational | DamageTier::Damaged => {
                    control_sources.0.set_offline(sid.clone(), false);
                }
            }
        }
        // Per-arc hull tier sync (issue #514).
        if let Some(arc_hull_component) = arc_hull_opt {
            let arc_hull = &arc_hull_component.0;
            for (arc_id, _entry) in arc_hull.iter() {
                let Some(sid) = crate::ship::system_registry::shield_arc_system_id(arc_id) else {
                    continue;
                };
                let tier = arc_hull.tier_for(arc_id);
                match tier {
                    DamageTier::Disabled | DamageTier::Destroyed => {
                        control_sources.0.set_offline(sid, true);
                    }
                    DamageTier::Operational | DamageTier::Damaged => {
                        control_sources.0.set_offline(sid, false);
                    }
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
    mut ships: Query<(
        Entity,
        &crate::entities::spawner::EntitySystemHull,
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

                let system_config = config.0.system(system_id);
                let (station_id, station_label) = system_config
                    .and_then(|s| s.station.as_ref())
                    .map(|station| {
                        (
                            station.0.clone(),
                            crate::ship::coordination::coordination_addressee_label(
                                &crate::core::messages::CoordinationAddress::Station(
                                    station.clone(),
                                ),
                            ),
                        )
                    })
                    .unwrap_or_else(|| {
                        (
                            crate::console::repair::visibility::CORE_BUCKET_ID.to_string(),
                            crate::ship::coordination::CHATTER_ADDRESSEE_CORE.to_string(),
                        )
                    });
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
                            system_id: system_id.clone(),
                            station_id,
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
                let system_config = config.0.system(system_id);
                let station_id = system_config
                    .and_then(|s| s.station.as_ref())
                    .map(|s| s.0.clone())
                    .unwrap_or_else(|| {
                        crate::console::repair::visibility::CORE_BUCKET_ID.to_string()
                    });
                if let Some(ref mut a) = alerted {
                    if crate::console::repair::server::all_systems_in_station_are_operational(
                        &station_id,
                        hull,
                        &config.0,
                    ) {
                        a.0.remove(&station_id);
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
mod tests {
    use super::*;
    use crate::lobby::LobbyPlugin;
    use crate::modifiers::ShipModifiers;
    use crate::server_app::{LocalShip, Ship, ShipBoost, ShipImpulse};
    use crate::ship::components::{
        ActiveStationRatings, CoordinationQueue, HelmWaypointClearance, LastHelmInput,
    };
    use crate::ship::control_source::ControlTickPolicy;
    use crate::ship::state::ShipPhysics;
    use crate::ship::test_support::*;
    use crate::ship_plugin::ShipPlugin;

    // ── sync_console_damage_tiers integration tests ───────────────────────────

    /// Helper: get the policy for a system from the ship's ControlSourceResolver.
    fn get_policy(app: &mut App, system_id: &str) -> ControlTickPolicy {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemControlSources, With<Ship>>();
        let sources = q
            .single(app.world())
            .expect("Ship with ShipSystemControlSources");
        sources
            .0
            .policy_for(&crate::core::messages::SystemId(system_id.into()))
    }

    fn set_hp(app: &mut App, system_id: crate::core::messages::SystemId, hp: f32) {
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let mut binding = app.world_mut().entity_mut(ship);
        let mut hull_component = binding
            .get_mut::<crate::entities::spawner::EntitySystemHull>()
            .unwrap();
        // Wipe then restore to exact HP.
        hull_component
            .0
            .apply_damage(1_000_000.0, &mut crate::sim_rng::unseeded_test_rng());
        hull_component.0.restore(&system_id, hp);
    }

    #[test]
    fn disabled_console_gates_human_and_ai_input() {
        let mut app = test_app();
        // Helm console max_hp = 25. Disabled threshold = 25 % = 6.25 HP.
        // Set Helm to 5 HP (below disabled threshold) → Disabled tier.
        set_hp(
            &mut app,
            crate::core::messages::SystemId("helm".into()),
            5.0,
        );
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            !policy.accept_human_input,
            "Disabled console must not accept human input"
        );
        assert!(!policy.operate_ai, "Disabled console must not operate AI");
    }

    #[test]
    fn destroyed_console_gates_human_and_ai_input() {
        let mut app = test_app();
        // Wipe helm to 0 HP → Destroyed tier.
        set_hp(
            &mut app,
            crate::core::messages::SystemId("helm".into()),
            0.0,
        );
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            !policy.accept_human_input,
            "Destroyed console must not accept human input"
        );
        assert!(!policy.operate_ai, "Destroyed console must not operate AI");
    }

    #[test]
    fn restored_console_re_enables_input() {
        let mut app = test_app();
        // First disable helm.
        set_hp(
            &mut app,
            crate::core::messages::SystemId("helm".into()),
            5.0,
        );
        tick(&mut app);
        // Verify it is gated.
        assert!(!get_policy(&mut app, "helm").accept_human_input);

        // Now restore to operational HP.
        set_hp(
            &mut app,
            crate::core::messages::SystemId("helm".into()),
            25.0,
        );
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            policy.accept_human_input,
            "Restored console must accept human input again"
        );
    }

    #[test]
    fn damaged_tier_does_not_gate_input() {
        let mut app = test_app();
        // Helm at 50% = 12.5 HP → Damaged tier (25 % < 50 % < 75 %).
        // Damaged tier must NOT block input — only Disabled and Destroyed do.
        set_hp(
            &mut app,
            crate::core::messages::SystemId("helm".into()),
            12.5,
        );
        tick(&mut app);

        let policy = get_policy(&mut app, "helm");
        assert!(
            policy.accept_human_input,
            "Damaged (but not Disabled) console must still accept human input"
        );
    }

    #[test]
    fn engine_port_hull_damage_gates_engine_offline() {
        let mut app = test_app_with_engine_hull();

        // Zero out the port engine HP (destroyed tier).
        set_console_hp_direct(
            &mut app,
            crate::core::messages::SystemId("helm-engine-port".into()),
            0.0,
        );
        tick(&mut app);

        // After sync_console_damage_tiers, offline_systems should contain helm-engine-port.
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let control_sources = app
            .world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap();
        let port_id = crate::ship::system_registry::helm_engine_port_system_id();
        assert!(
            control_sources.0.is_offline(&port_id),
            "helm-engine-port should be in offline_systems when HP = 0"
        );
    }

    /// Regression test for the reviewer's finding on issue #617.
    ///
    /// Before the fix, `sync_console_damage_tiers` iterated BOTH
    /// `EntityConsoleHull` AND `EntitySystemHull`. In production only the
    /// former was mutated by damage code, so the second (unmodified)
    /// iteration silently cleared every `offline_systems` entry that the
    /// first correctly inserted — meaning a hull-destroyed system would be
    /// re-marked online on the very next tick.
    ///
    /// This test spawns a ship carrying only `EntitySystemHull`, damages the
    /// helm system to 0 HP, runs the sync system TWICE, and asserts the
    /// SystemId stays in `offline_systems` across both ticks. Under the old
    /// buggy behaviour the second tick would have cleared the entry.
    #[test]
    fn sync_damage_tiers_keeps_disabled_system_offline_across_ticks() {
        let mut app = test_app();
        let helm_sid = crate::core::messages::SystemId("helm".into());

        // Damage the helm system to 0 HP (Destroyed tier).
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut hull = entity_mut
                .get_mut::<crate::entities::spawner::EntitySystemHull>()
                .unwrap();
            hull.0.set_hp(&helm_sid, 0.0);
        }

        // Tick 1: sync_console_damage_tiers runs, must insert helm into
        // offline_systems.
        tick(&mut app);
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let control_sources = app
                .world()
                .entity(ship)
                .get::<ShipSystemControlSources>()
                .unwrap();
            assert!(
                control_sources.0.is_offline(&helm_sid),
                "after tick 1, helm should be in offline_systems (HP = 0)"
            );
        }

        // Tick 2: no damage change. Under the pre-fix bug the second loop
        // (over the unmutated sibling component) would have re-marked helm
        // as Operational and cleared it from offline_systems. After the fix
        // there is only one iteration, so the entry must persist.
        tick(&mut app);
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let control_sources = app
                .world()
                .entity(ship)
                .get::<ShipSystemControlSources>()
                .unwrap();
            assert!(
                control_sources.0.is_offline(&helm_sid),
                "after tick 2, helm MUST still be in offline_systems (regression \
                 for issue #617 dual-iteration clobber bug)"
            );
        }
    }

    #[test]
    fn engine_port_offline_reduces_thrust_compared_to_both_online() {
        // With both engines online, terminal velocity = max_speed (25 m/s by default).
        // With one engine offline, effective thrust = 0.5, so terminal = 0.5 * max_speed = 12.5.
        // We run enough ticks to approach terminal velocity at the 50%-thrust case,
        // then verify the one-engine-offline ship is slower than the both-online ship.
        const TICK_MS: u64 = 34; // slightly above 1/30s, the physics' own scale
        const TICKS: usize = 120; // 120 ticks × 34ms ≈ 4s, enough to reach ~12.5 m/s terminal

        let make_app = || {
            let mut app = test_app_with_engine_hull();
            // Re-pin the harness to a 34 ms logical tick (issue #895): the
            // helper keeps timestep == frame advance, one step per update.
            crate::ship::test_support::drive_one_fixed_step_per_update(
                &mut app,
                std::time::Duration::from_millis(TICK_MS),
            );
            app
        };

        // Full-ahead helm intent, seeded directly on the intent component
        // (issue #824): `process_helm_inputs` applies admitted commands to
        // the intents rather than replaying `LastHelmInput` every tick, so a
        // test that wants sustained thrust seeds the intent the integrator
        // actually reads.
        let set_full_thrust = |app: &mut App| {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            app.world_mut()
                .entity_mut(ship)
                .insert(crate::ship::helm::ThrustInput(1.0));
        };

        // ── Both engines online ────────────────────────────────────────────
        let mut app_both = make_app();
        set_full_thrust(&mut app_both);
        for _ in 0..TICKS {
            tick(&mut app_both);
        }
        let speed_both = app_both
            .world_mut()
            .query_filtered::<&ShipPhysics, With<LocalShip>>()
            .single(app_both.world())
            .unwrap()
            .forward_speed;

        // ── Port engine disabled ───────────────────────────────────────────
        // Zero the port engine HP, tick once so sync_console_damage_tiers runs
        // (populating offline_systems), then drive at full thrust for TICKS more.
        let mut app_one = make_app();
        set_console_hp_direct(
            &mut app_one,
            crate::core::messages::SystemId("helm-engine-port".into()),
            0.0,
        );
        tick(&mut app_one); // let Damage tier propagate
        set_full_thrust(&mut app_one);
        for _ in 0..TICKS {
            tick(&mut app_one);
        }
        let speed_one = app_one
            .world_mut()
            .query_filtered::<&ShipPhysics, With<LocalShip>>()
            .single(app_one.world())
            .unwrap()
            .forward_speed;

        // With enough ticks, app_both should be near 25 m/s and app_one near 12.5 m/s.
        assert!(
            speed_one < speed_both,
            "forward_speed with one engine offline ({speed_one:.4}) should be less than \
             with both engines online ({speed_both:.4})"
        );
    }

    // ── Fine Power system → offline_systems tests (issue #513) ────────────────

    /// Build an app whose ship carries PowerReactor + PowerBattery hull
    /// entries. Used to exercise the hull → offline_systems chain for the
    /// fine power kinds.
    fn test_app_with_power_hull() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .add_plugins(ShipPlugin);
        // One fixed step per update (issue #895): ShipPlugin's systems run on
        // the logical tick, and each harness tick advances it once.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );
        let hull_config = &[
            (crate::core::messages::SystemId("helm".into()), 25.0_f32),
            (crate::core::messages::SystemId("tactical".into()), 25.0),
            (
                crate::core::messages::SystemId("power-reactor".into()),
                15.0,
            ),
            (
                crate::core::messages::SystemId("power-battery".into()),
                10.0,
            ),
            (crate::core::messages::SystemId("shields".into()), 25.0),
        ];
        let ship = app
            .world_mut()
            .spawn((
                Ship,
                LocalShip,
                Transform::default(),
                ShipPhysics::default(),
                ShipConfigComponent::default(),
                ShipSystemControlSources::default(),
                ActiveStationRatings::default(),
                CoordinationQueue::default(),
                crate::core::messages::AdmittedCommands::default(),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::entities::spawner::EntitySystemHull(
                    crate::ship::damage::SystemHull::from_config(hull_config),
                ),
                LastHelmInput::default(),
                crate::server_app::ShipShields(
                    crate::weapons::shield::ShieldSystem::default(),
                    0.5,
                ),
                ShipImpulse(crate::ship::impulse::ImpulseState::new()),
            ))
            .id();
        app.world_mut()
            .entity_mut(ship)
            .insert((ShipModifiers::new(), ShipBoost::default()));
        app
    }

    #[test]
    fn damaging_power_reactor_hull_to_disabled_puts_power_reactor_in_offline_systems() {
        let mut app = test_app_with_power_hull();
        set_console_hp_direct(
            &mut app,
            crate::core::messages::SystemId("power-reactor".into()),
            0.0,
        );
        tick(&mut app);

        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let control_sources = app
            .world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap();
        let reactor_id = crate::ship::system_registry::power_reactor_system_id();
        assert!(
            control_sources.0.is_offline(&reactor_id),
            "power-reactor should be in offline_systems when its hull HP is 0 (Disabled/Destroyed)"
        );
    }

    #[test]
    fn damaging_power_battery_hull_to_disabled_puts_power_battery_in_offline_systems() {
        let mut app = test_app_with_power_hull();
        set_console_hp_direct(
            &mut app,
            crate::core::messages::SystemId("power-battery".into()),
            0.0,
        );
        tick(&mut app);

        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let control_sources = app
            .world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap();
        let battery_id = crate::ship::system_registry::power_battery_system_id();
        assert!(
            control_sources.0.is_offline(&battery_id),
            "power-battery should be in offline_systems when its hull HP is 0 (Disabled/Destroyed)"
        );
    }

    // ── Issue #514 shield-arc hull tier sync tests ────────────────────────────

    /// Build a test app with a shield-arc-hull equipped ship. Uses a
    /// small hull budget so `set_arc_hp` is trivial for tests.
    fn test_app_with_shield_arc_hull() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .add_plugins(ShipPlugin);
        // One fixed step per update (issue #895): ShipPlugin's systems run on
        // the logical tick, and each harness tick advances it once.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(200),
        );

        let tc = crate::ship::damage::ConsoleTierConfig::default();
        let arc_hull = crate::ship::damage::ShipArcHull::from_entries(vec![
            (
                "fore".into(),
                crate::ship::damage::ArcHullEntry {
                    current: 10.0,
                    max: 10.0,
                    tier_config: tc,
                },
            ),
            (
                "aft".into(),
                crate::ship::damage::ArcHullEntry {
                    current: 10.0,
                    max: 10.0,
                    tier_config: tc,
                },
            ),
        ]);
        let hull_config = &[(crate::core::messages::SystemId("helm".into()), 25.0_f32)];
        let ship = app
            .world_mut()
            .spawn((
                Ship,
                LocalShip,
                Transform::default(),
                ShipPhysics::default(),
                ShipConfigComponent::default(),
                ShipSystemControlSources::default(),
                ActiveStationRatings::default(),
                CoordinationQueue::default(),
                crate::core::messages::AdmittedCommands::default(),
                crate::server_app::ShipSystemBlackboards::default(),
                crate::ai::server::AiHighFidelity,
                crate::entities::spawner::EntitySystemHull(
                    crate::ship::damage::SystemHull::from_config(hull_config),
                ),
                LastHelmInput::default(),
                crate::server_app::ShipShields(
                    crate::weapons::shield::ShieldSystem::default(),
                    0.5,
                ),
            ))
            .id();
        app.world_mut().entity_mut(ship).insert((
            ShipModifiers::new(),
            ShipBoost::default(),
            ShipImpulse(crate::ship::impulse::ImpulseState::new()),
            crate::console_ai::server::ShipFrequencyHintState::default(),
            crate::entities::spawner::EntityShipArcHull(arc_hull),
        ));
        app.world_mut().entity_mut(ship).insert((
            crate::ship::helm::ThrustInput::default(),
            crate::ship::helm::SteeringInput::default(),
            crate::ship::helm::LateralThrustInput::default(),
            crate::ship::helm::VerticalThrustInput::default(),
            crate::ship::helm::ImpulseCommand::default(),
            crate::ship::helm::BoostCommand::default(),
            // The console-owned surfaces the AI helm derives its goals from
            // (issue #702) — see `HelmAiSurfaces`.
            crate::console::weapons::TacticalRadarSelection::default(),
            crate::console::navigation::NavigationWaypoint::default(),
            HelmWaypointClearance::default(),
            crate::ai::server::ObjectiveCursors::default(),
        ));
        app
    }

    #[test]
    fn sync_console_damage_tiers_flips_shield_arc_offline_on_disabled_hp() {
        let mut app = test_app_with_shield_arc_hull();
        // Zero the fore arc hull HP.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut arc_hull = entity_mut
                .get_mut::<crate::entities::spawner::EntityShipArcHull>()
                .unwrap();
            arc_hull.0.set_hp("fore", 0.0);
        }
        tick(&mut app);
        // After sync, offline_systems must contain shield-arc-fore.
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let cs = app
            .world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap();
        let fore_sid = crate::ship::system_registry::shield_arc_system_id("fore").expect("fore");
        assert!(
            cs.0.is_offline(&fore_sid),
            "shield-arc-fore must be in offline_systems when its arc HP is 0"
        );
        let aft_sid = crate::ship::system_registry::shield_arc_system_id("aft").expect("aft");
        assert!(
            !cs.0.is_offline(&aft_sid),
            "shield-arc-aft must NOT be in offline_systems (still at full HP)"
        );
    }

    #[test]
    fn sync_console_damage_tiers_removes_shield_arc_from_offline_on_repair() {
        let mut app = test_app_with_shield_arc_hull();
        // Zero fore, tick to insert into offline_systems.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut arc_hull = entity_mut
                .get_mut::<crate::entities::spawner::EntityShipArcHull>()
                .unwrap();
            arc_hull.0.set_hp("fore", 0.0);
        }
        tick(&mut app);
        // Restore fore to full.
        {
            let ship = app
                .world_mut()
                .query_filtered::<Entity, With<LocalShip>>()
                .single(app.world())
                .unwrap();
            let mut entity_mut = app.world_mut().entity_mut(ship);
            let mut arc_hull = entity_mut
                .get_mut::<crate::entities::spawner::EntityShipArcHull>()
                .unwrap();
            arc_hull.0.set_hp("fore", 10.0);
        }
        tick(&mut app);
        let ship = app
            .world_mut()
            .query_filtered::<Entity, With<LocalShip>>()
            .single(app.world())
            .unwrap();
        let cs = app
            .world()
            .entity(ship)
            .get::<ShipSystemControlSources>()
            .unwrap();
        let fore_sid = crate::ship::system_registry::shield_arc_system_id("fore").expect("fore");
        assert!(
            !cs.0.is_offline(&fore_sid),
            "shield-arc-fore must be removed from offline_systems after repair"
        );
    }

    // ── Issue #841: tier-crossing family balance emission ─────────────────────

    /// Tier-crossing family: destroying the only weapon system on a NON-LOCAL
    /// ship emits both `SystemTierCrossed` (to `Destroyed`) and `Disarmed`,
    /// keyed on that ship — guarding the unconditional, all-ships convention.
    #[test]
    fn weapon_system_destruction_emits_tier_crossed_and_disarmed_for_a_non_local_ship() {
        use crate::core::balance::BalanceEvent;
        use crate::core::messages::SystemId;
        use bevy::ecs::message::Messages;

        let mut app = App::new();
        app.add_message::<CoordinationEnqueue>()
            .add_message::<BalanceEvent>()
            .add_systems(Update, detect_damage_tier_crossings);

        // A ship whose only weapon is one phaser bank — so knocking it out is
        // both a tier crossing and a full disarm. `ShipConfigComponent::default`
        // ships a full default weapons suite, so clear it to the single system
        // this fixture actually carries in its hull.
        let mut config = ShipConfigComponent::default();
        config.0.systems.clear();
        config
            .0
            .systems
            .push(crate::ship::config::SystemInstanceConfig {
                id: SystemId("phaser-fore".into()),
                kind: crate::ship::system_registry::PHASER_BANK_KIND.into(),
                station: None,
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            });

        let hull = crate::ship::damage::SystemHull::from_config(&[(
            SystemId("phaser-fore".into()),
            100.0,
        )]);
        let ship = app
            .world_mut()
            .spawn((
                crate::entities::spawner::EntityUuid("raider".into()),
                crate::entities::spawner::EntitySystemHull(hull),
                LastSystemTiers::default(),
                config,
                ShipSystemControlSources::default(),
            ))
            .id();

        // Seed LastSystemTiers at full HP (Operational), then discard events.
        app.update();
        app.world_mut()
            .resource_mut::<Messages<BalanceEvent>>()
            .clear();

        // Destroy the bank.
        {
            let mut e = app.world_mut().entity_mut(ship);
            let mut hull = e
                .get_mut::<crate::entities::spawner::EntitySystemHull>()
                .unwrap();
            hull.0.set_hp(&SystemId("phaser-fore".into()), 0.0);
        }
        app.update();

        let messages = app.world().resource::<Messages<BalanceEvent>>();
        let mut cursor = messages.get_cursor();
        let events: Vec<BalanceEvent> = cursor.read(messages).cloned().collect();
        assert!(
            events.iter().any(|e| matches!(
                e,
                BalanceEvent::SystemTierCrossed { ship, system_id, to_tier, .. }
                    if ship == "raider" && system_id == "phaser-fore" && to_tier == "Destroyed"
            )),
            "destroying the bank must emit SystemTierCrossed to Destroyed, got {events:?}"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, BalanceEvent::Disarmed { ship } if ship == "raider")),
            "a ship whose only weapon is destroyed must emit Disarmed, got {events:?}"
        );
    }

    // ── Issue #893: destroying the tactical radar drops the standing lock ────

    /// **AC1.** A tactical radar reaching Destroyed clears the ship's own
    /// `TacticalRadarSelection` — the SAME way whichever origin holds the
    /// radar, because the clear in `detect_damage_tier_crossings` is keyed on
    /// the SYSTEM crossing tiers, never on who set the lock. Run for both
    /// origins so a reintroduced origin branch (AGENTS.md #6) would have to
    /// break BOTH iterations, not just one.
    #[test]
    fn destroying_the_tactical_radar_clears_the_lock_for_either_origin() {
        use crate::console::weapons::TacticalRadarSelection;
        use crate::ship::control_source::ControlSource;

        for origin in [ControlSource::Human, ControlSource::Ai] {
            let mut app = App::new();
            app.add_message::<CoordinationEnqueue>();

            let radar_id = crate::ship::system_registry::tactical_radar_system_id();
            let hull = crate::ship::damage::SystemHull::from_config(&[(radar_id.clone(), 15.0)]);

            let mut sources = ShipSystemControlSources::default();
            sources.0.set(radar_id.clone(), origin);

            let ship = app
                .world_mut()
                .spawn((
                    crate::entities::spawner::EntityUuid("raider".into()),
                    crate::entities::spawner::EntitySystemHull(hull),
                    LastSystemTiers::default(),
                    ShipConfigComponent::default(),
                    sources,
                    TacticalRadarSelection(Some("the-enemy".to_string())),
                ))
                .id();

            app.add_systems(Update, detect_damage_tier_crossings);
            // Seed LastSystemTiers at full HP (Operational) before the kill shot.
            app.update();

            {
                let mut e = app.world_mut().entity_mut(ship);
                let mut hull = e
                    .get_mut::<crate::entities::spawner::EntitySystemHull>()
                    .unwrap();
                hull.0.set_hp(&radar_id, 0.0);
            }
            app.update();

            let lock = app
                .world()
                .entity(ship)
                .get::<TacticalRadarSelection>()
                .unwrap();
            assert_eq!(
                lock.0, None,
                "{origin:?}-held tactical radar reaching Destroyed must clear the \
                 standing lock"
            );
        }
    }

    /// The companion regression: Disabled (not yet Destroyed) must NOT clear
    /// the lock. A radar that is merely damaged already refuses to admit a NEW
    /// lock (the unchanged #887 admission gate, `sync_console_damage_tiers`
    /// marking it offline on Disabled OR Destroyed) but keeps the one it
    /// already holds — exactly today's behaviour. Only Destroyed is the
    /// drop-lock transition #893 decided on.
    #[test]
    fn a_merely_disabled_tactical_radar_does_not_clear_the_lock() {
        use crate::console::weapons::TacticalRadarSelection;

        let mut app = App::new();
        app.add_message::<CoordinationEnqueue>();

        let radar_id = crate::ship::system_registry::tactical_radar_system_id();
        let hull = crate::ship::damage::SystemHull::from_config(&[(radar_id.clone(), 100.0)]);

        let ship = app
            .world_mut()
            .spawn((
                crate::entities::spawner::EntityUuid("raider".into()),
                crate::entities::spawner::EntitySystemHull(hull),
                LastSystemTiers::default(),
                ShipConfigComponent::default(),
                ShipSystemControlSources::default(),
                TacticalRadarSelection(Some("the-enemy".to_string())),
            ))
            .id();

        app.add_systems(Update, detect_damage_tier_crossings);
        app.update();

        // Drop to 20 % — below the default 25 % disabled threshold, still above 0.
        {
            let mut e = app.world_mut().entity_mut(ship);
            let mut hull = e
                .get_mut::<crate::entities::spawner::EntitySystemHull>()
                .unwrap();
            hull.0.set_hp(&radar_id, 20.0);
        }
        app.update();

        let lock = app
            .world()
            .entity(ship)
            .get::<TacticalRadarSelection>()
            .unwrap();
        assert_eq!(
            lock.0.as_deref(),
            Some("the-enemy"),
            "a Disabled (not Destroyed) tactical radar must NOT clear the standing \
             lock — only Destroyed does"
        );
    }

    // ── Issue #1013: a destruction is repair work, so it files a request ─────

    /// A system knocked from `Operational` straight to `Destroyed` by one hit
    /// files a `RepairRequest` naming its station at tier `Destroyed`, AND
    /// raises the captain Alert.
    ///
    /// This is the one-hit case, and it is the one that used to fall through
    /// the floor: the Destroyed arm `continue`d after the Alert, so a system
    /// that never passed through Damaged or Disabled was never reported to
    /// Repair at all. However capable the on-site sweep is, a station nobody
    /// files a request for gets no team — the AI queue is fed by these
    /// requests and nothing else (#830 removed the raw hull poll).
    #[test]
    fn a_one_hit_destruction_files_a_repair_request_as_well_as_the_alert() {
        use crate::core::messages::SystemId;
        use bevy::ecs::message::Messages;

        let mut app = App::new();
        app.add_message::<CoordinationEnqueue>()
            .add_systems(Update, detect_damage_tier_crossings);

        let sid = SystemId("helm-drive".into());
        let mut config = ShipConfigComponent::default();
        config.0.systems.clear();
        config
            .0
            .systems
            .push(crate::ship::config::SystemInstanceConfig {
                id: sid.clone(),
                kind: "generic".into(),
                station: Some(crate::core::messages::StationId("helm".into())),
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            });
        // Typed Coordination addresses resolve only from authored Systems.
        // This fixture expects both messages, so author their real receivers
        // instead of relying on the pre-#1254 synthetic Station fallback.
        for (id, kind, station) in [
            (
                crate::ship::system_registry::repair_system_id(),
                crate::ship::system_registry::REPAIR_KIND,
                "repair",
            ),
            (
                crate::ship::system_registry::captain_system_id(),
                crate::ship::system_registry::CAPTAIN_KIND,
                "captain",
            ),
        ] {
            config
                .0
                .systems
                .push(crate::ship::config::SystemInstanceConfig {
                    id,
                    kind: kind.into(),
                    station: Some(crate::core::messages::StationId(station.into())),
                    ai_only: false,
                    human_seeking: false,
                    seek_order: Vec::new(),
                    power_group: None,
                    marker: None,
                    config: None,
                });
        }

        let hull = crate::ship::damage::SystemHull::from_config(&[(sid.clone(), 100.0)]);
        let ship = app
            .world_mut()
            .spawn((
                crate::entities::spawner::EntityUuid("raider".into()),
                crate::entities::spawner::EntitySystemHull(hull),
                LastSystemTiers::default(),
                config,
                ShipSystemControlSources::default(),
            ))
            .id();

        // Seed LastSystemTiers at full HP (Operational), then discard.
        app.update();
        app.world_mut()
            .resource_mut::<Messages<CoordinationEnqueue>>()
            .clear();

        // One hit, full HP to zero: Operational → Destroyed with no tier in
        // between for an earlier request to have covered.
        {
            let mut e = app.world_mut().entity_mut(ship);
            let mut hull = e
                .get_mut::<crate::entities::spawner::EntitySystemHull>()
                .unwrap();
            hull.0.set_hp(&sid, 0.0);
        }
        app.update();

        let messages = app.world().resource::<Messages<CoordinationEnqueue>>();
        let mut cursor = messages.get_cursor();
        let emitted: Vec<CoordinationEnqueue> = cursor.read(messages).cloned().collect();

        let requests: Vec<&CoordinationEnqueue> = emitted
            .iter()
            .filter(|e| matches!(&e.payload, CoordinationPayload::RepairRequest { .. }))
            .collect();
        assert_eq!(
            requests.len(),
            1,
            "a one-hit destruction must file exactly one RepairRequest, got {emitted:?}"
        );
        let CoordinationPayload::RepairRequest {
            system_id,
            station_id,
            tier,
            deficit,
            ..
        } = &requests[0].payload
        else {
            unreachable!("filtered above");
        };
        assert_eq!(system_id, &sid);
        assert_eq!(
            station_id, "helm",
            "the request must name the owning station"
        );
        assert_eq!(
            *tier,
            DamageTier::Destroyed,
            "the request must carry the Destroyed tier the AI queue ranks on"
        );
        assert_eq!(*deficit, Some(100.0), "the whole hull row is the deficit");
        assert_eq!(
            requests[0].address,
            crate::core::messages::CoordinationAddress::Station(crate::core::messages::StationId(
                "repair".into()
            )),
            "the request must be addressed explicitly to the Repair Station"
        );

        assert!(
            emitted
                .iter()
                .any(|e| matches!(&e.payload, CoordinationPayload::Alert { .. })
                    && e.address
                        == crate::core::messages::CoordinationAddress::Station(
                            crate::core::messages::StationId("captain".into())
                        )),
            "the captain Alert is KEPT — the request is an addition to it, not a \
             replacement, got {emitted:?}"
        );
    }
}
