//! Authoring a GM role preset may not move the simulation (issue #1477,
//! criterion 4).
//!
//! # The claim, and why it is proved rather than asserted
//!
//! A `[[gm_role_preset]]` block is presentation-only: it is personal browser
//! narrowing a Game Master selects for themselves, and a widget selects among
//! surfaces this build already draws. `src/world/config.rs` says so in prose and
//! `gui/gm-role-presets.js` says so in its header, but the Workshop's new preset
//! form (#1477) puts that claim in an AUTHOR's hands — so the claim has to be
//! measurable. If any authored preset field reached `GmOperator`, a `GmAction`,
//! a snapshot or the sim, then an author adding a widget to a desk would be
//! changing the mission that desk is watching, and the bug would be invisible
//! in the Workshop and fatal in a fleet.
//!
//! So this runs the same seeded world twice in one process, changing exactly one
//! thing between the runs — whether the world carries authored presets — and
//! compares `sim_digest::world_digest` on **every tick**.
//!
//! Two things keep that comparison from being vacuous. The loaded `WorldConfig`
//! of the two arms is asserted IDENTICAL once `gm_role_presets` is set aside, so
//! the preset block is shown to change no other authored input — nothing a GM's
//! authority, the Admission gate or the script reads. And a THIRD arm is run
//! over the same world with one hull moved two metres, whose digest series must
//! DIFFER: a `world_digest` blind to the world would otherwise pass the
//! neutrality comparison for the wrong reason.
//!
//! The authored copy is not hand-typed: it is built by the Workshop's own
//! `presets::new_preset_source` and `presets::compose`, one `AppendTable` edit
//! per widget exactly as one Apply press is, so what this proves neutral is what
//! the panel actually writes. All four widget types are covered, because a
//! fifth surface is not authorable (`GM_WIDGET_TYPES` is closed on purpose).
//!
//! # Why it is its own test binary
//!
//! The same reason `tests/gm_presentation_neutrality.rs` is: comparing two Apps
//! tick for tick needs `--deterministic`, which pins the scheduler with a
//! one-thread `TaskPoolOptions`; Bevy's task pools are process-global, so a
//! determinism claim made in a binary where other tests build apps first is a
//! claim about whoever won that race. Do not add unrelated tests here.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;

use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::sim_tick::SimTick;
use project_phoenix::workshop::document::{Edit, EditRequest, Segment};
use project_phoenix::workshop::presets;
use project_phoenix::workshop::WorkshopDependencies;
use project_phoenix::world::config::{parse_world, GM_WIDGET_TYPES};

/// The probe the other neutrality guards use: two crewed hulls and a scripted
/// hostile, so the comparison covers per-victim RNG draws and mid-run mints
/// rather than hulls coasting in a straight line.
const WORLD: &str = "assets/worlds/probe_fleet_duel.toml";
/// The draft path the Workshop edits under. Only the preset rules read it.
const DRAFT: &str = "assets/worlds/probe_fleet_duel.toml";
const TICKS: u64 = 400;
const SEED: u64 = 1_477_026;

/// One widget of each type, with the facets that type OWNS. No `ship`: this
/// world names its hulls with `id` rather than an `[[entity]] name`, and a
/// widget narrowing to a name no world declares is a refusal in its own right.
fn widget_fields(kind: &str) -> Vec<(String, String)> {
    let mut fields = vec![
        ("id".to_owned(), format!("\"{kind}-card\"")),
        ("type".to_owned(), format!("\"{kind}\"")),
        (
            "label".to_owned(),
            format!("\"world.probe_fleet_duel.widget.{kind}\""),
        ),
    ];
    match kind {
        "attention" => {
            fields.push(("band".to_owned(), "\"urgent\"".to_owned()));
            fields.push(("category".to_owned(), "\"pending_comms\"".to_owned()));
        }
        "actions" => fields.push((
            "actions".to_owned(),
            "[\"gm-session-pause\", \"gm-session-resume\"]".to_owned(),
        )),
        "note" => fields.push((
            "text".to_owned(),
            "\"world.probe_fleet_duel.note.brief\"".to_owned(),
        )),
        _ => {}
    }
    fields
}

/// The shipped world with one preset composing all four widget types, written
/// through the Workshop's own authoring path.
fn authored(stripped: &str) -> String {
    let block = presets::new_preset_source("tactical", "world.probe_fleet_duel.preset.tactical")
        .expect("the new-preset skeleton is one the runtime reads");
    let mut source = format!("{stripped}\n{block}");
    let dependencies = WorkshopDependencies::default();
    for kind in GM_WIDGET_TYPES {
        let files = BTreeMap::from([(DRAFT.to_owned(), source.clone())]);
        source = presets::compose(
            &files,
            &dependencies,
            &EditRequest {
                document_path: DRAFT.to_owned(),
                expected_source: source.clone(),
                edits: vec![Edit::AppendTable {
                    path: vec![
                        Segment::Key("gm_role_preset".to_owned()),
                        Segment::Index(0),
                        Segment::Key("widget".to_owned()),
                    ],
                    fields: widget_fields(kind),
                }],
            },
        )
        .unwrap_or_else(|error| panic!("the panel's own {kind} widget must be accepted: {error}"));
    }
    source
}

/// A temp world that removes itself. `build_headless_app` loads a world by
/// PATH, so each arm needs a file — but a fixture per run, keyed by pid, would
/// leave two behind on every invocation. The repo's other temp-world tests
/// (`tests/headless_runner.rs`, `tests/replay_binary.rs`) remove theirs, and a
/// `Drop` does it here on the panicking path too.
struct Fixture(std::path::PathBuf);

impl Fixture {
    fn write(name: &str, source: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("phoenix-1477-{name}-{}.toml", std::process::id()));
        std::fs::write(&path, source).expect("write the world fixture");
        Self(path)
    }

    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn args(world_path: String) -> HeadlessArgs {
    HeadlessArgs {
        world_path,
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: TICKS,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

/// Fold the authoritative digest after every tick.
fn digests(world_path: String) -> Vec<(u64, u64)> {
    let mut app = build_headless_app(&args(world_path)).expect("app should build");
    let mut digests = Vec::with_capacity(TICKS as usize);
    for _ in 0..TICKS {
        run(&mut app, 1);
        let tick = app.world().resource::<SimTick>().0;
        digests.push((tick, world_digest(app.world())));
    }
    digests
}

#[test]
fn a_world_carrying_authored_gm_role_presets_runs_the_identical_simulation() {
    let stripped = std::fs::read_to_string(WORLD).expect("the shipped probe world");
    assert!(
        !stripped.contains("gm_role_preset"),
        "the stripped arm must author no preset at all"
    );
    let authored = authored(&stripped);

    // The two arms differ in the preset block and in NOTHING else.
    assert!(
        authored.starts_with(&stripped),
        "the authored arm must be the same world with presets appended"
    );
    let mut parsed = parse_world(&authored).expect("the authored world loads");
    assert_eq!(parsed.gm_role_presets.len(), 1);
    assert_eq!(
        parsed.gm_role_presets[0]
            .widget
            .iter()
            .map(|widget| widget.kind.as_str())
            .collect::<Vec<_>>(),
        GM_WIDGET_TYPES.to_vec(),
        "every widget type this build draws must be under test"
    );
    let bare = parse_world(&stripped).expect("the shipped world loads");
    assert!(bare.gm_role_presets.is_empty());
    // The LOADED world is identical once the presets are set aside: authoring a
    // preset changes no other authored value, so nothing downstream of the world
    // — a GM's authority, the Admission gate, the roster, the script — reads a
    // different input because of it. The digest comparison below then shows the
    // one field that DID change moves nothing.
    parsed.gm_role_presets.clear();
    assert_eq!(
        format!("{parsed:?}"),
        format!("{bare:?}"),
        "a preset block may change nothing in the loaded world but the presets"
    );

    // A positive control: the same digest fold over a world whose CONTENT moved,
    // proving the measurement responds to the world at all. Without it, a digest
    // blind to the world would pass the neutrality comparison for the wrong
    // reason.
    let nudged = stripped.replace(
        "transform = { position = [60.0, 0.0, 0.0] }",
        "transform = { position = [62.0, 0.0, 0.0] }",
    );
    assert_ne!(
        nudged, stripped,
        "the control must actually move the probe world's second hull"
    );

    let stripped_world = Fixture::write("stripped", &stripped);
    let authored_world = Fixture::write("authored", &authored);
    let nudged_world = Fixture::write("nudged", &nudged);
    let without = digests(stripped_world.path());
    let with = digests(authored_world.path());
    assert_ne!(
        without,
        digests(nudged_world.path()),
        "the digest must respond to world content, or proving it does not respond \
         to a preset would prove nothing"
    );
    assert_eq!(
        without.len(),
        with.len(),
        "both runs should reach the same tick"
    );
    assert!(
        without.last().is_some_and(|(tick, _)| *tick > 1),
        "a run of one tick would prove nothing"
    );
    // A run whose digest never moves is a world where nothing happened, and
    // two of those would agree for the wrong reason.
    assert!(
        without
            .iter()
            .map(|(_, digest)| *digest)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1,
        "the probe must actually simulate for the comparison to mean anything"
    );
    for (left, right) in without.iter().zip(with.iter()) {
        assert_eq!(
            left, right,
            "the digest moved when the world carried authored GM role presets: \
             a preset is presentation, and this is the proof it stays that way"
        );
    }
}
