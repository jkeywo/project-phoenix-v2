//! Native Workshop preview capture. The result contains bytes only inside the
//! host process; the embedded page receives nonce-scoped HTTP members instead.
use super::{assets::Sources, NativeWorkshopProvider, Response};
use crate::workshop::test_protocol::PreviewSelection;

#[derive(Debug)]
pub struct PreviewSnapshot {
    pub files: super::Files,
    pub selection: PreviewSelection,
    pub revision: String,
}

impl NativeWorkshopProvider {
    pub fn prepare_preview(
        &self,
        sources: Sources,
        selection: PreviewSelection,
    ) -> Result<PreviewSnapshot, Response> {
        let files =
            self.prepare_runtime_capture(sources, "Runtime validation refused Workshop preview")?;
        let Some(subject) = selection.subject() else {
            return Err(super::refused(
                "Workshop preview must select exactly one subject",
            ));
        };
        let valid_kind = (selection.model.is_some() && subject.ends_with(".glb"))
            || (selection.entity.is_some()
                && subject.starts_with("assets/entities/")
                && subject.ends_with(".toml"));
        if !valid_kind || !files.contains_key(subject) {
            return Err(super::refused(
                "Workshop preview subject is absent from the captured draft",
            ));
        }
        Ok(PreviewSnapshot {
            revision: super::revision(&files),
            files,
            selection,
        })
    }
}
