use crate::entities::config::EntityConfig;
use phoenix_content::findings::WorldFinding;
pub use phoenix_content::include_resolve::*;

/// Parse composed content with Phoenix's full entity schema and validation.
pub trait ParseEntityTemplate {
    fn parse(&self) -> Result<EntityConfig, IncludeError>;
}

impl ParseEntityTemplate for ResolvedTemplate {
    fn parse(&self) -> Result<EntityConfig, IncludeError> {
        self.parse_with(EntityConfig::from_toml)
    }
}

pub fn composition_finding(path: &str, source: &dyn FragmentSource) -> Option<WorldFinding> {
    composition_finding_with(path, source, EntityConfig::from_toml)
}
