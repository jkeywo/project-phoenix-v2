use serde::{Deserialize, Serialize};
/// The complete authored band vocabulary, in queue order.
///
/// Deliberately closed: an author names one of these three or the world fails
/// to load. There is no numerical score to reverse-engineer and no fourth band
/// a mod pack can invent.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmAttentionBand {
    Urgent,
    Attention,
    Background,
}

impl GmAttentionBand {
    /// The authored spelling, which is also the wire spelling.
    pub fn as_authored(self) -> &'static str {
        match self {
            Self::Urgent => "urgent",
            Self::Attention => "attention",
            Self::Background => "background",
        }
    }

    /// Parse one authored override. Exact, lower-case, no aliases: a world that
    /// says `Urgent` or `critical` is a world whose author believed something
    /// this build does not do, and guessing on their behalf is how a scenario
    /// ships with a priority nobody chose.
    pub fn from_authored(value: &str) -> Option<Self> {
        match value {
            "urgent" => Some(Self::Urgent),
            "attention" => Some(Self::Attention),
            "background" => Some(Self::Background),
            _ => None,
        }
    }

    /// Every band an author may write, for an error message that tells them
    /// what to write instead.
    pub fn authored_vocabulary() -> String {
        [Self::Urgent, Self::Attention, Self::Background]
            .iter()
            .map(|band| format!("'{}'", band.as_authored()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Why a row is in the queue.
///
/// `StationHealth` (issue #1437) is the one category whose band is system-fixed
/// at [`GmAttentionBand::Urgent`] and whose rows no authored table can invent,
/// re-band or suppress. It is deliberately still a category, and therefore
/// still filterable *in the list*: the guarantee that a Game Master cannot hide
/// a connection failure from themselves is held by the unfilterable banner
/// region beside the list (`#gm-attention-banners`), not by making one row type
/// un-narrowable in a triage tool.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GmAttentionCategory {
    PendingComms,
    /// An authored beat a Game Master could release right now (issue #1434).
    EligibleBeat,
    /// An NPC ship that has been given nothing to do (issue #1435).
    IdleNpc,
    /// A Station or fleet hull whose human is gone (issue #1437). Always
    /// Urgent, always system-defined.
    StationHealth,
    /// The crew has done nothing meaningful for the authored interval (issue
    /// #1436). Always `Background`, never escalated by age — the whole rule,
    /// and the activity adapter behind it, live in [`crate::gm_quiet`].
    QuietTime,
}

impl GmAttentionCategory {
    /// The authored spelling, which is also the wire spelling and the id a
    /// browser filter carries.
    pub fn as_authored(self) -> &'static str {
        match self {
            Self::PendingComms => "pending_comms",
            Self::EligibleBeat => "eligible_beat",
            Self::IdleNpc => "idle_npc",
            Self::StationHealth => "station_health",
            Self::QuietTime => "quiet_time",
        }
    }

    /// Every category, in queue-vocabulary order. The one list a validator
    /// checks an authored reference against, so a new producer cannot be added
    /// without the authoring vocabulary growing with it.
    pub fn all() -> [Self; 5] {
        [
            Self::PendingComms,
            Self::EligibleBeat,
            Self::IdleNpc,
            Self::StationHealth,
            Self::QuietTime,
        ]
    }

    /// Parse one authored reference. Exact and lower-case, for
    /// [`GmAttentionBand::from_authored`]'s reason: a world naming a category
    /// this build does not produce is a world whose author believed something
    /// that is not true, and quietly widening their filter to everything is
    /// how a preset ships showing rows nobody chose.
    pub fn from_authored(value: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|category| category.as_authored() == value)
    }

    /// Every category an author may write, for an error message that tells
    /// them what to write instead.
    pub fn authored_vocabulary() -> String {
        Self::all()
            .iter()
            .map(|category| format!("'{}'", category.as_authored()))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The grace an NPC ship must spend with nothing to do before the queue
/// mentions it, in SIMULATION seconds.
pub const DEFAULT_IDLE_NPC_GRACE_SECS: f32 = 30.0;

/// The authored `[gm_attention]` table.
///
/// Every knob here is scoped to the GM's own advisory queue: none of it changes
/// what a ship flies, what a crew console renders, or what the digest folds. It
/// is a separate table from `[[gm_comms_route]].attention_band` because that
/// override belongs to one route, while these belong to the world.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GmAttentionSettings {
    /// How long an NPC ship must be idle, in simulation seconds, before it
    /// reaches the queue. Positive and finite; validated at world load.
    #[serde(default = "default_idle_npc_grace_secs")]
    pub idle_npc_grace_secs: f32,
    /// The band an idle-NPC row lands in, from the same closed three-word
    /// vocabulary [`GmAttentionBand`] parses. `None` keeps the system default,
    /// [`GmAttentionBand::Background`] — an idle hull is a thing to notice, not
    /// a thing to drop a conversation for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_npc_band: Option<String>,
    /// Silence THIS advisory and nothing else. A scenario whose NPCs are meant
    /// to be parked — a dockyard, a field of derelicts — does not want thirty
    /// rows saying so, and it must not have to give up the pending-Comms queue
    /// to say that.
    #[serde(default)]
    pub idle_npc_disabled: bool,
    /// How long the whole session must go without meaningful crew activity, in
    /// simulation seconds, before the quiet-time advisory appears (issue
    /// #1436). Positive and finite; validated at world load.
    ///
    /// It takes no band: a quiet row is always
    /// [`GmAttentionBand::Background`], which is how "age alone never escalates
    /// it" is true by construction rather than by a rule somebody could relax.
    #[serde(default = "default_quiet_time_secs")]
    pub quiet_time_secs: f32,
    /// Silence THAT advisory and nothing else, whatever the interval says. A
    /// scenario built around a long approach does not want to be told the crew
    /// are quiet, and must not have to change the interval — or give up any
    /// other row — to say so.
    #[serde(default)]
    pub quiet_time_disabled: bool,
    /// Distinct outstanding human demands at which a Station becomes a
    /// candidate for Overloaded (issue #1438). Positive; validated at world
    /// load. Zero is not "off" — see [`Self::workload_disabled`].
    #[serde(default = "default_workload_overload_count")]
    pub workload_overload_count: u32,
    /// How long that count must hold, in SIMULATION seconds, before Overloaded
    /// is the answer. Positive and finite; validated at world load. There is no
    /// zero sentinel: a zero-second duration is a threshold with no duration at
    /// all, which is a different rule wearing this one's name.
    #[serde(default = "default_workload_overload_secs")]
    pub workload_overload_secs: f32,
    /// Silence the Station-workload advisory and nothing else. A scenario that
    /// does not want workload advice must not have to give up the pending-Comms
    /// queue or the technical banners to say so.
    #[serde(default)]
    pub workload_disabled: bool,
}

fn default_idle_npc_grace_secs() -> f32 {
    DEFAULT_IDLE_NPC_GRACE_SECS
}

fn default_quiet_time_secs() -> f32 {
    crate::gm_quiet::DEFAULT_QUIET_SECONDS
}

fn default_workload_overload_count() -> u32 {
    crate::gm_workload::DEFAULT_OVERLOAD_COUNT
}

fn default_workload_overload_secs() -> f32 {
    crate::gm_workload::DEFAULT_OVERLOAD_SECS
}

impl Default for GmAttentionSettings {
    fn default() -> Self {
        Self {
            idle_npc_grace_secs: DEFAULT_IDLE_NPC_GRACE_SECS,
            quiet_time_secs: crate::gm_quiet::DEFAULT_QUIET_SECONDS,
            quiet_time_disabled: false,
            idle_npc_band: None,
            idle_npc_disabled: false,
            workload_overload_count: crate::gm_workload::DEFAULT_OVERLOAD_COUNT,
            workload_overload_secs: crate::gm_workload::DEFAULT_OVERLOAD_SECS,
            workload_disabled: false,
        }
    }
}

impl GmAttentionSettings {
    /// Refuse an unusable authored value at world load, naming the section and
    /// the key so the author is told what to write instead.
    ///
    /// A non-positive or non-finite grace is not a slow advisory, it is an
    /// advisory with no boundary at all: zero fires on the first idle step,
    /// negative fires before the ship has done anything, and `nan` never fires
    /// while looking exactly like a setting that should. Guessing on the
    /// author's behalf is how a scenario ships with a threshold nobody chose —
    /// the same argument the band vocabulary is closed for.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.idle_npc_grace_secs.is_finite() && self.idle_npc_grace_secs > 0.0) {
            return Err(format!(
                "[gm_attention] idle_npc_grace_secs = {} must be a positive, finite number of \
                 simulation seconds",
                self.idle_npc_grace_secs
            ));
        }
        if let Some(band) = &self.idle_npc_band {
            if GmAttentionBand::from_authored(band).is_none() {
                return Err(format!(
                    "[gm_attention] declares idle_npc_band '{band}'; the GM attention bands are {}",
                    GmAttentionBand::authored_vocabulary()
                ));
            }
        }
        // Same argument for the quiet-time interval (issue #1436), and one more
        // besides: `quiet_time_disabled` is the way to switch that advisory
        // off, and it is deliberately a SEPARATE field, so a zero interval is a
        // mistake rather than a second spelling of the disable.
        if !(self.quiet_time_secs.is_finite() && self.quiet_time_secs > 0.0) {
            return Err(format!(
                "[gm_attention] quiet_time_secs = {} must be a positive, finite number of                  simulation seconds; to switch the quiet-time advisory off write                  quiet_time_disabled = true",
                self.quiet_time_secs
            ));
        }
        // The workload thresholds (issue #1438), refused on the same argument.
        // A zero count would report a Station with nothing to do as a candidate
        // for Overloaded; a zero, negative or `nan` duration would make the
        // "continuously for" half of the rule vanish while still looking like a
        // setting somebody chose.
        if self.workload_overload_count == 0 {
            return Err(
                "[gm_attention] workload_overload_count = 0 must be a positive number of \
                 outstanding human demands; use workload_disabled = true to silence the advisory"
                    .to_string(),
            );
        }
        if !(self.workload_overload_secs.is_finite() && self.workload_overload_secs > 0.0) {
            return Err(format!(
                "[gm_attention] workload_overload_secs = {} must be a positive, finite number of \
                 simulation seconds; use workload_disabled = true to silence the advisory",
                self.workload_overload_secs
            ));
        }
        Ok(())
    }

    /// The count threshold in force, floored at one so a value that somehow
    /// reached the runtime without passing [`Self::validate`] still describes a
    /// Station that has at least one thing to do.
    pub fn workload_overload_count(&self) -> u32 {
        self.workload_overload_count.max(1)
    }

    /// The authored overload duration as an exact whole number of simulation
    /// ticks at `hz`.
    ///
    /// Rounded to nearest and floored at one, for
    /// [`Self::idle_grace_ticks`]'s reason: a duration shorter than a tick is
    /// one the fixed loop cannot express, and answering zero would make
    /// "continuously for" mean "on the step it happened".
    pub fn workload_overload_ticks(&self, hz: f32) -> u64 {
        let hz = f64::from(hz);
        let secs = f64::from(self.workload_overload_secs);
        if !(hz.is_finite() && hz > 0.0 && secs.is_finite() && secs > 0.0) {
            return u64::MAX;
        }
        let ticks = (hz * secs).round();
        if !ticks.is_finite() || ticks >= u64::MAX as f64 {
            return u64::MAX;
        }
        (ticks as u64).max(1)
    }

    /// The band an idle-NPC row lands in.
    pub fn idle_npc_band(&self) -> GmAttentionBand {
        self.idle_npc_band
            .as_deref()
            .and_then(GmAttentionBand::from_authored)
            .unwrap_or(GmAttentionBand::Background)
    }

    /// The authored grace as an exact whole number of simulation ticks at `hz`.
    ///
    /// Floored at one tick by [`ticks_at`], so a grace shorter than a step
    /// cannot put every NPC in the queue on the step it stopped working.
    pub fn idle_grace_ticks(&self, hz: f32) -> u64 {
        ticks_at(self.idle_npc_grace_secs, hz)
    }

    /// The authored quiet-time interval as an exact whole number of simulation
    /// ticks at `hz`, by the same rule and the same arithmetic (issue #1436).
    pub fn quiet_time_ticks(&self, hz: f32) -> u64 {
        ticks_at(self.quiet_time_secs, hz)
    }
}

/// Authored seconds as an exact whole number of simulation ticks at `hz`.
///
/// Rounded to the nearest tick and floored at one: an interval shorter than a
/// tick is one the fixed loop cannot express, and answering "zero ticks" would
/// fire the advisory on the step its condition began. The rounding is
/// IEEE-deterministic, so every peer turns the same authored seconds into the
/// same tick count. An unusable pair answers `u64::MAX` — unreachable rather
/// than immediate — which the load-time validation above makes unreachable in
/// its own right.
fn ticks_at(secs: f32, hz: f32) -> u64 {
    let hz = f64::from(hz);
    let secs = f64::from(secs);
    if !(hz.is_finite() && hz > 0.0 && secs.is_finite() && secs > 0.0) {
        return u64::MAX;
    }
    let ticks = (hz * secs).round();
    if !ticks.is_finite() || ticks >= u64::MAX as f64 {
        return u64::MAX;
    }
    (ticks as u64).max(1)
}
