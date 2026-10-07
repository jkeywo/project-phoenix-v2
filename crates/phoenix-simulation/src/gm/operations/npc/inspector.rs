//! Constrained Live reading tokens over the exact field and selected authored
//! choice. Derived AI scores, hull, position and unrelated choices do not stale
//! a doctrine edit. This is optimistic concurrency, never an authority token.
use super::*;
use crate::inspector::{FieldDescriptor, LiveMutability};

pub fn valid_revision(revision: &str) -> bool {
    revision.len() == 16
        && revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

pub fn revision(
    target: &str,
    doctrine: &[DoctrineObjective],
    state: &NpcDoctrineState,
    choice: &NpcDoctrinePaletteEntry,
) -> String {
    // Reuse the existing canonical doctrine representation: the TOML schema's
    // flattened fields cannot be serialized directly through postcard.
    let fields = (
        "npc-live-inspector-v1",
        target,
        doctrine
            .iter()
            .map(doctrine_digest_fields)
            .collect::<Vec<_>>(),
        state.0.as_ref().map(applied_digest_fields),
        &choice.id,
        &choice.label,
        &choice.targets,
        &choice.origin_layer,
        choice
            .doctrine
            .iter()
            .map(doctrine_digest_fields)
            .collect::<Vec<_>>(),
    );
    format!("{:016x}", vellum_digest::digest_postcard(&fields))
}

pub fn descriptor(kind: &str, mutability: LiveMutability, schema_path: &str) -> FieldDescriptor {
    FieldDescriptor::runtime(kind, mutability, schema_path)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NpcInspector {
    pub current: FieldDescriptor,
    pub intent: FieldDescriptor,
    pub definition: FieldDescriptor,
}

impl Default for NpcInspector {
    fn default() -> Self {
        let mut current = descriptor("enum", LiveMutability::NamedAction, "behaviour.doctrine");
        current.validation = vec![
            "inspector.validation.authored_choice".into(),
            "inspector.validation.npc_compatible".into(),
            "inspector.validation.current_reading".into(),
        ];
        Self {
            current,
            intent: descriptor(
                "string",
                LiveMutability::Derived,
                "scored_objectives.chosen",
            ),
            definition: descriptor(
                "string",
                LiveMutability::RecreateRequired,
                "gm_npc_doctrine_palette.doctrine",
            ),
        }
    }
}

#[cfg(test)]
#[path = "inspector_tests.rs"]
mod tests;
