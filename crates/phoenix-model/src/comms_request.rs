//! Closed generic request content and recipient-local inbox progress.
//! Physical delivery/payment, Console permission and cross-copy lineage are
//! validated by the actual Comms owner before these records become knowledge.
use crate::{comms_channel::identity, identity::SystemId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenericCommsRequest {
    ReportStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommsRequestDecision {
    Report,
    Decline,
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
    pub read_tick: Option<u64>,
    pub decision: Option<DatedCommsDecision>,
    pub report: Option<CommsStatusReport>,
}

impl CommsRequestFacts {
    pub fn stage(&self) -> CommsRequestStage {
        if self.report.is_some() {
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
                (CommsRequestDecision::Decline, None) => {}
                (CommsRequestDecision::Report, Some(report))
                    if identity(&report.system.0) && report.observed_tick == decision.tick => {}
                _ => return Err("Report Status decision and actual local report disagree"),
            }
        } else if self.report.is_some() {
            return Err("request completion requires the actual receiving decision");
        }
        Ok(())
    }
}

/// Immutable binding carried by a returned claim, not a remote actuator token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsRequestReference {
    pub origin_id: String,
    pub source_ship: String,
    pub recipient_ship: String,
    pub emitted_tick: u64,
    pub heard_tick: u64,
    pub request: GenericCommsRequest,
}

impl CommsRequestReference {
    pub fn validate(&self, now: u64) -> Result<(), &'static str> {
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
        self.facts.validate(self.request.heard_tick, now)
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

#[cfg(test)]
#[path = "comms_request_tests.rs"]
mod tests;
