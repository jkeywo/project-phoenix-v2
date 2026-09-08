use super::{browser_resume_versions, import_resume_after_scenario, load_resume_after_scenario};
use crate::content_ledger;
use crate::world::script::load::{NoSiblingScripts, ScriptResolver};
use bevy::prelude::{App, IntoScheduleConfigs, Startup};

const WORLD: &str = "assets/worlds/browser-resume-content.toml";
const HULL: &str = "assets/entities/alliance_cruiser.toml";

fn begin_preload(text: &str) {
    content_ledger::reset();
    content_ledger::record(WORLD, text);
    // A non-script dependency must survive preparation unchanged too.
    content_ledger::record(HULL, "authored hull bytes");
}

fn startup_versions(text: &str) -> vellum_save::Versions {
    let mut app = App::new();
    app.insert_resource(crate::world::server::RawWorldSource {
        path: WORLD.into(),
        toml: toml::from_str(text).unwrap(),
    });
    app.insert_resource(crate::boot::PendingHostContentFreeze);
    app.add_systems(
        Startup,
        (
            crate::world::server::compile_world_scripts,
            crate::world::server::freeze_host_preloaded_content,
        )
            .chain(),
    );
    // Browser execution and its ledger both live on the host thread.
    app.edit_schedule(Startup, |schedule| {
        schedule.set_executor_kind(bevy::ecs::schedule::ExecutorKind::SingleThreaded);
    });
    app.update();
    assert!(content_ledger::is_frozen());
    assert!(!app
        .world()
        .contains_resource::<crate::boot::PendingHostContentFreeze>());
    crate::snapshot::versions(&content_ledger::frozen_or_live())
}

fn artifact(versions: vellum_save::Versions) -> String {
    crate::snapshot::run_for(
        crate::snapshot::PhoenixSnapshot {
            boot_identity: Some(crate::snapshot::BootIdentity {
                selected_ship: HULL.into(),
                fleet: crate::lockstep::FleetRoster::default(),
                game_start_entity_uuids: vec![crate::snapshot::GameStartEntityUuid {
                    authored_index: 0,
                    entity_uuid: "00000000-0000-8000-8000-000000000000".into(),
                }],
            }),
            ..Default::default()
        },
        0xfeed,
        17,
        WORLD,
        versions,
    )
    .to_ron()
    .unwrap()
}

struct SavedArtifact<'a>(&'a str);

impl vellum_save::Store for SavedArtifact<'_> {
    type Error = std::io::Error;

    fn read(&self, slot: &str) -> Result<Option<String>, Self::Error> {
        Ok((slot == "saved").then(|| self.0.into()))
    }

    fn write(&self, _slot: &str, _contents: &str) -> Result<(), Self::Error> {
        Err(std::io::Error::other("restore preparation must not write"))
    }

    fn remove(&self, _slot: &str) -> Result<(), Self::Error> {
        Err(std::io::Error::other("restore preparation must not delete"))
    }

    fn slots(&self) -> Result<Vec<String>, Self::Error> {
        Ok(vec!["saved".into()])
    }
}

fn assert_both_content_gates(artifact: &str, current: &vellum_save::Versions, compatible: bool) {
    let store = SavedArtifact(artifact);
    let world = crate::world::config::parse_world(
        r#"
[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
"#,
    )
    .unwrap();
    for result in [
        load_resume_after_scenario(&store, "saved", current, HULL, &world),
        import_resume_after_scenario(artifact, current, HULL, &world),
    ] {
        if compatible {
            let run = result.expect("unchanged content passes the full early gate");
            assert_eq!(run.to_ron().unwrap(), artifact);
        } else {
            assert!(matches!(
                result,
                Err(super::BrowserResumeRefusal::Load(
                    crate::snapshot::LoadRefusal::Moved(vellum_save::Moved::Content { .. })
                ))
            ));
        }
    }
}

#[test]
fn browser_preinit_versions_match_actual_startup_for_inline_and_script_free_roots() {
    for text in [
        "[global]\nseed = 17\n",
        "[script]\nzeta = 'fn zeta(ctx) {}'\nalpha = 'fn alpha(ctx) {}'\n",
    ] {
        begin_preload(text);
        let saved = startup_versions(text);
        let frozen = content_ledger::frozen_or_live();
        let artifact = artifact(saved.clone());

        begin_preload(text);
        let before = content_ledger::snapshot();
        let prepared = browser_resume_versions(WORLD, text, &NoSiblingScripts).unwrap();
        assert_eq!(prepared, saved);
        assert_eq!(content_ledger::snapshot(), frozen);
        assert!(
            !content_ledger::is_frozen(),
            "Startup still owns the freeze"
        );
        if text.contains("[script]") {
            assert_ne!(before, frozen, "the old pre-init gate missed a record");
        } else {
            assert_eq!(before, frozen, "a script-free load is unchanged");
        }
        assert_both_content_gates(&artifact, &prepared, true);
    }
    content_ledger::reset();
}

#[test]
fn browser_preinit_changed_inline_source_still_refuses_local_and_portable_saves() {
    let original = "[script]\nsetup = 'fn unchanged(ctx) {}'\n";
    begin_preload(original);
    let artifact = artifact(startup_versions(original));
    for changed in [
        "[script]\nsetup = 'fn changed(ctx) {}'\n",
        "[script]\nsetup = 'this is invalid Rhai'\n",
    ] {
        begin_preload(changed);
        let prepared = browser_resume_versions(WORLD, changed, &NoSiblingScripts).unwrap();
        assert_both_content_gates(&artifact, &prepared, false);
    }
    content_ledger::reset();
}

struct FixedSibling(&'static str);

impl ScriptResolver for FixedSibling {
    fn read(&self, _path: &str) -> Option<String> {
        Some(self.0.into())
    }
}

#[test]
fn browser_preinit_records_resolved_sibling_and_overlay_bytes_before_versioning() {
    use crate::entities::config_cache::{
        overlay_test_guard, push_mod_pack, ActivePack, OverlayScriptResolver,
    };
    let _overlay = overlay_test_guard();
    let text = "script = 'resume-content.rhai'\n";
    let sibling = "assets/worlds/resume-content.rhai";
    let base_source = "fn from_base(ctx) {}";
    let pack_source = "fn from_pack(ctx) {}";
    let resolver = OverlayScriptResolver::new(FixedSibling(base_source));

    for source in [base_source, ""] {
        begin_preload(text);
        browser_resume_versions(
            WORLD,
            text,
            &OverlayScriptResolver::new(FixedSibling(source)),
        )
        .unwrap();
        assert_eq!(
            content_ledger::snapshot().get(sibling),
            Some(vellum_digest::fnv1a(source.as_bytes()))
        );
        assert!(content_ledger::snapshot()
            .get(&format!("{WORLD}#scripts"))
            .is_some());
    }
    begin_preload(text);
    let base = browser_resume_versions(WORLD, text, &resolver).unwrap();

    push_mod_pack(ActivePack {
        id: "browser-resume-content".into(),
        files: [(sibling.into(), pack_source.into())].into(),
        ..Default::default()
    });
    begin_preload(text);
    let saved = startup_versions(text);
    let frozen = content_ledger::frozen_or_live();
    let artifact = artifact(saved.clone());
    begin_preload(text);
    let prepared = browser_resume_versions(WORLD, text, &resolver).unwrap();
    assert_eq!(prepared, saved);
    assert_eq!(content_ledger::snapshot(), frozen);
    assert_eq!(
        frozen.get(sibling),
        Some(vellum_digest::fnv1a(pack_source.as_bytes()))
    );
    assert_both_content_gates(&artifact, &prepared, true);
    assert_both_content_gates(&artifact, &base, false);
    content_ledger::reset();
}

#[test]
fn browser_preinit_refuses_missing_or_invalid_source_declarations() {
    for text in [
        "script = 'missing.rhai'",
        "script = 3",
        "[script]\nsetup = 3",
        "[script]\nvalid = 'fn valid(ctx) {}'\ninvalid = 3",
        "not valid TOML [",
    ] {
        begin_preload(text);
        let before = content_ledger::snapshot();
        let refusal = browser_resume_versions(WORLD, text, &NoSiblingScripts).unwrap_err();
        assert!(refusal.starts_with("the scenario"));
        assert_eq!(content_ledger::snapshot(), before);
        assert!(!content_ledger::is_frozen());
    }
    content_ledger::reset();
}

#[test]
fn browser_preinit_does_not_execute_source_or_reopen_a_frozen_ledger() {
    let text = "[script]\nsetup = 'throw \"must not execute at preparation\";'";
    begin_preload(text);
    browser_resume_versions(WORLD, text, &NoSiblingScripts).unwrap();
    assert!(content_ledger::snapshot()
        .get(&format!("{WORLD}#scripts"))
        .is_some());
    content_ledger::freeze();
    let frozen = content_ledger::frozen_or_live();

    struct MustNotResolve;
    impl ScriptResolver for MustNotResolve {
        fn read(&self, _path: &str) -> Option<String> {
            panic!("an already-frozen identity must not resolve new content");
        }
    }
    let prepared = browser_resume_versions(WORLD, "script = 'later.rhai'", &MustNotResolve)
        .expect("frozen identity remains authoritative");
    assert_eq!(prepared, crate::snapshot::versions(&frozen));
    assert_eq!(content_ledger::snapshot(), frozen);
    content_ledger::reset();
}
