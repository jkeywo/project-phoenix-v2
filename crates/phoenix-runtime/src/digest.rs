//! Canonical tick-indexed digest history and comparison.
/// Hash an application-owned canonical byte representation.
pub use vellum_digest::fnv1a as hash_bytes;
/// One sampled digest and the tick it was taken on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub tick: u64,
    pub digest: u64,
}

/// Where two runs of the same log first stopped agreeing.
///
/// The `window` is the point of the whole mechanism: a bare end-state mismatch
/// says only "these two runs differ", which localises a bug to the entire run.
/// A checkpoint pair says "they agreed at tick `after`, and disagreed at tick
/// `tick`", which is a window to read a log over.
///
/// `at_end` is what keeps those two cases from being told the same story. A
/// sampled-tick mismatch (`at_end: false`) is "the state at tick `tick` already
/// disagreed" — `tick` is somewhere a divergence actually happened. Every
/// sampled checkpoint agreeing and the two runs still finishing on different
/// digests (`at_end: true`) is a DIFFERENT claim: nothing this ledger sampled
/// ever disagreed, and `tick` here is the *last agreed* checkpoint, not a tick
/// that itself diverged — the two runs parted ways somewhere in the unsampled
/// tail after it. Reporting both shapes through the same "digests first
/// disagree at tick N" sentence would say a specific tick disagreed when in
/// the second case none sampled ever did — self-contradictory, since `after`
/// and `tick` would then name the same checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The first tick whose sampled digest disagreed. Meaningless as "the tick
    /// that disagreed" when `at_end` is true — see the field's own doc.
    pub tick: u64,
    /// The last tick both runs agreed on, if there was one. `None` means they
    /// disagreed at the very first sample.
    pub after: Option<u64>,
    /// True when every sampled checkpoint agreed and only the final digests
    /// differ — the run diverged somewhere after the last checkpoint, in the
    /// tail no sample covers. False for an ordinary sampled-tick mismatch.
    pub at_end: bool,
    pub recorded: u64,
    pub replayed: u64,
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.at_end {
            return match self.after {
                Some(after) => write!(
                    f,
                    "every sampled tick agreed through {}; the final states differ: recorded {:#018x}, replayed {:#018x}",
                    after, self.recorded, self.replayed
                ),
                None => write!(
                    f,
                    "no ticks were sampled, and the final states differ: recorded {:#018x}, replayed {:#018x}",
                    self.recorded, self.replayed
                ),
            };
        }
        match self.after {
            Some(after) => write!(
                f,
                "digests first disagree at tick {} (last agreement tick {}): recorded {:#018x}, replayed {:#018x}",
                self.tick, after, self.recorded, self.replayed
            ),
            None => write!(
                f,
                "digests disagree from the first sample, tick {}: recorded {:#018x}, replayed {:#018x}",
                self.tick, self.recorded, self.replayed
            ),
        }
    }
}

/// The periodic digest samples a run took, plus the digest it ended on.
///
/// `interval` of `0` means sampling was off, in which case `checkpoints` is
/// empty and the ledger carries the final digest alone — the "0 disables it and
/// costs nothing" half of the design. Nothing computes a digest on a run that
/// did not ask for one.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DigestLedger {
    /// Sample every N logical ticks. `0` is off.
    pub interval: u64,
    pub checkpoints: Vec<Checkpoint>,
    /// The digest at the end of the run. Always recorded — a run that samples
    /// nothing still says where it finished.
    pub final_digest: u64,
    /// How many of the commands this run *submitted* across the production
    /// admission boundary never made it into the `CommandLog` — i.e. the
    /// authority gate refused them (issue #901 review). `PhoenixSim` computes
    /// this as submitted-minus-admitted at the game's replay seal time: cheap (no per-command
    /// bookkeeping beyond a counter) and honest (it reads the same `CommandLog`
    /// a recording run writes down, rather than re-deriving authorization
    /// itself). A refusal used to be silent — a command that stopped being
    /// admitted between record and replay left no trace anywhere but a
    /// possibly-unnoticed `pwarn!` line. Comparing this field between a
    /// recorded and a replayed ledger names that a command no longer admits
    /// instead of leaving it to be inferred from a digest mismatch.
    pub refused: u64,
}

impl DigestLedger {
    pub fn new(interval: u64) -> Self {
        Self {
            interval,
            checkpoints: Vec::new(),
            final_digest: 0,
            refused: 0,
        }
    }

    /// Whether `tick` is a sampling tick. `interval == 0` is never.
    pub fn samples(&self, tick: u64) -> bool {
        self.interval != 0 && tick.is_multiple_of(self.interval)
    }

    /// Record a sample, unless one for `tick` is already the most recent.
    ///
    /// The guard matters because a frame can run zero or several fixed steps:
    /// the same `SimTick` can be observed at the top of two consecutive frames
    /// (the first frame establishes the time baseline and steps nothing), and a
    /// duplicate entry would shift every later index and make two identical
    /// runs' ledgers compare unequal.
    pub fn record(&mut self, tick: u64, digest: u64) {
        if self.checkpoints.last().is_some_and(|c| c.tick == tick) {
            return;
        }
        self.checkpoints.push(Checkpoint { tick, digest });
    }

    /// The digest this ledger sampled at `tick`, if it sampled one.
    ///
    /// Added by issue #1116 so a host can answer a peer's digest frame the
    /// moment it arrives — "did I fold the same thing at tick 300?" — rather
    /// than waiting until it has a whole ledger to compare.
    /// [`Self::first_divergence`] remains the comparator for two complete runs;
    /// this is the same question asked one sample at a time, off the same
    /// checkpoints, so the two can never disagree about what was sampled.
    pub fn digest_at(&self, tick: u64) -> Option<u64> {
        self.checkpoints
            .iter()
            .find(|c| c.tick == tick)
            .map(|c| c.digest)
    }

    /// Drop every sampled checkpoint at or before `tick`, keeping only later ones.
    ///
    /// Divergence recovery (#1118) calls this on every host's ledger once a
    /// recovery resolves: the samples at and before the recovery boundary are the
    /// divergent history the restore has just healed, so forgetting them keeps the
    /// same stale disagreement from re-triggering recovery while the post-boundary
    /// samples (which now agree) are retained.
    pub fn forget_through(&mut self, tick: u64) {
        self.checkpoints.retain(|c| c.tick > tick);
    }

    /// The first tick at which this ledger and `other` disagree.
    ///
    /// Pairs samples by *tick*, not by index, so two runs that sampled
    /// different tick sets still compare on the ticks they share. A tick only
    /// one of them sampled is not evidence of anything and is skipped. When
    /// every shared sample agrees, the final digests are compared and reported
    /// against the last agreed tick — so "they matched all the way through and
    /// then ended differently" is still a located answer rather than silence.
    pub fn first_divergence(&self, other: &Self) -> Option<Divergence> {
        let mut last_agreed = None;
        let mut theirs = other.checkpoints.iter().peekable();
        for mine in &self.checkpoints {
            // Skip any of theirs that this run never sampled.
            while theirs.peek().is_some_and(|c| c.tick < mine.tick) {
                theirs.next();
            }
            let Some(match_) = theirs.peek().filter(|c| c.tick == mine.tick) else {
                continue;
            };
            if match_.digest != mine.digest {
                return Some(Divergence {
                    tick: mine.tick,
                    after: last_agreed,
                    at_end: false,
                    recorded: mine.digest,
                    replayed: match_.digest,
                });
            }
            last_agreed = Some(mine.tick);
            theirs.next();
        }

        if self.final_digest != other.final_digest {
            return Some(Divergence {
                tick: self
                    .checkpoints
                    .last()
                    .map_or(0, |c| c.tick)
                    .max(other.checkpoints.last().map_or(0, |c| c.tick)),
                after: last_agreed,
                at_end: true,
                recorded: self.final_digest,
                replayed: other.final_digest,
            });
        }
        None
    }
}
