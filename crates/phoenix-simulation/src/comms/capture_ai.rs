//! Decoder choice at the SAME existing Comms policy cadence. No world query,
//! private captured payload, memory, background controller or extra queue.
use crate::{
    server_app::ShipSystemBlackboards,
    ship_plugin::{ShipConfigComponent, ShipSystemControlSources},
};

pub(crate) fn decision(
    config: &ShipConfigComponent,
    boards: &ShipSystemBlackboards,
    sources: &ShipSystemControlSources,
    radio: &super::radio::RadioProfile,
    policy: Option<&crate::ai::policy::AiPolicy>,
    flags: &[&crate::world::flags::FlagStore],
    available: bool,
) -> Option<u64> {
    let policy = policy.filter(|policy| !policy.idle)?;
    let owner = &radio.0.system;
    if !available || radio.0.capture.is_none() || !crate::ai::host::ai_operates(&sources.0, owner) {
        return None;
    }
    let view = config.0.observations_for_system(owner, &boards.0);
    let Some(crate::core::messages::SystemBlackboard::Comms(board)) = view.get(owner) else {
        return None;
    };
    // Prioritise existing actual conversation/reading and released requests.
    // The decision sees exactly the public own Console view, never another
    // audience's request or private captured body/decoded request machinery.
    if !board.planned_requests.is_empty()
        || board.messages.iter().any(|message| {
            message.selected_response.is_none()
                && message.responses.iter().any(|response| response.available)
        })
    {
        return None;
    }
    let captured = board.captured.as_ref()?;
    if captured.rows.iter().any(|row| row.selected) {
        return None;
    }
    let candidate = captured
        .rows
        .iter()
        .filter(|row| row.feasible && row.evidence.is_none())
        .min_by_key(|row| (row.captured_tick, row.capture))?;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(
        phoenix_sim_gameplay::entities::ai_flag_hosts::CAPTURE_FEASIBLE,
        1.0,
    );
    facts.set_fact(
        phoenix_sim_gameplay::entities::ai_flag_hosts::COMMS_AVAILABLE,
        1.0,
    );
    let tick = crate::ai::host::HostTick {
        system: owner.clone(),
        channel: crate::entities::config::COMMS_DECODE_CHANNEL,
        facts: &facts,
        flags,
        state: None,
    };
    matches!(
        crate::ai::host::decide(&sources.0, Some(policy), &tick),
        crate::ai::host::HostOutcome::Act(crate::ai::policy::AiPolicyVerb::DecodeCapturedComms)
    )
    .then_some(candidate.capture)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_declared_policy_chooses_only_permitted_feasible_headers_after_priority_work() {
        use crate::core::messages::{CommsBlackboard, SystemBlackboard};
        use phoenix_model::comms_capture::{CommsCaptureControlsView, CommsCaptureView};
        let authored = crate::entities::include_resolve::load_entity_config(
            "assets/entities/capture_cruiser.toml",
        )
        .unwrap();
        let (_, policy, _) = crate::console::comms::server::comms_console_ai_components(&authored);
        let policy = policy.unwrap();
        let mut config = ShipConfigComponent(authored.ship_config.unwrap());
        let radio = super::super::radio::RadioProfile(authored.comms.unwrap().radio.unwrap());
        let owner = radio.0.system.clone();
        let mut sources = ShipSystemControlSources::default();
        sources.0.set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let mut boards = ShipSystemBlackboards::default();
        let row = |capture, captured_tick, feasible| CommsCaptureView {
            capture,
            channel: "fleet".into(),
            level: 1,
            captured_tick,
            feasible,
            selected: false,
            progress_per_mille: 0,
            decoded_tick: None,
            evidence: None,
        };
        boards.0.insert(
            owner.clone(),
            SystemBlackboard::Comms(CommsBlackboard {
                captured: Some(CommsCaptureControlsView {
                    rows: vec![row(1, 10, false), row(3, 12, true), row(2, 11, true)],
                    controllable: false,
                }),
                ..Default::default()
            }),
        );
        assert_eq!(
            decision(
                &config,
                &boards,
                &sources,
                &radio,
                Some(&policy.0),
                &[],
                true
            ),
            Some(2)
        );
        assert_eq!(
            decision(&config, &boards, &sources, &radio, None, &[], true),
            None
        );
        assert_eq!(
            decision(
                &config,
                &boards,
                &sources,
                &radio,
                Some(&policy.0),
                &[],
                false
            ),
            None
        );
        sources.0.set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Human,
        );
        assert_eq!(
            decision(
                &config,
                &boards,
                &sources,
                &radio,
                Some(&policy.0),
                &[],
                true
            ),
            None
        );
        sources.0.set(
            owner.clone(),
            crate::ship::control_source::ControlSource::Ai,
        );
        let SystemBlackboard::Comms(board) = boards.0.get_mut(&owner).unwrap() else {
            unreachable!()
        };
        board.captured.as_mut().unwrap().rows[2].selected = true;
        assert_eq!(
            decision(
                &config,
                &boards,
                &sources,
                &radio,
                Some(&policy.0),
                &[],
                true
            ),
            None
        );
        let SystemBlackboard::Comms(board) = boards.0.get_mut(&owner).unwrap() else {
            unreachable!()
        };
        board.captured.as_mut().unwrap().rows[2].selected = false;
        let console = config.0.system(&owner).unwrap().console.clone().unwrap();
        config
            .0
            .consoles
            .iter_mut()
            .find(|row| row.id == console)
            .unwrap()
            .feeds
            .clear();
        assert_eq!(
            decision(
                &config,
                &boards,
                &sources,
                &radio,
                Some(&policy.0),
                &[],
                true
            ),
            Some(2),
            "a System retains its own Console publication independently of cross-System feeds"
        );
        boards.0.remove(&owner);
        assert_eq!(
            decision(
                &config,
                &boards,
                &sources,
                &radio,
                Some(&policy.0),
                &[],
                true
            ),
            None,
            "an actually absent own publication cannot be replaced by private captured state"
        );
    }
}
