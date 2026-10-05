//! Pre-init discovery for the root and its statically declared supporting worlds
//! (#1248). Source reads retain the production overlay authority. This pass lifts
//! and hashes scripts but never compiles, executes, or freezes them.

use std::collections::BTreeSet;

use crate::world::script::load::{
    declared_sibling_script_path, lift_world_scripts, script_source_ledger_digest, ScriptResolver,
};

#[derive(Default)]
pub(crate) struct Dependencies {
    pub pending: BTreeSet<String>,
    pub templates: BTreeSet<String>,
}

/// `Ok(None)` is an outstanding read; `Err` is terminal. The caller issues the
/// returned requests and advances this pass when a response arrives. Only root
/// `extra_worlds` are boot dependencies, matching `world::load::Activate`.
pub(crate) fn discover(
    path: &str,
    text: &str,
    curated_ships: &[String],
    read: &dyn Fn(&str) -> Result<Option<String>, String>,
    scripts: &dyn ScriptResolver,
) -> Result<Dependencies, String> {
    let root = crate::world::config::parse_world(text).map_err(|e| format!("{path}: {e}"))?;
    let mut documents = vec![(path.to_string(), text.to_string(), true)];
    let mut result = Dependencies::default();
    for child in root.extra_worlds.iter().collect::<BTreeSet<_>>() {
        match read(child).map_err(|e| format!("{child}: {e}"))? {
            Some(source) => documents.push((child.clone(), source, false)),
            None => {
                result.pending.insert(child.clone());
            }
        }
    }
    for (path, text, is_root) in documents {
        let config =
            crate::world::config::parse_world(&text).map_err(|e| format!("{path}: {e}"))?;
        crate::content_ledger::record(&path, &text);
        result
            .templates
            .extend(crate::world::config::entity_template_paths(
                &config,
                if is_root { curated_ships } else { &[] },
            ));
        if let Some(sibling) = declared_sibling_script_path(&path, &text) {
            if read(&sibling)
                .map_err(|e| format!("{sibling}: {e}"))?
                .is_none()
            {
                result.pending.insert(sibling);
                continue;
            }
        }
        let raw = toml::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
        let (sources, findings) = lift_world_scripts(&path, &raw, scripts);
        let errors: Vec<_> = findings
            .iter()
            .filter(|f| f.is_error())
            .map(|f| f.message.as_str())
            .collect();
        if !errors.is_empty() {
            return Err(format!("{path}: {}", errors.join("; ")));
        }
        if let Some(digest) = script_source_ledger_digest(&path, &sources) {
            digest.apply();
        }
        for source in sources {
            result.templates.extend(
                crate::world::config::resolved_script_spawn_refs(source.path, &source.source)
                    .into_iter()
                    .map(|reference| reference.template_path),
            );
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "world_preload_tests.rs"]
mod tests;
