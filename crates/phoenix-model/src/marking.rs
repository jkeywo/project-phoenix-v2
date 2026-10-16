//! Closed physical marker equipment and retained causal state. No controller.
use crate::identity::SystemId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkingPayloadConfig {
    pub template: String,
}
impl MarkingPayloadConfig {
    pub fn validate(&self) -> Result<(), String> {
        let path = &self.template;
        if path.is_empty()
            || path.len() > 512
            || !path.ends_with(".toml")
            || path.starts_with('/')
            || path.contains('\\')
            || path.contains(':')
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || !path
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/_-.".contains(&byte))
        {
            return Err(
                "marking payload requires a bounded accepted relative template path".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenMarkingPayload {
    pub recipe: MarkingPayloadConfig,
    pub faction: uuid::Uuid,
}
impl FrozenMarkingPayload {
    pub fn validate(&self) -> Result<(), String> {
        self.recipe.validate()?;
        if self.faction.is_nil() {
            return Err("marker requires its real launch faction".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkingBeaconConfig {
    pub sensor: SystemId,
    pub radio: SystemId,
    pub channel: String,
}
impl MarkingBeaconConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.sensor == self.radio
            || [&self.sensor.0, &self.radio.0, &self.channel]
                .iter()
                .any(|id| {
                    id.is_empty()
                        || id.len() > 128
                        || !id
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                })
        {
            return Err(
                "marker requires distinct actual sensor/radio and a bounded channel".into(),
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MarkingLocation {
    Attached { subject: String, offset: [f32; 3] },
    Detached { tick: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkingReportFrontier {
    pub sample: String,
    pub observed_tick: u64,
    pub emitted_tick: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkingAttachment {
    pub round: String,
    pub source: String,
    pub payload: FrozenMarkingPayload,
    pub contact_tick: u64,
    pub location: MarkingLocation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<MarkingReportFrontier>,
}
impl MarkingAttachment {
    pub fn validate(&self, body: &str, now: u64) -> Result<(), String> {
        self.payload.validate()?;
        let valid =
            |id: &str| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control);
        if !valid(body)
            || !valid(&self.round)
            || !valid(&self.source)
            || self.source == body
            || self.round == body
            || self.round == self.source
            || self.contact_tick > now
        {
            return Err("marker has invalid actual causal identity/clock".into());
        }
        match &self.location {
            MarkingLocation::Attached { subject, offset }
                if !valid(subject)
                    || subject == body
                    || subject == &self.source
                    || subject == &self.round
                    || offset
                        .iter()
                        .any(|v| !v.is_finite() || v.abs() > 1_000_000.0) =>
            {
                return Err("marker has invalid actual attachment".into())
            }
            MarkingLocation::Detached { tick } if *tick < self.contact_tick || *tick > now => {
                return Err("marker detach must follow actual contact".into())
            }
            _ => {}
        }
        if self.report.as_ref().is_some_and(|report| {
            (report.sample.is_empty()
                || report.sample.len() > 1024
                || report.sample.chars().any(char::is_control))
                || report.observed_tick < self.contact_tick
                || report.observed_tick > report.emitted_tick
                || report.emitted_tick > now
        }) {
            return Err("marker report frontier has an invalid original clock".into());
        }
        Ok(())
    }
}
