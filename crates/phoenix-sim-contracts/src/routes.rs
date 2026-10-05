//! Shared authored route vocabulary.
use serde::{Deserialize, Serialize};

/// Default cruise fraction for a leg that does not author one.
///
/// A TOML-parse fallback, which is the only kind of hardcoded gameplay value
/// AGENTS.md #11 sanctions. Half throttle: ambient traffic that reads as
/// *going somewhere* without outrunning the crew's ability to talk to it.
pub fn default_leg_speed() -> f32 {
    0.5
}

/// What a civilian does when it runs out of legs.
///
/// `Loop` is the default because the vocabulary exists for *ambient traffic* —
/// a depot run, a shuttle circuit — and a mission that fills a sector with
/// haulers which all stop dead at their last anchor is the surprising outcome,
/// not the expected one. A convoy with somewhere final to be authors
/// `terminate`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteCompletion {
    /// Wrap back to the first leg and keep flying.
    #[default]
    Loop,
    /// Stop at the last leg and hold station there.
    Terminate,
}

impl RouteCompletion {
    /// The wire/script label, the same word the `[[route]]` vocabulary uses.
    /// Written by hand rather than derived, so the strings a console compares
    /// against are visible at the point they are promised.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Loop => "loop",
            Self::Terminate => "terminate",
        }
    }
}

/// One `[[route.leg]]` block: an anchor to make for, and how to fly it.
///
/// The anchor is a name in the world's `[anchors]` table — the same table every
/// `Patrol` / `Reach` doctrine directive resolves against. A leg naming an
/// anchor no world in the composition declares blocks activation
/// (`world::validate`), because a route whose leg reads as nothing is a civilian
/// that silently never goes anywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteLeg {
    /// Anchor name this leg makes for.
    pub anchor: String,
    /// Cruise fraction `0.0..=1.0` while flying this leg. Per leg rather than
    /// per route: a hauler slows on the approach to a depot and opens up again
    /// on the outbound run, and that is the whole of what "traffic has a shape"
    /// means at this altitude.
    #[serde(default = "default_leg_speed")]
    pub speed: f32,
    /// Whole seconds to sit still after reaching this leg's anchor before
    /// pressing on. `0` (the default) flies straight through.
    #[serde(default)]
    pub hold_secs: i64,
}

/// One authored `[[route]]` block.
///
/// ```toml
/// [[route]]
/// id = "depot_run"
/// on_complete = "loop"
///
/// [[route.leg]]
/// anchor = "depot_north"
/// speed = 0.4
/// hold_secs = 20
/// ```
///
/// Routes are **world** data, not entity data: an anchor chain belongs to the
/// map it crosses, and two haulers running the same lane should be running the
/// same authored record rather than two copies that can drift. An entity names
/// one by id in its own `[civilian]` table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    /// Stable id, unique within a world. An order, a `[civilian]` table and a
    /// script all name the route by this.
    pub id: String,
    /// Legs in authored order. The field is named `legs` and the TOML key is
    /// `leg`, matching the `[[route.leg]]` array-of-tables spelling.
    #[serde(default, rename = "leg", skip_serializing_if = "Vec::is_empty")]
    pub legs: Vec<RouteLeg>,
    /// What happens after the last leg.
    #[serde(default)]
    pub on_complete: RouteCompletion,
}

impl RouteConfig {
    /// Reject a `[[route]]` block that cannot mean anything.
    ///
    /// Called from `parse_world` so a typo is a load error naming the route,
    /// not a civilian that silently holds station forever. Anchor *resolution*
    /// is a separate, composition-wide pass (`world::validate`), because a
    /// route may legitimately cross anchors a sibling layer declares.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("[[route]] has an empty id; every route needs a stable id \
                        for an entity or an order to name it"
                .to_string());
        }
        if self.legs.is_empty() {
            return Err(format!(
                "route '{}' declares no [[route.leg]] blocks; a route with no legs \
                 is a civilian with nowhere to go",
                self.id
            ));
        }
        for (i, leg) in self.legs.iter().enumerate() {
            if leg.anchor.trim().is_empty() {
                return Err(format!(
                    "route '{}' leg #{i} has an empty anchor name",
                    self.id
                ));
            }
            if !leg.speed.is_finite() || leg.speed <= 0.0 || leg.speed > 1.0 {
                return Err(format!(
                    "route '{}' leg #{i} (anchor '{}') has speed {}; a leg's cruise \
                     fraction must be in (0.0, 1.0]",
                    self.id, leg.anchor, leg.speed
                ));
            }
            if leg.hold_secs < 0 {
                return Err(format!(
                    "route '{}' leg #{i} (anchor '{}') has hold_secs {}; a dwell \
                     cannot be negative",
                    self.id, leg.anchor, leg.hold_secs
                ));
            }
        }
        Ok(())
    }

    /// The leg anchors in authored order — exactly the `anchors` list an
    /// `AiDirective::Patrol` carries, which is how a route is flown.
    pub fn anchor_chain(&self) -> Vec<String> {
        self.legs.iter().map(|l| l.anchor.clone()).collect()
    }

    /// Whether the chain wraps, i.e. `AiDirective::Patrol { loop_path }`.
    pub fn loops(&self) -> bool {
        self.on_complete == RouteCompletion::Loop
    }

    /// The leg at `index`, wrapping for a looping route and saturating at the
    /// last leg for one that terminates.
    pub fn leg(&self, index: usize) -> Option<&RouteLeg> {
        if self.legs.is_empty() {
            return None;
        }
        if index < self.legs.len() {
            return self.legs.get(index);
        }
        if self.loops() {
            self.legs.get(index % self.legs.len())
        } else {
            self.legs.last()
        }
    }
}
