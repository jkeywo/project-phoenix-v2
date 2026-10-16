//! Own Radio coverage and contextual public intent. No message-volume resource.
use crate::{
    comms_channel::{channel_id, identity, valid_text},
    identity::SystemId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsListeningConfig {
    pub simultaneous_channels: u16,
    pub priority: Vec<String>,
}

impl CommsListeningConfig {
    pub fn validate(&self, channels: &[String]) -> Result<(), &'static str> {
        if self.simultaneous_channels == 0
            || usize::from(self.simultaneous_channels) > channels.len()
            || !exact_channel_order(&self.priority, channels)
        {
            return Err(
                "listening requires positive equipment-bounded coverage and an exact channel order",
            );
        }
        Ok(())
    }
}

pub fn exact_channel_order(order: &[String], channels: &[String]) -> bool {
    let expected: BTreeSet<_> = channels.iter().collect();
    let actual: BTreeSet<_> = order.iter().collect();
    !channels.is_empty()
        && channels.iter().all(|id| channel_id(id))
        && expected.len() == channels.len()
        && actual.len() == order.len()
        && order.len() == channels.len()
        && actual == expected
}

/// The one supported contextual producer reads a permitted Captain alert view.
/// Source binding is a publication key, never a seat or a hidden actor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsAlertTrafficConfig {
    pub source: SystemId,
    pub channel: String,
    pub minimum_spacing_ticks: u32,
    pub routine_text: String,
    pub red_alert_text: String,
}

impl CommsAlertTrafficConfig {
    pub fn validate(&self, channels: &[String]) -> Result<(), &'static str> {
        if !identity(&self.source.0)
            || !channel_id(&self.channel)
            || !channels.contains(&self.channel)
            || self.minimum_spacing_ticks == 0
            || !valid_text(&self.routine_text)
            || !valid_text(&self.red_alert_text)
            || self.routine_text == self.red_alert_text
        {
            return Err("alert traffic requires an explicit own source, supported channel, distinct valid templates and positive spacing");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsTrafficConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listening: Option<CommsListeningConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<CommsAlertTrafficConfig>,
}

impl CommsTrafficConfig {
    /// Native equipment validation additionally checks the actual unkeyed
    /// channel and exact permitted typed feed against its accepted ship recipe.
    pub fn validate(&self, channels: &[String]) -> Result<(), &'static str> {
        if channels.len() > usize::from(u16::MAX) {
            return Err("traffic channel count exceeds the supported equipment bound");
        }
        if self.listening.is_none() && self.alert.is_none() {
            return Err("traffic profile requires a supported listening or alert consumer");
        }
        if let Some(profile) = &self.listening {
            profile.validate(channels)?;
        }
        if let Some(profile) = &self.alert {
            profile.validate(channels)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportedAlert {
    Routine,
    RedAlert,
}

impl From<bool> for ReportedAlert {
    fn from(red: bool) -> Self {
        if red {
            Self::RedAlert
        } else {
            Self::Routine
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuccessfulAlertReport {
    pub semantic: ReportedAlert,
    pub emitted_tick: u64,
}

/// Permitted own intent and the latest actual Radio coverage, never traffic
/// activity, held keys or evidence about another endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsTrafficView {
    pub channels: Vec<String>,
    pub priority: Vec<String>,
    pub simultaneous_channels: u16,
    pub monitored: Vec<String>,
    pub observed_tick: Option<u64>,
    /// Supported verb, not a guarantee of fresh context, funding or readiness.
    pub report_supported: bool,
    pub last_report: Option<SuccessfulAlertReport>,
    pub controllable: bool,
}

#[cfg(test)]
#[path = "comms_traffic_tests.rs"]
mod tests;
