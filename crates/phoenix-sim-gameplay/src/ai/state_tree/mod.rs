//! Closed, deterministic per-System AI trees. No world or ECS reads.
mod compile;
mod evaluate;
mod schema;
pub use compile::{compile, compile_with_numbers, Program, TreeError};
pub use evaluate::{
    step, step_with_numbers, validate_runtime, FrameRuntime, StepResult, TraceEvent, TreeRuntime,
};
pub use schema::*;
#[cfg(test)]
mod numeric_tests;
#[cfg(test)]
mod tests;
