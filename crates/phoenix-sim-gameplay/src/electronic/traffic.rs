//! Radio intent and the last successful contextual report, not another inbox.
use phoenix_model::{
    comms_channel::identity,
    comms_traffic::{
        exact_channel_order, CommsTrafficConfig, ReportedAlert, SuccessfulAlertReport,
    },
    identity::SystemId,
    observation::{
        ObservationMetadata, ObservationSource, ObservationSubject, ObservationUncertainty,
    },
};
use serde::{Deserialize, Serialize};

/// Borrowed/derived from an actually permitted own publication. Never captured
/// as another fact store or constructed by looking up a private machinery flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AlertSample {
    semantic: ReportedAlert,
    observed_tick: u64,
}

impl AlertSample {
    /// Caller must first resolve the accepted explicit Captain feed and its
    /// actual Alert System. This validates that producer's exact local metadata.
    pub fn from_publication(
        red_alert: bool,
        owner: &SystemId,
        metadata: &ObservationMetadata,
        now: u64,
    ) -> Option<Self> {
        let observed_tick = metadata.observed_tick?;
        (identity(&owner.0)
            && metadata.subject == ObservationSubject::System { id: owner.clone() }
            && metadata.provenance.kind == ObservationSource::SystemReport
            && metadata.provenance.system.as_ref() == Some(owner)
            && metadata.provenance.label.is_empty()
            && metadata.received_tick.is_none()
            && metadata.uncertainty == ObservationUncertainty::Unspecified
            && observed_tick <= now
            && now - observed_tick <= 1)
            .then_some(Self {
                semantic: red_alert.into(),
                observed_tick,
            })
    }

    fn is_current(self, now: u64) -> bool {
        self.observed_tick <= now && now - self.observed_tick <= 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommsTrafficState {
    pub owner: SystemId,
    /// Empty only when this equipment has no optional finite coverage profile.
    pub priority: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<SuccessfulAlertReport>,
}

impl CommsTrafficState {
    pub fn new(
        owner: SystemId,
        profile: &CommsTrafficConfig,
        channels: &[String],
    ) -> Result<Self, &'static str> {
        profile.validate(channels)?;
        if !identity(&owner.0) {
            return Err("traffic requires an actual System owner");
        }
        Ok(Self {
            owner,
            priority: profile
                .listening
                .as_ref()
                .map_or_else(Vec::new, |config| config.priority.clone()),
            last_report: None,
        })
    }

    pub fn validate(
        &self,
        profile: &CommsTrafficConfig,
        channels: &[String],
        now: u64,
    ) -> Result<(), &'static str> {
        profile.validate(channels)?;
        if !identity(&self.owner.0)
            || match &profile.listening {
                Some(_) => !exact_channel_order(&self.priority, channels),
                None => !self.priority.is_empty(),
            }
            || self
                .last_report
                .is_some_and(|reported| profile.alert.is_none() || reported.emitted_tick > now)
        {
            return Err("traffic owner, order or successful-report frontier is invalid");
        }
        Ok(())
    }

    pub fn set_order(
        &mut self,
        profile: &CommsTrafficConfig,
        channels: &[String],
        priority: Vec<String>,
    ) -> Result<(), &'static str> {
        profile.validate(channels)?;
        if profile.listening.is_none() || !exact_channel_order(&priority, channels) {
            return Err("this Radio requires its exact supported listening order");
        }
        self.priority = priority;
        Ok(())
    }

    /// Configuration intent only. Actual receivers must additionally use the
    /// existing current paid endpoint, Operation, keys and physical path gate.
    pub fn configured_receive_channels<'a>(
        &'a self,
        profile: &CommsTrafficConfig,
        channels: &'a [String],
    ) -> Result<Vec<&'a str>, &'static str> {
        profile.validate(channels)?;
        match &profile.listening {
            Some(coverage) if exact_channel_order(&self.priority, channels) => Ok(self
                .priority
                .iter()
                .take(usize::from(coverage.simultaneous_channels))
                .map(String::as_str)
                .collect()),
            Some(_) => Err("invalid current listening order"),
            None if self.priority.is_empty() => Ok(channels.iter().map(String::as_str).collect()),
            None => Err("unexpected listening intent without equipment"),
        }
    }

    /// Exclusive prepared token performs no write. Actual Radio payment and
    /// transmission must succeed before consuming it; failed work drops it.
    pub fn prepare_report<'a>(
        &'a mut self,
        profile: &CommsTrafficConfig,
        channels: &[String],
        sample: Option<AlertSample>,
        now: u64,
    ) -> Result<Option<PreparedAlertReport<'a>>, &'static str> {
        self.validate(profile, channels, now)?;
        let Some(config) = &profile.alert else {
            return Ok(None);
        };
        let Some(sample) = sample.filter(|sample| sample.is_current(now)) else {
            return Ok(None);
        };
        if self.last_report.is_some_and(|previous| {
            previous.semantic == sample.semantic
                || now - previous.emitted_tick < u64::from(config.minimum_spacing_ticks)
        }) {
            return Ok(None);
        }
        let body = match sample.semantic {
            ReportedAlert::Routine => &config.routine_text,
            ReportedAlert::RedAlert => &config.red_alert_text,
        }
        .clone();
        Ok(Some(PreparedAlertReport {
            state: self,
            body,
            semantic: sample.semantic,
            now,
        }))
    }
}

pub struct PreparedAlertReport<'a> {
    state: &'a mut CommsTrafficState,
    body: String,
    semantic: ReportedAlert,
    now: u64,
}

impl PreparedAlertReport<'_> {
    pub fn body(&self) -> &str {
        &self.body
    }

    /// Infallible same-interval commit AFTER actual paid emission. This is not
    /// remote delivery, reading, completion or a stored copy of its text.
    pub fn commit(self) {
        self.state.last_report = Some(SuccessfulAlertReport {
            semantic: self.semantic,
            emitted_tick: self.now,
        });
    }
}

#[cfg(test)]
#[path = "traffic_tests.rs"]
mod tests;
