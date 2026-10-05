//! Authored geometry and appearance definitions.
use serde::{Deserialize, Serialize};
pub mod celestial;
pub mod hull;
pub mod visual;
pub use celestial::*;
pub use hull::*;
pub use visual::*;
