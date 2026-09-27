//! Owner-loss continuation decisions. The transport authenticates the service
//! epoch and retains/merges ordered tails; this machine checks the frozen roster
//! and refuses to release the world until every survivor acknowledges ingestion.
use crate::command_admission::HostSlot;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ContinuationRequest {
    Begin {
        epoch: u64,
        previous_owner: HostSlot,
        next_owner: HostSlot,
        participants: Vec<HostSlot>,
    },
    Replayed {
        epoch: u64,
    },
    Commit {
        epoch: u64,
        previous_owner: HostSlot,
        next_owner: HostSlot,
        loss_tick: u64,
        acked: Vec<HostSlot>,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContinuationPhase {
    #[default]
    Idle,
    Pending,
    Held,
    Replayed,
    Committed,
    Refused,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ContinuationStatus {
    pub generation: u64,
    pub epoch: u64,
    pub status: ContinuationPhase,
    pub reason: Option<String>,
    pub loss_tick: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub epoch: u64,
    pub previous_owner: HostSlot,
    pub next_owner: HostSlot,
    pub participants: Vec<HostSlot>,
    pub replayed: bool,
}

/// Peer-local state: neither captured nor folded into authoritative snapshots.
#[derive(Clone, Debug, Default)]
pub struct Continuation {
    pub status: ContinuationStatus,
    pub transaction: Option<Transaction>,
    pub committed: Option<Transaction>,
    pub committed_loss_tick: Option<u64>,
}
impl Continuation {
    pub fn held(&self) -> bool {
        self.transaction.is_some()
    }
    pub fn refuse(&mut self, reason: &str) {
        self.status.status = ContinuationPhase::Refused;
        self.status.reason = Some(reason.into());
    }
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        epoch: u64,
        previous_owner: HostSlot,
        next_owner: HostSlot,
        participants: Vec<HostSlot>,
        owner: HostSlot,
        local: HostSlot,
        mut live: Vec<HostSlot>,
    ) -> Result<(), &'static str> {
        let mut survivors = participants.clone();
        survivors.sort_unstable();
        live.retain(|slot| *slot != previous_owner);
        live.sort_unstable();
        if epoch == 0
            || previous_owner != owner
            || next_owner == previous_owner
            || local == previous_owner
            || survivors != live
            || survivors.is_empty()
            || survivors.windows(2).any(|pair| pair[0] == pair[1])
            || !survivors.contains(&next_owner)
            || !survivors.contains(&local)
        {
            return Err("invalid-successor-topology");
        }
        let proposed = Transaction {
            epoch,
            previous_owner,
            next_owner,
            participants: survivors,
            replayed: false,
        };
        if let Some(active) = &self.transaction {
            if active.epoch == epoch
                && active.previous_owner == previous_owner
                && active.next_owner == next_owner
                && active.participants == proposed.participants
            {
                return Ok(());
            }
            return Err("continuation-already-active");
        }
        if epoch <= self.committed.as_ref().map_or(0, |tx| tx.epoch) {
            return Err("stale-continuation-epoch");
        }
        self.transaction = Some(proposed);
        self.status.epoch = epoch;
        self.status.status = ContinuationPhase::Held;
        self.status.reason = None;
        self.status.loss_tick = None;
        Ok(())
    }
    pub fn replayed(&mut self, epoch: u64) -> Result<(), &'static str> {
        if self.status.status == ContinuationPhase::Refused {
            return Err("continuation-refused");
        }
        let tx = self
            .transaction
            .as_mut()
            .filter(|tx| tx.epoch == epoch)
            .ok_or("no-matching-continuation")?;
        tx.replayed = true;
        self.status.status = ContinuationPhase::Replayed;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn validate_commit(
        &self,
        epoch: u64,
        previous_owner: HostSlot,
        next_owner: HostSlot,
        loss_tick: u64,
        acked: &[HostSlot],
        watermark: Option<u64>,
        now: u64,
    ) -> Result<(), &'static str> {
        let tx = self
            .transaction
            .as_ref()
            .ok_or("no-matching-continuation")?;
        if self.status.status == ContinuationPhase::Refused || !tx.replayed {
            return Err("tails-not-replayed");
        }
        if tx.epoch != epoch || tx.previous_owner != previous_owner || tx.next_owner != next_owner {
            return Err("continuation-identity-mismatch");
        }
        let mut acked = acked.to_vec();
        acked.sort_unstable();
        if acked != tx.participants {
            return Err("survivor-acknowledgements-incomplete");
        }
        if watermark.and_then(|tick| tick.checked_add(1)) != Some(loss_tick) || now > loss_tick {
            return Err("unsafe-loss-frontier");
        }
        Ok(())
    }
    pub fn commit(&mut self, loss_tick: u64) {
        self.committed = self.transaction.take();
        self.committed_loss_tick = Some(loss_tick);
        self.status.status = ContinuationPhase::Committed;
        self.status.reason = None;
    }
}
