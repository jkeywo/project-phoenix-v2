use crate::entities::{config::EntityConfig, include_resolve::ParseEntityTemplate};
use phoenix_content::include_resolve::{
    resolve_template, FragmentSource, IncludeError, ResolvedTemplate,
};
struct RepositoryFragments;
impl FragmentSource for RepositoryFragments {
    fn read(&self, path: &str) -> Option<String> {
        crate::repo_fixtures::fs::read_to_string(path).ok()
    }
    fn absence_is_final(&self) -> bool {
        true
    }
}
pub fn resolve_from_disk(path: &str) -> Result<ResolvedTemplate, IncludeError> {
    resolve_template(path, &RepositoryFragments)
}
pub fn load_entity_config(path: &str) -> Result<EntityConfig, IncludeError> {
    resolve_from_disk(path)?.parse()
}
