//! #1400 slice 1: schedule debt only. Task pools remain isolated in this binary.
//! No executor, RNG, mint, digest boundary or commutativity policy is changed.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]
use bevy::prelude::*;
use project_phoenix::headless::determinism_audit::{
    fixed_update_census, parse_census, require_unambiguous_fixed_update, uncovered, Ambiguity,
};
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::sim_rng::InstallSimRng;
use project_phoenix::world_id::InstallWorldIdMint;
use std::{path::Path, process::Command};

#[path = "fixed_update_ambiguities/bootstrap.rs"]
mod bootstrap;

const LEDGER: &str = "tests/fixtures/determinism/fixed-update-ambiguities.json";
fn simulation() -> App {
    build_headless_app(&HeadlessArgs {
        world_path: "assets/worlds/rng_coverage.toml".into(),
        seed: Some(20260899),
        deterministic: true,
        ..Default::default()
    })
    .expect("ordinary deterministic headless app")
}

/// A capture command is intentionally explicit and never edits the ledger.
/// Review the output on the final integrated graph before introducing debt.
#[test]
#[ignore = "explicit initial census capture; prints real graph, never blesses it"]
fn print_fixed_update_census() {
    let rows = fixed_update_census(&mut simulation()).unwrap();
    println!("{}", serde_json::to_string_pretty(&rows).unwrap());
}

#[test]
fn live_debt_is_a_subset_and_the_allowlist_does_not_grow() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let allowed = parse_census(
        &std::fs::read_to_string(root.join(LEDGER))
            .expect("introduce the reviewed original census before enabling this gate"),
    )
    .expect("canonical checked-in census");
    let mut app = simulation();
    let live = fixed_update_census(&mut app).unwrap();
    assert!(
        uncovered(&live, &allowed).is_empty(),
        "new unordered access:\n{:#?}",
        uncovered(&live, &allowed)
    );

    // The caller supplies a trusted PR base SHA, push-before SHA or local
    // integration base. Never use HEAD as an implicit self-comparison, and
    // never fetch from a remote from inside a test.
    let reference = std::env::var("PHOENIX_AMBIGUITY_BASE_REF")
        .expect("set PHOENIX_AMBIGUITY_BASE_REF to the trusted pre-change commit");
    assert!(
        !reference.starts_with('-') && !reference.contains(':') && !reference.is_empty(),
        "invalid trusted ref"
    );
    let commit = Command::new("git")
        .current_dir(root)
        .args(["rev-parse", "--verify", &format!("{reference}^{{commit}}")])
        .output()
        .unwrap();
    assert!(
        commit.status.success(),
        "trusted base must exist locally; no network fetch is performed"
    );
    let sha = String::from_utf8(commit.stdout).unwrap();
    let object = format!("{}:{LEDGER}", sha.trim());
    let exists = Command::new("git")
        .current_dir(root)
        .args(["ls-tree", "--name-only", sha.trim(), "--", LEDGER])
        .output()
        .unwrap();
    assert!(exists.status.success(), "cannot inspect trusted base tree");
    if !exists.stdout.is_empty() {
        let prior = Command::new("git")
            .current_dir(root)
            .args(["show", &object])
            .output()
            .unwrap();
        assert!(prior.status.success(), "cannot read trusted baseline");
        let prior = parse_census(&String::from_utf8(prior.stdout).unwrap()).unwrap();
        let growth = uncovered(&allowed, &prior);
        assert!(
            growth.is_empty(),
            "allowlist grew relative to trusted base:\n{growth:#?}"
        );
    } else {
        let shallow = Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "--is-shallow-repository"])
            .output()
            .unwrap();
        assert!(
            shallow.status.success(),
            "cannot inspect history completeness"
        );
        assert_eq!(
            String::from_utf8(shallow.stdout).unwrap().trim(),
            "false",
            "first introduction requires complete local history"
        );
        let history = Command::new("git")
            .current_dir(root)
            .args([
                "log",
                "--full-history",
                "--format=%H",
                "-1",
                sha.trim(),
                "--",
                LEDGER,
            ])
            .output()
            .unwrap();
        assert!(
            history.status.success(),
            "cannot inspect trusted ledger history"
        );
        assert!(history.stdout.is_empty(),
            "trusted history already contained the ledger: deletion cannot authorize reintroduction");
        // The PR base may predate every commit in this batch. Bind that first
        // introduction to the reviewed capture instead of trusting a later
        // candidate ledger as its own baseline. History checks above still win.
        bootstrap::enforce_first_allowance(root, &allowed).unwrap();
    }
    if allowed.is_empty() {
        require_unambiguous_fixed_update(&mut app).unwrap();
    }
}

fn inert_registration_probe() {}
#[test]
fn inert_registration_does_not_change_the_actual_census() {
    let mut ordinary = simulation();
    let expected = fixed_update_census(&mut ordinary).unwrap();
    let probe_name = std::any::type_name_of_val(&inert_registration_probe);
    assert!(expected
        .iter()
        .all(|row| row.systems.iter().all(|name| name != probe_name)));
    // Read exclusivity independently from the initialized systems, not from
    // the conflict rows whose meaning this test is checking.
    let exclusive_names: std::collections::BTreeSet<_> = ordinary
        .world()
        .resource::<Schedules>()
        .get(FixedUpdate)
        .unwrap()
        .systems()
        .unwrap()
        .filter(|(_, system)| system.is_exclusive())
        .map(|(_, system)| system.name().as_string())
        .collect();
    let mut perturbed = simulation();
    perturbed.add_systems(
        FixedUpdate,
        inert_registration_probe.in_set(project_phoenix::sim_sets::SimSet::Input),
    );
    let actual = fixed_update_census(&mut perturbed).unwrap();
    let (probe_rows, production_rows): (Vec<_>, Vec<_>) = actual
        .into_iter()
        .partition(|row| row.systems.iter().any(|name| name == probe_name));
    // Preserve every production pair, access vector and repeated instance.
    // The production census remains unfiltered; this partition is test-only.
    assert_eq!(production_rows, expected);
    assert!(!probe_rows.is_empty());
    for row in probe_rows {
        assert_eq!(row.access, ["<exclusive World access>"]);
        let others: Vec<_> = row
            .systems
            .iter()
            .filter(|name| name.as_str() != probe_name)
            .collect();
        assert_eq!(others.len(), 1);
        assert!(exclusive_names.contains(others[0]));
    }
    // Bevy conservatively conflicts even this no-access probe with exclusive
    // World systems. Adding it changes no production access or ordering.
    // This digest assertion covers registration/initialization only: neither
    // app has run a mission tick, so it is not a measured simulation proof.
    assert_eq!(
        project_phoenix::sim_digest::world_digest(ordinary.world()),
        project_phoenix::sim_digest::world_digest(perturbed.world())
    );
    // Long-run digest perturbation remains tests/registration_order_determinism.rs;
    // no additional long mission is needed for this graph-only probe.
}

#[derive(Resource)]
struct Probe;
fn first(_: ResMut<Probe>) {}
fn second(_: ResMut<Probe>) {}
fn third(_: Res<Probe>) {}
#[test]
fn actual_conflicts_are_reported_and_explicit_order_removes_them() {
    let mut app = App::new();
    app.insert_resource(Probe)
        .add_systems(FixedUpdate, (first, second));
    let debt = fixed_update_census(&mut app).unwrap();
    assert_eq!(debt.len(), 1);
    assert!(debt[0].access.iter().any(|name| name.ends_with("::Probe")));
    app.add_systems(FixedUpdate, third.after(first).after(second));
    assert_eq!(fixed_update_census(&mut app).unwrap(), debt);
    assert!(require_unambiguous_fixed_update(&mut app).is_err());
    let mut ordered = App::new();
    ordered
        .insert_resource(Probe)
        .add_systems(FixedUpdate, (first, second).chain());
    assert!(fixed_update_census(&mut ordered).unwrap().is_empty());
    require_unambiguous_fixed_update(&mut ordered).unwrap();
}

#[test]
fn debt_comparison_retains_access_changes_and_duplicate_instances() {
    let row = Ambiguity {
        systems: ["a".into(), "b".into()],
        access: vec!["R".into()],
    };
    assert_eq!(
        uncovered(&[row.clone(), row.clone()], std::slice::from_ref(&row)),
        vec![row.clone()]
    );
    let changed = Ambiguity {
        access: vec!["S".into()],
        ..row.clone()
    };
    assert_eq!(
        uncovered(std::slice::from_ref(&changed), &[row]),
        vec![changed]
    );
}

fn exclusive_probe(_: &mut World) {}
#[test]
fn access_shrinks_without_losing_instance_limits_or_matching_alternatives() {
    let row = |access: &[&str]| Ambiguity {
        systems: ["first".into(), "second".into()],
        access: access.iter().map(|name| (*name).into()).collect(),
    };
    let a = row(&["A"]);
    let b = row(&["B"]);
    let ab = row(&["A", "B"]);
    assert!(uncovered(std::slice::from_ref(&a), std::slice::from_ref(&ab)).is_empty());
    assert_eq!(
        uncovered(std::slice::from_ref(&ab), std::slice::from_ref(&a)),
        vec![ab.clone()]
    );
    assert_eq!(
        uncovered(std::slice::from_ref(&b), std::slice::from_ref(&a)),
        vec![b.clone()]
    );
    assert_eq!(
        uncovered(&[a.clone(), a.clone()], std::slice::from_ref(&ab)).len(),
        1
    );
    // A greedily takes AB first; an augmenting path must move it to A for B.
    assert!(uncovered(&[a.clone(), b.clone()], &[ab.clone(), a.clone()]).is_empty());
    assert!(uncovered(&[b.clone(), a.clone()], &[ab.clone(), a.clone()]).is_empty());
    let exclusive = row(&["<exclusive World access>"]);
    assert!(uncovered(std::slice::from_ref(&ab), std::slice::from_ref(&exclusive)).is_empty());
    assert_eq!(
        uncovered(std::slice::from_ref(&exclusive), &[ab]),
        vec![exclusive]
    );
    let unrelated = Ambiguity {
        systems: ["different".into(), "second".into()],
        ..a.clone()
    };
    assert_eq!(
        uncovered(std::slice::from_ref(&unrelated), &[a]),
        vec![unrelated]
    );
}

#[test]
fn exclusive_access_is_explicit_and_instance_multiplicity_survives() {
    let mut app = App::new();
    app.insert_resource(Probe)
        .add_systems(FixedUpdate, (first, first, exclusive_probe));
    let rows = fixed_update_census(&mut app).unwrap();
    assert_eq!(rows.len(), 3);
    let exclusive: Vec<_> = rows
        .iter()
        .filter(|row| row.access == ["<exclusive World access>"])
        .collect();
    assert_eq!(exclusive.len(), 2);
    assert_eq!(
        exclusive[0], exclusive[1],
        "same-named instances retain multiplicity"
    );
}

#[derive(Resource, Default)]
struct FirstDraw(u32);
#[derive(Resource, Default)]
struct SecondDraw(u32);
fn first_stream_writer(
    rng: project_phoenix::sim_rng::LiveStream<
        '_,
        { project_phoenix::sim_rng::SimStream::BeamDamage as usize },
    >,
    mut output: ResMut<FirstDraw>,
) {
    output.0 =
        project_phoenix::sim_rng::with_live_stream(rng.as_deref(), |stream| stream.next_u32());
}
fn second_stream_writer(
    rng: project_phoenix::sim_rng::LiveStream<
        '_,
        { project_phoenix::sim_rng::SimStream::BeamDamage as usize },
    >,
    mut output: ResMut<SecondDraw>,
) {
    output.0 =
        project_phoenix::sim_rng::with_live_stream(rng.as_deref(), |stream| stream.next_u32());
}

#[test]
fn same_stream_writers_expose_a_real_conflict_until_ordered() {
    use project_phoenix::sim_rng::{SeedSource, SimRng, SimStream};
    let build = || {
        let mut app = App::new();
        app.insert_sim_rng(SimRng::new(1400, SeedSource::Cli))
            .init_resource::<FirstDraw>()
            .init_resource::<SecondDraw>();
        app
    };
    let mut unordered = build();
    unordered.add_systems(FixedUpdate, (first_stream_writer, second_stream_writer));
    let debt = fixed_update_census(&mut unordered).unwrap();
    assert_eq!(debt.len(), 1);
    let mut names = [
        std::any::type_name_of_val(&first_stream_writer).to_owned(),
        std::any::type_name_of_val(&second_stream_writer).to_owned(),
    ];
    names.sort();
    assert_eq!(debt[0].systems, names);
    assert_eq!(
        debt[0].access,
        [std::any::type_name::<project_phoenix::sim_rng::BeamRng>()]
    );
    let mut ordered = build();
    ordered.add_systems(
        FixedUpdate,
        (first_stream_writer, second_stream_writer).chain(),
    );
    assert!(fixed_update_census(&mut ordered).unwrap().is_empty());
    ordered.world_mut().run_schedule(FixedUpdate);
    let reference = SimRng::new(1400, SeedSource::Cli);
    let first = reference.stream(SimStream::BeamDamage).next_u32();
    let second = reference.stream(SimStream::BeamDamage).next_u32();
    assert_eq!(ordered.world().resource::<FirstDraw>().0, first);
    assert_eq!(ordered.world().resource::<SecondDraw>().0, second);
}

#[derive(Resource, Default)]
struct FirstIdentity(String);
#[derive(Resource, Default)]
struct SecondIdentity(String);
fn first_identity_writer(
    mint: project_phoenix::world_id::LiveMint<
        '_,
        { project_phoenix::world_id::IdNamespace::Projectile as usize },
    >,
    mut output: ResMut<FirstIdentity>,
) {
    output.0 = project_phoenix::world_id::mint_live_id_with(
        mint.as_deref(),
        project_phoenix::world_id::IdNamespace::Projectile,
    );
}
fn second_identity_writer(
    mint: project_phoenix::world_id::LiveMint<
        '_,
        { project_phoenix::world_id::IdNamespace::Projectile as usize },
    >,
    mut output: ResMut<SecondIdentity>,
) {
    output.0 = project_phoenix::world_id::mint_live_id_with(
        mint.as_deref(),
        project_phoenix::world_id::IdNamespace::Projectile,
    );
}

#[test]
fn same_tick_mint_writers_expose_a_real_conflict_until_ordered() {
    use project_phoenix::world_id::{IdNamespace, WorldIdMint};
    let build = || {
        let mut app = App::new();
        let mint = WorldIdMint::default();
        mint.begin_tick(42);
        app.insert_world_id_mint(mint)
            .init_resource::<FirstIdentity>()
            .init_resource::<SecondIdentity>();
        app
    };
    let mut unordered = build();
    unordered.add_systems(FixedUpdate, (first_identity_writer, second_identity_writer));
    let debt = fixed_update_census(&mut unordered).unwrap();
    assert_eq!(debt.len(), 1);
    let mut names = [
        std::any::type_name_of_val(&first_identity_writer).to_owned(),
        std::any::type_name_of_val(&second_identity_writer).to_owned(),
    ];
    names.sort();
    assert_eq!(debt[0].systems, names);
    assert_eq!(
        debt[0].access,
        [std::any::type_name::<
            project_phoenix::world_id::ProjectileMint,
        >()]
    );
    let mut ordered = build();
    ordered.add_systems(
        FixedUpdate,
        (first_identity_writer, second_identity_writer).chain(),
    );
    assert!(fixed_update_census(&mut ordered).unwrap().is_empty());
    ordered.world_mut().run_schedule(FixedUpdate);
    let reference = WorldIdMint::default();
    reference.begin_tick(42);
    let first = reference.mint(IdNamespace::Projectile).render();
    let second = reference.mint(IdNamespace::Projectile).render();
    assert_ne!(first, second);
    assert_eq!(ordered.world().resource::<FirstIdentity>().0, first);
    assert_eq!(ordered.world().resource::<SecondIdentity>().0, second);
}

#[test]
#[ignore = "explicit instance graph capture; no ledger or simulation execution"]
fn print_fixed_update_instance_graph() {
    let mut app = simulation();
    let capture = project_phoenix::headless::determinism_audit::graph::fixed_update_graph(&mut app)
        .expect("fresh actual FixedUpdate graph");
    let census = fixed_update_census(&mut app).unwrap();
    assert_eq!(capture.conflicts.len(), census.len());
    println!(
        "GRAPH_CAPTURE_BEGIN\n{}\nGRAPH_CAPTURE_END",
        serde_json::to_string_pretty(&capture).unwrap()
    );
}

#[test]
fn instance_graph_retains_duplicate_systems_sets_and_deferred_paths() {
    use project_phoenix::headless::determinism_audit::graph::fixed_update_graph;
    #[derive(Resource, Default)]
    struct NeverRun(u32);
    #[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Phase {
        Parent,
        Before,
        After,
    }
    fn writer(mut commands: Commands, mut count: ResMut<NeverRun>) {
        count.0 += 1;
        commands.spawn_empty();
    }
    fn reader(_: Res<NeverRun>) {}
    fn fixture() -> App {
        let mut app = App::new();
        app.init_resource::<NeverRun>();
        app.configure_sets(
            FixedUpdate,
            (Phase::Before, Phase::After).chain().in_set(Phase::Parent),
        );
        app.add_systems(
            FixedUpdate,
            (
                writer.in_set(Phase::Before),
                writer.in_set(Phase::Before),
                reader.in_set(Phase::After),
            ),
        );
        app
    }
    fn reaches(edges: &[[String; 2]], from: &str, to: &str) -> bool {
        let mut pending = vec![from.to_owned()];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(at) = pending.pop() {
            if at == to {
                return true;
            }
            if seen.insert(at.clone()) {
                pending.extend(
                    edges
                        .iter()
                        .filter(|edge| edge[0] == at)
                        .map(|edge| edge[1].clone()),
                );
            }
        }
        false
    }
    let expected = fixed_update_census(&mut fixture()).unwrap();
    let mut app = fixture();
    let graph = fixed_update_graph(&mut app).unwrap();
    assert_eq!(
        app.world().resource::<NeverRun>().0,
        0,
        "capture must not execute systems"
    );
    assert_eq!(
        fixed_update_census(&mut app).unwrap(),
        expected,
        "observer cannot alter debt"
    );
    let writers: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| n.name.ends_with("::writer"))
        .collect();
    assert_eq!(writers.len(), 2);
    assert_ne!(writers[0].id, writers[1].id);
    assert!(writers.iter().all(|n| n.has_deferred));
    let reader = graph
        .nodes
        .iter()
        .find(|n| n.name.ends_with("::reader"))
        .unwrap();
    let barriers: Vec<_> = graph.nodes.iter().filter(|n| n.apply_deferred).collect();
    assert!(
        !barriers.is_empty(),
        "ordinary Commands chain needs a real deferred boundary"
    );
    for writer in writers {
        assert!(barriers.iter().any(|barrier| reaches(
            &graph.effective_dependency,
            &writer.id,
            &barrier.id
        ) && reaches(
            &graph.effective_dependency,
            &barrier.id,
            &reader.id
        )));
    }
    assert!(!graph.hierarchy.is_empty());
    assert!(!graph.dependency.is_empty());
    let ids: std::collections::BTreeSet<_> = graph.nodes.iter().map(|n| &n.id).collect();
    assert_eq!(ids.len(), graph.nodes.len());
    for edge in graph
        .hierarchy
        .iter()
        .chain(&graph.dependency)
        .chain(&graph.effective_dependency)
    {
        assert!(
            ids.contains(&edge[0]) && ids.contains(&edge[1]),
            "dangling endpoint {edge:?}"
        );
    }
    assert_eq!(graph.conflicts.len(), expected.len());
    assert!(
        !graph.conflicts.is_empty(),
        "duplicate mutable writers remain conflicting"
    );
}

#[test]
fn instance_graph_refuses_initialized_schedule_without_forcing_rebuild() {
    let mut app = App::new();
    app.add_systems(FixedUpdate, || {});
    fixed_update_census(&mut app).unwrap();
    assert!(
        project_phoenix::headless::determinism_audit::graph::fixed_update_graph(&mut app)
            .unwrap_err()
            .contains("uninitialized")
    );
}

fn cycle_stream_writer(
    rng: project_phoenix::sim_rng::LiveStream<
        { project_phoenix::sim_rng::SimStream::BeamCycleJitter as usize },
    >,
    mut output: ResMut<SecondDraw>,
) {
    output.0 =
        project_phoenix::sim_rng::with_live_stream(rng.as_deref(), |stream| stream.next_u32());
}

#[test]
fn different_live_streams_need_no_order_and_keep_the_aggregate_positions() {
    use project_phoenix::sim_rng::{SeedSource, SimRng, SimStream};
    let mut app = App::new();
    app.insert_sim_rng(SimRng::new(1400, SeedSource::Cli))
        .init_resource::<FirstDraw>()
        .init_resource::<SecondDraw>()
        .add_systems(FixedUpdate, (first_stream_writer, cycle_stream_writer));
    assert!(fixed_update_census(&mut app).unwrap().is_empty());
    app.world_mut().run_schedule(FixedUpdate);
    let expected = SimRng::new(1400, SeedSource::Cli);
    assert_eq!(
        app.world().resource::<FirstDraw>().0,
        expected.stream(SimStream::BeamDamage).next_u32()
    );
    assert_eq!(
        app.world().resource::<SecondDraw>().0,
        expected.stream(SimStream::BeamCycleJitter).next_u32()
    );
    assert_eq!(app.world().resource::<SimRng>().state(), expected.state());
}

fn entity_identity_writer(
    mint: project_phoenix::world_id::LiveMint<
        '_,
        { project_phoenix::world_id::IdNamespace::Entity as usize },
    >,
    mut output: ResMut<SecondIdentity>,
) {
    output.0 = project_phoenix::world_id::mint_live_id_with(
        mint.as_deref(),
        project_phoenix::world_id::IdNamespace::Entity,
    );
}

#[test]
fn different_namespace_writers_need_no_order_and_preserve_identity_assignment() {
    use project_phoenix::world_id::{IdNamespace, WorldIdMint};
    let mut app = App::new();
    let mint = WorldIdMint::default();
    mint.begin_tick(42);
    app.insert_world_id_mint(mint)
        .init_resource::<FirstIdentity>()
        .init_resource::<SecondIdentity>()
        .add_systems(FixedUpdate, (first_identity_writer, entity_identity_writer));
    assert!(fixed_update_census(&mut app).unwrap().is_empty());
    app.world_mut().run_schedule(FixedUpdate);
    let reference = WorldIdMint::default();
    reference.begin_tick(42);
    assert_eq!(
        app.world().resource::<FirstIdentity>().0,
        reference.mint(IdNamespace::Projectile).render()
    );
    assert_eq!(
        app.world().resource::<SecondIdentity>().0,
        reference.mint(IdNamespace::Entity).render()
    );
    assert_eq!(
        app.world().resource::<WorldIdMint>().state(),
        reference.state()
    );
}
