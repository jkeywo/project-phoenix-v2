use super::source_scan::*;
use super::*;
use crate::ai::policy::AiPolicy;
use crate::ai::selector::TargetSelector;
use bevy::prelude::*;
use std::collections::BTreeSet;

// ── The table is complete and internally consistent ──────────────────────

#[test]
fn every_ai_host_owns_exactly_one_fine_system_kind() {
    let hosted: Vec<&str> = FINE_SYSTEM_KINDS.iter().map(|k| k.host.block).collect();
    let unique: BTreeSet<&str> = hosted.iter().copied().collect();
    assert_eq!(
        hosted.len(),
        unique.len(),
        "two kinds claim the same authored block, so the worklist would name the \
             wrong system"
    );
    let all_hosts: BTreeSet<&str> = ai_flag_hosts::AI_HOSTS.iter().map(|h| h.block).collect();
    assert_eq!(
        unique, all_hosts,
        "the manifest's kinds and ai_flag_hosts::AI_HOSTS must be the same twenty \
             hosts. A host with no kind is a fine system whose missing declaration \
             nothing counts — which is exactly the invisibility #885a closes."
    );
}

#[test]
fn every_kind_key_is_unique() {
    let keys: BTreeSet<&str> = FINE_SYSTEM_KINDS.iter().map(|k| k.key.as_str()).collect();
    assert_eq!(
        keys.len(),
        FINE_SYSTEM_KINDS.len(),
        "manifest keys are the committed worklist's identifiers; a duplicate would \
             merge two systems into one line"
    );
}

/// AC: the declared attachment site is not hand-maintained trivia — the
/// spawn path is read and checked, on EVERY path the kind claims.
///
/// This is the test that would have caught #785, #786 and #882: each of them
/// wired a declaration into `spawn_entity` and forgot
/// `spawn_game_start_entities`, so the player ship silently ran on a
/// Rust-side fallback instead of its own authored block.
#[test]
fn every_kind_is_attached_at_every_one_of_its_spawn_sites() {
    for kind in FINE_SYSTEM_KINDS {
        for site in kind.spawn_sites {
            let body = spawn_site_source(site.file, site.func);
            assert!(
                    body.contains(kind.component),
                    "{}: {}::{} is declared an attachment site for `{}` but never                      mentions it. Either the attachment moved (point the site at where                      it went) or this path never got it — which for                      `spawn_game_start_entities` means the PLAYER ship runs without                      the declaration its own TOML authors.",
                    kind.key.as_str(),
                    site.file,
                    site.func,
                    kind.component
                );
        }
    }
}

/// AC: the nineteen synthesisers are GONE and cannot come back unnoticed.
///
/// #885b stage 5d deleted every `default_*_ai_config()` /
/// `default_*_target_selector_config()`. A re-introduced one would restore
/// exactly what PRD #774 US7 forbids — automation supplied by Rust for a
/// system nobody declared — and would do it silently, because strict mode
/// only rejects what is *missing*, not what is quietly filled in. So the
/// scan is over both halves: the definitions in `config.rs`, and any call on
/// a spawn path.
#[test]
fn no_synthesiser_is_defined_or_called_anywhere() {
    let src = read_non_test_source("crates/phoenix-simulation/src/entities/config.rs");
    let defined: Vec<String> = src
        .lines()
        .filter_map(|line| {
            let t = line.trim_start();
            let rest = t
                .strip_prefix("pub fn ")
                .or_else(|| t.strip_prefix("fn "))?;
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let is_synth = name.starts_with("default_")
                && (name.ends_with("_ai_config") || name.ends_with("_target_selector_config"));
            is_synth.then_some(name)
        })
        .collect();
    assert!(
            defined.is_empty(),
            "crates/phoenix-simulation/src/entities/config.rs defines AI synthesiser(s) again: {defined:?}.              Stage 5d deleted all nineteen; a hull that wants a baseline authors it              in TOML, and strict AI-declaration mode is what makes omitting it an              error rather than a silent Rust default."
        );

    let sites: BTreeSet<(&str, &str)> = FINE_SYSTEM_KINDS
        .iter()
        .flat_map(|k| k.spawn_sites.iter().map(|s| (s.file, s.func)))
        .collect();
    assert!(!sites.is_empty(), "the scan has no spawn sites to walk");
    let mut called: Vec<String> = Vec::new();
    for (file, func) in sites {
        let body = spawn_site_source(file, func);
        for name in synthesiser_names(&body) {
            called.push(format!("{file}::{func} calls {name}"));
        }
    }
    called.sort();
    assert!(
        called.is_empty(),
        "these spawn paths call an AI synthesiser again: {called:?}"
    );
}

// ── Gating ───────────────────────────────────────────────────────────────

/// One shipped template through the real load path — include resolution
/// included (issue #906), so a composed hull is judged on its resolved
/// document rather than on the text of its own file.
fn parse(rel: &str) -> EntityConfig {
    let path = crate_root().join(rel);
    let key = path.to_string_lossy().replace('\\', "/");
    crate::entities::include_resolve::load_entity_config(&key)
        .unwrap_or_else(|e| panic!("{rel} must parse: {e}"))
}

/// The RESOLVED text of one shipped template (issue #906).
///
/// For the handful of assertions that need the TOML itself rather than a
/// parsed config — `from_toml_in_mode`, which has no resolver-aware wrapper,
/// and `spawn`, which parses the text it is handed. Reading the file
/// directly would hand them the UNRESOLVED text the day a hull declares
/// `includes`, which is exactly the silent coverage loss this issue exists
/// to prevent.
fn shipped_toml(rel: &str) -> String {
    let path = crate_root().join(rel);
    let key = path.to_string_lossy().replace('\\', "/");
    crate::entities::include_resolve::resolve_from_disk(&key)
        .unwrap_or_else(|e| panic!("{rel} must compose: {e}"))
        .toml
}

fn hull_files() -> Vec<String> {
    let dir = crate_root().join("assets/entities");
    let mut out: Vec<String> = Vec::new();
    for entry in crate::repo_fixtures::fs::read_dir(&dir).expect("assets/entities must be readable")
    {
        let path = entry.expect("readable dir entry").path();
        if path.extension().is_some_and(|e| e == "toml") {
            out.push(
                path.file_stem()
                    .expect("toml file has a stem")
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    out.sort();
    out
}

/// Scenery is not an AI actor, and strict mode must never demand
/// declarations from it.
#[test]
fn an_unarmed_static_entity_has_no_slots() {
    let station = parse("assets/entities/station_outpost.toml");
    assert!(station.behaviour.is_none(), "precondition");
    assert!(
        manifest(&station).is_empty(),
        "an unarmed station is hailable and damageable but not an AI actor: no \
             `[behaviour]`, no weapons ⇒ no AI-capable fine system at all."
    );
    assert!(strict_error(&station).is_none());
}

/// A minimal AI-bearing entity: `[behaviour]` and NOT ONE console section.
///
/// This was the shape of the three-weapon escort now at
/// `assets/entities/test/rng_coverage_lancer.toml` until #885b stage 5b, and
/// it is the shape the gating rule below is about. It is a fixture rather
/// than a shipped hull because every shipped hull now authors all five
/// selectors, and authoring `[sensors_console.selector]` necessarily brings
/// `[sensors_console]` into existence — so no shipped file omits them any
/// more. The RULE is unchanged, and a fixture is the only thing left that
/// can still exercise it.
///
/// Since stage 5d it only loads under [`AiDeclarationMode::Lenient`]: strict
/// mode is the default and rejects it, which is the whole point of it.
const BARE_BEHAVIOUR_HULL: &str = r#"
name = "test.bare_behaviour_hull"
tags = ["ship"]

[hull]
hull_integrity = 100.0

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
directive_kind = "Destroy"
base_priority = 40.0
"#;

/// The gate that a per-hull migration is most likely to get wrong.
#[test]
fn ship_level_slots_gate_on_behaviour_alone_not_on_the_console_section() {
    let bare = EntityConfig::from_toml_in_mode(BARE_BEHAVIOUR_HULL, AiDeclarationMode::Lenient)
        .expect("the bare-behaviour fixture must parse in lenient mode");
    assert!(
        bare.sensors_console.is_none()
            && bare.navigation_console.is_none()
            && bare.repair.is_none()
            && bare.comms_console.is_none()
            && bare.weapons_console.is_none(),
        "precondition: the fixture declares no console section at all."
    );
    let keys = undeclared_keys(&bare);
    for expected in [
        "sensors_selector",
        "tactical_selector",
        "navigation_selector",
        "repair_selector",
        "comms_selector",
        "comms_response",
    ] {
        assert!(
            keys.contains(&expected.to_string()),
            "{expected} must be counted for an entity that declares no matching \
                 console section — `[behaviour]` is the only gate, so a worklist \
                 computed from 'which sections does this file have?' would \
                 under-report by five selectors per bare hull. Got: {keys:?}"
        );
    }
}

/// No shipped hull is in the state where a declaration exists but synthesis
/// still fires. When one appears, #885b has to reckon with it.
#[test]
fn no_shipped_hull_declares_idle_out_of_band() {
    for stem in hull_files() {
        let c = parse(&format!("assets/entities/{stem}.toml"));
        for slot in manifest(&c) {
            assert_ne!(
                slot.declared,
                Declared::IdleLever,
                "{stem}/{}: the out-of-band idle lever declares intent but does NOT \
                     stop the synthesiser — the selector is still built and attached. \
                     The first hull to pull it needs that asymmetry resolved.",
                slot.key()
            );
        }
    }
}

/// Four of the five selectors cannot express an explicit idle at all, and
/// the manifest records that distinctly rather than papering over it.
#[test]
fn only_tactical_of_the_five_selectors_has_an_idle_lever() {
    let with_lever: Vec<&str> = FINE_SYSTEM_KINDS
        .iter()
        .filter(|k| matches!(k.idle_lever, IdleLever::Field(_)))
        .map(|k| k.key.as_str())
        .collect();
    assert_eq!(
        with_lever,
        vec!["tactical_selector"],
        "Tactical's `selector_idle` is the only out-of-band idle field in the \
             schema. Adding a sibling for Sensors/Navigation/Repair/Comms-hail is the \
             schema change PRD #774 US7's 'or explicit idle' half needs for them — \
             update this when it lands."
    );
    let absent: Vec<&str> = FINE_SYSTEM_KINDS
        .iter()
        .filter(|k| k.idle_lever == IdleLever::Absent)
        .map(|k| k.key.as_str())
        .collect();
    assert_eq!(
        absent,
        vec![
            "sensors_selector",
            "navigation_selector",
            "repair_selector",
            "comms_selector"
        ],
        "these four can declare a policy but cannot declare idle. Strict mode must \
             not demand something the schema has no field for."
    );
}

// ── Strict mode ──────────────────────────────────────────────────────────

/// **PRD #774 US7, as a switch.** The default load mode is strict, so a
/// missing declaration is a load error on every path — and every shipped
/// hull still loads.
#[test]
fn strict_mode_is_on_by_default_and_every_shipped_hull_still_loads() {
    assert_eq!(
        AiDeclarationMode::DEFAULT,
        AiDeclarationMode::Strict,
        "strict AI-declaration mode is the default since #885b stage 5d: with the \
             synthesisers deleted, an undeclared AI-capable fine system would simply \
             never act, and US7 requires that to be an error rather than a silence"
    );
    for stem in hull_files() {
        // Resolved first (issue #906): the default mode applies to the
        // COMPOSED document, which is the only thing that ever reaches
        // `EntityConfig` on a real load path.
        let src = shipped_toml(&format!("assets/entities/{stem}.toml"));
        EntityConfig::from_toml(&src)
            .unwrap_or_else(|e| panic!("{stem} must still load in the default mode: {e}"));
    }
}

/// **The completion gate for #885b, asserted directly.**
///
/// Every shipped hull declares every one of its 172 AI-capable fine-system
/// slots, so the strict default cannot stop a hull loading.
///
/// This is deliberately stated as "strict mode ACCEPTS them" rather than as
/// "the worklist is empty": the worklist and the load path are two different
/// pieces of code, and a hull could in principle satisfy the manifest's
/// gating while failing [`strict_error`]. Running the real strict load over
/// every shipped file is what makes the claim about the switch rather than
/// about the ledger.
#[test]
fn strict_mode_accepts_every_shipped_hull() {
    let mut checked = 0usize;
    for stem in hull_files() {
        // Resolved first (issue #906) — strict mode judges the COMPOSED
        // document, so a composed hull must not be read raw here.
        let src = shipped_toml(&format!("assets/entities/{stem}.toml"));
        let config = EntityConfig::from_toml(&src).expect("shipped entity parses");
        assert_eq!(
            strict_error(&config),
            None,
            "{stem}: this hull still owes a declaration, and with strict mode the \
                 default it will not load at all. Author the block named in the message."
        );
        EntityConfig::from_toml_in_mode(&src, AiDeclarationMode::Strict)
            .unwrap_or_else(|e| panic!("{stem} must load in STRICT mode: {e}"));
        checked += 1;
    }
    assert!(
        checked > 0,
        "no entity was checked — the scan is looking in the wrong place"
    );
}

/// …and strict mode still rejects, and still names the worklist, for
/// anything that declares nothing.
///
/// This was pointed at the three-weapon escort (then
/// `assets/entities/ship_harrow_lancer.toml`, and since #954 a test fixture
/// under `assets/entities/test/`) until #885b stage 5c authored its last
/// fourteen policies. No shipped hull can play the part any more — that is
/// the whole achievement — so the fixture takes over, which keeps the RULE
/// under test rather than loosening the assertion to whatever the fleet
/// happens to still owe.
#[test]
fn strict_mode_rejects_an_undeclared_policy_and_names_the_worklist() {
    let err = EntityConfig::from_toml_in_mode(BARE_BEHAVIOUR_HULL, AiDeclarationMode::Strict)
        .expect_err("the fixture declares nothing, so strict mode must reject it")
        .to_string();
    for (block, component) in [
        ("[captain_console.ai]", "CaptainAiPolicy"),
        // Since #1209 every helm axis decodes into the one keyed
        // `FineSystemAiPolicies` map, so the strict-mode error names that
        // component for the lateral slot rather than a per-axis newtype.
        ("[helm_console.lateral_ai]", "FineSystemAiPolicies"),
        ("[shields_console.ai_policy]", "ShieldsFocusAiPolicy"),
        ("[power.ai_policy]", "PowerAiPolicy"),
        ("[comms_console.ai]", "CommsResponseAiPolicy"),
    ] {
        assert!(
            err.contains(block) && err.contains(component),
            "the error must name the block to author and the runtime component that \
                 will otherwise never be attached ({block} / {component}): {err}"
        );
    }
    assert!(
        err.contains("or `idle = true` inside it"),
        "a POLICY can declare idle in band, and the message must say so — unlike \
             the four selectors with no idle field at all: {err}"
    );
}

/// …and the idle-less-selector wording is still produced, for anything that
/// has NOT authored one.
///
/// Four of the five selectors have no idle field, so strict mode must ask
/// for the block rather than for an explicit idle it has no way to write.
/// Exercised on the bare fixture now that no shipped hull is in that state.
#[test]
fn strict_mode_asks_for_the_block_where_no_idle_field_exists() {
    let err = EntityConfig::from_toml_in_mode(BARE_BEHAVIOUR_HULL, AiDeclarationMode::Strict)
        .expect_err("the fixture declares nothing, so strict mode must reject it")
        .to_string();
    assert!(
        err.contains("sensors_selector") && err.contains("[sensors_console.selector]"),
        "the error must name the selector block to author: {err}"
    );
    assert!(
        err.contains("NO idle field"),
        "the four idle-less selectors must say so rather than demanding an \
             unwritable explicit idle: {err}"
    );
}

#[test]
fn strict_mode_accepts_scenery() {
    let src = shipped_toml("assets/entities/station_outpost.toml");
    EntityConfig::from_toml_in_mode(&src, AiDeclarationMode::Strict)
        .expect("scenery has no AI-capable fine system, so strict mode has nothing to demand");
}

// ── The manifest against the real spawner ────────────────────────────────

fn spawn(toml: &str, what: &str) -> (App, Entity) {
    let config =
        EntityConfig::from_toml(toml).unwrap_or_else(|e| panic!("{what} template must parse: {e}"));
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    let entity = {
        let mut commands = app.world_mut().commands();
        crate::entities::spawner::spawn_entity(
            &mut commands,
            &config,
            Vec3::ZERO,
            format!("manifest-{what}"),
            None,
        )
    };
    app.update();
    (app, entity)
}

/// The fine-system id each helm axis's policy is keyed under in the one
/// [`crate::ship::helm_ai::FineSystemAiPolicies`] map (issue #1209); `None`
/// for every non-helm kind. The six axes share one component now, so the
/// cross-check resolves the entry by id rather than reading six newtypes.
fn helm_axis_system_id(key: FineSystemKey) -> Option<crate::core::messages::SystemId> {
    use crate::ship::system_registry as sr;
    Some(match key {
        FineSystemKey::Engines => sr::helm_thrust_system_id(),
        FineSystemKey::Steering => sr::helm_steering_system_id(),
        FineSystemKey::Lateral => sr::lateral_thrust_system_id(),
        FineSystemKey::Vertical => sr::vertical_thrust_system_id(),
        FineSystemKey::Impulse => sr::helm_impulse_system_id(),
        FineSystemKey::Boost => sr::helm_boost_system_id(),
        _ => return None,
    })
}

/// The policy the real spawner attached for one slot, if any.
///
/// Exhaustive over [`FineSystemKey`], so a twentieth fine system cannot be
/// added without the compiler demanding a cross-check for it.
fn attached(w: &World, e: Entity, slot: &Slot) -> Option<Attached> {
    let map = |m: Option<&std::collections::HashMap<String, AiPolicy>>| {
        let id = slot.instance.as_ref()?;
        m?.get(id).cloned().map(Attached::Policy)
    };
    match slot.kind.key {
        FineSystemKey::Captain => w
            .get::<crate::console::captain::server::CaptainAiPolicy>(e)
            .map(|c| Attached::Policy(c.0.clone())),
        FineSystemKey::CommsResponse => w
            .get::<crate::console::comms::server::CommsResponseAiPolicy>(e)
            .map(|c| Attached::Policy(c.0.clone())),
        // The six helm axes now decode into ONE keyed `FineSystemAiPolicies`
        // map (issue #1209); each is looked up by its fine-system id, the
        // ship-level analogue of the per-bank `map(..)` closure above.
        FineSystemKey::Engines
        | FineSystemKey::Steering
        | FineSystemKey::Lateral
        | FineSystemKey::Vertical
        | FineSystemKey::Impulse
        | FineSystemKey::Boost => {
            let id = helm_axis_system_id(slot.kind.key)
                .expect("the six helm axes each map to a fine-system id");
            w.get::<crate::ship::helm_ai::FineSystemAiPolicies>(e)
                .and_then(|c| c.0.get(&id).cloned())
                .map(Attached::Policy)
        }
        FineSystemKey::ShieldsFocus => w
            .get::<crate::ship::shields::ShieldsFocusAiPolicy>(e)
            .map(|c| Attached::Policy(c.0.clone())),
        FineSystemKey::Power => w
            .get::<crate::ship::power::PowerAiPolicy>(e)
            .map(|c| Attached::Policy(c.0.clone())),
        FineSystemKey::TorpedoMagazine => w
            .get::<crate::console::weapons::TorpedoMagazineAiPolicy>(e)
            .map(|c| Attached::Policy(c.0.clone())),
        FineSystemKey::WeaponsDoctrine => w
            .get::<crate::console::weapons::WeaponsDoctrineAiPolicy>(e)
            .map(|c| Attached::Policy(c.0.clone())),
        FineSystemKey::PhaserBank => map(w
            .get::<crate::console::weapons::PhaserBankAiPolicies>(e)
            .map(|c| &c.0)),
        FineSystemKey::BlasterBank => map(w
            .get::<crate::console::weapons::BlasterBankAiPolicies>(e)
            .map(|c| &c.0)),
        FineSystemKey::TorpedoTube => map(w
            .get::<crate::console::weapons::TorpedoTubeAiPolicies>(e)
            .map(|c| &c.0)),
        FineSystemKey::SensorsSelector => w
            .get::<crate::ship::sensors::SensorsTargetSelector>(e)
            .map(|c| Attached::Selector(c.selector.clone())),
        FineSystemKey::TacticalSelector => w
            .get::<crate::console::weapons::TacticalTargetSelector>(e)
            .map(|c| Attached::Selector(c.selector.clone())),
        FineSystemKey::NavigationSelector => w
            .get::<crate::console::navigation::NavigationTargetSelector>(e)
            .map(|c| Attached::Selector(c.selector.clone())),
        FineSystemKey::RepairSelector => w
            .get::<crate::console::repair::server::RepairTargetSelector>(e)
            .map(|c| Attached::Selector(c.selector.clone())),
        FineSystemKey::CommsSelector => w
            .get::<crate::console::comms::server::CommsTargetSelector>(e)
            .map(|c| Attached::Selector(c.selector.clone())),
        // The operate kinds (issue #1162) attach no per-hull AI policy /
        // selector component: their behaviour is directive-driven, so there
        // is nothing for this cross-check to find. Never reached in practice
        // (they are absent from `FINE_SYSTEM_KINDS`, which is all the callers
        // iterate); present only to keep the match exhaustive.
        FineSystemKey::Tractor
        | FineSystemKey::Umbilical
        | FineSystemKey::Dock
        | FineSystemKey::ExternalRepair => None,
    }
}

#[derive(Debug, PartialEq)]
enum Attached {
    Policy(AiPolicy),
    Selector(TargetSelector),
}

/// How many declarations of one kind the real spawner attached: 1/0 for a
/// ship-level system, the policy map's length for a per-weapon one.
///
/// The counterpart to [`attached`]: that one catches the manifest claiming a
/// slot the spawner never fills, this one catches the spawner filling a slot
/// the manifest never claims — the under-report that would let a whole
/// system drop off #885b's worklist unnoticed.
fn attached_count(w: &World, e: Entity, key: FineSystemKey) -> usize {
    let one = |present: bool| usize::from(present);
    match key {
        FineSystemKey::Captain => one(w
            .get::<crate::console::captain::server::CaptainAiPolicy>(e)
            .is_some()),
        FineSystemKey::CommsResponse => one(w
            .get::<crate::console::comms::server::CommsResponseAiPolicy>(e)
            .is_some()),
        // Each helm axis's entry in the one keyed `FineSystemAiPolicies` map
        // (issue #1209): present ⇒ 1, exactly as a standalone newtype's
        // presence used to read, so the manifest count still mirrors the
        // spawner one axis at a time.
        FineSystemKey::Engines
        | FineSystemKey::Steering
        | FineSystemKey::Lateral
        | FineSystemKey::Vertical
        | FineSystemKey::Impulse
        | FineSystemKey::Boost => {
            let id =
                helm_axis_system_id(key).expect("the six helm axes each map to a fine-system id");
            w.get::<crate::ship::helm_ai::FineSystemAiPolicies>(e)
                .map_or(0, |c| usize::from(c.0.contains_key(&id)))
        }
        FineSystemKey::ShieldsFocus => one(w
            .get::<crate::ship::shields::ShieldsFocusAiPolicy>(e)
            .is_some()),
        FineSystemKey::Power => one(w.get::<crate::ship::power::PowerAiPolicy>(e).is_some()),
        FineSystemKey::TorpedoMagazine => one(w
            .get::<crate::console::weapons::TorpedoMagazineAiPolicy>(e)
            .is_some()),
        FineSystemKey::WeaponsDoctrine => one(w
            .get::<crate::console::weapons::WeaponsDoctrineAiPolicy>(e)
            .is_some()),
        FineSystemKey::SensorsSelector => one(w
            .get::<crate::ship::sensors::SensorsTargetSelector>(e)
            .is_some()),
        FineSystemKey::TacticalSelector => one(w
            .get::<crate::console::weapons::TacticalTargetSelector>(e)
            .is_some()),
        FineSystemKey::NavigationSelector => one(w
            .get::<crate::console::navigation::NavigationTargetSelector>(e)
            .is_some()),
        FineSystemKey::RepairSelector => one(w
            .get::<crate::console::repair::server::RepairTargetSelector>(e)
            .is_some()),
        FineSystemKey::CommsSelector => one(w
            .get::<crate::console::comms::server::CommsTargetSelector>(e)
            .is_some()),
        FineSystemKey::PhaserBank => w
            .get::<crate::console::weapons::PhaserBankAiPolicies>(e)
            .map_or(0, |c| c.0.len()),
        FineSystemKey::BlasterBank => w
            .get::<crate::console::weapons::BlasterBankAiPolicies>(e)
            .map_or(0, |c| c.0.len()),
        FineSystemKey::TorpedoTube => w
            .get::<crate::console::weapons::TorpedoTubeAiPolicies>(e)
            .map_or(0, |c| c.0.len()),
        // Operate kinds (issue #1162): no per-hull policy component, so zero
        // attached. Never reached (absent from `FINE_SYSTEM_KINDS`).
        FineSystemKey::Tractor
        | FineSystemKey::Umbilical
        | FineSystemKey::Dock
        | FineSystemKey::ExternalRepair => 0,
    }
}

/// AC: the manifest's gating is what the real spawner does.
///
/// Every slot the manifest claims must actually be filled at spawn — and,
/// since #885b stage 5d, filled from the hull's OWN authored block, because
/// there is nothing else left to fill it from. Change the spawn gate without
/// changing [`slots_of_kind`] and this fails naming the hull and the system.
#[test]
fn the_manifest_matches_the_real_spawner() {
    let mut checked = 0usize;
    for stem in hull_files() {
        // Resolved first (issue #906): `spawn` below is handed this same
        // text, so a composed hull would otherwise be spawned from its
        // unresolved file and the manifest compared against the wrong ship.
        let src = shipped_toml(&format!("assets/entities/{stem}.toml"));
        let config = EntityConfig::from_toml(&src).expect("shipped entity parses");
        let slots = manifest(&config);
        if slots.is_empty() {
            continue;
        }
        let (app, e) = spawn(&src, &stem);
        let w = app.world();
        for slot in &slots {
            let got = attached(w, e, slot).unwrap_or_else(|| {
                panic!(
                    "{stem}/{}: the manifest claims this AI-capable fine system \
                         exists on this hull, but the real spawn path attached nothing \
                         for it. The gating in slots_of_kind has drifted from \
                         spawner.rs — over-reporting inflates the worklist with systems \
                         that do not exist.",
                    slot.key()
                )
            });
            assert_ne!(
                slot.declared,
                Declared::Nothing,
                "{stem}/{}: an undeclared slot on a hull that LOADED. Since #885b \
                     stage 5d strict mode is the default, so this is unreachable \
                     through `from_toml` — reaching it means the load path stopped \
                     checking.",
                slot.key()
            );
            assert!(
                matches!(got, Attached::Policy(_) | Attached::Selector(_)),
                "{stem}/{}: the attached declaration must decode",
                slot.key()
            );
            checked += 1;
        }
        // …and the other direction: no kind may be attached more times than
        // the manifest claims slots for it. Gating the Repair selector on
        // `[repair]` (the plausible-but-wrong "which sections does this hull
        // declare?" rule) would pass every assertion above and silently drop
        // that system off the worklist for every hull without the section.
        for kind in FINE_SYSTEM_KINDS {
            let claimed = slots_of_kind(kind, &config).len();
            assert_eq!(
                attached_count(w, e, kind.key),
                claimed,
                "{stem}/{}: the manifest claims {claimed} slot(s) of this kind but \
                     the real spawn path attached a different number. Under-reporting \
                     drops a whole fine system out of #885b's worklist without \
                     changing anything a reader would notice.",
                kind.key.as_str()
            );
        }
    }
    assert!(
        checked > 0,
        "no slots were checked — the scan is looking in the wrong place"
    );
}

/// The negative half: scenery gets nothing attached, so strict mode's
/// silence about it is real rather than a gap in the manifest.
#[test]
fn an_entity_with_no_slots_gets_no_ai_declaration_attached() {
    // Resolved first (issue #906) — `spawn` below is handed this same text.
    let src = shipped_toml("assets/entities/station_outpost.toml");
    let config = EntityConfig::from_toml(&src).expect("station parses");
    assert!(manifest(&config).is_empty(), "precondition");

    let (app, e) = spawn(&src, "station");
    let w = app.world();
    for kind in FINE_SYSTEM_KINDS {
        let probe = Slot {
            kind,
            instance: Some("any".to_string()),
            declared: Declared::Nothing,
        };
        assert!(
            attached(w, e, &probe).is_none(),
            "station/{}: an entity with no `[behaviour]` and no weapons must get no \
                 AI declaration attached at all.",
            kind.key.as_str()
        );
    }
}

// ── The rendered surface ─────────────────────────────────────────────────

/// The UNDECLARED rendering, on the fixture — no shipped hull produces one
/// any more, which is stage 5c's result rather than a reason to stop
/// checking the rendering.
#[test]
fn manifest_lines_name_the_block_to_author() {
    let bare = EntityConfig::from_toml_in_mode(BARE_BEHAVIOUR_HULL, AiDeclarationMode::Lenient)
        .expect("the bare-behaviour fixture must parse in lenient mode");
    let lines = manifest_lines("bare_behaviour_hull", &bare);
    assert_eq!(lines.len(), manifest(&bare).len(), "one line per slot");
    let captain = lines
        .iter()
        .find(|l| l.contains(" captain "))
        .expect("the fixture's captain slot is rendered");
    assert!(
        captain.contains("UNDECLARED") && captain.contains("[captain_console.ai]"),
        "a rendered line must be actionable on its own: {captain}"
    );
}

/// …and every slot on every shipped hull renders as DECLARED.
///
/// The other half of the same surface: the load-time report a developer
/// actually reads must show no `SYNTHESISED` line for shipped content, or
/// the ledger and the report disagree about whether #885b is done.
#[test]
fn no_shipped_hull_renders_an_undeclared_line() {
    for stem in hull_files() {
        let c = parse(&format!("assets/entities/{stem}.toml"));
        for line in manifest_lines(&stem, &c) {
            assert!(
                !line.contains("UNDECLARED"),
                "{stem}: this slot is undeclared, and with the synthesisers deleted \
                     it would never act at all: {line}"
            );
        }
    }
}

#[test]
fn a_declared_slot_renders_as_declared() {
    let cruiser = parse("assets/entities/alliance_cruiser.toml");
    assert!(
        cruiser
            .captain_console
            .as_ref()
            .is_some_and(|c| c.ai.is_some()),
        "precondition: the Alliance Cruiser hand-authors `[captain_console.ai]`"
    );
    let lines = manifest_lines("alliance_cruiser", &cruiser);
    let captain = lines
        .iter()
        .find(|l| l.contains(" captain "))
        .expect("captain slot rendered");
    assert!(
        captain.contains("declared") && !captain.contains("UNDECLARED"),
        "an authored block must not be reported as undeclared: {captain}"
    );
}
