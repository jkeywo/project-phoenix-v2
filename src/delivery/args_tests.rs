use super::*;

fn parse(args: &[&str]) -> Result<ParseOutcome, String> {
    parse_args(args.iter().map(|s| s.to_string()))
}

fn run(args: &[&str]) -> HostArgs {
    match parse(args).expect("parses") {
        ParseOutcome::Run(a) => *a,
        ParseOutcome::Help => panic!("expected a run, got help"),
    }
}

fn err(args: &[&str]) -> String {
    parse(args).expect_err("expected a refusal")
}

#[test]
fn singleton_values_override_while_panes_and_save_actions_append_in_order() {
    let args = run(&[
        "--world=first.toml",
        "--world=last world.toml",
        "--client-dir=dist",
        "--pane=Ada",
        "--save-create=First",
        "--save-list",
        "--save-rename",
        "slot-a",
        "New name",
        "--pane=Grace",
        "--save-export",
        "slot-a",
        "export path.json",
        "--save-list",
        "--save-create=Second",
        "--save-delete=slot-b",
        "--confirm-delete",
        "--solo",
        "--solo",
    ]);
    let sim = args.sim.unwrap();
    assert_eq!(sim.world.as_deref(), Some("last world.toml"));
    assert_eq!(sim.panes, ["Ada", "Grace"]);
    assert_eq!(
        sim.save_actions,
        vec![
            SaveOperatorAction::Create {
                display_name: "First".into()
            },
            SaveOperatorAction::List,
            SaveOperatorAction::Rename {
                slot_id: "slot-a".into(),
                display_name: "New name".into()
            },
            SaveOperatorAction::Export {
                slot_id: "slot-a".into(),
                path: "export path.json".into()
            },
            SaveOperatorAction::List,
            SaveOperatorAction::Create {
                display_name: "Second".into()
            },
            SaveOperatorAction::Delete {
                slot_id: "slot-b".into(),
                confirmed: true
            },
        ]
    );
    assert!(sim.solo);
}

#[test]
fn action_arity_and_workshop_repetition_are_rejected() {
    for tail in [
        vec!["--save-rename", "slot"],
        vec!["--save-export", "slot"],
        vec!["extra"],
    ] {
        let mut args = vec!["--world=w.toml"];
        args.extend(tail);
        assert!(parse(&args).is_err(), "{args:?}");
    }
    assert!(err(&[
        "--workshop-project=one",
        "--workshop-project=two",
        "--client-dir=dist"
    ])
    .contains("exactly one"));
}

#[test]
fn workshop_selects_one_offline_root_and_never_inherits_lan_delivery() {
    for (flag, project) in [("--workshop-project", true), ("--workshop-mod", false)] {
        let args = run(&[flag, "chosen root", "--client-dir", "dist"]);
        assert_eq!(
            args.workshop,
            Some(WorkshopArgs {
                root: "chosen root".into(),
                project,
                open: None,
            })
        );
        assert_eq!(args.addr, "127.0.0.1:0");
        assert!(args.sim.is_none());
        assert!(!args.setup);
    }
}

#[test]
fn workshop_open_is_presentation_only_and_requires_a_workshop_root() {
    let args = run(&[
        "--workshop-project",
        ".",
        "--client-dir",
        "dist",
        "--workshop-open",
        "panel=models&model=assets%2Fmodels%2Fship.glb",
    ]);
    assert_eq!(
        args.workshop.unwrap().open.as_deref(),
        Some("panel=models&model=assets%2Fmodels%2Fship.glb")
    );
    assert!(err(&["--workshop-open", "panel=models"]).contains("requires"));
    assert!(err(&[
        "--workshop-project",
        ".",
        "--client-dir",
        "dist",
        "--workshop-open",
        "bad\nvalue"
    ])
    .contains("bounded URL query"));
}

#[test]
fn workshop_refuses_live_authority_and_ambiguous_launches() {
    assert!(err(&["--workshop-project", "root"]).contains("--client-dir"));
    for extra in [
        vec!["--workshop-mod", "second"],
        vec!["--world", "w.toml"],
        vec!["--lobby"],
        vec!["--solo"],
        vec!["--pane", "operator"],
        vec!["--addr", "0.0.0.0:8080"],
        vec!["--rendezvous", "wss://example.test"],
        vec!["--profile", "bridge.toml"],
        vec!["--seed", "42"],
        vec!["--resume-save", "slot"],
        vec!["--mod-pack-dir", "shelf"],
    ] {
        let mut args = vec!["--workshop-project", "root", "--client-dir", "dist"];
        args.extend(extra);
        assert!(parse(&args).is_err(), "{args:?}");
    }
}

#[test]
fn a_bare_invocation_serves_the_full_catalogue_to_the_lan_with_no_bundle() {
    let a = run(&[]);
    assert_eq!(a.addr, DEFAULT_ADDR);
    assert_eq!(a.addr, "0.0.0.0:8080");
    assert_eq!(a.manifest, DEFAULT_MANIFEST);
    assert_eq!(a.content_dir, DEFAULT_CONTENT_DIR);
    assert_eq!(a.client, ClientSource::Hosted);
    assert!(!a.skip_bundle_check);
    // PRD #855's host, unchanged: no `--world`, no simulation. Issue #1121
    // added a mode to this binary, not a new default.
    assert_eq!(a.sim, None);
}

#[test]
fn a_world_turns_the_delivery_host_into_an_authoritative_one() {
    let a = run(&[
        "--world",
        "assets/worlds/combat_test.toml",
        "--client-dir",
        "dist",
    ]);
    let sim = a.sim.expect("--world selects the simulation");
    assert_eq!(sim.world.as_deref(), Some("assets/worlds/combat_test.toml"));
    assert_eq!(sim.ship, None);
    assert_eq!(sim.seed, None);
    assert!(!sim.solo);
    assert_eq!(sim.save_dir, DEFAULT_SAVE_DIR);
    // And it is still the same delivery host underneath.
    assert_eq!(
        a.client,
        ClientSource::Bundled {
            dir: "dist".to_string()
        }
    );
    assert_eq!(a.manifest, DEFAULT_MANIFEST);
}

#[test]
fn lobby_selects_the_simulation_without_naming_a_world(/* issue #1326 */) {
    let a = run(&["--lobby", "--client-dir", "dist"]);
    let sim = a.sim.expect("--lobby selects the simulation");
    assert_eq!(
        sim.world, None,
        "the world is chosen from the lobby, not at the prompt"
    );
    // Still the same delivery host underneath, exactly as --world leaves it.
    assert_eq!(
        a.client,
        ClientSource::Bundled {
            dir: "dist".to_string()
        }
    );
    assert_eq!(a.manifest, DEFAULT_MANIFEST);
}

#[test]
fn the_simulation_flags_apply_to_a_lobby_host_too() {
    let a = run(&["--lobby", "--solo", "--seed", "7"]);
    let sim = a.sim.expect("--lobby selects the simulation");
    assert!(sim.solo, "--solo starts the mission once a world is picked");
    assert_eq!(sim.seed, Some(7));
}

#[test]
fn a_bare_invocation_is_still_delivery_only() {
    assert!(
        run(&["--client-dir", "dist"]).sim.is_none(),
        "PRD #855's delivery-only mode is what a host with neither flag is"
    );
}

#[test]
fn a_world_and_a_lobby_are_the_same_decision_twice() {
    let err = err(&["--lobby", "--world", "assets/worlds/combat_test.toml"]);
    assert!(err.contains("--lobby"), "{err}");
    assert!(err.contains("--world"), "{err}");
}

#[test]
fn native_save_directory_is_configurable_only_for_an_authoritative_host() {
    let a = run(&[
        "--world",
        "assets/worlds/combat_test.toml",
        "--save-dir",
        "private/saves",
    ]);
    assert_eq!(a.sim.unwrap().save_dir, "private/saves");

    let err = parse(&["--save-dir", "private/saves"]).unwrap_err();
    assert!(err.contains("--save-dir"), "{err}");
    assert!(err.contains("--world"), "{err}");
}

#[test]
fn a_simulation_flag_alone_still_names_both_ways_in() {
    let err = err(&["--solo"]);
    assert!(err.contains("--world"), "{err}");
    assert!(err.contains("--lobby"), "{err}");
}

#[test]
fn setup_refuses_a_lobby_the_way_it_refuses_a_world() {
    let err = err(&["--setup", "--lobby"]);
    assert!(err.contains("--lobby"), "{err}");
}

#[test]
fn a_bridge_profile_may_be_pinned_for_a_lobby_host() {
    let a = run(&["--lobby", "--profile", "bridge.toml"]);
    assert_eq!(a.profile.as_deref(), Some("bridge.toml"));
}

#[test]
fn a_lobby_host_may_be_given_a_mod_pack_shelf_to_scan() {
    // Issue #1366. The folder IS the file picker on this surface, so the
    // one thing the flag has to do is arrive intact.
    let a = run(&["--lobby", "--mod-pack-dir", "mods"]);
    let sim = a.sim.expect("--lobby selects the simulation");
    assert_eq!(sim.mod_pack_dir.as_deref(), Some("mods"));
    assert!(!sim.mod_pack_dir_is_default);
}

#[test]
fn a_lobby_host_gets_the_default_mod_pack_shelf() {
    let a = run(&["--lobby"]);
    let sim = a.sim.expect("--lobby");
    assert_eq!(sim.mod_pack_dir.as_deref(), Some(DEFAULT_MOD_PACK_DIR));
    assert!(sim.mod_pack_dir_is_default);
}

#[test]
fn non_lobby_modes_do_not_get_the_default_mod_pack_shelf() {
    let delivery = run(&[]);
    assert!(delivery.sim.is_none());

    let direct = run(&["--world", "assets/worlds/combat_test.toml"]);
    let sim = direct.sim.expect("--world selects the simulation");
    assert_eq!(sim.mod_pack_dir, None);
    assert!(!sim.mod_pack_dir_is_default);
}

#[test]
fn a_mod_pack_shelf_without_a_lobby_is_refused_at_the_prompt() {
    // The shelf is offered by the LANDING, which only a world-less host
    // shows, and a pack only widens the catalogue a world is chosen from.
    // Scanning a folder for a shelf nobody can open would let an operator
    // conclude their packs were broken.
    for argv in [
        vec!["--mod-pack-dir", "mods"],
        vec![
            "--world",
            "assets/worlds/combat_test.toml",
            "--mod-pack-dir",
            "mods",
        ],
    ] {
        let err = err(&argv);
        assert!(err.contains("--mod-pack-dir"), "{err}");
        assert!(err.contains("--lobby"), "{err}");
    }
}

#[test]
fn setup_refuses_a_mod_pack_shelf_like_every_other_simulation_flag() {
    let err = err(&["--setup", "--mod-pack-dir", "mods"]);
    assert!(err.contains("--mod-pack-dir"), "{err}");
}

#[test]
fn help_documents_the_exclusive_native_save_directory_claim() {
    assert!(help().contains("one phoenix-host may claim it at a time"));
    assert!(help().contains("authoritative process's lifetime"));
    assert!(help().contains("concurrent native peers need distinct paths"));
}

#[test]
fn native_save_operator_actions_preserve_order_and_values() {
    let sim = run(&[
        "--world",
        "assets/worlds/combat_test.toml",
        "--save-list",
        "--save-create",
        "Before Lyra",
        "--save-rename",
        "slot-a",
        "After Lyra",
        "--save-export",
        "slot-a",
        "exports/lyra.ron",
        "--save-delete",
        "slot-b",
        "--confirm-delete",
    ])
    .sim
    .expect("an authoritative simulation");

    assert_eq!(
        sim.save_actions,
        vec![
            SaveOperatorAction::List,
            SaveOperatorAction::Create {
                display_name: "Before Lyra".into(),
            },
            SaveOperatorAction::Rename {
                slot_id: "slot-a".into(),
                display_name: "After Lyra".into(),
            },
            SaveOperatorAction::Export {
                slot_id: "slot-a".into(),
                path: "exports/lyra.ron".into(),
            },
            SaveOperatorAction::Delete {
                slot_id: "slot-b".into(),
                confirmed: true,
            },
        ]
    );
    assert_eq!(sim.resume_slot, None);
}

#[test]
fn native_save_create_and_resume_are_mutually_exclusive() {
    for args in [
        [
            "--world",
            "w.toml",
            "--save-create",
            "Fresh capture",
            "--resume-save",
            "slot-a",
        ],
        [
            "--world",
            "w.toml",
            "--resume-save",
            "slot-a",
            "--save-create",
            "Fresh capture",
        ],
    ] {
        let error = parse(&args).unwrap_err();
        assert!(error.contains("--save-create"), "{error}");
        assert!(error.contains("--resume-save"), "{error}");
        assert!(error.contains("cannot be combined"), "{error}");
    }

    let resumed = run(&["--world", "w.toml", "--resume-save", "slot-a"])
        .sim
        .expect("resume alone remains a valid native invocation");
    assert_eq!(resumed.resume_slot.as_deref(), Some("slot-a"));
}

#[test]
fn native_delete_requires_an_explicit_confirmation_pair() {
    let missing = parse(&["--world", "w.toml", "--save-delete", "slot-a"]).unwrap_err();
    assert!(missing.contains("--confirm-delete"), "{missing}");

    let orphan = parse(&["--world", "w.toml", "--confirm-delete"]).unwrap_err();
    assert!(orphan.contains("--save-delete"), "{orphan}");
}

#[test]
fn native_catalogue_controls_require_a_world_and_setup_refuses_them() {
    let delivery_only = parse(&["--save-list"]).unwrap_err();
    assert!(delivery_only.contains("--world"), "{delivery_only}");

    let setup = parse(&["--setup", "--save-list"]).unwrap_err();
    assert!(setup.contains("--setup"), "{setup}");
}

#[test]
fn a_crew_needs_both_a_service_and_an_origin_to_claim(/* issue #1113 */) {
    let a = run(&[
        "--world",
        "assets/worlds/combat_test.toml",
        "--rendezvous",
        "https://phoenix-rendezvous.project-phoenix.workers.dev",
        "--origin",
        "https://pp-dev.kiwigamedesign.co.uk",
    ]);
    let sim = a.sim.expect("a simulation");
    assert_eq!(
        sim.rendezvous.as_deref(),
        Some("https://phoenix-rendezvous.project-phoenix.workers.dev")
    );
    assert_eq!(
        sim.origin.as_deref(),
        Some("https://pp-dev.kiwigamedesign.co.uk")
    );
}

#[test]
fn a_service_without_an_origin_is_refused_at_the_prompt() {
    // The service refuses an upgrade whose Origin is not on its deployed
    // allowlist, and a native host has no page origin to send. Defaulting
    // one would produce a 403 whose cause is invisible; refusing here says
    // what is missing while the operator is still looking at the terminal.
    let err = parse(&["--world", "w.toml", "--rendezvous", "https://x.test"]).unwrap_err();
    assert!(err.contains("--origin"), "{err}");
    let err = parse(&["--world", "w.toml", "--origin", "https://x.test"]).unwrap_err();
    assert!(err.contains("--rendezvous"), "{err}");
}

#[test]
fn joining_a_native_ship_requires_a_loaded_world_and_live_bridge() {
    let args = run(&[
        "--world",
        "w.toml",
        "--client-dir",
        "dist",
        "--rendezvous",
        "https://x.test",
        "--origin",
        "https://host.test",
        "--fleet-code",
        "ABCDWXYZ",
    ]);
    assert_eq!(args.sim.unwrap().fleet_code.as_deref(), Some("ABCDWXYZ"));
    for args in [
        vec!["--world", "w.toml", "--fleet-code", "ABCDWXYZ"],
        vec![
            "--lobby",
            "--client-dir",
            "dist",
            "--rendezvous",
            "https://x.test",
            "--origin",
            "https://host.test",
            "--fleet-code",
            "ABCDWXYZ",
        ],
        vec![
            "--world",
            "w.toml",
            "--solo",
            "--client-dir",
            "dist",
            "--rendezvous",
            "https://x.test",
            "--origin",
            "https://host.test",
            "--fleet-code",
            "ABCDWXYZ",
        ],
    ] {
        assert!(parse(&args).unwrap_err().contains("--fleet-code"));
    }
}

#[test]
fn the_crew_flags_need_a_world_like_every_other_simulation_flag() {
    // Delivery-only hosts serve files; there is no mission for a crew to
    // join, and silently ignoring the flags would be the worst answer.
    let err = parse(&[
        "--rendezvous",
        "https://x.test",
        "--origin",
        "https://y.test",
    ])
    .unwrap_err();
    assert!(err.contains("--world"), "{err}");
}

#[test]
fn the_simulation_flags_are_read_into_the_simulation_half() {
    let a = run(&[
        "--world",
        "assets/worlds/combat_test.toml",
        "--ship",
        "assets/entities/alliance_destroyer.toml",
        "--seed",
        "20260894",
        "--solo",
        "--log",
        "info,ai=debug",
        "--log-entity",
        "Ironveil",
    ]);
    let sim = a.sim.expect("a simulation");
    assert_eq!(
        sim.ship.as_deref(),
        Some("assets/entities/alliance_destroyer.toml")
    );
    assert_eq!(sim.seed, Some(20260894));
    assert!(sim.solo);
    assert_eq!(sim.log_spec, "info,ai=debug");
    assert_eq!(sim.log_entity, "Ironveil");
}

#[test]
fn local_station_panes_are_named_participants_in_the_order_they_were_given() {
    // The value is a participant NAME, not a station: a pane joins the
    // lobby and claims a seat from inside its own console, exactly as a
    // phone does. Giving a native participant a way to skip that would be
    // the first bypass.
    let a = run(&[
        "--world",
        "assets/worlds/combat_test.toml",
        "--client-dir",
        "dist",
        "--pane",
        "Ada",
        "--pane",
        "Grace",
    ]);
    let sim = a.sim.expect("a simulation");
    assert_eq!(sim.panes, vec!["Ada".to_string(), "Grace".to_string()]);
}

#[test]
fn a_pane_without_a_client_bundle_is_refused_at_the_prompt() {
    // A pane loads the client bundle this process serves. Without one the
    // failure is a blank embedded browser rather than a sentence.
    let err = parse(&["--world", "w.toml", "--pane", "Ada"]).unwrap_err();
    assert!(err.contains("--pane"), "{err}");
    assert!(err.contains("--client-dir"), "{err}");
}

#[test]
fn frame_stats_rides_with_the_simulation_and_is_refused_without_one() {
    assert!(
        run(&["--world", "w.toml", "--frame-stats"])
            .sim
            .unwrap()
            .frame_stats
    );
    assert!(run(&["--lobby", "--frame-stats"]).sim.unwrap().frame_stats);
    assert!(!run(&["--world", "w.toml"]).sim.unwrap().frame_stats);
    // A delivery-only host has no frame to account for.
    let err = parse(&["--frame-stats"]).unwrap_err();
    assert!(err.contains("--frame-stats"), "{err}");
    // Nor does the enumerate-and-exit diagnostic.
    let err = parse(&["--setup", "--frame-stats"]).unwrap_err();
    assert!(err.contains("--frame-stats"), "{err}");
}

#[test]
fn a_simulation_flag_without_a_world_is_refused_rather_than_ignored() {
    // Silently serving files to an operator who asked for a mission is the
    // worst available answer.
    let err = parse(&["--solo"]).unwrap_err();
    assert!(err.contains("--solo"), "{err}");
    assert!(err.contains("--world"), "{err}");
    assert!(parse(&["--seed", "1"]).unwrap_err().contains("--world"));
    assert!(parse(&["--client-dir", "dist", "--pane", "Ada"])
        .unwrap_err()
        .contains("--world"));
}

#[test]
fn setup_is_a_standalone_diagnostic_that_needs_no_world() {
    // Enumerate-and-exit; no simulation, no bundle required.
    let a = run(&["--setup"]);
    assert!(a.setup);
    assert_eq!(a.sim, None);
    assert_eq!(a.profile, None);
}

#[test]
fn setup_refuses_every_simulation_and_crew_flag_rather_than_ignoring_them() {
    // `--setup` short-circuits before a world or a crew transport is ever
    // touched (phoenix_host.rs). Silently accepting these alongside it
    // would mean an operator who wrote `--setup --world w.toml --solo`
    // gets a monitor list with no acknowledgement their mission never ran.
    for bad in [
        vec!["--setup", "--world", "w.toml"],
        vec!["--setup", "--ship", "assets/entities/x.toml"],
        vec!["--setup", "--seed", "1"],
        vec!["--setup", "--log", "info"],
        vec!["--setup", "--log-entity", "Ironveil"],
        vec!["--setup", "--solo"],
        vec!["--setup", "--pane", "Ada"],
        vec!["--setup", "--rendezvous", "https://x.test"],
        vec!["--setup", "--origin", "https://x.test"],
    ] {
        let err = parse(&bad).unwrap_err();
        assert!(err.contains("--setup"), "{bad:?}: {err}");
    }
}

#[test]
fn setup_still_allows_profile_alongside_it() {
    // --profile is the one flag --setup itself consumes.
    let a = run(&["--setup", "--profile", "bridge.toml"]);
    assert!(a.setup);
    assert_eq!(a.profile.as_deref(), Some("bridge.toml"));
}

#[test]
fn an_output_test_requires_an_explicit_setup_profile_and_surface() {
    for args in [
        vec!["--test-output", "comms"],
        vec!["--setup", "--test-output", "comms"],
    ] {
        assert!(parse(&args)
            .unwrap_err()
            .contains("--test-output needs --setup and --profile"));
    }
    let args = run(&[
        "--setup",
        "--profile",
        "bridge.toml",
        "--test-output",
        "comms",
    ]);
    assert_eq!(args.test_output.as_deref(), Some("comms"));
}

#[test]
fn capture_tests_require_explicit_setup_profile_and_one_action() {
    for flag in ["--meter-microphone", "--preview-camera"] {
        for args in [
            vec![flag, "comms"],
            vec!["--setup", flag, "comms"],
            vec!["--profile", "bridge.toml", flag, "comms"],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        let parsed = run(&["--setup", "--profile", "bridge.toml", flag, "comms"]);
        assert!(parsed.setup);
        for other in ["--test-output", "--meter-microphone", "--preview-camera"] {
            if flag != other {
                assert!(parse(&[
                    "--setup",
                    "--profile",
                    "bridge.toml",
                    flag,
                    "comms",
                    other,
                    "viewscreen"
                ])
                .is_err());
            }
        }
    }
}

#[test]
fn setup_validates_a_profile_without_a_world() {
    let a = run(&["--setup", "--profile", "bridge.toml"]);
    assert!(a.setup);
    assert_eq!(a.profile.as_deref(), Some("bridge.toml"));
    assert_eq!(a.sim, None);
}

#[test]
fn a_world_applies_a_bridge_profile() {
    let a = run(&[
        "--world",
        "assets/worlds/combat_test.toml",
        "--profile",
        "bridge.toml",
    ]);
    assert!(a.sim.is_some());
    assert_eq!(a.profile.as_deref(), Some("bridge.toml"));
    assert!(!a.setup);
}

#[test]
fn a_profile_without_a_world_or_setup_is_refused() {
    // On its own a profile has nothing to apply to or validate against.
    let err = parse(&["--profile", "bridge.toml"]).unwrap_err();
    assert!(err.contains("--profile"), "{err}");
    assert!(err.contains("--world"), "{err}");
    assert!(err.contains("--setup"), "{err}");
}

#[test]
fn a_non_numeric_seed_is_refused_at_parse_time() {
    let err = parse(&["--world", "w.toml", "--seed", "later"]).unwrap_err();
    assert!(err.contains("--seed"), "{err}");
}

#[test]
fn a_client_dir_selects_the_bundled_source() {
    let a = run(&["--client-dir", "dist"]);
    assert_eq!(
        a.client,
        ClientSource::Bundled {
            dir: "dist".to_string()
        }
    );
}

#[test]
fn the_demo_manifest_is_selected_the_same_way_the_browser_selects_it() {
    let a = run(&["--manifest", "assets/scenarios.demo.toml"]);
    assert_eq!(a.manifest, "assets/scenarios.demo.toml");
}

#[test]
fn help_short_circuits_everything_after_it() {
    assert_eq!(parse(&["--help", "--addr"]).unwrap(), ParseOutcome::Help);
}

#[test]
fn a_flag_missing_its_value_is_an_error_rather_than_swallowing_the_next_flag() {
    let err = parse(&["--addr", "--client-dir", "dist"]).unwrap_err();
    assert!(err.contains("--addr"));
}

#[test]
fn an_unknown_argument_is_refused() {
    assert!(parse(&["--nope"]).unwrap_err().contains("--nope"));
}
