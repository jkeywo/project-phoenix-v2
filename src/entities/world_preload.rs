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
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const ROOT: &str = "assets/worlds/preload.toml";
    const TEXT: &str =
        "script = \"root.rhai\"\nextra_worlds = [\"assets/worlds/child.toml\"]\n[global]\n";
    const CHILD: &str = "assets/worlds/child.toml";
    const ROOT_SCRIPT: &str = "assets/worlds/root.rhai";
    const CHILD_SCRIPT: &str = "assets/worlds/child.rhai";

    struct Sources(BTreeMap<String, String>);
    impl ScriptResolver for Sources {
        fn read(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }
    }
    fn run(sources: &Sources) -> Dependencies {
        discover(
            ROOT,
            TEXT,
            &[],
            &|path| Ok(sources.0.get(path).cloned()),
            &crate::entities::config_cache::OverlayScriptResolver::new(sources),
        )
        .unwrap()
    }
    impl ScriptResolver for &Sources {
        fn read(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }
    }

    #[test]
    fn root_and_static_child_scripts_hold_discovery_and_find_literal_spawns() {
        crate::content_ledger::reset();
        let mut sources = Sources(BTreeMap::new());
        assert_eq!(
            run(&sources).pending,
            BTreeSet::from([CHILD.into(), ROOT_SCRIPT.into()])
        );
        sources
            .0
            .insert(CHILD.into(), "script = \"child.rhai\"\n[global]\n".into());
        sources.0.insert(ROOT_SCRIPT.into(), "fn wave(ctx) { ctx.effects.spawn_entity(#{ template_path: \"assets/entities/root-only.toml\" }); }".into());
        let half = run(&sources);
        assert_eq!(half.pending, BTreeSet::from([CHILD_SCRIPT.into()]));
        assert!(half.templates.contains("assets/entities/root-only.toml"));
        sources.0.insert(CHILD_SCRIPT.into(), "fn wave(ctx) { ctx.effects.spawn_entity(#{ template_path: \"assets/entities/child-only.toml\" }); ctx.effects.spawn_entity(#{ template_path: computed() }); }".into());
        let ready = run(&sources);
        assert!(ready.pending.is_empty());
        assert_eq!(
            ready.templates,
            BTreeSet::from([
                "assets/entities/root-only.toml".into(),
                "assets/entities/child-only.toml".into()
            ])
        );
    }

    #[test]
    fn completion_order_preserves_digest_and_changed_sibling_moves_it() {
        let entries = [
            (CHILD, "script = \"child.rhai\"\n[global]\n"),
            (ROOT_SCRIPT, "fn root() {}"),
            (CHILD_SCRIPT, "fn child() {}"),
        ];
        let mut digests = Vec::new();
        for reverse in [false, true] {
            crate::content_ledger::reset();
            let mut sources = Sources(BTreeMap::new());
            let mut ordered = entries.to_vec();
            if reverse {
                ordered.reverse();
            }
            for (path, text) in ordered {
                sources.0.insert(path.into(), text.into());
                run(&sources);
            }
            assert!(run(&sources).pending.is_empty());
            digests.push(crate::snapshot::versions(
                &crate::content_ledger::frozen_or_live(),
            ));
            sources
                .0
                .insert(CHILD_SCRIPT.into(), "fn changed() {}".into());
            run(&sources);
            assert_ne!(
                digests.last().unwrap(),
                &crate::snapshot::versions(&crate::content_ledger::frozen_or_live())
            );
        }
        assert_eq!(digests[0], digests[1]);
    }

    #[test]
    fn missing_sibling_is_a_path_specific_terminal_refusal() {
        crate::content_ledger::reset();
        let error = discover(
            ROOT,
            "script = \"root.rhai\"\n[global]\n",
            &[],
            &|_| Err("HTTP 404".into()),
            &crate::world::script::load::NoSiblingScripts,
        )
        .err()
        .expect("terminal failure");
        assert!(error.contains(ROOT_SCRIPT));
        assert!(error.contains("HTTP 404"));
        assert!(!crate::content_ledger::is_frozen());
    }
}
