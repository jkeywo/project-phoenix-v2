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

/// The authoritative simulation's arguments, present when `--world` (issue
/// #1121) or `--lobby` (issue #1326) was given.
///
/// A separate struct rather than five `Option` fields on [`HostArgs`] because
/// they travel together: every one of them is meaningless without an
/// authoritative host, and `Option<SimArgs>` makes "is this an authoritative
/// host" a single question with a single answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimArgs {
    /// Root world TOML, relative to `content_dir`, or `None` under `--lobby`:
    /// the host boots into an empty lobby holding the scenario catalogue and
    /// takes its world from a runtime `SelectScenario` + `SelectPlayerShip`
    /// pair instead (issue #1326).
    pub world: Option<String>,
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
    /// Private save-slot directory, resolved against the launch directory
    /// before `--content-dir` re-roots an authoritative process.
    pub save_dir: String,
    /// One-shot native operator actions, applied in order after this scenario's
    /// content ledger is loaded and the private Store is installed.
    pub save_actions: Vec<SaveOperatorAction>,
    /// A compatible local slot to stage into this newly constructed native App.
    /// Resume is startup-only; there is deliberately no live-session restore.
    pub resume_slot: Option<String>,
    /// The directory the landing's mod-pack shelf is scanned from
    /// (`--mod-pack-dir <DIR>`, issue #1366), resolved against the launch
    /// directory before `--content-dir` re-roots the process.
    ///
    /// `None` is a host with no shelf: the landing's Load-mod-pack entry stays
    /// the inert row #1360 shipped, because nothing behind it can answer. That
    /// is the same rule the rest of this surface follows — a control exists
    /// exactly when something behind it answers it — and it is why this is an
    /// `Option` rather than a defaulted path that would offer an empty shelf on
    /// every host in the world.
    ///
    /// It lives in [`SimArgs`] and is refused without `--lobby`, because the
    /// landing that offers the shelf is only ever shown by a world-less host
    /// (`native_host::host_lobby::feed_landing_panel`) and a pack has to be
    /// installed BEFORE a World is ingested to change anything at all.
    pub mod_pack_dir: Option<String>,
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
    /// Log a one-line per-second breakdown of where each frame goes
    /// (`--frame-stats`) — see `native_host::panes::frame_stats`. A
    /// diagnostic on the windowed host; meaningless without a simulation
    /// running, which is why it lives here and is refused without one.
    pub frame_stats: bool,
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

/// A native operator's one-shot local catalogue action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOperatorAction {
    List,
    Create {
        display_name: String,
    },
    Rename {
        slot_id: String,
        display_name: String,
    },
    Export {
        slot_id: String,
        path: String,
    },
    Delete {
        slot_id: String,
        confirmed: bool,
    },
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
pub const DEFAULT_SAVE_DIR: &str = ".phoenix/saves";

pub const HELP: &str = "\
phoenix-host — serve the Phoenix client bundle, the content manifest and the
scenario catalogue from a native PC process instead of a browser tab, and
(with --world) run the authoritative simulation with a native viewscreen.

USAGE:
    phoenix-host [OPTIONS]

SIMULATION
    --world <PATH>        Run the authoritative simulation for this world,
                          relative to --content-dir, and open a native
                          viewscreen window. Without it (and without --lobby)
                          this process serves delivery only, exactly as it
                          always has.
    --lobby               Open the same viewscreen window with NO world: the
                          host waits in the lobby publishing its scenario
                          catalogue, and loads a world when a participant picks
                          a scenario and a hull — the same first-valid-wins
                          arbitration the browser host runs before its own world
                          load. Every flag below still applies to the mission
                          that eventually starts. Mutually exclusive with
                          --world, which is simply the same pick made up front.
    --ship <PATH>         The player's hull [default: the world's first
                          [[available_ships]] entry]
    --seed <N>            Override the world's [global] seed
    --solo                Start the mission immediately with nobody connected;
                          every station runs on Backfill. Without it the host
                          waits in the lobby for participants to ready up,
                          which needs --rendezvous below — a host with neither
                          waits for a crew that has no way in, and says so.
                          With --lobby it starts on the tick the chosen world
                          lands, not at boot: there is nothing to fly until
                          someone has picked something.
    --save-dir <PATH>     Private native save-slot directory, relative to the
                          launch directory [default: .phoenix/saves]. Exactly
                          one phoenix-host may claim it at a time; the claim is
                          held for that authoritative process's lifetime, so
                          concurrent native peers need distinct paths.
    --save-list           Print this peer's local save catalogue, then run
    --save-create <NAME>  Capture a named manual save at this new session's
                          first deterministic in-progress tick; cannot be
                          combined with --resume-save
    --save-rename <SLOT> <NAME>
                          Rename a local manual save (its Store key is unchanged)
    --save-export <SLOT> <PATH>
                          Export one local slot to a new file; never overwrites
    --save-delete <SLOT> --confirm-delete
                          Delete one local slot only with the explicit paired
                          confirmation flag
    --resume-save <SLOT>  Boot this --world as a NEW session from a compatible
                          local slot; incompatible saves are refused before run
    --log <SPEC>          Log filter, e.g. info,ai=debug,admit=trace
    --log-entity <NAMES>  Restrict logging to these entity names
    --frame-stats         Log, once a second at info level (so with --log
                          info), one line saying where each frame went: the
                          Bevy frame time, the embedded panes' update / pump /
                          render / copy phases, pixels copied, Image assets
                          re-uploaded, fixed-tick catch-up, and the residual
                          left to the render thread. A diagnostic for the
                          multi-screen bridge. The PHOENIX_FRAME_EXPERIMENTS
                          environment variable (a comma list of untracked,
                          noforce, novsync, raf33) switches one suspected cost
                          off per run so the lines can be compared.

MOD PACKS (issue #1366)
    --mod-pack-dir <DIR>  Scan this directory for mod-pack .zip archives and
                          offer them on the landing screen, relative to the
                          launch directory. This window has no file dialog, so
                          the folder IS the file picker. A chosen pack goes
                          through exactly the validation a browser upload does,
                          and is refused whole if any of it fails. Needs
                          --lobby: a pack changes the catalogue a world is
                          chosen FROM, and a --world host was told at the prompt
                          what it is flying.

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
                          refuses every simulation/crew flag (--world, --lobby,
                          --ship, --seed, --solo, --pane, --log, --log-entity,
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
                          The typed code the service issues is printed at
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
    let mut save_dir = DEFAULT_SAVE_DIR.to_string();
    let mut save_dir_given = false;
    let mut save_actions = Vec::new();
    let mut confirm_delete = false;
    let mut resume_slot: Option<String> = None;
    let mut panes: Vec<String> = Vec::new();
    let mut frame_stats = false;
    let mut mod_pack_dir: Option<String> = None;
    let mut setup = false;
    let mut profile: Option<String> = None;
    let mut lobby = false;

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
            "--lobby" => lobby = true,
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
            "--save-dir" => {
                save_dir = value_for(&arg, &mut it)?;
                save_dir_given = true;
            }
            "--save-list" => save_actions.push(SaveOperatorAction::List),
            "--save-create" => save_actions.push(SaveOperatorAction::Create {
                display_name: value_for(&arg, &mut it)?,
            }),
            "--save-rename" => save_actions.push(SaveOperatorAction::Rename {
                slot_id: value_for(&arg, &mut it)?,
                display_name: value_for(&arg, &mut it)?,
            }),
            "--save-export" => save_actions.push(SaveOperatorAction::Export {
                slot_id: value_for(&arg, &mut it)?,
                path: value_for(&arg, &mut it)?,
            }),
            "--save-delete" => save_actions.push(SaveOperatorAction::Delete {
                slot_id: value_for(&arg, &mut it)?,
                confirmed: false,
            }),
            "--confirm-delete" => confirm_delete = true,
            "--resume-save" => resume_slot = Some(value_for(&arg, &mut it)?),
            "--pane" => panes.push(value_for(&arg, &mut it)?),
            "--frame-stats" => frame_stats = true,
            "--mod-pack-dir" => mod_pack_dir = Some(value_for(&arg, &mut it)?),
            "--rendezvous" => rendezvous = Some(value_for(&arg, &mut it)?),
            "--origin" => origin = Some(value_for(&arg, &mut it)?),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }

    let has_delete = save_actions
        .iter()
        .any(|action| matches!(action, SaveOperatorAction::Delete { .. }));
    if has_delete && !confirm_delete {
        return Err("--save-delete requires the explicit --confirm-delete flag".to_string());
    }
    if confirm_delete && !has_delete {
        return Err("--confirm-delete requires --save-delete <SLOT>".to_string());
    }
    if confirm_delete {
        for action in &mut save_actions {
            if let SaveOperatorAction::Delete { confirmed, .. } = action {
                *confirmed = true;
            }
        }
    }
    let has_create = save_actions
        .iter()
        .any(|action| matches!(action, SaveOperatorAction::Create { .. }));
    if has_create && resume_slot.is_some() {
        return Err(
            "--save-create cannot be combined with --resume-save: choose a fresh-session capture or resume an existing slot"
                .to_string(),
        );
    }
    let save_operator_given = !save_actions.is_empty() || resume_slot.is_some();

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
        if save_operator_given {
            return Err(
                "--setup is a standalone diagnostic and refuses save catalogue controls"
                    .to_string(),
            );
        }
        for (flag, given) in [
            ("--world", world.is_some()),
            ("--lobby", lobby),
            ("--ship", ship.is_some()),
            ("--seed", seed.is_some()),
            ("--log", !log_spec.is_empty()),
            ("--log-entity", !log_entity.is_empty()),
            ("--solo", solo),
            ("--save-dir", save_dir_given),
            ("--pane", !panes.is_empty()),
            ("--frame-stats", frame_stats),
            ("--mod-pack-dir", mod_pack_dir.is_some()),
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
    // `--lobby` IS `--world` deferred (issue #1326): both ask for an
    // authoritative host, and they differ only in whether the scenario is named
    // up front or picked from the lobby. Given together, one of them is being
    // ignored — say which rather than guessing.
    if lobby && world.is_some() {
        return Err(
            "--lobby and --world are the same decision made at two different moments: \
             --world names the scenario up front, --lobby waits for someone to pick one"
                .to_string(),
        );
    }
    // A bridge-display profile is applied by a running host (`--world` or
    // `--lobby`) or validated by `--setup`; on its own it has nothing to act on.
    // Refuse at the prompt rather than reading a file nothing will use.
    if profile.is_some() && world.is_none() && !lobby && !setup {
        return Err(
            "--profile needs --world or --lobby (to apply the bridge display profile) or \
             --setup (to validate it against the connected displays)"
                .to_string(),
        );
    }

    // The mod-pack shelf is offered by the LANDING, and only a world-less host
    // ever shows one (`native_host::host_lobby::feed_landing_panel` requires a
    // `LobbyScenarioCatalog`). A pack also only changes anything before a World
    // is ingested — it widens the catalogue a world is chosen from. So on a
    // `--world` host, and on a delivery-only one, the folder would be scanned
    // for a shelf nobody can ever open: say so at the prompt rather than let an
    // operator conclude their packs are broken (issue #1366).
    if mod_pack_dir.is_some() && !lobby {
        return Err(
            "--mod-pack-dir needs --lobby: the shelf is offered on the landing screen, which \
             only a host with no --world shows, and a pack widens the catalogue a world is \
             chosen FROM"
                .to_string(),
        );
    }

    // The save-catalogue controls and --resume-save act on a concrete scenario
    // at startup — before a --lobby host has picked one — so they need --world
    // itself, not merely a lobby to choose from.
    if save_operator_given && world.is_none() {
        return Err(
            "save catalogue controls need --world — they operate on an authoritative \
             native peer's scenario at startup, which a --lobby host has not yet picked"
                .to_string(),
        );
    }
    let sim = if world.is_some() || lobby {
        Some(SimArgs {
            world,
            ship,
            seed,
            log_spec,
            log_entity,
            solo,
            save_dir,
            save_actions,
            resume_slot,
            mod_pack_dir,
            panes,
            frame_stats,
            rendezvous,
            origin,
        })
    } else {
        // No world and not in lobby: this host serves delivery only, so every
        // simulation-configuring flag is a mistake here (the save-catalogue
        // controls were already refused above, since they need --world itself).
        for (flag, given) in [
            ("--ship", ship.is_some()),
            ("--seed", seed.is_some()),
            ("--log", !log_spec.is_empty()),
            ("--log-entity", !log_entity.is_empty()),
            ("--solo", solo),
            ("--save-dir", save_dir_given),
            ("--pane", !panes.is_empty()),
            ("--frame-stats", frame_stats),
            ("--rendezvous", rendezvous.is_some()),
            ("--origin", origin.is_some()),
        ] {
            if given {
                return Err(format!(
                    "{flag} needs --world or --lobby — it configures the simulation, and \
                     without one of those this host serves delivery only"
                ));
            }
        }
        None
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

    fn err(args: &[&str]) -> String {
        parse(args).expect_err("expected a refusal")
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
    }

    #[test]
    fn a_host_with_no_shelf_is_the_default_rather_than_an_empty_one() {
        // `None` is "this host has no shelf", which keeps the landing's
        // Load-mod-pack row the inert one #1360 shipped. An empty-string or
        // current-directory default would offer a shelf on every host in the
        // world and make the row lie.
        let a = run(&["--lobby"]);
        assert_eq!(a.sim.expect("--lobby").mod_pack_dir, None);
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
        assert!(HELP.contains("one phoenix-host may claim it at a time"));
        assert!(HELP.contains("authoritative process's lifetime"));
        assert!(HELP.contains("concurrent native peers need distinct paths"));
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
