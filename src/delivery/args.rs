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
                          waits in the lobby for participants to ready up —
                          but browser clients CANNOT JOIN A NATIVE HOST YET
                          (issue #1112: PeerJS is browser JavaScript), so
                          --solo is currently the only mode that ever reaches
                          a running mission.
    --log <SPEC>          Log filter, e.g. info,ai=debug,admit=trace
    --log-entity <NAMES>  Restrict logging to these entity names

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
    let mut ship: Option<String> = None;
    let mut seed: Option<u64> = None;
    let mut log_spec = String::new();
    let mut log_entity = String::new();
    let mut solo = false;

    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::Help),
            "--addr" => addr = value_for(&arg, &mut it)?,
            "--client-dir" => client_dir = Some(value_for(&arg, &mut it)?),
            "--manifest" => manifest = value_for(&arg, &mut it)?,
            "--content-dir" => content_dir = value_for(&arg, &mut it)?,
            "--skip-bundle-check" => skip_bundle_check = true,
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
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    // The simulation flags are meaningless without a world, and silently
    // ignoring them would be the worst answer: an operator who wrote `--solo`
    // and no `--world` asked for a mission and would get a file server.
    let sim = match world {
        Some(world) => Some(SimArgs {
            world,
            ship,
            seed,
            log_spec,
            log_entity,
            solo,
        }),
        None => {
            for (flag, given) in [
                ("--ship", ship.is_some()),
                ("--seed", seed.is_some()),
                ("--log", !log_spec.is_empty()),
                ("--log-entity", !log_entity.is_empty()),
                ("--solo", solo),
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
    fn a_simulation_flag_without_a_world_is_refused_rather_than_ignored() {
        // Silently serving files to an operator who asked for a mission is the
        // worst available answer.
        let err = parse(&["--solo"]).unwrap_err();
        assert!(err.contains("--solo"), "{err}");
        assert!(err.contains("--world"), "{err}");
        assert!(parse(&["--seed", "1"]).unwrap_err().contains("--world"));
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
