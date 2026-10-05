pub mod balance;
pub mod broadcast;
pub mod codec;
/// Shared fixed-tick collision attribution for digests and saved continuations.
pub mod collision_history;
/// The ship's-computer message vocabulary and authoritative state machine
/// (issue #1342, PRD #1337).
pub mod computer_message;
pub mod debug_surface;
pub mod messages;
/// The authored mission-timeline event vocabulary (issue #1338, PRD #1337).
pub mod narrative;
/// The rendezvous service's frame vocabulary, for the native host (issue #1113).
pub mod rendezvous;
/// The structured post-mission report vocabulary and accumulator (issue #1344,
/// PRD #1337).
pub mod report;
/// The continuous-task lifecycle vocabulary — one start, exactly one terminal
/// (issue #1341, PRD #1337).
pub mod task_lifecycle;
/// Optional report telemetry; authoritative collision history has its own owner.
pub mod telemetry;
