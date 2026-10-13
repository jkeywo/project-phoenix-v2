//! Closed, deterministic per-System AI trees. No world or ECS reads.
mod compile;
mod evaluate;
mod schema;
pub use compile::{compile, Program, TreeError};
pub use evaluate::{step, FrameRuntime, StepResult, TraceEvent, TreeRuntime};
pub use schema::*;
#[cfg(test)]
mod tests;
