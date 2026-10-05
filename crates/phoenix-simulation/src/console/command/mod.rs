//! The Command console (issue #1107) — an auxiliary, human-seeking station,
//! normally hosted by Captain through hull data, that directs one AI-controlled
//! proving Station by selecting a stance from that Station's authored catalogue.
//!
//! `server` holds the Bevy plugin: the admitted-command consumer for
//! `SetStationStance`, the alert-level neutral-to-neutral switch, the AI Command
//! operator, the stance→AI-host posture seam, and the console blackboard. The
//! stance MATHS is pure and lives in `crate::ship::command_stance`.

pub mod server;
