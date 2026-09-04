//! Debris hazards (issue #1347) — a mass on a course, and what a crew do about
//! it.
//!
//! The Falling Skyway corridor comes apart above a working depot ladder, and
//! what falls out of it is the first thing in this game that is dangerous
//! without being hostile: no faction, no doctrine, no lock on anybody, just a
//! rock going somewhere. That makes it a different problem from a warship, and
//! the seat that solves it first is Sensors rather than Tactical.
//!
//! Split the way `tractor` is (rule 10):
//!
//! * [`threat`] — the pure, Bevy-free half: the authored `[debris]` table, the
//!   closest-approach projection and the assessment it returns. Unit-tested in
//!   isolation, and the module where the "no authored threat text" rule lives.
//! * [`server`] — the Bevy adapter: the per-contact [`server::DebrisThreat`]
//!   component, the fixed-tick drift, and the four world flags a scenario hangs
//!   its beat on.
//!
//! # The chain, end to end
//!
//! 1. A scenario spawns a contact carrying `[debris]` — a drift, the asset it is
//!    aimed at, an impact radius and four flag names.
//! 2. Somebody **looks**: an ordinary `ScanTarget` on the `sensors` system, taken
//!    by `science::server::tick_scans` through the ordinary scan lifecycle
//!    (issue #1341), which folds [`threat::assess`] into the reading it returns.
//! 3. [`server::tick_debris_state`] latches that assessment onto the contact and
//!    raises the authored *read* / *confirmed* / *urgent* flags.
//! 4. The **scenario** reacts — posts the interception objective, shows the
//!    ship's-computer cue, and owns every consequence of the strike.
//! 5. Tactical acts: a human on the lock, or Backfill through the
//!    `confirmed-debris-threat` candidate source, which ranks confirmed contacts
//!    by the crew's own dead-reckoned deadline.
//!
//! Nothing short-circuits step 2. A rock the simulation privately knows is on
//! course is still an unknown contact until a crew read it, which is what makes
//! the Sensors seat load-bearing rather than decorative.

/// The pure, Bevy-free authored table, projection and assessment.
pub mod threat;

/// The Bevy adapter: the component, the drift tick and the flag raiser.
pub mod server;

pub use server::{
    tick_debris_drift, tick_debris_state, DebrisAssessed, DebrisPlugin, DebrisThreat,
};
pub use threat::{assess, DebrisAssessment, DebrisConfig, DebrisSaveState, DebrisSubject};
