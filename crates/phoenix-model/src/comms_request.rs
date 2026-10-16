//! Closed generic request content and recipient-local inbox progress.
//! Physical delivery/payment, Console permission and cross-copy lineage are
//! validated by the actual Comms owner before these records become knowledge.
use crate::{comms_channel::identity, identity::SystemId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum GenericCommsRequest {
    ReportStatus,
    FormUp { slot: [i32; 2] },
    MediaCall { mode: crate::comms_media::MediaMode },
    ReceiveCraft { craft: String },
}
/// Technical finite slot bound, exactly representable by the motion owner's f32.
pub const MAX_FORMATION_SLOT: i32 = 1_000_000;
impl GenericCommsRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if matches!(self,Self::ReceiveCraft {craft} if !identity(craft)) {
            return Err("reception requires a bounded proposed craft identity");
        }
        if matches!(self, Self::FormUp { slot } if slot.iter().any(|axis| !(-MAX_FORMATION_SLOT..=MAX_FORMATION_SLOT).contains(axis)))
        {
            return Err("formation slot exceeds supported finite World coordinates");
        }
        Ok(())
    }
    pub fn is_formation(&self) -> bool {
        matches!(self, Self::FormUp { .. })
    }
    pub fn is_reception(&self) -> bool {
        matches!(self, Self::ReceiveCraft { .. })
    }
    pub fn is_media(&self) -> bool {
        matches!(self, Self::MediaCall { .. })
    }
    pub fn body(&self) -> &'static str {
        match self {
            Self::ReportStatus => REPORT_STATUS_BODY,
            Self::FormUp { .. } => FORM_UP_BODY,
            Self::ReceiveCraft { .. } => RECEIVE_CRAFT_BODY,
            Self::MediaCall {
                mode: crate::comms_media::MediaMode::Audio,
            } => MEDIA_AUDIO_BODY,
            Self::MediaCall {
                mode: crate::comms_media::MediaMode::AudioVideo,
            } => MEDIA_VIDEO_BODY,
        }
    }
    pub fn positive_response(&self) -> &'static str {
        match self {
            Self::ReportStatus => REPORT_RESPONSE,
            Self::FormUp { .. } => ACCEPT_RESPONSE,
            Self::ReceiveCraft { .. } => RECEIVE_CRAFT_RESPONSE,
            Self::MediaCall { .. } => MEDIA_ACCEPT_RESPONSE,
        }
    }
    pub fn validate_sequence(&self, sequence: Option<u64>) -> Result<(), &'static str> {
        self.validate()?;
        match (self.is_formation(), sequence) {
            (true, Some(1..)) | (false, None) => Ok(()),
            _ => Err("formation sequence disagrees with its closed request kind"),
        }
    }
    fn validate_facts(&self, facts: &CommsRequestFacts) -> Result<(), &'static str> {
        if facts
            .reception
            .as_ref()
            .is_some_and(|receipt| !self.is_reception() || &receipt.order.request.request != self)
        {
            return Err("reception receipt differs from its actual receiving request");
        }
        if let Some(decision) = &facts.decision {
            match (self, decision.choice) {
                (_, CommsRequestDecision::Decline)
                | (Self::ReportStatus, CommsRequestDecision::Report)
                | (
                    Self::FormUp { .. } | Self::MediaCall { .. } | Self::ReceiveCraft { .. },
                    CommsRequestDecision::Accept,
                ) => {}
                _ => return Err("request decision does not belong to this closed action"),
            }
        }
        Ok(())
    }
}

/// Immutable correlation for one real own-ship optional work offer. This is not
/// a permission, plan/job identity or physical command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpportunityRef {
    pub source: String,
    pub ordinal: u16,
    pub actor: String,
}
impl OpportunityRef {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.source.is_empty()
            || self.source.len() > 128
            || !self
                .source
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || !(1..=8).contains(&self.ordinal)
            || !identity(&self.actor)
        {
            return Err("invalid immutable opportunity source, ordinal or actor");
        }
        Ok(())
    }
    pub fn instance_id(&self) -> String {
        format!("opportunity/{}/{}", self.source, self.ordinal)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityDecision {
    Accept,
    Decline,
}

/// A released own task whose recipient is ALREADY in this Console's permitted
/// contact view. This is a request choice, not a new contact or actuator grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedCommsRequest {
    pub id: String,
    pub released_tick: u64,
    pub recipient_uuid: String,
    pub channel: String,
    pub request: GenericCommsRequest,
}
impl PlannedCommsRequest {
    pub fn validate(&self, now: u64) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.id.trim().is_empty()
            || self.id.len() > 2048
            || self.id.chars().any(char::is_control)
            || !identity(&self.recipient_uuid)
            || !crate::comms_channel::channel_id(&self.channel)
            || self.released_tick > now
        {
            return Err("invalid published planned Comms request identity/channel/clock");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommsRequestDecision {
    Report,
    Decline,
    /// Coordination only; no claim about Navigation or Helm execution.
    Accept,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatedCommsDecision {
    pub choice: CommsRequestDecision,
    pub tick: u64,
}

/// A report of the reporting Console's own supported Comms capability only.
/// It says nothing about alert condition, other machinery or remote listeners.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsStatusReport {
    pub system: SystemId,
    pub available: bool,
    pub observed_tick: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommsRequestStage {
    Heard,
    Read,
    Decision,
    Completed,
}

/// Original local events. Receipt never creates read/decision/completion.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsRequestFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reception: Option<crate::craft::CraftReceptionReceipt>,
    pub read_tick: Option<u64>,
    pub decision: Option<DatedCommsDecision>,
    pub report: Option<CommsStatusReport>,
}

impl CommsRequestFacts {
    pub fn stage(&self) -> CommsRequestStage {
        if self.report.is_some() || self.reception.is_some() {
            CommsRequestStage::Completed
        } else if self.decision.is_some() {
            CommsRequestStage::Decision
        } else if self.read_tick.is_some() {
            CommsRequestStage::Read
        } else {
            CommsRequestStage::Heard
        }
    }

    pub fn validate(&self, heard_tick: u64, now: u64) -> Result<(), &'static str> {
        if heard_tick > now
            || self
                .read_tick
                .is_some_and(|tick| tick < heard_tick || tick > now)
        {
            return Err("request reading must follow actual receipt on the original clock");
        }
        if let Some(decision) = &self.decision {
            let read = self
                .read_tick
                .ok_or("request decision requires an actual read event")?;
            if decision.tick < read || decision.tick > now {
                return Err("request decision must follow reading on the original clock");
            }
            match (decision.choice, &self.report) {
                (CommsRequestDecision::Decline, None) if self.reception.is_none() => {}
                (CommsRequestDecision::Accept, None) => {
                    if let Some(receipt) = &self.reception {
                        receipt.validate(now)?;
                        if receipt.order.decision_tick != decision.tick
                            || receipt.order.request.heard_tick != heard_tick
                        {
                            return Err("reception disagrees with actual consent/read clock");
                        }
                    }
                }
                (CommsRequestDecision::Report, Some(report))
                    if self.reception.is_none()
                        && identity(&report.system.0)
                        && report.observed_tick == decision.tick => {}
                _ => return Err("Report Status decision and actual local report disagree"),
            }
        } else if self.report.is_some() || self.reception.is_some() {
            return Err("request completion requires the actual receiving decision");
        }
        Ok(())
    }
}

/// Partial reading inside the existing pending-response entry. Mechanical
/// elapsed ticks do not choose or emit a response. Human/Offline pauses preserve
/// accumulated work; callers keep the same actual operator/message ownership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsRequestReading {
    pub required_ticks: u32,
    pub remaining_ticks: u32,
    pub last_tick: u64,
}

impl CommsRequestReading {
    pub fn start(total_ticks: u32, now: u64) -> Result<Self, &'static str> {
        let state = Self {
            required_ticks: total_ticks,
            remaining_ticks: total_ticks,
            last_tick: now,
        };
        if total_ticks == 0 || state.due_tick().is_none() {
            return Err("request reading needs positive authored ticks within the logical clock");
        }
        Ok(state)
    }

    pub fn due_tick(&self) -> Option<u64> {
        self.last_tick.checked_add(u64::from(self.remaining_ticks))
    }

    pub fn validate(&self, total_ticks: u32, now: u64) -> Result<(), &'static str> {
        if total_ticks == 0
            || self.required_ticks != total_ticks
            || self.remaining_ticks > total_ticks
            || self.last_tick > now
            || self.due_tick().is_none()
        {
            return Err("invalid request reading ownership/clock frontier");
        }
        Ok(())
    }

    pub fn advance(&mut self, now: u64, eligible_ai: bool) -> Result<bool, &'static str> {
        self.validate(self.required_ticks, now)?;
        if now < self.last_tick {
            return Err("request reading cannot rewind logical time");
        }
        let elapsed = now - self.last_tick;
        let remaining = if eligible_ai {
            u64::from(self.remaining_ticks).saturating_sub(elapsed) as u32
        } else {
            self.remaining_ticks
        };
        let next = Self {
            required_ticks: self.required_ticks,
            remaining_ticks: remaining,
            last_tick: now,
        };
        if next.due_tick().is_none() {
            return Err("request reading exceeds logical clock");
        }
        *self = next;
        Ok(remaining == 0)
    }
}

/// Immutable binding carried by a returned claim, not a remote actuator token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsRequestReference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formation_sequence: Option<u64>,
    pub origin_id: String,
    pub source_ship: String,
    pub recipient_ship: String,
    pub emitted_tick: u64,
    pub heard_tick: u64,
    pub request: GenericCommsRequest,
}

impl CommsRequestReference {
    pub fn from_received(message: &crate::messages::CommsMessage) -> Option<Self> {
        let wire = message.transmission.as_ref()?;
        let CommsRadioContent::Request { request } = &wire.content else {
            return None;
        };
        Some(Self {
            formation_sequence: wire.formation_sequence,
            origin_id: wire.origin_id.clone(),
            source_ship: message.sender_uuid.clone(),
            recipient_ship: message.recipient_ship.as_ref()?.0.clone(),
            emitted_tick: wire.emitted_tick,
            heard_tick: wire.received_tick?,
            request: request.clone(),
        })
    }

    pub fn validate(&self, now: u64) -> Result<(), &'static str> {
        self.request.validate_sequence(self.formation_sequence)?;
        if !identity(&self.origin_id)
            || !identity(&self.source_ship)
            || !identity(&self.recipient_ship)
            || self.source_ship == self.recipient_ship
            || self.emitted_tick > self.heard_tick
            || self.heard_tick > now
        {
            return Err("request reference has invalid original identity, audience or clocks");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsRequestOutcome {
    pub request: CommsRequestReference,
    pub facts: CommsRequestFacts,
}

impl CommsRequestOutcome {
    pub fn validate(&self, now: u64) -> Result<(), &'static str> {
        self.request.validate(now)?;
        self.facts.validate(self.request.heard_tick, now)?;
        if self
            .facts
            .reception
            .as_ref()
            .is_some_and(|receipt| receipt.order.request != self.request)
        {
            return Err("returned reception changes its original request lineage");
        }
        self.request.request.validate_facts(&self.facts)
    }
}

/// Immutable transmitted content. Ordinary text retains its existing body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommsRadioContent {
    #[default]
    Text,
    Request {
        request: GenericCommsRequest,
    },
    Outcome {
        outcome: CommsRequestOutcome,
    },
}

impl CommsRadioContent {
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text)
    }
}

pub const RECEIVE_CRAFT_BODY: &str = "comms.request.receive_craft";
pub const RECEIVE_CRAFT_RESPONSE: &str = "comms.request.accept_reception";
pub const REPORT_STATUS_BODY: &str = "comms.request.report_status";
pub const FORM_UP_BODY: &str = "comms.request.form_up";
pub const MEDIA_AUDIO_BODY: &str = "comms.request.media_audio";
pub const MEDIA_VIDEO_BODY: &str = "comms.request.media_video";
pub const MEDIA_ACCEPT_RESPONSE: &str = "comms.request.accept_media";
pub const ACCEPT_RESPONSE: &str = "comms.request.accept_coordination";
pub const OUTCOME_BODY: &str = "comms.request.returned_outcome";
pub const REPORT_RESPONSE: &str = "comms.request.report";
pub const DECLINE_RESPONSE: &str = "comms.request.decline";

/// Same inbox copy, local facts only. Sent copies have no recipient events.
/// Dismissal is presentation, never cancellation or a new read event. Last
/// returned advances only when the actual paid reverse delivery occurs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsRequestProgress {
    pub facts: CommsRequestFacts,
    pub last_returned: Option<CommsRequestStage>,
    pub dismissed: bool,
}

impl CommsRequestProgress {
    pub fn validate(&self, received_tick: Option<u64>, now: u64) -> Result<(), &'static str> {
        if let Some(heard) = received_tick {
            self.facts.validate(heard, now)?;
            if self
                .last_returned
                .is_some_and(|stage| stage > self.facts.stage())
            {
                return Err("returned request frontier exceeds actual recipient events");
            }
        } else if self.facts != CommsRequestFacts::default() || self.last_returned.is_some() {
            return Err("outgoing request cannot claim recipient events");
        }
        Ok(())
    }

    pub fn mark_read(&mut self, heard_tick: u64, now: u64) -> Result<bool, &'static str> {
        self.validate(Some(heard_tick), now)?;
        if self.facts.read_tick.is_some() {
            return Ok(false);
        }
        self.facts.read_tick = Some(now);
        Ok(true)
    }

    /// Atomic local Report/Decline, preserving the first actual read tick.
    pub fn decide(
        &mut self,
        heard_tick: u64,
        now: u64,
        choice: CommsRequestDecision,
        report: Option<CommsStatusReport>,
    ) -> Result<(), &'static str> {
        self.validate(Some(heard_tick), now)?;
        if self.facts.decision.is_some() {
            return Err("request already has a local receiving decision");
        }
        let mut next = self.clone();
        next.facts.read_tick.get_or_insert(now);
        next.facts.decision = Some(DatedCommsDecision { choice, tick: now });
        next.facts.report = report;
        next.validate(Some(heard_tick), now)?;
        *self = next;
        Ok(())
    }
}

/// Validate one canonical inbox record; projection-only reach flags do not
/// control a local request decision and never become a returned claim.
pub(crate) fn validate_copy(
    message: &crate::messages::CommsMessage,
    now: u64,
) -> Result<(), &'static str> {
    use crate::comms_request::CommsRadioContent::*;
    let transmission = message
        .transmission
        .as_ref()
        .ok_or("request copy lacks transmission")?;
    if !message.body_params.is_empty() {
        return Err("radio copy cannot interpolate unbound contents");
    }
    match &transmission.content {
        Text => {
            if !message.literal_body
                || !crate::comms_channel::valid_text(&message.body)
                || !message.responses.is_empty()
                || message.selected_response.is_some()
                || message.request_progress.is_some()
            {
                return Err("ordinary radio text cannot carry request state or responses");
            }
        }
        Request { .. } | Outcome { .. } => {
            let progress = message
                .request_progress
                .as_ref()
                .ok_or("structured radio copy requires local progress")?;
            if message.literal_body
                || message.is_orphaned
                || (progress.dismissed && !message.is_read)
            {
                return Err("request dismissal requires an eligible local inbox copy");
            }
            if let Request { request } = &transmission.content {
                progress.validate(transmission.received_tick, now)?;
                request.validate_facts(&progress.facts)?;
                if progress.facts.reception.as_ref().is_some_and(|receipt| {
                    Some(receipt.order.request.clone())
                        != CommsRequestReference::from_received(message)
                }) {
                    return Err("local reception changes its heard request lineage");
                }
                if message.body != request.body() {
                    return Err("request body is not its closed definition");
                }
                if transmission.received_tick.is_some() {
                    let expected = match progress.facts.decision.as_ref().map(|value| value.choice)
                    {
                        Some(CommsRequestDecision::Report | CommsRequestDecision::Accept) => {
                            Some(0)
                        }
                        Some(CommsRequestDecision::Decline) => Some(1),
                        None => None,
                    };
                    if message.is_read != progress.facts.read_tick.is_some()
                        || message.selected_response != expected
                        || message.responses.len() != 2
                        || message.responses[0].text != request.positive_response()
                        || message.responses[1].text != DECLINE_RESPONSE
                        || message
                            .responses
                            .iter()
                            .any(|response| response.important || !response.available)
                    {
                        return Err(
                            "received request surface disagrees with its actual local decision",
                        );
                    }
                } else if !message.responses.is_empty() || message.selected_response.is_some() {
                    return Err("issuing copy cannot make the recipient decision");
                }
            } else if progress.facts != CommsRequestFacts::default()
                || progress.last_returned.is_some()
                || message.body != OUTCOME_BODY
                || !message.responses.is_empty()
                || message.selected_response.is_some()
            {
                return Err("returned outcome cannot invent local request progress");
            }
        }
    }
    Ok(())
}

fn contains_facts(current: &CommsRequestFacts, older: &CommsRequestFacts) -> bool {
    older
        .read_tick
        .is_none_or(|value| current.read_tick == Some(value))
        && older
            .decision
            .as_ref()
            .is_none_or(|value| current.decision.as_ref() == Some(value))
        && older
            .report
            .as_ref()
            .is_none_or(|value| current.report.as_ref() == Some(value))
        && older
            .reception
            .as_ref()
            .is_none_or(|value| current.reception.as_ref() == Some(value))
}

/// Historical lineages stay in these same inbox records after local dismissal.
/// A return is not validated against a live remote actor or its private state.
pub(crate) fn validate_lineage(
    messages: &[crate::messages::CommsMessage],
) -> Result<(), &'static str> {
    use crate::comms_request::CommsRadioContent::*;
    let mut stages = std::collections::BTreeMap::new();
    for message in messages {
        let Some(transmission) = &message.transmission else {
            continue;
        };
        match &transmission.content {
            Text => {}
            Request { request } => {
                if let Some(heard_tick) = transmission.received_tick {
                    let issued = messages.iter().any(|source| {
                        source.sender_uuid == message.sender_uuid
                            && source.transmission.as_ref().is_some_and(|original| {
                                original.origin_id == transmission.origin_id
                                    && original.received_tick.is_none()
                                    && original.content
                                        == (Request {
                                            request: request.clone(),
                                        })
                            })
                    });
                    if !issued || heard_tick < transmission.emitted_tick {
                        return Err("heard request lacks its retained original issue");
                    }
                    let progress = message
                        .request_progress
                        .as_ref()
                        .ok_or("heard request lacks local progress")?;
                    if let Some(stage) = progress.last_returned {
                        let returned = messages.iter().any(|copy| {
                            copy.transmission.as_ref().is_some_and(|receipt| {
                                match &receipt.content {
                                    Outcome { outcome } => {
                                        receipt.received_tick.is_some()
                                            && outcome.request.origin_id == transmission.origin_id
                                            && Some(outcome.request.recipient_ship.as_str())
                                                == message
                                                    .recipient_ship
                                                    .as_ref()
                                                    .map(|key| key.0.as_str())
                                            && outcome.facts.stage() == stage
                                    }
                                    _ => false,
                                }
                            })
                        });
                        if !returned {
                            return Err("returned frontier lacks actual retained reverse delivery");
                        }
                    }
                }
            }
            Outcome { outcome } => {
                let reference = &outcome.request;
                if message.sender_uuid != reference.recipient_ship
                    || transmission.audience
                        != (crate::comms_channel::CommsAudience::Contact {
                            target_uuid: reference.source_ship.clone(),
                        })
                {
                    return Err("returned claim disagrees with its reporting source and requester");
                }
                let actual = messages.iter().find(|request| {
                    request.sender_uuid == reference.source_ship
                        && request
                            .recipient_ship
                            .as_ref()
                            .is_some_and(|key| key.0 == reference.recipient_ship)
                        && request.transmission.as_ref().is_some_and(|original| {
                            original.origin_id == reference.origin_id
                                && original.emitted_tick == reference.emitted_tick
                                && original.formation_sequence == reference.formation_sequence
                                && original.received_tick == Some(reference.heard_tick)
                                && original.content
                                    == (Request {
                                        request: reference.request.clone(),
                                    })
                        })
                });
                let facts = actual
                    .and_then(|actual| actual.request_progress.as_ref())
                    .map(|progress| &progress.facts)
                    .ok_or(
                        "returned claim lacks the request actually heard by its reporting source",
                    )?;
                if !contains_facts(facts, &outcome.facts) {
                    return Err("returned claim exceeds actual original recipient events");
                }
                let progress = actual
                    .and_then(|actual| actual.request_progress.as_ref())
                    .expect("checked actual request");
                let delivered = messages.iter().any(|copy| {
                    copy.transmission.as_ref().is_some_and(|record| {
                        record.origin_id == transmission.origin_id && record.received_tick.is_some()
                    })
                });
                let locally_sent = messages.iter().any(|copy| {
                    copy.transmission.as_ref().is_some_and(|record| {
                        record.origin_id == transmission.origin_id && record.received_tick.is_none()
                    })
                });
                if !locally_sent
                    || (delivered
                        && progress
                            .last_returned
                            .is_none_or(|stage| stage < outcome.facts.stage()))
                {
                    return Err(
                        "returned outcome lacks its local emission or actual delivery frontier",
                    );
                }
                let key = (
                    &reference.origin_id,
                    &reference.recipient_ship,
                    outcome.facts.stage(),
                );
                if stages
                    .insert(key, &transmission.origin_id)
                    .is_some_and(|old| old != &transmission.origin_id)
                {
                    return Err("one actual request stage cannot mint multiple returned emissions");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "comms_request_tests.rs"]
mod tests;

#[cfg(test)]
mod opportunity_tests {
    use super::*;
    #[test]
    fn opportunity_reference_is_closed_and_never_resolves_by_source_alone() {
        let reference = OpportunityRef {
            source: "maintenance".into(),
            ordinal: 1,
            actor: "own-ship".into(),
        };
        reference.validate().unwrap();
        assert_eq!(reference.instance_id(), "opportunity/maintenance/1");
        assert_ne!(
            reference.instance_id(),
            OpportunityRef {
                ordinal: 2,
                ..reference.clone()
            }
            .instance_id()
        );
        for source in ["", "a/b", "x\n", &"a".repeat(129)] {
            assert!(OpportunityRef {
                source: source.into(),
                ..reference.clone()
            }
            .validate()
            .is_err());
        }
        for ordinal in [0, 9, u16::MAX] {
            assert!(OpportunityRef {
                ordinal,
                ..reference.clone()
            }
            .validate()
            .is_err());
        }
        assert!(serde_json::from_value::<OpportunityRef>(serde_json::json!({"source":"maintenance","ordinal":1,"actor":"own-ship","job":"forged"})).is_err());
    }
}
