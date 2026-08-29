//! Command-line parsing for `phoenix-host`.
//!
//! Hand-rolled and pure, for the same two reasons `headless::args` is: this
//! crate ships to `wasm32-unknown-unknown` under `lto = true`, so an argument
//! parser is a real cost paid by the browser build; and a pure function over an
//! iterator is directly unit-testable, which is how the rest of the crate is
//! tested.

/// Where the client bundle comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientSource {
    /// Serve a built bundle from disk (a `trunk dist/`).
    Bundled { dir: String },
    /// Serve no assets at all — the client is hosted elsewhere (Cloudflare
    /// Pages, GitHub Pages) and only the manifest/stamp endpoints are served.
    /// PRD #855 story 2's "use compatible hosted clients" half; the version pin
    /// still applies, at request time, because that is exactly the case where a
    /// mismatched pair is likeliest.
    Hosted,
}

/// A parsed `phoenix-host` invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostArgs {
    pub addr: String,
    pub client: ClientSource,
    /// Scenario manifest to serve, relative to `content_dir`. Choosing this
    /// file IS the catalogue restriction (issue #917) — the same lever
    /// `?manifest=` pulls in the browser.
    pub manifest: String,
    /// Root the manifest and its referenced world TOMLs are read from.
    pub content_dir: String,
    /// Serve a bundle whose `[content]` identity could not be read, instead of
    /// refusing to start. For serving a bundle that predates the identity
    /// block; never for papering over a real mismatch, which this does not
    /// suppress.
    pub skip_bundle_check: bool,
    /// The simulation half (issue #1121): what `--world` selected, or `None`
    /// for the delivery-only host PRD #855 shipped.
    ///
    /// This is the one flag that changes what the process *is*, which is why
    /// #1121 evolved this binary rather than adding a parallel one: the bundle
    /// serving, the catalogue restriction and the startup version pin are the
    /// same code either way, and a second binary would have had to either fork
    /// them or link them anyway.
    pub sim: Option<SimArgs>,
    /// `--setup`: enumerate the connected monitors, print their stable
    /// identities and geometry, validate `--profile` against them if one was
    /// given, and exit (issue #1123). A standalone diagnostic — it needs no
    /// `--world`, and when present it short-circuits the run.
    pub setup: bool,
    /// `--profile <PATH>`: a bridge-display profile (issue #1123), relative to
    /// the working directory. With `--world` the authoritative host applies it
    /// (viewscreen and Station monitors as borderless-fullscreen surfaces); with
    /// `--setup` it is validated against the connected displays. `None` keeps the
    /// single-window #1121 behaviour. Kept at the top level rather than in
    /// [`SimArgs`] because `--setup` reads it without a world.
    pub profile: Option<String>,
}

/// The authoritative simulation's arguments, present only when `--world` was
/// given (issue #1121).
///
/// A separate struct rather than five `Option` fields on [`HostArgs`] because
/// they travel together: every one of them is meaningless without a world, and
/// `Option<SimArgs>` makes "is this an authoritative host" a single question
/// with a single answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimArgs {
    /// Root world TOML, relative to `content_dir`.
    pub world: String,
    /// The player's hull. `None` takes the world's first `available_ships`
    /// entry — what the browser's ship picker pre-selects.
    pub ship: Option<String>,
    /// Overrides the world's `[global] seed`.
    pub seed: Option<u64>,
    /// `--log` spec, in `phoenix-headless`'s exact grammar.
    pub log_spec: String,
    /// `--log-entity` names, in `phoenix-headless`'s exact grammar.
    pub log_entity: String,
    /// Start the mission with nobody connected, every station on `Backfill`.
    pub solo: bool,
    /// Local Station panes to open, one per `--pane <NAME>`, in the order given
    /// (issue #1122).
    ///
    /// The value is the **participant's name**, not a station: a pane joins the
    /// lobby exactly as a phone does and claims a Station from inside its own
    /// console, because "which seat" is a lobby decision and giving a native
    /// participant a way to skip it would be the first bypass.
    ///
    /// A name rather than a generated label because a participant name is
    /// player-visible text, and this repository keeps player-visible text in
    /// `assets/strings/strings.csv` rather than in Rust. A crew member's own
    /// name is neither — it is operator input.
    pub panes: Vec<String>,
    /// The rendezvous service to register with, so browser clients can join
    /// this native host over the WebSocket game relay (issue #1113). `None`
    /// keeps the pre-#1113 behaviour: a host nobody can connect to, which is
    /// exactly what `--solo` is for.
    pub rendezvous: Option<String>,
    /// The `Origin` header the rendezvous socket claims.
    ///
    /// Required with `--rendezvous`, and deliberately not defaulted: the
    /// service refuses an upgrade whose Origin is not on its deployed
    /// allowlist (worker-rendezvous/src/index.js), and a native host does not
    /// have one the way a page does. Which origin a given deployment allows is
    /// an operator decision recorded in docs/delivery-checklist.md §3a, not
    /// something this binary can invent — inventing one would produce a 403
    /// whose cause is invisible.
    pub origin: Option<String>,
}

/// What `parse_args` decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    Help,
    Run(Box<HostArgs>),
}

pub const DEFAULT_ADDR: &str = "0.0.0.0:8080";
pub const DEFAULT_MANIFEST: &str = "assets/scenarios.toml";
pub const DEFAULT_CONTENT_DIR: &str = ".";

pub const HELP: &str = "\
phoenix-host — serve the Phoenix client bundle, the content manifest and the
scenario catalogue from a native PC process instead of a browser tab, and
(with --world) run the authoritative simulation with a native viewscreen.

USAGE:
    phoenix-host [OPTIONS]

SIMULATION
    --world <PATH>        Run the authoritative simulation for this world,
                          relative to --content-dir, and open a native
                          viewscreen window. Without it this process serves
                          delivery only, exactly as it always has.
    --ship <PATH>         The player's hull [default: the world's first
                          [[available_ships]] entry]
    --seed <N>            Override the world's [global] seed
    --solo                Start the mission immediately with nobody connected;
                          every station runs on Backfill. Without it the host
                          waits in the lobby for participants to ready up,
                          which needs --rendezvous below — a host with neither
                          waits for a crew that has no way in, and says so.
    --log <SPEC>          Log filter, e.g. info,ai=debug,admit=trace
    --log-entity <NAMES>  Restrict logging to these entity names

LOCAL STATIONS (requires a build with --features ultralight)
    --pane <NAME>         Open a local Station pane for a participant of this
                          name, in an embedded browser view showing the ordinary
                          console surface. Repeatable; panes are tiled left to
                          right. Each one joins, claims a Station and readies
                          through exactly the contracts a phone does, with its
                          own minted session token. Needs --client-dir: a pane
                          loads the client bundle this host serves.

BRIDGE DISPLAYS (issue #1123)
    --setup               Enumerate the connected monitors, print their stable
                          hardware identities, geometry and current assignment,
                          then exit. Validates --profile against them if given.
                          A standalone diagnostic — needs no --world, and
                          refuses every simulation/crew flag (--world, --ship,
                          --seed, --solo, --pane, --log, --log-entity,
                          --rendezvous, --origin) rather than silently
                          discarding them.
    --profile <PATH>      A bridge-display profile (TOML). With --world the host
                          covers every configured monitor with one borderless
                          full-screen surface — the viewscreen, or a Station
                          hosting one or two panes. With --setup it is validated
                          against the connected displays. A missing or changed
                          monitor is reported, never silently re-homed.

CREW (issue #1113)
    --rendezvous <URL>    Register with this rendezvous service so browser
                          clients can join, e.g.
                          https://phoenix-rendezvous.project-phoenix.workers.dev
                          The five-letter code the service issues is printed at
                          startup. A native host has no WebRTC, so every crew
                          member is carried over the service's WebSocket game
                          relay; it registers saying so, and joiners skip the
                          direct ladder rather than spending ninety seconds
                          discovering it.
    --origin <URL>        The Origin header that socket claims. REQUIRED with
                          --rendezvous and deliberately not defaulted: the
                          service refuses an upgrade whose Origin is not on its
                          deployed allowlist, and which origins a deployment
                          allows is an operator decision recorded in
                          docs/delivery-checklist.md §3a.

CLIENT
    --client-dir <PATH>   Serve a built client bundle from this directory
                          (a `trunk dist/`). Omit to serve no assets at all and
                          publish only the manifest/stamp endpoints, for a
                          client hosted elsewhere.
    --skip-bundle-check   Start even when the bundle's [content] identity
                          cannot be read. Does NOT suppress a real mismatch.

CONTENT
    --manifest <PATH>     Scenario manifest to serve, relative to --content-dir
                          [default: assets/scenarios.toml]. Point it at
                          assets/scenarios.demo.toml for the curated public
                          catalogue.
    --content-dir <PATH>  Root the manifest and its world TOMLs are read from
                          [default: .]

NETWORK
    --addr <ADDR>         Bind address [default: 0.0.0.0:8080] — accepts
                          connections from the LAN by default; Windows will
                          prompt to allow it through the firewall on first run.
                          Use 127.0.0.1:<port> to restrict to this machine only.

    -h, --help            Show this help

ENDPOINTS
    /host/stamp.json      This host's protocol + content version stamp
    /host/manifest.json   The version-pinned content manifest and catalogue.
                          Callers must present their own stamp, either as
                          ?protocol=&content_id=&content_epoch= or as the
                          x-phoenix-client-stamp: <protocol>/<id>/<epoch>
                          header. A mismatch is answered 409 with a body
                          naming both sides.
    everything else       Served from --client-dir, when one was given.
";

/// Parse `phoenix-host`'s arguments.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<ParseOutcome, String> {
    let mut addr = DEFAULT_ADDR.to_string();
    let mut client_dir: Option<String> = None;
    let mut manifest = DEFAULT_MANIFEST.to_string();
    let mut content_dir = DEFAULT_CONTENT_DIR.to_string();
    let mut skip_bundle_check = false;
    let mut world: Option<String> = None;
    let mut rendezvous: Option<String> = None;
    let mut origin: Option<String> = None;
    let mut ship: Option<String> = None;
    let mut seed: Option<u64> = None;
    let mut log_spec = String::new();
    let mut log_entity = String::new();
    let mut solo = false;
    let mut panes: Vec<String> = Vec::new();
    let mut setup = false;
    let mut profile: Option<String> = None;

    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::Help),
            "--addr" => addr = value_for(&arg, &mut it)?,
            "--client-dir" => client_dir = Some(value_for(&arg, &mut it)?),
            "--manifest" => manifest = value_for(&arg, &mut it)?,
            "--content-dir" => content_dir = value_for(&arg, &mut it)?,
            "--skip-bundle-check" => skip_bundle_check = true,
            "--setup" => setup = true,
            "--profile" => profile = Some(value_for(&arg, &mut it)?),
            "--world" => world = Some(value_for(&arg, &mut it)?),
            "--ship" => ship = Some(value_for(&arg, &mut it)?),
            "--seed" => {
                let raw = value_for(&arg, &mut it)?;
                seed = Some(
                    raw.parse::<u64>()
                        .map_err(|_| format!("--seed needs a whole number, got {raw:?}"))?,
                );
            }
            "--log" => log_spec = value_for(&arg, &mut it)?,
            "--log-entity" => log_entity = value_for(&arg, &mut it)?,
            "--solo" => solo = true,
            "--pane" => panes.push(value_for(&arg, &mut it)?),
            "--rendezvous" => rendezvous = Some(value_for(&arg, &mut it)?),
            "--origin" => origin = Some(value_for(&arg, &mut it)?),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    // `--setup` is a standalone enumerate-and-exit diagnostic (issue #1123): it
    // opens a hidden window purely to list monitors and exits before a world
    // or a crew transport is ever touched (see phoenix_host.rs, where --setup
    // short-circuits before the world is even read). Given alongside a
    // simulation/crew flag it would otherwise silently discard whatever the
    // operator asked for rather than run it — the exact silent-ignore failure
    // mode the "needs --world" refusals below exist to close for the
    // no-`--world` case. `--profile` is deliberately NOT in this list: it is
    // the one flag `--setup` itself consumes, to validate against the
    // connected displays.
    if setup {
        for (flag, given) in [
            ("--world", world.is_some()),
            ("--ship", ship.is_some()),
            ("--seed", seed.is_some()),
            ("--log", !log_spec.is_empty()),
            ("--log-entity", !log_entity.is_empty()),
            ("--solo", solo),
            ("--pane", !panes.is_empty()),
            ("--rendezvous", rendezvous.is_some()),
            ("--origin", origin.is_some()),
        ] {
            if given {
                return Err(format!(
                    "--setup is a standalone enumerate-and-exit diagnostic and refuses {flag}"
                ));
            }
        }
    }
    // The simulation flags are meaningless without a world, and silently
    // ignoring them would be the worst answer: an operator who wrote `--solo`
    // and no `--world` asked for a mission and would get a file server.
    // A pane loads the client bundle this very process serves, from this
    // process's own address. Without a bundle there is nothing for it to show,
    // and the failure would otherwise be a blank embedded browser rather than a
    // sentence at the prompt.
    if !panes.is_empty() && client_dir.is_none() {
        return Err(
            "--pane needs --client-dir: a local Station pane loads the client bundle this \
             host serves, from this host's own address"
                .to_string(),
        );
    }
    // `--origin` is only meaningful with `--rendezvous`, and a host given one
    // without the other would dial nothing or claim nothing. Refuse both ways at
    // the prompt rather than 403-ing against a live service later.
    if rendezvous.is_some() && origin.is_none() {
        return Err(
            "--rendezvous needs --origin: the service refuses an upgrade whose Origin is \
             not on its deployed allowlist, and a native host has no page origin to send"
                .to_string(),
        );
    }
    if origin.is_some() && rendezvous.is_none() {
        return Err("--origin only means anything with --rendezvous".to_string());
    }
    // A bridge-display profile is applied by a running host (`--world`) or
    // validated by `--setup`; on its own it has nothing to act on. Refuse at the
    // prompt rather than reading a file nothing will use.
    if profile.is_some() && world.is_none() && !setup {
        return Err(
            "--profile needs --world (to apply the bridge display profile) or --setup (to \
             validate it against the connected displays)"
                .to_string(),
        );
    }

    let sim = match world {
        Some(world) => Some(SimArgs {
            world,
            ship,
            seed,
            log_spec,
            log_entity,
            solo,
            panes,
            rendezvous,
            origin,
        }),
        None => {
            for (flag, given) in [
                ("--ship", ship.is_some()),
                ("--seed", seed.is_some()),
                ("--log", !log_spec.is_empty()),
                ("--log-entity", !log_entity.is_empty()),
                ("--solo", solo),
                ("--pane", !panes.is_empty()),
                ("--rendezvous", rendezvous.is_some()),
                ("--origin", origin.is_some()),
            ] {
                if given {
                    return Err(format!("{flag} needs --world — it configures the simulation, and without a world this host serves delivery only"));
                }
            }
            None
        }
    };

    Ok(ParseOutcome::Run(Box::new(HostArgs {
        addr,
        client: match client_dir {
            Some(dir) => ClientSource::Bundled { dir },
            None => ClientSource::Hosted,
        },
        manifest,
        content_dir,
        skip_bundle_check,
        sim,
        setup,
        profile,
    })))
}

fn value_for<I: Iterator<Item = String>>(flag: &str, it: &mut I) -> Result<String, String> {
    it.next()
        .filter(|v| !v.starts_with("--"))
        .ok_or_else(|| format!("{flag} needs a value"))
}

#[cfg(test)]
mod tests {
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
        assert_eq!(sim.world, "assets/worlds/combat_test.toml");
        assert_eq!(sim.ship, None);
        assert_eq!(sim.seed, None);
        assert!(!sim.solo);
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
}
