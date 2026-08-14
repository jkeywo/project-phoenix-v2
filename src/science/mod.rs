//! Science: the **scan of an external structure** (issue #1032, parent #851
//! "Falling Skyway").
//!
//! The slice where Phoenix honours its own stated principle. A scan is an
//! admitted command aimed at a piece of world furniture, and what comes back is
//! **derived from that structure's live condition track** — #1025's number, its
//! operational flags and its capacity levels — rather than from a block of
//! authored scan prose. Move the structure's condition and the readout moves
//! with it, with nobody editing content.
//!
//! [`scan`] is the pure arithmetic and the whole of the rule: the input port
//! that has nowhere for authored result text to arrive, the fidelity ladder,
//! and the refusal vocabulary. [`server`] is the thin adapter that reads the
//! admitted command off the `sensors` system, gathers this tick's real range,
//! power and weather, stores what the pure half hands back, and publishes it on
//! the ship's `scan` blackboard channel for `gui/destroyer/captain.html`.

/// The pure, Bevy-free derivation: the authored `[scan]` vocabulary, the
/// fidelity ladder, the refusal reasons, and the rule that there is no authored
/// scan text.
pub mod scan;
/// The Bevy adapter: the per-ship record, the admitted `ScanTarget` command,
/// the fixed-tick derivation, and the blackboard publisher.
pub mod server;

pub use scan::{
    derive, quantise, ScanBandConfig, ScanConditions, ScanConfig, ScanReading, ScanRefusal,
    ScanSubject,
};
pub use server::{
    publish_scan_blackboard, scan_blackboard_key, tick_scans, ScanSaveState, SciencePlugin,
    ShipScanRecord, SCAN_BLACKBOARD_KEY,
};
