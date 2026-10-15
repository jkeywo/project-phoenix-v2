//! Authored in-world radio vocabulary. Phoenix transport remains independent.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsChannelConfig {
    pub id: String,
    pub range: f32,
    pub work_units: u32,
    /// Forward compatible traffic through this actual paid radio channel.
    #[serde(default)]
    pub relay: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default = "default_protection_level")]
    pub protection_level: u8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsRadioConfig {
    pub system: SystemId,
    /// Explicit actual equipment capability; absent equipment cannot carry media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<crate::comms_media::MediaMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<crate::comms_capture::CommsCaptureConfig>,
    #[serde(default = "default_request_reading_ticks")]
    pub request_reading_ticks: u32,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(rename = "channel")]
    pub channels: Vec<CommsChannelConfig>,
}

fn default_protection_level() -> u8 {
    1
}

fn default_request_reading_ticks() -> u32 {
    30
}

/// Own equipment presentation; held key identities are never published here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommsChannelView {
    pub id: String,
    pub range: f32,
    pub work_units: u32,
    pub protected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommsAudience {
    Contact { target_uuid: String },
    Fleet,
    Faction { faction_uuid: String },
    All,
}

/// An immutable local emission fact or an actually received dated copy.
/// No receipt/reading/acceptance claim is implied by an outgoing record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsTransmission {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formation_sequence: Option<u64>,
    /// Only an actually accepted planned Contact request supports retained delivery.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sustained: bool,
    pub origin_id: String,
    pub channel: String,
    pub audience: CommsAudience,
    pub emitted_tick: u64,
    pub received_tick: Option<u64>,
    #[serde(
        default,
        skip_serializing_if = "crate::comms_request::CommsRadioContent::is_text"
    )]
    pub content: crate::comms_request::CommsRadioContent,
}

impl CommsTransmission {
    pub fn validate(&self, now: u64) -> Result<(), &'static str> {
        if matches!(&self.content, crate::comms_request::CommsRadioContent::Request { request } if request.is_media())
            && !matches!(self.audience, CommsAudience::Contact { .. })
        {
            return Err("human media call requires one explicit Contact recipient");
        }
        match &self.content {
            crate::comms_request::CommsRadioContent::Request { request } => {
                request.validate_sequence(self.formation_sequence)?
            }
            _ if self.formation_sequence.is_some() => {
                return Err("formation sequence requires its actual request")
            }
            _ => {}
        }
        if matches!(&self.content, crate::comms_request::CommsRadioContent::Request { request } if request.is_formation())
            && (!self.sustained || !matches!(self.audience, CommsAudience::Contact { .. }))
        {
            return Err("first formation request requires one explicit Contact recipient");
        }
        if self.sustained
            && (!matches!(
                self.content,
                crate::comms_request::CommsRadioContent::Request { .. }
            ) || !matches!(self.audience, CommsAudience::Contact { .. }))
        {
            return Err("sustained delivery requires a scoped planned Contact request");
        }
        if !identity(&self.origin_id)
            || !channel_id(&self.channel)
            || !self.audience.is_valid()
            || self.emitted_tick > now
            || self
                .received_tick
                .is_some_and(|tick| tick < self.emitted_tick || tick > now)
        {
            return Err("radio transmission has invalid identity, audience or original clock");
        }
        if let crate::comms_request::CommsRadioContent::Outcome { outcome } = &self.content {
            outcome.validate(self.emitted_tick)?;
        }
        Ok(())
    }
}

impl CommsAudience {
    pub fn is_valid(&self) -> bool {
        match self {
            Self::Contact { target_uuid } => identity(target_uuid),
            Self::Faction { faction_uuid } => identity(faction_uuid),
            Self::Fleet | Self::All => true,
        }
    }
}

pub fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

pub fn channel_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || value == b'-')
}

pub fn valid_text(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 4096
        && !value
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
}

impl CommsRadioConfig {
    pub fn validate(&self, endpoint_range: f32) -> Result<(), &'static str> {
        use std::collections::BTreeSet;
        if let Some(capture) = &self.capture {
            capture.validate()?;
        }
        let mut channels = BTreeSet::new();
        let mut keys = BTreeSet::new();
        if !identity(&self.system.0)
            || self.request_reading_ticks == 0
            || !endpoint_range.is_finite()
            || endpoint_range <= 0.0
            || self.channels.is_empty()
            || self
                .keys
                .iter()
                .any(|key| !identity(key) || !keys.insert(key.clone()))
            || self.channels.iter().any(|channel| {
                !(1..=3).contains(&channel.protection_level)
                    || !channel_id(&channel.id)
                    || !channels.insert(channel.id.clone())
                    || !channel.range.is_finite()
                    || channel.range <= 0.0
                    || channel.range > endpoint_range
                    || channel.work_units == 0
                    || channel.key.as_ref().is_some_and(|key| !keys.contains(key))
            })
        {
            return Err("radio channels require unique identities, finite endpoint-bounded reach, positive work and actually held keys");
        }
        Ok(())
    }
}

/// Whole-inbox preflight: copies may outlive their sender's cleared local
/// history, but every surviving copy of one emission must retain its contents.
pub fn validate_transmissions(
    messages: &[crate::messages::CommsMessage],
    now: u64,
) -> Result<(), &'static str> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut ids = BTreeSet::new();
    let mut originals = BTreeMap::new();
    let mut recipients = BTreeSet::new();
    for message in messages {
        if !ids.insert(&message.id) {
            return Err("Comms message identities must be unique");
        }
        let Some(transmission) = &message.transmission else {
            if message.request_progress.is_some() {
                return Err("generic request progress requires a physical transmission");
            }
            continue;
        };
        transmission.validate(now)?;
        let recipient = message
            .recipient_ship
            .as_ref()
            .ok_or("radio copies require an actual recipient scope")?;
        if !recipients.insert((&transmission.origin_id, &recipient.0)) {
            return Err("one emission may have only one copy for each actual recipient");
        }
        if !identity(&message.sender_uuid)
            || !identity(&recipient.0)
            || (transmission.received_tick.is_none()
                && (recipient.0 != message.sender_uuid || !message.is_read))
            || (transmission.received_tick.is_some() && recipient.0 == message.sender_uuid)
            || matches!(&transmission.audience, CommsAudience::Contact { target_uuid } if transmission.received_tick.is_some() && target_uuid != &recipient.0)
        {
            return Err("radio copy disagrees with its sent/received ownership");
        }
        crate::comms_request::validate_copy(message, now)?;
        let origin = (
            &message.sender_uuid,
            &message.sender_name,
            &message.body,
            &transmission.channel,
            &transmission.audience,
            transmission.emitted_tick,
            transmission.sustained,
            transmission.formation_sequence,
            &transmission.content,
        );
        if originals
            .insert(&transmission.origin_id, origin)
            .is_some_and(|previous| previous != origin)
        {
            return Err("radio copies disagree with their original emission");
        }
    }
    crate::comms_request::validate_lineage(messages)?;
    validate_formation_order(messages)
}

#[cfg(test)]
#[path = "comms_channel_tests.rs"]
mod tests;

fn is_false(value: &bool) -> bool {
    !*value
}

/// Derived from the same retained actual source records, never a second counter.
fn validate_formation_order(
    messages: &[crate::messages::CommsMessage],
) -> Result<(), &'static str> {
    let mut owners =
        std::collections::BTreeMap::<&str, std::collections::BTreeMap<u64, (u64, &str)>>::new();
    for message in messages {
        let Some(wire) = message
            .transmission
            .as_ref()
            .filter(|wire| wire.received_tick.is_none())
        else {
            continue;
        };
        let Some(sequence) = wire.formation_sequence else {
            continue;
        };
        let rows = owners.entry(&message.sender_uuid).or_default();
        if rows
            .insert(sequence, (wire.emitted_tick, &wire.origin_id))
            .is_some()
        {
            return Err("one source cannot reuse a formation sequence");
        }
    }
    let mut accepted =
        std::collections::BTreeMap::<(&str, &str), std::collections::BTreeMap<u64, u64>>::new();
    for message in messages {
        let Some(wire) = message
            .transmission
            .as_ref()
            .filter(|wire| wire.received_tick.is_some())
        else {
            continue;
        };
        let (Some(sequence), Some(recipient), Some(decision)) = (
            wire.formation_sequence,
            message.recipient_ship.as_ref(),
            message
                .request_progress
                .as_ref()
                .and_then(|progress| progress.facts.decision.as_ref()),
        ) else {
            continue;
        };
        if decision.choice == crate::comms_request::CommsRequestDecision::Accept {
            accepted
                .entry((&message.sender_uuid, &recipient.0))
                .or_default()
                .insert(sequence, decision.tick);
        }
    }
    for rows in accepted.values() {
        let mut previous_tick = 0;
        for tick in rows.values() {
            if *tick < previous_tick {
                return Err("an older formation sequence was accepted after a newer one");
            }
            previous_tick = *tick;
        }
    }
    for rows in owners.values() {
        let mut previous_tick = 0;
        for (index, (sequence, (tick, _))) in rows.iter().enumerate() {
            if *sequence != index as u64 + 1 || *tick < previous_tick {
                return Err("formation source history has a missing or reordered sequence");
            }
            previous_tick = *tick;
        }
    }
    Ok(())
}
