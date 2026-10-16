// Frozen full producer from 1bf59f86; only function name differs.
// Test-only actual owner oracle: no optimized helper or copied decision subset.
pub(crate) fn original_tick_frequency_hint_high_fidelity(
    contact_runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    // The AUTHORED tick period, not `Time::delta` (issue #889). This system is
    // gated by `run_if(ai_tick_ready)`, so it observes one shared AI tick per
    // run — feeding it the frame delta would accumulate only the frames it
    // happened to run on and stretch the authored hint delay by the
    // frame-rate-to-tick-rate ratio (2x at 60 Hz, ~4.8x at 144 Hz), reversing
    // the frame-independence the gate exists to provide. Same shape as
    // `ai_policy_state_tick`'s `AiPolicyTickClock`.
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut ships: Query<
        (
            Entity,
            Option<&crate::entities::spawner::EntityUuid>,
            &crate::ship_plugin::ShipSystemControlSources,
            &crate::server_app::ShipSystemBlackboards,
            &crate::ship_plugin::ShipConfigComponent,
            &mut ShipFrequencyHintState,
            Option<&crate::ship::sensors::SensorsAiConfigResource>,
        ),
        (
            With<crate::ai::server::AiHighFidelity>,
            With<crate::server_app::Ship>,
        ),
    >,
    pictures: Query<(
        Option<&crate::science::broad::BroadTracks>,
        Option<&crate::science::broad::SensingControls>,
    )>,
    mut writer: MessageWriter<crate::ship_plugin::CoordinationEnqueue>,
) {
    let hz = world_config
        .as_deref()
        .map(|wc| wc.global.ai_tick_hz)
        .unwrap_or_else(|| crate::entities::config::GlobalConfig::default().ai_tick_hz);
    let dt = if hz > 0.0 { 1.0 / hz } else { 0.0 };

    for (
        entity,
        observer_uuid,
        control_sources,
        blackboards,
        ship_config,
        mut hint_state,
        ai_config_comp,
    ) in ships.iter_mut()
    {
        let (tracks, controls) = pictures.get(entity).unwrap_or((None, None));
        let Some(owner) = crate::science::broad::science_owner(controls, tracks.is_some()) else {
            continue;
        };
        // Frozen Combat Lock from this ship's viewscreen (issue #829, spec §3),
        // identical to how the low-fidelity twin `tick_sensors_frequency_hint`
        // and the firing paths read it — never the tactical radar's live
        // selection.
        let locked_target = console_combat_lock(&ship_config.0, &owner, blackboards);

        let locked_target = locked_target.filter(|target| {
            observer_uuid
                .and_then(|observer| {
                    contact_runtime.as_ref().map(|runtime| {
                        crate::gm::operations::contact::mode(
                            &runtime.contact_overrides,
                            &observer.0,
                            target,
                        )
                    })
                })
                .unwrap_or_default()
                != crate::gm::operations::contact::ContactMode::Conceal
        });
        let public_target = locked_target.as_deref().and_then(|target| {
            crate::science::contact_routing::alias_for_source(
                &ship_config.0,
                blackboards,
                tracks,
                &owner,
                target,
            )
        });
        let acquired = locked_target.as_deref().and_then(|target| {
            let aliases = crate::science::contact_routing::aliases_for_source(
                &ship_config.0,
                blackboards,
                tracks,
                &owner,
                target,
            );
            crate::science::shield_enquiry::latest_permitted_finding_for_subjects(
                &ship_config.0,
                blackboards,
                &owner,
                &aliases,
            )
        });
        let Some((subject, finding)) = public_target.zip(acquired) else {
            hint_state.0 = Default::default();
            continue;
        };
        let correct_frequency = finding.frequency;
        // The debounce key is held evidence, never the target's current configuration.
        let locked_target = Some(format!(
            "{}:{}/{}",
            subject.len(),
            subject,
            finding.configuration
        ));

        // Per-ship tuning only (issue #738). This used to fall back to the
        // global `SensorsAiConfigResource` Resource while iterating every ship,
        // so whatever last wrote that Resource applied fleet-wide. Nothing in
        // the shipped app writes it — unlike the shields Resource, which
        // `server_app` dual-writes from the PLAYER ship's TOML — so the leak
        // was latent rather than live: it is registered purely for structural
        // symmetry with shields/power, and never seeded. The fallback is now
        // the parse-time default the type already supplies for a ship TOML that
        // omits the section.
        let default_ai_cfg;
        let ai_cfg: &crate::ship::sensors::SensorsAiConfigResource = match ai_config_comp {
            Some(c) => c,
            None => {
                default_ai_cfg = crate::ship::sensors::SensorsAiConfigResource::default();
                &default_ai_cfg
            }
        };

        let input = crate::console_ai::FrequencyHintInput {
            locked_target,
            correct_frequency,
            dt,
            delay_secs: ai_cfg.frequency_hint_delay_secs,
        };

        let output = crate::console_ai::tick_frequency_hint(&mut hint_state.0, &input);

        if let crate::console_ai::FrequencyHintOutput::Hint { frequency } = output {
            let sender_origin = control_sources.0.source_for(&owner);
            let Some(address) = crate::ship::coordination::address_for_system(
                &ship_config.0,
                &crate::science::broad::combat_owner(controls, tracks.is_some())
                    .unwrap_or_else(crate::ship::system_registry::tactical_radar_system_id),
            ) else {
                continue;
            };
            writer.write(crate::ship_plugin::CoordinationEnqueue {
                source_entity: entity,
                sender_origin,
                address,
                payload: crate::core::messages::CoordinationPayload::FrequencyHint { frequency },
                presentation: crate::core::messages::CoordinationPresentation::new(
                    "coordination.frequency_hint.title",
                    "coordination.frequency_hint.body",
                )
                .with_body_param("frequency", frequency),
                sender_label: crate::ship::coordination::CHATTER_SENDER_SENSORS.to_string(),
                sender_system: owner.clone(),
            });
        }
    }
}
