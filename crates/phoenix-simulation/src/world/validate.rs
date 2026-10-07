use crate::entities::loader::TemplateLoader;
pub use phoenix_content_preparation::world::validate::*;
pub fn validate_composition(root: &WorldSource, children: &[WorldSource]) -> Vec<WorldFinding> {
    validate_composition_with(root, children, &crate::entities::loader::WasmTemplateLoader)
}

/// [`validate_composition`] with an explicit entity-template source.
///
/// Include fragments still come from the host source
/// ([`crate::entities::include_resolve::HostFragmentSource`]) — a `TemplateLoader` serves
/// parsed configs and cannot serve raw fragment text. Use
/// [`validate_composition_with_fragments`] to control both.
pub fn validate_composition_with(
    root: &WorldSource,
    children: &[WorldSource],
    template_loader: &dyn TemplateLoader,
) -> Vec<WorldFinding> {
    validate_composition_with_fragments(
        root,
        children,
        template_loader,
        &crate::entities::include_resolve::HostFragmentSource,
    )
}
