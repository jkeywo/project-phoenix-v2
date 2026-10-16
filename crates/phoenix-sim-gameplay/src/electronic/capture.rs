//! Captured text and one paid decoder frontier. Acquisition/payment/authority
//! are supplied by the actual radio owner; this module never queries the world.
use phoenix_model::{
    comms_capture::{
        CapturedContentKind, CommsCaptureConfig, CommsCaptureView, DecodedCommsClaim,
        DecodedCommsEvidence, DecodedDecisionClaim, DecodedOutcomeClaim, DecodedRequestClaim,
        DecodedStatusClaim, MAX_CAPTURE_ROWS, MAX_CAPTURE_SEQUENCE, WORK_PER_TICK,
    },
    comms_channel::{identity, valid_text, CommsTransmission},
    comms_request::{CommsRadioContent, GenericCommsRequest},
    identity::SystemId,
};
use serde::{Deserialize, Serialize};

fn request_claim(request: &GenericCommsRequest) -> DecodedRequestClaim {
    match request {
        GenericCommsRequest::ReportStatus => DecodedRequestClaim::ReportStatus,
        GenericCommsRequest::FormUp { slot } => DecodedRequestClaim::FormUp { slot: *slot },
        GenericCommsRequest::MediaCall { mode } => DecodedRequestClaim::MediaCall { mode: *mode },
    }
}

fn decoded_claim(content: &CommsRadioContent) -> DecodedCommsClaim {
    match content {
        CommsRadioContent::Text => DecodedCommsClaim::Text,
        CommsRadioContent::Request { request } => DecodedCommsClaim::Request {
            request: request_claim(request),
        },
        CommsRadioContent::Outcome { outcome } => {
            DecodedCommsClaim::Outcome {
                outcome: DecodedOutcomeClaim {
                    request: request_claim(&outcome.request.request),
                    request_emitted_tick: outcome.request.emitted_tick,
                    heard_tick: outcome.request.heard_tick,
                    read_tick: outcome.facts.read_tick,
                    decision: outcome.facts.decision.as_ref().map(|decision| {
                        DecodedDecisionClaim {
                            choice: decision.choice,
                            tick: decision.tick,
                        }
                    }),
                    report: outcome
                        .facts
                        .report
                        .as_ref()
                        .map(|report| DecodedStatusClaim {
                            available: report.available,
                            observed_tick: report.observed_tick,
                        }),
                },
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmissionKey {
    tick: u64,
    source: String,
    origin: String,
}

/// Private actual emission evidence. Never use this type as a public DTO or AI
/// input: even its identities are not known from the undecoded radio header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedPayload {
    pub source: String,
    pub name: String,
    pub body: String,
    pub literal_body: bool,
    pub transmission: CommsTransmission,
    pub level: u8,
    pub key: Option<String>,
}
impl CapturedPayload {
    fn key(&self) -> EmissionKey {
        EmissionKey {
            tick: self.transmission.emitted_tick,
            source: self.source.clone(),
            origin: self.transmission.origin_id.clone(),
        }
    }
    pub fn validate(&self, now: u64) -> Result<(), &'static str> {
        self.transmission.validate(now)?;
        if !identity(&self.source)
            || !valid_text(&self.name)
            || !valid_text(&self.body)
            || self.transmission.received_tick.is_some()
            || self.literal_body != self.transmission.content.is_text()
            || match &self.key {
                Some(key) => !identity(key) || !(1..=3).contains(&self.level),
                None => self.level != 0,
            }
        {
            return Err("capture payload must retain an original actual text emission and closed protection");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Capture {
    sequence: u64,
    captured_tick: u64,
    payload: CapturedPayload,
    work: u64,
    started_tick: Option<u64>,
    last_work_tick: Option<u64>,
    decoded_tick: Option<u64>,
}

/// Canonical continuation, held by the existing Comms owner. The private
/// retirement watermark prevents an old sustained emission being recaptured
/// after bounded evidence retirement, without an unbounded deduplication set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureState {
    owner: SystemId,
    next_sequence: u64,
    retired: Option<EmissionKey>,
    selected: Option<u64>,
    rows: Vec<Capture>,
}
impl CaptureState {
    pub fn new(owner: SystemId) -> Self {
        Self {
            owner,
            next_sequence: 1,
            retired: None,
            selected: None,
            rows: vec![],
        }
    }
    pub fn owner(&self) -> &SystemId {
        &self.owner
    }
    pub fn selected(&self) -> Option<u64> {
        self.selected
    }
    /// Trusted whole-candidate validation only; never a Console/AI publication.
    pub fn payloads(&self) -> impl Iterator<Item = &CapturedPayload> {
        self.rows.iter().map(|row| &row.payload)
    }
    pub fn observe(&mut self, payload: CapturedPayload, now: u64) -> bool {
        if payload.validate(now).is_err()
            || payload.transmission.emitted_tick != now
            || self.next_sequence > MAX_CAPTURE_SEQUENCE
        {
            return false;
        }
        let key = payload.key();
        if self.retired.as_ref().is_some_and(|retired| &key <= retired)
            || self.rows.iter().any(|row| row.payload.key() == key)
        {
            return false;
        }
        let unprotected = payload.level == 0;
        self.rows.push(Capture {
            sequence: self.next_sequence,
            captured_tick: now,
            payload,
            work: 0,
            started_tick: None,
            last_work_tick: None,
            decoded_tick: unprotected.then_some(now),
        });
        self.next_sequence += 1;
        if self.rows.len() > MAX_CAPTURE_ROWS {
            let index = self
                .rows
                .iter()
                .enumerate()
                .min_by_key(|(_, row)| row.payload.key())
                .expect("nonempty history")
                .0;
            let retired = self.rows.remove(index);
            self.retired = Some(retired.payload.key());
            if self.selected == Some(retired.sequence) {
                self.selected = None;
            }
        }
        true
    }
    /// `keys` is the immutable owning accepted radio key set. Runtime key
    /// acquisition/revocation requires a later dated provenance/history consumer.
    pub fn required_work(
        &self,
        sequence: u64,
        config: &CommsCaptureConfig,
        keys: &[String],
    ) -> Option<u64> {
        let row = self
            .rows
            .iter()
            .find(|row| row.sequence == sequence && row.decoded_tick.is_none())?;
        config.required_work(
            row.payload.level,
            row.payload
                .key
                .as_ref()
                .is_some_and(|key| keys.contains(key)),
        )
    }
    pub fn select(
        &mut self,
        sequence: Option<u64>,
        config: &CommsCaptureConfig,
        keys: &[String],
    ) -> bool {
        if sequence.is_some_and(|sequence| self.required_work(sequence, config, keys).is_none()) {
            return false;
        }
        self.selected = sequence;
        true
    }
    /// Caller supplies only the fraction from THIS actual extra continuous work
    /// grant. A stale/double interval, unavailable route or invalid fraction does
    /// not advance plaintext. Quantisation always rounds partial payment down.
    pub fn advance(
        &mut self,
        now: u64,
        paid_fraction: f32,
        config: &CommsCaptureConfig,
        keys: &[String],
    ) -> bool {
        let Some(sequence) = self.selected else {
            return false;
        };
        let Some(required) = self.required_work(sequence, config, keys) else {
            return false;
        };
        if !paid_fraction.is_finite() || paid_fraction <= 0.0 || paid_fraction > 1.0 {
            return false;
        }
        if self
            .rows
            .iter()
            .any(|row| row.last_work_tick.is_some_and(|last| last >= now))
        {
            return false;
        }
        let units = (f64::from(paid_fraction) * WORK_PER_TICK as f64).floor() as u64;
        let row = self
            .rows
            .iter_mut()
            .find(|row| row.sequence == sequence)
            .expect("selected row");
        if units == 0
            || now < row.captured_tick
            || row.last_work_tick.is_some_and(|last| last >= now)
        {
            return false;
        }
        row.started_tick.get_or_insert(now);
        row.last_work_tick = Some(now);
        row.work = row.work.saturating_add(units).min(required);
        if row.work == required {
            row.decoded_tick = Some(now);
            self.selected = None;
        }
        true
    }
    pub fn views(&self, config: &CommsCaptureConfig, keys: &[String]) -> Vec<CommsCaptureView> {
        self.rows
            .iter()
            .map(|row| {
                let required = self.required_work(row.sequence, config, keys);
                CommsCaptureView {
                    capture: row.sequence,
                    channel: row.payload.transmission.channel.clone(),
                    level: row.payload.level,
                    captured_tick: row.captured_tick,
                    feasible: required.is_some(),
                    selected: self.selected == Some(row.sequence),
                    progress_per_mille: if row.decoded_tick.is_some() {
                        1000
                    } else {
                        required.map_or(0, |required| {
                            (row.work.saturating_mul(1000) / required).min(999) as u16
                        })
                    },
                    decoded_tick: row.decoded_tick,
                    evidence: row.decoded_tick.map(|_| DecodedCommsEvidence {
                        sender_name: row.payload.name.clone(),
                        body: row.payload.body.clone(),
                        literal_body: row.payload.literal_body,
                        kind: match row.payload.transmission.content {
                            CommsRadioContent::Text => CapturedContentKind::Text,
                            CommsRadioContent::Request { .. } => CapturedContentKind::Request,
                            CommsRadioContent::Outcome { .. } => CapturedContentKind::Outcome,
                        },
                        emitted_tick: row.payload.transmission.emitted_tick,
                        claim: decoded_claim(&row.payload.transmission.content),
                    }),
                }
            })
            .collect()
    }
    pub fn validate(
        &self,
        owner: &SystemId,
        config: &CommsCaptureConfig,
        keys: &[String],
        now: u64,
    ) -> Result<(), &'static str> {
        use std::collections::BTreeSet;
        config.validate()?;
        if &self.owner != owner
            || !identity(&owner.0)
            || self.next_sequence == 0
            || self.next_sequence > MAX_CAPTURE_SEQUENCE + 1
            || self.rows.len() > MAX_CAPTURE_ROWS
            || self.retired.as_ref().is_some_and(|key| {
                key.tick > now || !identity(&key.source) || !identity(&key.origin)
            })
        {
            return Err("capture state has invalid equipment owner, sequence or retirement");
        }
        let mut origins = BTreeSet::new();
        let mut paid_ends = BTreeSet::new();
        let mut last_sequence = 0;
        for row in &self.rows {
            row.payload.validate(now)?;
            let max_work =
                u64::from(config.ticks_per_level) * u64::from(row.payload.level) * WORK_PER_TICK;
            let key_work = config
                .held_key_ticks
                .map_or(0, |ticks| u64::from(ticks) * WORK_PER_TICK);
            let protected = row.payload.level > 0;
            if row.sequence <= last_sequence
                || row.sequence >= self.next_sequence
                || !origins.insert(row.payload.key())
                || self
                    .retired
                    .as_ref()
                    .is_some_and(|retired| row.payload.key() <= *retired)
                || row.captured_tick != row.payload.transmission.emitted_tick
                || row.captured_tick > now
                || row.work > max_work.max(key_work)
                || (row.work > 0
                    && config
                        .required_work(
                            row.payload.level,
                            row.payload
                                .key
                                .as_ref()
                                .is_some_and(|key| keys.contains(key)),
                        )
                        .is_none())
                || (row.work > 0) != row.started_tick.is_some()
                || row.started_tick.is_some() != row.last_work_tick.is_some()
                || row
                    .last_work_tick
                    .is_some_and(|last| !paid_ends.insert(last))
                || row
                    .started_tick
                    .is_some_and(|tick| tick < row.captured_tick || tick > now)
                || row
                    .last_work_tick
                    .is_some_and(|tick| tick < row.started_tick.unwrap_or(0) || tick > now)
                || row.last_work_tick.is_some_and(|last| {
                    u128::from(row.work)
                        > u128::from(
                            last.saturating_sub(row.started_tick.unwrap_or(last))
                                .saturating_add(1),
                        ) * u128::from(WORK_PER_TICK)
                })
                || row
                    .decoded_tick
                    .is_some_and(|tick| tick < row.captured_tick || tick > now)
                || (!protected
                    && (row.work != 0
                        || row.started_tick.is_some()
                        || row.decoded_tick != Some(row.captured_tick)))
                || (protected
                    && row.decoded_tick.is_some()
                    && (row.last_work_tick != row.decoded_tick
                        || config.required_work(
                            row.payload.level,
                            row.payload
                                .key
                                .as_ref()
                                .is_some_and(|key| keys.contains(key)),
                        ) != Some(row.work)))
                || (protected
                    && row.decoded_tick.is_none()
                    && self
                        .required_work(row.sequence, config, keys)
                        .is_some_and(|required| row.work >= required))
            {
                return Err(
                    "capture evidence has invalid immutable lineage, clocks or paid frontier",
                );
            }
            last_sequence = row.sequence;
        }
        if self
            .selected
            .is_some_and(|sequence| self.required_work(sequence, config, keys).is_none())
        {
            return Err("selected decode is absent, completed or unsupported");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
