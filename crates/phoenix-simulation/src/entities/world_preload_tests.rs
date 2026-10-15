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

#[test]
fn imported_domain_waits_for_existing_source_queue_then_freezes_exact_closure() {
    crate::content_ledger::reset();
    let world =
        crate::content_fs::read_to_string("tests/fixtures/maintenance-htn-world.toml").unwrap();
    let library = "assets/worlds/domains/maintenance.toml";
    let missing = discover(
        ROOT,
        &world,
        &[],
        &|_| Ok(None),
        &crate::world::script::load::NoSiblingScripts,
    )
    .unwrap();
    assert_eq!(missing.pending, BTreeSet::from([library.into()]));
    assert!(missing.planning.is_none());
    assert!(!crate::content_ledger::is_frozen());
    let text = crate::content_fs::read_to_string(library).unwrap();
    let ready = discover(
        ROOT,
        &world,
        &[],
        &|path| Ok((path == library).then(|| text.clone())),
        &crate::world::script::load::NoSiblingScripts,
    )
    .unwrap();
    assert!(ready.pending.is_empty());
    let compiled = ready.planning.unwrap();
    assert_eq!(compiled.libraries().count(), 1);
    let denied = discover(
        ROOT,
        &world,
        &[],
        &|_| Err("HTTP 404".into()),
        &crate::world::script::load::NoSiblingScripts,
    )
    .err()
    .unwrap();
    assert!(denied.contains(library) && denied.contains("HTTP 404"));
}

#[test]
fn dormant_destination_support_scripts_join_preload_without_activation_requests() {
    crate::content_ledger::reset();
    let text = "[[transit_destination]]\nid='outer'\nlabel='world.outer'\ntarget={kind='layer',path='assets/worlds/outer.toml',anchor='arrival'}";
    let mut sources = Sources(BTreeMap::new());
    let discover_route = |sources: &Sources| {
        discover(
            ROOT,
            text,
            &[],
            &|path| Ok(sources.0.get(path).cloned()),
            &sources,
        )
    };
    assert_eq!(
        discover_route(&sources).unwrap().pending,
        BTreeSet::from(["assets/worlds/outer.toml".into()])
    );
    sources.0.insert("assets/worlds/outer.toml".into(), "script='outer.rhai'\nextra_worlds=['assets/worlds/support.toml']\n[anchors]\narrival=[1000,0]".into());
    assert_eq!(
        discover_route(&sources).unwrap().pending,
        BTreeSet::from([
            "assets/worlds/outer.rhai".into(),
            "assets/worlds/support.toml".into()
        ])
    );
    sources.0.insert("assets/worlds/outer.rhai".into(), "fn wave(ctx) { ctx.effects.spawn_entity(#{template_path: \"assets/entities/destination-only.toml\"}); }".into());
    sources.0.insert(
        "assets/worlds/support.toml".into(),
        "[global]\ntitle='Support'".into(),
    );
    let ready = discover_route(&sources).unwrap();
    assert!(ready.pending.is_empty());
    assert_eq!(
        ready.templates,
        BTreeSet::from(["assets/entities/destination-only.toml".into()])
    );
    assert!(crate::world::config::parse_world(text)
        .unwrap()
        .extra_worlds
        .is_empty());
}
