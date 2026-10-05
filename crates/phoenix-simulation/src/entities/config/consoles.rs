//! Entity schema: consoles. Public paths remain in the parent module.
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringConsoleConfig {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CaptainConsoleConfig {
    /// Inline stateless AI policy for the Captain's Red Alert fine system
    /// (`[captain_console.ai]`, issue #775). When present it is validated at
    /// content load and drives `operate_captain_ai`; when absent the canonical
    /// [`default_captain_ai_config`] policy is synthesised at spawn.
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}

/// Config block for the Comms CONSOLE's AI (issue #786), loaded from
/// `[comms_console]`.
///
/// Deliberately separate from the top-level `[comms]` section: that one is the
/// per-ENTITY comms RANGE (`CommsConfig`), present on stations and NPCs that are
/// merely hailable, and has nothing to do with who operates the console. The AI
/// policy belongs to the console, next to `[captain_console.ai]` and
/// `[sensors_console.selector]`.
///
/// Comms is the FIRST system to author BOTH fine-system AI machines: a #776
/// `selector` (WHO to hail — a variable candidate set keyed by real contact
/// UUIDs) and a #775 channel/verb `ai` policy (HOW to answer an open dialogue —
/// a fixed, index-addressed response list). See [`COMMS_RESPOND_CHANNEL`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CommsConsoleConfig {
    /// Inline per-system target selector for hail target ranking (issue #786).
    /// Loaded from `[comms_console.selector]`; absent ⇒ the canonical
    /// [`default_comms_target_selector_config`] is synthesised at spawn.
    /// Validated in [`EntityConfig::from_toml`] against
    /// [`COMMS_SELECTOR_SOURCES`].
    #[serde(default)]
    pub selector: Option<FineSystemAiSelectorToml>,
    /// Inline stateless AI policy for the Comms dialogue-response fine system
    /// (issue #786). Loaded from `[comms_console.ai]`; absent ⇒ the canonical
    /// [`default_comms_response_ai_config`] is synthesised at spawn (baseline
    /// preservation). Validated against [`COMMS_RESPOND_CHANNELS`] /
    /// [`COMMS_RESPOND_VERBS`].
    #[serde(default)]
    pub ai: Option<FineSystemAiConfigToml>,
}
