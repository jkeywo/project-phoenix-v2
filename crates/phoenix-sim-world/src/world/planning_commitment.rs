//! Existing narrative acceptance gates the one root; no new ledger or machinery.
use super::*;
use crate::world::commitments::{Commitment, CommitmentLedger, CommitmentState};
impl CompiledPlanning {
    fn required_record<'a>(
        &self,
        ledger: &'a CommitmentLedger,
        now: u64,
    ) -> Result<Option<&'a Commitment>, String> {
        let Some(id) = &self.config.required_commitment else {
            return Ok(None);
        };
        let mut records = ledger.records.iter().filter(|row| &row.id == id);
        let Some(record) = records.next() else {
            return Ok(None);
        };
        if records.next().is_some() || record.made_at_tick > now {
            return Err("required promise has duplicate identity or a future made clock".into());
        }
        let valid = match (record.state, record.resolved_at_tick) {
            (CommitmentState::Open, None) => true,
            (CommitmentState::Kept | CommitmentState::Broken, Some(tick)) => {
                record.made_at_tick <= tick && tick <= now
            }
            _ => false,
        };
        if !valid {
            return Err("required promise state and original clocks disagree".into());
        }
        Ok(Some(record))
    }
    /// Evaluate only before selecting a root. Later resolution does not revoke work.
    pub fn commitment_ready(&self, ledger: &CommitmentLedger, now: u64) -> Result<bool, String> {
        if self.config.required_commitment.is_none() {
            return Ok(true);
        }
        Ok(self
            .required_record(ledger, now)?
            .is_some_and(|record| record.state == CommitmentState::Open))
    }
    /// The script pass follows collection. A newly accepted promise may await
    /// the next pass; an earlier collection opportunity cannot lose its choice.
    pub fn dormant_commitment_allowed(
        &self,
        ledger: &CommitmentLedger,
        now: u64,
    ) -> Result<bool, String> {
        if self.config.required_commitment.is_none() {
            return Ok(false);
        }
        Ok(self.required_record(ledger, now)?.is_none_or(|record| {
            record.made_at_tick == now || record.resolved_at_tick == Some(record.made_at_tick)
        }))
    }
    pub fn validate_required_commitment(
        &self,
        choice: &PlanChoice,
        ledger: &CommitmentLedger,
        now: u64,
    ) -> Result<(), String> {
        if choice.required_commitment != self.config.required_commitment {
            return Err("saved root promise binding differs from source".into());
        }
        if self.config.required_commitment.is_none() {
            return Ok(());
        }
        let record = self
            .required_record(ledger, now)?
            .ok_or("accepted root lost its required promise")?;
        if record.made_at_tick > choice.chosen_tick
            || record
                .resolved_at_tick
                .is_some_and(|tick| tick < choice.chosen_tick)
            || choice.chosen_tick > now
        {
            return Err("accepted root was not chosen during its promise's open interval".into());
        }
        Ok(())
    }
}
