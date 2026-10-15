//! Held reference annotations and deliberate hypotheses; never live actor facts.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownReferenceMetadata {
    pub id: String,
    pub unusual: bool,
    /// Semantic controller work, independent of locale/display text length.
    pub reading_ticks: u32,
}
impl KnownReferenceMetadata {
    pub fn validate(&self) -> Result<(), String> {
        if self.id.is_empty() || self.id.len() > 128 || self.id.chars().any(char::is_control) {
            return Err("reference id must be nonempty, bounded and printable".into());
        }
        Ok(())
    }
}

/// Derived from an already permitted dossier finding. Attachment is a hypothesis,
/// not an assertion that its text describes this contact's actual current state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachedSensorReference {
    pub reference: KnownReferenceMetadata,
    pub text: String,
    pub provenance: String,
    pub gathered_at_tick: u64,
    pub attached_at_tick: u64,
    pub source_name: String,
}
