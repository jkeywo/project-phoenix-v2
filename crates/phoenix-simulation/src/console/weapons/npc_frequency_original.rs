// Frozen full producer from 1bf59f86; only function name differs.
// Test-only actual owner oracle: no optimized helper or copied decision subset.
fn original_tick_npc_auto_match_frequency(
    time: Res<Time>,
    content: Option<Res<crate::world::server::WorldContentRuntime>>,
    mut ship_q: Query<(
        Entity,
        &ShipSystemControlSources,
        &crate::ship_plugin::ShipConfigComponent,
        &crate::server_app::ShipSystemBlackboards,
        &mut crate::ship::state::ShipPhaserFrequency,
        Has<crate::ai::server::AiHighFidelity>,
        Option<&crate::entities::spawner::EntityUuid>,
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::science::broad::SensingControls>,
        Option<&super::assignments::ShipWeaponTargets>,
    )>,
    mut states: ResMut<NpcFrequencyMatchStates>,
) {
    let dt = time.delta_secs();
    // Gate: this frequency-hint system only runs for high-fidelity NPCs
    // (issue #692 AC — both frequency-hint systems gated on `AiHighFidelity`).
    //
    // `AiHighFidelity` is read via `Has<>` and folded into the in-loop gate
    // below rather than applied as a `With<AiHighFidelity>` query FILTER on
    // purpose: `NpcFrequencyMatchStates` is only cleaned up here, in the gate's
    // cleanup branch (`states.0.remove(&entity)`), which runs only while the
    // entity is still iterated. A `With<>` filter would stop iterating demoted
    // (no-longer-high-fidelity) NPCs entirely, orphaning their HashMap entries
    // forever — a state leak. `Has<>` + in-loop gate keeps the cleanup path
    // alive so demoted ships' state is pruned. Do not "simplify" this into a
    // query filter.
    for (
        entity,
        control_sources,
        ship_config,
        blackboards,
        mut phaser_freq,
        has_high_fidelity,
        observer,
        tracks,
        controls,
        assignments,
    ) in ship_q.iter_mut()
    {
        if !has_high_fidelity || !any_tactical_system_operates_ai(control_sources, &ship_config.0) {
            states.0.remove(&entity);
            continue;
        }
        // One existing frequency actuator serves this hull. Each eligible bank
        // must obtain evidence through its own Console, never a Station union.
        // Conflicting held findings cannot select an arbitrary bank's advice.
        let mut acquired = None;
        let mut conflicting = false;
        for system in ship_config.0.systems.iter().filter(|system| {
            system.kind == crate::ship::system_registry::PHASER_BANK_KIND
                && control_sources.0.policy_for(&system.id).operate_ai
        }) {
            let Some(public) = super::assignments::effective_weapon_contact(
                &ship_config.0,
                &system.id,
                blackboards,
                tracks,
                controls,
                assignments,
            ) else {
                continue;
            };
            let Some(route) = crate::science::contact_routing::resolve_machinery(
                Some(&ship_config.0),
                Some(blackboards),
                tracks,
                controls,
                &system.id,
                &public,
            ) else {
                continue;
            };
            if content.as_ref().is_some_and(|content| {
                observer.is_none_or(|observer| {
                    crate::gm::operations::contact::mode(
                        &content.contact_overrides,
                        &observer.0,
                        &route,
                    ) == crate::gm::operations::contact::ContactMode::Conceal
                })
            }) {
                continue;
            }
            // A scanner's original contact handle may differ from this bank's
            // target handle. Both must currently be permitted by this Console;
            // only the ordinary actuator resolver may establish equivalence.
            let aliases = crate::science::contact_routing::aliases_for_source(
                &ship_config.0,
                blackboards,
                tracks,
                &system.id,
                &route,
            );
            let Some(finding) =
                crate::science::shield_enquiry::latest_permitted_finding_for_subjects(
                    &ship_config.0,
                    blackboards,
                    &system.id,
                    &aliases,
                )
            else {
                continue;
            };
            let candidate = (
                format!("{}:{}/{}", route.len(), route, finding.configuration),
                finding.frequency,
            );
            if acquired
                .as_ref()
                .is_some_and(|previous| previous != &candidate)
            {
                conflicting = true;
                break;
            }
            acquired = Some(candidate);
        }
        let Some((subject_configuration, target_frequency)) = acquired.filter(|_| !conflicting)
        else {
            states.0.remove(&entity);
            continue;
        };
        let input = crate::console_ai::FrequencyMatchInput {
            locked_target: Some(subject_configuration),
            target_frequency,
            dt,
            delay_secs: NPC_FREQ_MATCH_DELAY,
            trigger_active: true,
        };

        let state = states.0.entry(entity).or_default();
        let output = crate::console_ai::tick_auto_match_frequency(state, &input);

        if let crate::console_ai::FrequencyMatchOutput::Match { frequency } = output {
            phaser_freq.0 = frequency;
        }
    }
}
