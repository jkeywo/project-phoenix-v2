//! Canonical Debug Surface identity catalogue (issue #1267, PRD #1249).
//!
//! A Debug Surface has one protocol identity, one stable position, and one wire
//! name.  Those used to be repeated by `DebugFlag`, `DebugToggleKind`, bridge
//! matches, and read-back matches.  The macro invocation below is now the only
//! declaration: it generates the enum, the stable catalogue, and the
//! wire-name conversion used by serde and the WASM bridge.
//!
//! This module is deliberately Bevy-free.  The module-owned resource adapters
//! live in `crate::debug::catalogue`; protocol messages can therefore use the
//! identity without depending on presentation/runtime machinery.

use serde::{Deserialize, Serialize};

/// One row in the canonical Debug Surface catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DebugSurfaceDescriptor {
    pub surface: DebugSurface,
    pub wire_name: &'static str,
}

macro_rules! define_debug_surface_catalogue {
    ($( $(#[$meta:meta])* $variant:ident => $wire_name:literal ),+ $(,)?) => {
        /// One diagnostic presentation surface.
        ///
        /// Pause and simulation-changing cheats are deliberately not members:
        /// callers may toggle this enum generically without gaining a route to
        /// authoritative simulation state.
        #[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
        pub enum DebugSurface {
            $(
                $(#[$meta])*
                #[serde(rename = $wire_name)]
                $variant,
            )+
        }

        /// Every Debug Surface in deterministic report/bridge order.
        pub const DEBUG_SURFACE_CATALOGUE: [DebugSurfaceDescriptor;
            define_debug_surface_catalogue!(@count $($variant),+)] = [
            $(
                DebugSurfaceDescriptor {
                    surface: DebugSurface::$variant,
                    wire_name: $wire_name,
                },
            )+
        ];

        impl DebugSurface {
            /// Every surface in the canonical stable order.
            pub const ALL: [Self; define_debug_surface_catalogue!(@count $($variant),+)] = [
                $(Self::$variant,)+
            ];

            /// The exact serde/WASM name owned by the catalogue.
            pub const fn wire_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $wire_name,)+
                }
            }

            /// Resolve a bridge or wire name through the same catalogue.
            pub fn from_wire_name(name: &str) -> Option<Self> {
                match name {
                    $($wire_name => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
    (@count $($variant:ident),+) => {
        <[()]>::len(&[$(define_debug_surface_catalogue!(@unit $variant)),+])
    };
    (@unit $variant:ident) => { () };
}

define_debug_surface_catalogue! {
    /// Region wireframes.
    Regions => "Regions",
    /// Modifier values and contributions.
    Modifiers => "Modifiers",
    /// Recent damage events.
    Damage => "Damage",
    /// AI entity behaviour.
    Entities => "Entities",
    /// Detailed entity inspection.
    Inspector => "Inspector",
    /// Per-station human/Backfill activity.
    StationActivity => "StationActivity",
    /// AI doctrine choice and candidate pool.
    AiDoctrine => "AiDoctrine",
    /// Scenario flags, objectives, deadlines, and triggers.
    ScenarioState => "ScenarioState",
    /// Console input-to-feedback latency.
    ConsoleLatency => "ConsoleLatency",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_order_identity_and_wire_names_are_one_to_one() {
        assert_eq!(DEBUG_SURFACE_CATALOGUE.len(), DebugSurface::ALL.len());
        for (descriptor, surface) in DEBUG_SURFACE_CATALOGUE.iter().zip(DebugSurface::ALL) {
            assert_eq!(descriptor.surface, surface);
            assert_eq!(descriptor.wire_name, surface.wire_name());
            assert_eq!(
                DebugSurface::from_wire_name(descriptor.wire_name),
                Some(surface)
            );
        }
    }

    #[test]
    fn pause_is_not_a_debug_surface_wire_name() {
        assert_eq!(DebugSurface::from_wire_name("Pause"), None);
    }
}
