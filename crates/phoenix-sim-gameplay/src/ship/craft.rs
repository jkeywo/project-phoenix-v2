//! One actual handling Job per installed bay. No craft, stock or motion copy.
use crate::ai::state_tree::TreeRuntime;
use crate::jobs::Job;
use phoenix_model::{
    craft::BayConfig,
    identity::SystemId,
    job::{JobId, JobOutcome},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BayOperation {
    Launch,
    Recover,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayActivity {
    pub craft_uuid: String,
    pub operation: BayOperation,
    pub accepted_tick: u64,
    pub profile: BayConfig,
    pub energy_per_sec: f32,
    pub worked_secs: f64,
}
impl BayActivity {
    pub fn required_secs(&self) -> f64 {
        f64::from(match self.operation {
            BayOperation::Launch => self.profile.launch_work_secs,
            BayOperation::Recover => self.profile.recovery_work_secs,
        })
    }
    pub fn ready(&self) -> bool {
        self.worked_secs == self.required_secs()
    }
    fn validate(&self, now: u64) -> Result<(), String> {
        self.profile.validate()?;
        if !identity(&self.craft_uuid)
            || self.accepted_tick > now
            || !self.energy_per_sec.is_finite()
            || self.energy_per_sec <= 0.0
            || !self.worked_secs.is_finite()
            || self.worked_secs < 0.0
            || self.worked_secs > self.required_secs()
        {
            return Err("invalid accepted finite bay work/identity/clock".into());
        }
        Ok(())
    }
}

/// The existing Job is the sole phase/pause/outcome and progress wrapper. A
/// terminal row may be replaced by the next real assignment, never a second job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayState {
    pub next_id: u64,
    pub job: Option<Job<BayActivity>>,
}
impl Default for BayState {
    fn default() -> Self {
        Self {
            next_id: 1,
            job: None,
        }
    }
}
impl BayState {
    pub fn validate(&self, owner: &SystemId, now: u64) -> Result<(), String> {
        if !identity(&owner.0) || self.next_id == 0 {
            return Err("bay requires actual owner and monotonic Job identity".into());
        }
        if let Some(job) = &self.job {
            job.activity.validate(now)?;
            if job.owner != *owner
                || job.id.0 == 0
                || job.id.0 >= self.next_id
                || job.execution_tick < job.activity.accepted_tick
                || job.execution_tick > now
                || job.runtime != TreeRuntime::default()
                || job.workflow_error
                || job.outcome == Some(JobOutcome::Failed)
                || job.outcome.is_some() && job.paused
                || job.outcome == Some(JobOutcome::Completed) && !job.activity.ready()
            {
                return Err("bay Job disagrees with its owner/work/clock frontier".into());
            }
        }
        Ok(())
    }
    /// Same paused assignment resumes without resetting work or minting a Job.
    /// Actual roster/space/source/resource guards are staged by the consumer.
    pub fn begin(
        &mut self,
        owner: &SystemId,
        craft_uuid: &str,
        operation: BayOperation,
        profile: &BayConfig,
        energy_per_sec: f32,
        now: u64,
    ) -> Result<(), String> {
        self.validate(owner, now)?;
        if let Some(job) = &mut self.job {
            if job.outcome.is_none() {
                if job.activity.craft_uuid == craft_uuid && job.activity.operation == operation {
                    job.paused = false;
                    return Ok(());
                }
                return Err("actual bay already owns another handling Job".into());
            }
        }
        let activity = BayActivity {
            craft_uuid: craft_uuid.into(),
            operation,
            accepted_tick: now,
            profile: profile.clone(),
            energy_per_sec,
            worked_secs: 0.0,
        };
        activity.validate(now)?;
        let next = self
            .next_id
            .checked_add(1)
            .ok_or("bay Job identity frontier exhausted")?;
        self.job = Some(Job {
            id: JobId(self.next_id),
            owner: owner.clone(),
            activity,
            runtime: TreeRuntime::default(),
            execution_tick: now,
            outcome: None,
            paused: false,
            workflow_error: false,
        });
        self.next_id = next;
        Ok(())
    }
    pub fn cancel(&mut self) -> bool {
        let Some(job) = self.job.as_mut().filter(|job| job.outcome.is_none()) else {
            return false;
        };
        job.outcome = Some(JobOutcome::Cancelled);
        job.paused = false;
        true
    }
    /// Return readiness for the consumer's single atomic location handoff. The
    /// paid callback runs at most once per positive interval, never on replay,
    /// paused/unavailable gear or already-finished work awaiting clear space.
    pub fn advance(
        &mut self,
        owner: &SystemId,
        now: u64,
        dt: f32,
        operation: f32,
        mut paid_fraction: impl FnMut(f32, f32) -> f32,
    ) -> Result<bool, String> {
        self.validate(owner, now)?;
        if !dt.is_finite()
            || dt < 0.0
            || !operation.is_finite()
            || !(0.0..=1.0).contains(&operation)
        {
            return Err("bay interval/Operation must be finite and bounded".into());
        }
        let Some(mut job) = self.job.clone().filter(|job| job.outcome.is_none()) else {
            return Ok(false);
        };
        if now <= job.execution_tick {
            return Ok(false);
        }
        job.execution_tick = now;
        // Interruption is current machinery availability, never a sticky
        // operator pause: operation recovery resumes this very assignment.
        job.paused = operation == 0.0;
        if job.paused || dt == 0.0 {
            self.job = Some(job);
            return Ok(false);
        }
        if job.activity.ready() {
            self.job = Some(job);
            return Ok(true);
        }
        let remaining = job.activity.required_secs() - job.activity.worked_secs;
        let work = (f64::from(dt) * f64::from(operation)).min(remaining);
        // Round the paid interval up, but credit only the bounded work. This
        // also settles a representable residual smaller than one f32 interval.
        let mut seconds = work as f32;
        if f64::from(seconds) < work {
            seconds = f32::from_bits(seconds.to_bits() + 1);
        }
        if seconds <= 0.0 {
            self.job = Some(job);
            return Ok(false);
        }
        let fraction = paid_fraction(job.activity.energy_per_sec, seconds);
        if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
            return Err("actual bay work grant must be finite within [0,1]".into());
        }
        job.paused = fraction == 0.0;
        job.activity.worked_secs = (job.activity.worked_secs + work * f64::from(fraction))
            .min(job.activity.required_secs());
        let ready = job.activity.ready();
        self.job = Some(job);
        Ok(ready)
    }
    /// Call only in the same transaction as the actual location/physical handoff.
    pub fn complete(&mut self) -> bool {
        let Some(job) = self
            .job
            .as_mut()
            .filter(|job| job.outcome.is_none() && job.activity.ready() && !job.paused)
        else {
            return false;
        };
        job.outcome = Some(JobOutcome::Completed);
        true
    }
}
fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

#[cfg(test)]
#[path = "craft_tests.rs"]
mod tests;
