//! Stable game identities and operator coordination vocabulary.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable, designer-authored identifier for a claimable ship station.
///
/// Station ids are ship-local authoring keys, not player tokens and not world
/// entity UUIDs. They are intended to replace console bundles as the wire
/// addressing unit for station ownership in the station/system architecture.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct StationId(pub String);

/// Maximum UTF-8 byte length of one opaque semantic-action correlation.
///
/// This is a protocol/resource bound, not a gameplay value.  The client mints
/// UUID-shaped ids today, but every receiver treats the contents as opaque.
pub const MAX_ACTION_CORRELATION_BYTES: usize = 64;

/// Opaque identity connecting one semantic-action press to its targeted host
/// acknowledgement (issue #1276).
///
/// The value is deliberately absent from [`SystemControlPayload`], command
/// logs, mesh frames, snapshots and replay.  It is transient reply-routing
/// metadata, validated at the wire boundary and bounded before it can key any
/// client/host map.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ActionCorrelationId(String);

impl ActionCorrelationId {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.is_empty() {
            return Err("action correlation must not be empty");
        }
        if value.len() > MAX_ACTION_CORRELATION_BYTES {
            return Err("action correlation is too long");
        }
        if !value.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
            return Err("action correlation must contain visible ASCII only");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ActionCorrelationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Terminal authoritative outcomes carried by [`ServerMessage::ActionFeedback`].
/// `TimedOut` is client-local: by definition no host response produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionFeedbackOutcome {
    Applied,
    Refused,
}

/// Explicit destination for a delayed Coordination message.
///
/// Coordination is addressed either to one authored crew Station or to the
/// whole source ship. A System id is deliberately not accepted here: Systems
/// remain command-authority targets, while Coordination is an operator/bridge
/// message whose recipient is resolved at delivery time.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum CoordinationAddress {
    Station(StationId),
    Ship,
}

/// One deterministic interpolation value in a Coordination presentation.
///
/// The variants are intentionally scalar and untagged on the wire: the client
/// string-table resolver already accepts string and number parameters. Keeping
/// the distinction here means a shield frequency remains a JSON number rather
/// than becoming pre-formatted prose, while a label may remain either a String
/// Table id or literal authored text. `BTreeMap` owns parameter ordering in
/// [`CoordinationPresentation`], so equal emissions encode identically.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum CoordinationParam {
    Text(String),
    Integer(i64),
    Decimal(f32),
}

impl CoordinationParam {
    /// A localised-or-literal text parameter.
    ///
    /// Literal calls are deliberately easy for `scripts/check-strings.mjs` to
    /// discover: dotted ids supplied here must name a String Table row.
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }
}

impl From<String> for CoordinationParam {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for CoordinationParam {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<i64> for CoordinationParam {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

impl From<u8> for CoordinationParam {
    fn from(value: u8) -> Self {
        Self::Integer(i64::from(value))
    }
}

impl From<f32> for CoordinationParam {
    fn from(value: f32) -> Self {
        Self::Decimal(value)
    }
}

/// Producer-owned words for one Coordination emission.
///
/// `title` and `body` each carry either a known String Table id or literal
/// authored text. Their sibling parameter maps use the repository-wide
/// `<field>_params` convention, so `localiseTree` resolves ids and interpolates
/// values at the phone and Viewscreen ingress boundaries without either
/// presenter inspecting [`CoordinationPayload`]. Empty maps are omitted from
/// the wire but the title/body fields are required: every producer must make
/// the presentation decision at the same boundary as the semantic payload.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CoordinationPresentation {
    pub title: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub title_params: BTreeMap<String, CoordinationParam>,
    pub body: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub body_params: BTreeMap<String, CoordinationParam>,
}

impl CoordinationPresentation {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            title_params: BTreeMap::new(),
            body: body.into(),
            body_params: BTreeMap::new(),
        }
    }

    pub fn titled(title: impl Into<String>) -> Self {
        Self::new(title, "")
    }

    pub fn with_title_param(
        mut self,
        key: impl Into<String>,
        value: impl Into<CoordinationParam>,
    ) -> Self {
        self.title_params.insert(key.into(), value.into());
        self
    }

    pub fn with_body_param(
        mut self,
        key: impl Into<String>,
        value: impl Into<CoordinationParam>,
    ) -> Self {
        self.body_params.insert(key.into(), value.into());
        self
    }
}

/// Stable, designer-authored identifier for one capability instance on a ship.
///
/// System ids are ship-wide unique authoring keys such as `phaser-fore` or
/// `torpedo-tube-aft`. They are distinct from world entity UUIDs.
/// `Ord` so system ids can key a `BTreeMap`. The per-ship blackboard map is one,
/// because its iteration order reaches the wire: a `HashMap` ordered those
/// updates by `RandomState`'s per-process seed, so no two runs of the same
/// seeded binary emitted the same byte stream.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SystemId(pub String);

/// Presentation taxonomy for the console surface that renders a System's
/// state. This is deliberately separate from [`StationId`] (ownership) and
/// [`SystemId`] (command authority): a Dock System belongs to whichever Station
/// the hull authors, but its state is presented by the Helm console family.
///
/// Every authored System-kind descriptor projects one of these values to the
/// client. Reserved/aggregate blackboard keys use the same vocabulary through
/// a separate map so presentation metadata never turns a channel into a System.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConsoleFamily {
    Captain,
    Helm,
    Tactical,
    Sensors,
    Navigation,
    Comms,
    Shields,
    Power,
    Repair,
    Command,
    Tractor,
    Umbilical,
    /// Security teams (issue #1346). A family of its own rather than reusing
    /// `Tactical`, for the reason this enum exists: the Alliance Destroyer's
    /// Tactical STATION owns the Security System, but its readout is nothing
    /// like a weapons view — a team list, their assignments and progress, and
    /// the targets they can be sent to — and another hull may hang the same
    /// system off Command or Engineering without any of that changing. The
    /// tractor and the umbilical are the same shape: engineering-owned, drawn by
    /// their own family.
    Security,
    /// The rescue transporter (issue #1348). Its own family, the way the tractor
    /// and umbilical have theirs: engineering-owned, drawn by a bespoke readout
    /// (a selected contact, its life signs, the recovery progress) that is
    /// nothing like any other console.
    Transporter,
}

impl ConsoleFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Captain => "captain",
            Self::Helm => "helm",
            Self::Tactical => "tactical",
            Self::Sensors => "sensors",
            Self::Navigation => "navigation",
            Self::Comms => "comms",
            Self::Shields => "shields",
            Self::Power => "power",
            Self::Repair => "repair",
            Self::Command => "command",
            Self::Tractor => "tractor",
            Self::Umbilical => "umbilical",
            Self::Security => "security",
            Self::Transporter => "transporter",
        }
    }
}

/// Stable, designer-authored identifier for an operator-facing power group.
/// `Ord` so it can sit inside a `ModifierSource` that keys a `BTreeMap`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PowerGroupId(pub String);
