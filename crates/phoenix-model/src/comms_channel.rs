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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsRadioConfig {
    pub system: SystemId,
    #[serde(default = "default_request_reading_ticks")]
    pub request_reading_ticks: u32,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(rename = "channel")]
    pub channels: Vec<CommsChannelConfig>,
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
                !channel_id(&channel.id)
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
            &transmission.content,
        );
        if originals
            .insert(&transmission.origin_id, origin)
            .is_some_and(|previous| previous != origin)
        {
            return Err("radio copies disagree with their original emission");
        }
    }
    crate::comms_request::validate_lineage(messages)
}

#[cfg(test)]
#[path = "comms_channel_tests.rs"]
mod tests;
