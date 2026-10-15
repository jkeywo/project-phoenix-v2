//! Planned choices use the same own Console, policy, reading and radio owners.
use super::server::{PendingAiResponse, PendingAiResponseKey};
use crate::{
    core::messages::{SystemBlackboard, SystemId},
    server_app::ShipSystemBlackboards,
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};
use phoenix_model::comms_request::{CommsRequestReading, PlannedCommsRequest};
use std::collections::BTreeMap;

pub(crate) fn rows(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    now: u64,
) -> Vec<PlannedCommsRequest> {
    let view = config.0.observations_for_system(owner, &boards.0);
    let Some(SystemBlackboard::Comms(board)) = view.get(owner) else {
        return Vec::new();
    };
    let mut rows: Vec<_> = board
        .planned_requests
        .iter()
        .filter(|row| {
            row.validate(now).is_ok()
                && board
                    .contacts
                    .iter()
                    .any(|contact| contact.uuid == row.recipient_uuid)
                && board
                    .channels
                    .iter()
                    .any(|channel| channel.id == row.channel)
        })
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        a.released_tick
            .cmp(&b.released_tick)
            .then_with(|| a.id.cmp(&b.id))
    });
    rows
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn matches(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    owner: &SystemId,
    id: &str,
    audience: &phoenix_model::comms_channel::CommsAudience,
    channel: &str,
    request: &phoenix_model::comms_request::GenericCommsRequest,
    now: u64,
) -> bool {
    let phoenix_model::comms_channel::CommsAudience::Contact { target_uuid } = audience else {
        return false;
    };
    rows(config, boards, owner, now).iter().any(|row| {
        row.id == id
            && row.recipient_uuid == *target_uuid
            && row.channel == channel
            && row.request == *request
    })
}

/// Called from the existing Comms response decision host at its normal cadence.
/// The same pending map owns reading, including a completed read awaiting energy.
#[allow(clippy::too_many_arguments)]
pub(crate) fn decision(
    ship: &str,
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    sources: &ShipSystemControlSources,
    radio: &super::radio::RadioProfile,
    policy: Option<&crate::ai::policy::AiPolicy>,
    flags: &[&crate::world::flags::FlagStore],
    pending: &mut BTreeMap<PendingAiResponseKey, PendingAiResponse>,
    now: u64,
    available: bool,
) -> Option<PlannedCommsRequest> {
    let policy = policy?;
    if policy.idle {
        return None;
    }
    let owner = &radio.0.system;
    if !available
        || !sources.0.entries().any(|(id, _)| id == owner)
        || !crate::ai::host::ai_operates(&sources.0, owner)
    {
        return None;
    }
    let row = rows(config, boards, owner, now).into_iter().next()?;
    let key = PendingAiResponseKey::planned(ship, &row.id);
    if !pending.contains_key(&key) {
        let reading = CommsRequestReading::start(radio.0.request_reading_ticks, now).ok()?;
        pending.insert(
            key,
            PendingAiResponse {
                due_tick: reading.due_tick()?,
                response_fingerprint: 0,
                request_reading: Some(reading),
            },
        );
        return None;
    }
    if pending.get(&key)?.request_reading?.remaining_ticks != 0 {
        return None;
    }
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(
        phoenix_sim_gameplay::entities::ai_flag_hosts::REQUEST_REPORT_STATUS,
        1.0,
    );
    facts.set_fact(
        phoenix_sim_gameplay::entities::ai_flag_hosts::COMMS_AVAILABLE,
        1.0,
    );
    let tick = crate::ai::host::HostTick {
        system: owner.clone(),
        channel: crate::entities::config::COMMS_REQUEST_CHANNEL,
        facts: &facts,
        flags,
        state: None,
    };
    matches!(
        crate::ai::host::decide(&sources.0, Some(policy), &tick),
        crate::ai::host::HostOutcome::Act(crate::ai::policy::AiPolicyVerb::SendPlannedCommsRequest)
    )
    .then_some(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::{CommsBlackboard, CommsContact};
    use crate::ship::control_source::ControlSource;
    use phoenix_model::comms_channel::{CommsAudience, CommsChannelView};
    use phoenix_model::comms_request::GenericCommsRequest;

    fn fixture() -> (
        ShipConfigComponent,
        ShipSystemBlackboards,
        ShipSystemControlSources,
        super::super::radio::RadioProfile,
        crate::ai::policy::AiPolicy,
    ) {
        let authored = crate::entities::include_resolve::load_entity_config(
            "assets/entities/alliance_cruiser.toml",
        )
        .unwrap();
        let (_, policy, _) = crate::console::comms::server::comms_console_ai_components(&authored);
        let config = ShipConfigComponent(authored.ship_config.clone().unwrap());
        let radio = super::super::radio::RadioProfile(authored.comms.unwrap().radio.unwrap());
        let owner = radio.0.system.clone();
        let mut sources = ShipSystemControlSources::default();
        sources.0.set(owner.clone(), ControlSource::Ai);
        let mut boards = ShipSystemBlackboards::default();
        boards.0.insert(
            owner,
            SystemBlackboard::Comms(CommsBlackboard {
                contacts: vec![CommsContact {
                    uuid: "known-recipient".into(),
                    name: "Known".into(),
                    in_range: true,
                    is_urgent: false,
                }],
                channels: vec![CommsChannelView {
                    id: "public".into(),
                    range: 100.0,
                    work_units: 1,
                    protected: false,
                }],
                planned_requests: vec![PlannedCommsRequest {
                    id: "plan/request".into(),
                    released_tick: 10,
                    recipient_uuid: "known-recipient".into(),
                    channel: "public".into(),
                    request: GenericCommsRequest::ReportStatus,
                }],
                ..Default::default()
            }),
        );
        (config, boards, sources, radio, policy.unwrap().0)
    }

    #[test]
    fn released_request_requires_own_feed_current_contact_and_exact_context() {
        let (mut config, mut boards, _, radio, _) = fixture();
        let owner = &radio.0.system;
        let audience = CommsAudience::Contact {
            target_uuid: "known-recipient".into(),
        };
        assert!(matches(
            &config,
            &boards,
            owner,
            "plan/request",
            &audience,
            "public",
            &GenericCommsRequest::ReportStatus,
            10
        ));
        for (id, target, channel) in [
            ("other", "known-recipient", "public"),
            ("plan/request", "private-recipient", "public"),
            ("plan/request", "known-recipient", "protected"),
        ] {
            assert!(!matches(
                &config,
                &boards,
                owner,
                id,
                &CommsAudience::Contact {
                    target_uuid: target.into()
                },
                channel,
                &GenericCommsRequest::ReportStatus,
                10
            ));
        }
        assert!(rows(&config, &boards, owner, 9).is_empty());
        let saved = boards.0.get(owner).unwrap().clone();
        let SystemBlackboard::Comms(board) = boards.0.get_mut(owner).unwrap() else {
            unreachable!()
        };
        board.contacts.clear();
        assert!(
            rows(&config, &boards, owner, 10).is_empty(),
            "a released task grants no contact knowledge"
        );
        boards.0.insert(owner.clone(), saved);
        let console = config.0.system(owner).unwrap().console.clone().unwrap();
        config
            .0
            .consoles
            .iter_mut()
            .find(|item| item.id == console)
            .unwrap()
            .feeds
            .clear();
        assert!(
            rows(&config, &boards, owner, 10).is_empty(),
            "a co-hosted publication cannot bypass missing own Console feed"
        );
    }

    #[test]
    fn existing_policy_reads_before_initiation_and_handoff_retains_same_reading() {
        let (config, boards, mut sources, radio, policy) = fixture();
        let mut pending = BTreeMap::new();
        assert!(decision(
            "source",
            &config,
            &boards,
            &sources,
            &radio,
            Some(&policy),
            &[],
            &mut pending,
            10,
            true
        )
        .is_none());
        let key = PendingAiResponseKey::planned("source", "plan/request");
        let original = pending[&key];
        assert_eq!(
            original.request_reading.unwrap().remaining_ticks,
            radio.0.request_reading_ticks
        );
        assert!(
            decision(
                "source",
                &config,
                &boards,
                &sources,
                &radio,
                Some(&policy),
                &[],
                &mut pending,
                11,
                true
            )
            .is_none(),
            "a decision cadence is not reading work"
        );
        sources.0.set(radio.0.system.clone(), ControlSource::Human);
        assert!(decision(
            "source",
            &config,
            &boards,
            &sources,
            &radio,
            Some(&policy),
            &[],
            &mut pending,
            20,
            true
        )
        .is_none());
        assert_eq!(
            pending[&key], original,
            "human takeover preserves the real reading owner"
        );
        sources.0.set(radio.0.system.clone(), ControlSource::Ai);
        let row = pending.get_mut(&key).unwrap();
        let reading = row.request_reading.as_mut().unwrap();
        for tick in 11..=10 + u64::from(radio.0.request_reading_ticks) {
            reading.advance(tick, true).unwrap();
        }
        row.due_tick = reading.due_tick().unwrap();
        let now = 10 + u64::from(radio.0.request_reading_ticks);
        assert_eq!(
            decision(
                "source",
                &config,
                &boards,
                &sources,
                &radio,
                Some(&policy),
                &[],
                &mut pending,
                now,
                true
            )
            .unwrap()
            .id,
            "plan/request"
        );
        assert!(
            pending.contains_key(&key),
            "only the actual emission commit retires this reading"
        );
        assert_ne!(
            key,
            PendingAiResponseKey::new("source", "plan/request"),
            "an inbox id cannot alias a planned reading"
        );
    }

    #[test]
    fn missing_policy_human_offline_and_unavailable_never_arm_reading() {
        let (config, boards, sources, radio, policy) = fixture();
        for (source, unavailable, declared, offline) in [
            (ControlSource::Ai, false, false, false),
            (ControlSource::Human, false, true, false),
            (ControlSource::Ai, false, true, true),
            (ControlSource::Ai, true, true, false),
        ] {
            let mut sources = sources.clone();
            sources.0.set(radio.0.system.clone(), source);
            sources.0.set_offline(radio.0.system.clone(), offline);
            let mut pending = BTreeMap::new();
            assert!(decision(
                "source",
                &config,
                &boards,
                &sources,
                &radio,
                declared.then_some(&policy),
                &[],
                &mut pending,
                10,
                !unavailable
            )
            .is_none());
            assert!(pending.is_empty());
        }
    }
}
