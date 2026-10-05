//! Command-line parsing for `phoenix-host`.
//!
//! Native-only Clap declarations own syntax and generated help. The pure,
//! non-exiting conversion retains Phoenix's domain and mode validation.

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
    /// Explicit offline Authoring root; mutually exclusive with live/setup work.
    pub workshop: Option<WorkshopArgs>,
    /// `--setup`: enumerate the connected monitors, print their stable
    /// identities and geometry, validate `--profile` against them if one was
    /// given, and exit (issue #1123). A standalone diagnostic — it needs no
    /// `--world`, and when present it short-circuits the run.
    pub setup: bool,
    /// Explicit bounded tone on a profile's named media surface, setup only.
    pub test_output: Option<String>,
    pub meter_microphone: Option<String>,
    pub preview_camera: Option<String>,
    /// `--profile <PATH>`: a bridge-display profile (issue #1123), relative to
    /// the working directory. With `--world` the authoritative host applies it
    /// (viewscreen and Station monitors as borderless-fullscreen surfaces); with
    /// `--setup` it is validated against the connected displays. `None` keeps the
    /// single-window #1121 behaviour. Kept at the top level rather than in
    /// [`SimArgs`] because `--setup` reads it without a world.
    pub profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopArgs {
    pub root: String,
    pub project: bool,
    /// Presentation-only initial Workshop location. It never expands the
    /// selected root or grants filesystem authority.
    pub open: Option<String>,
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
    /// True when [`mod_pack_dir`](Self::mod_pack_dir) came from the lobby's
    /// implicit `./mod-packs` default. Only that path is created automatically;
    /// an explicit missing path remains an operator-visible scan error.
    pub mod_pack_dir_is_default: bool,
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
    /// Join an existing fleet as this process's selected player ship.
    pub fleet_code: Option<String>,
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
pub const DEFAULT_MOD_PACK_DIR: &str = "./mod-packs";

use clap::{CommandFactory, FromArgMatches, Parser};

#[derive(Parser)]
#[command(
    name = "phoenix-host",
    args_override_self = true,
    about = "Serve Phoenix content and run the authoritative native viewscreen",
    after_help = "ENDPOINTS
/host/stamp.json: protocol and content version stamp.
/host/manifest.json: version-pinned manifest and catalogue; callers must present ?protocol=&content_id=&content_epoch= or x-phoenix-client-stamp: <protocol>/<id>/<epoch>. Mismatches return 409 naming both sides. Other assets are served from --client-dir."
)]
struct Cli {
    #[arg(
        long,
        help = "Bind address [default: 0.0.0.0:8080] — accepts connections from the LAN by default; Windows will prompt to allow it through the firewall on first run. Use 127.0.0.1:<port> to restrict to this machine only."
    )]
    addr: Option<String>,
    #[arg(
        long,
        help = "Serve a built client bundle from this directory (a `trunk dist/`). Omit to serve no assets at all and publish only the manifest/stamp endpoints, for a client hosted elsewhere."
    )]
    client_dir: Option<String>,
    #[arg(
        long,
        help = "Scenario manifest to serve, relative to --content-dir [default: assets/scenarios.toml]. Point it at assets/scenarios.demo.toml for the curated public catalogue."
    )]
    manifest: Option<String>,
    #[arg(
        long,
        help = "Root the manifest and its world TOMLs are read from [default: .]"
    )]
    content_dir: Option<String>,
    #[arg(
        long,
        help = "Start even when the bundle's [content] identity cannot be read. Does NOT suppress a real mismatch."
    )]
    skip_bundle_check: bool,
    #[arg(
        long,
        help = "Run the authoritative simulation for this world, relative to --content-dir, and open a native viewscreen window. Without it (and without --lobby) this process serves delivery only, exactly as it always has."
    )]
    world: Option<String>,
    #[arg(
        long,
        help = "Register with this rendezvous service so browser clients can join, e.g. https://phoenix-rendezvous.project-phoenix.workers.dev The typed code the service issues is printed at startup. A native host has no WebRTC, so every crew member is carried over the service's WebSocket game relay; it registers saying so, and joiners skip the direct ladder rather than spending ninety seconds discovering it. These explicit flags override the built-in rendezvous used by the lobby's GM-only Join as Peer route; New Game remains LAN-direct by default."
    )]
    rendezvous: Option<String>,
    #[arg(
        long,
        help = "Join this ship to an existing fleet. Requires --world, --client-dir, --rendezvous and --origin; refuses --solo. The native fleet link uses WebSocket relay."
    )]
    fleet_code: Option<String>,
    #[arg(
        long,
        help = "The Origin header that socket claims. REQUIRED with --rendezvous and deliberately not defaulted: the service refuses an upgrade whose Origin is not on its deployed allowlist, and which origins a deployment allows is an operator decision recorded in docs/delivery-checklist.md §3a."
    )]
    origin: Option<String>,
    #[arg(
        long,
        help = "The player's hull [default: the world's first [[available_ships]] entry]"
    )]
    ship: Option<String>,
    #[arg(long, help = "Override the world's [global] seed")]
    seed: Option<u64>,
    #[arg(long, help = "Log filter, e.g. info,ai=debug,admit=trace")]
    log: Option<String>,
    #[arg(long, help = "Restrict logging to these entity names")]
    log_entity: Option<String>,
    #[arg(
        long,
        help = "Start the mission immediately with nobody connected; every station runs on Backfill. Without it the host waits in the lobby for participants to ready up, Ordinary crew can join on the host's own LAN port; --rendezvous adds a cloud route. With --lobby it starts on the tick the chosen world lands, not at boot: there is nothing to fly until someone has picked something."
    )]
    solo: bool,
    #[arg(
        long,
        help = "Private native save-slot directory, relative to the launch directory [default: .phoenix/saves]. Exactly one phoenix-host may claim it at a time; the claim is held for that authoritative process's lifetime, so concurrent native peers need distinct paths."
    )]
    save_dir: Option<String>,
    #[arg(long, action = clap::ArgAction::Append, num_args = 0, default_missing_value = "list", help = "Print this peer's local save catalogue, then run")]
    save_list: Vec<String>,
    #[arg(long, action = clap::ArgAction::Append, help = "Capture a named manual save at this new session's first deterministic in-progress tick; cannot be combined with --resume-save")]
    save_create: Vec<String>,
    #[arg(long, action = clap::ArgAction::Append, num_args = 2, value_names = ["SLOT", "NAME" ], help = " Rename a local manual save (its Store key is unchanged)")]
    save_rename: Vec<String>,
    #[arg(long, action = clap::ArgAction::Append, num_args = 2, value_names = ["SLOT", "PATH"], help = " Export one local slot to a new file; never overwrites")]
    save_export: Vec<String>,
    #[arg(long, action = clap::ArgAction::Append, help = "Delete one local slot only with the explicit paired confirmation flag")]
    save_delete: Vec<String>,
    #[arg(long, help = "Confirm all requested save deletions")]
    confirm_delete: bool,
    #[arg(
        long,
        help = "Boot this --world as a NEW session from a compatible local slot; incompatible saves are refused before run"
    )]
    resume_save: Option<String>,
    #[arg(long, action = clap::ArgAction::Append, help = "Open a local Station pane for a participant of this name, in an embedded browser view showing the ordinary console surface. Repeatable; panes are tiled left to right. Each one joins, claims a Station and readies through exactly the contracts a phone does, with its own minted session token. Needs --client-dir: a pane loads the client bundle this host serves.")]
    pane: Vec<String>,
    #[arg(
        long,
        help = "Log, once a second at info level (so with --log info), one line saying where each frame went: the Bevy frame time, the embedded panes' update / pump / render / copy phases, pixels copied, Image assets re-uploaded, fixed-tick catch-up, and the residual left to the render thread. A diagnostic for the multi-screen bridge. The PHOENIX_FRAME_EXPERIMENTS environment variable (a comma list of novsync, raf33) switches one suspected cost off per run so the lines can be compared."
    )]
    frame_stats: bool,
    #[arg(
        long,
        help = "Scan this directory for mod-pack .zip archives and offer them on the landing screen, relative to the launch directory [default with --lobby: ./mod-packs]. The default folder is created when absent. This window has no file dialog, so the folder IS the file picker. A chosen pack goes through exactly the validation a browser upload does, and is refused whole if any of it fails. Needs --lobby: a pack changes the catalogue a world is chosen FROM, and a --world host was told at the prompt what it is flying."
    )]
    mod_pack_dir: Option<String>,
    #[arg(
        long,
        help = "Enumerate the connected monitors, print their stable hardware identities, geometry and current assignment, then exit. Validates --profile against them if given. A standalone diagnostic — needs no --world, and refuses every simulation/crew flag (--world, --lobby, --ship, --seed, --solo, --pane, --log, --log-entity, --rendezvous, --origin) rather than silently discarding them."
    )]
    setup: bool,
    #[arg(
        long,
        help = "With --setup --profile, play a quiet one-second tone on each output assigned to that media surface and exit."
    )]
    test_output: Option<String>,
    #[arg(
        long,
        help = "With --setup --profile, show microphone levels for five seconds per assigned microphone; no recording."
    )]
    meter_microphone: Option<String>,
    #[arg(
        long,
        help = "With --setup --profile, open the assigned camera preview on Windows. Escape or close stops the preview."
    )]
    preview_camera: Option<String>,
    #[arg(
        long,
        help = "A bridge-display profile (TOML). With --world the host covers every configured monitor with one borderless full-screen surface — the viewscreen, or a Station hosting one or two panes. With --setup it is validated against the connected displays. A missing or changed monitor is reported, never silently re-homed."
    )]
    profile: Option<String>,
    #[arg(
        long,
        help = "Open the same viewscreen window with NO world: the landing offers New Game, Host as GM and Join as Peer. New Game and Host as GM use the scenario/hull picker; Join as Peer uses the typed fleet-code panel. Every flag below still applies to the mission that eventually starts. Mutually exclusive with --world, which is simply the same pick made up front."
    )]
    lobby: bool,
    #[arg(long, action = clap::ArgAction::Append, help = "Open an offline editable project in the shared Workshop UI. Requires --client-dir and an Ultralight build. Delivery binds loopback only; no live session.")]
    workshop_project: Vec<String>,
    #[arg(long, action = clap::ArgAction::Append, help = "Open an offline editable mod workspace instead. --content-dir supplies its read-only base content.")]
    workshop_mod: Vec<String>,
    #[arg(
        long,
        help = "Initial Workshop panel/source/preview selection. Valid only with one of the two Workshop roots."
    )]
    workshop_open: Option<String>,
}

/// Help generated from the same declarations used for parsing.
pub fn help() -> String {
    Cli::command().render_long_help().to_string()
}

// Clap records each value's occurrence index. Merge the five append-only
// action streams to preserve the operator's interleaved command order.
fn ordered_save_actions(matches: &clap::ArgMatches) -> Vec<SaveOperatorAction> {
    let mut actions = Vec::new();
    for (id, arity) in [
        ("save_list", 1),
        ("save_create", 1),
        ("save_rename", 2),
        ("save_export", 2),
        ("save_delete", 1),
    ] {
        let Some(occurrences) = matches.get_occurrences::<String>(id) else {
            continue;
        };
        let indices: Vec<_> = matches.indices_of(id).into_iter().flatten().collect();
        for (index, values) in indices.into_iter().step_by(arity).zip(occurrences) {
            let values: Vec<_> = values.cloned().collect();
            let action = match id {
                "save_list" => SaveOperatorAction::List,
                "save_create" => SaveOperatorAction::Create {
                    display_name: values[0].clone(),
                },
                "save_rename" => SaveOperatorAction::Rename {
                    slot_id: values[0].clone(),
                    display_name: values[1].clone(),
                },
                "save_export" => SaveOperatorAction::Export {
                    slot_id: values[0].clone(),
                    path: values[1].clone(),
                },
                "save_delete" => SaveOperatorAction::Delete {
                    slot_id: values[0].clone(),
                    confirmed: false,
                },
                _ => unreachable!(),
            };
            actions.push((index, action));
        }
    }
    actions.sort_by_key(|(index, _)| *index);
    actions.into_iter().map(|(_, action)| action).collect()
}

/// Parse `phoenix-host`'s arguments.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<ParseOutcome, String> {
    let matches = match Cli::command()
        .no_binary_name(true)
        .try_get_matches_from(args)
    {
        Ok(matches) => matches,
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            return Ok(ParseOutcome::Help)
        }
        Err(error) => return Err(error.to_string()),
    };
    let mut save_actions = ordered_save_actions(&matches);
    let raw = Cli::from_arg_matches(&matches).map_err(|error| error.to_string())?;
    let addr_given = raw.addr.is_some();
    let save_dir_given = raw.save_dir.is_some();
    let mod_pack_dir_given = raw.mod_pack_dir.is_some();
    let mut addr = raw.addr.unwrap_or_else(|| DEFAULT_ADDR.to_string());
    let manifest = raw.manifest.unwrap_or_else(|| DEFAULT_MANIFEST.to_string());
    let content_dir = raw
        .content_dir
        .unwrap_or_else(|| DEFAULT_CONTENT_DIR.to_string());
    let save_dir = raw.save_dir.unwrap_or_else(|| DEFAULT_SAVE_DIR.to_string());
    let log_spec = raw.log.unwrap_or_default();
    let log_entity = raw.log_entity.unwrap_or_default();
    let panes = raw.pane;
    let resume_slot = raw.resume_save;
    if raw.workshop_project.len() + raw.workshop_mod.len() > 1 {
        return Err("Select exactly one Workshop root".into());
    }
    let mut workshop = raw
        .workshop_project
        .first()
        .map(|root| WorkshopArgs {
            root: root.clone(),
            project: true,
            open: None,
        })
        .or_else(|| {
            raw.workshop_mod.first().map(|root| WorkshopArgs {
                root: root.clone(),
                project: false,
                open: None,
            })
        });
    let mut workshop_open = raw.workshop_open;
    if workshop_open
        .as_ref()
        .is_some_and(|value| value.len() > 2048 || value.contains(['\r', '\n', '#']))
    {
        return Err("--workshop-open needs one bounded URL query".into());
    }
    let client_dir = raw.client_dir;
    let skip_bundle_check = raw.skip_bundle_check;
    let world = raw.world;
    let rendezvous = raw.rendezvous;
    let fleet_code = raw.fleet_code;
    let origin = raw.origin;
    let ship = raw.ship;
    let seed = raw.seed;
    let solo = raw.solo;
    let confirm_delete = raw.confirm_delete;
    let frame_stats = raw.frame_stats;
    let mut mod_pack_dir = raw.mod_pack_dir;
    let setup = raw.setup;
    let test_output = raw.test_output;
    let meter_microphone = raw.meter_microphone;
    let preview_camera = raw.preview_camera;
    let profile = raw.profile;
    let lobby = raw.lobby;

    if let Some(selected) = workshop.as_mut() {
        selected.open = workshop_open.take();
        if addr_given {
            return Err(
                "Workshop owns a loopback-only delivery endpoint; --addr is not a Workshop option"
                    .into(),
            );
        }
        if client_dir.is_none() {
            return Err("Workshop needs --client-dir with the built Workshop bundle".into());
        }
        if world.is_some()
            || lobby
            || setup
            || profile.is_some()
            || rendezvous.is_some()
            || fleet_code.is_some()
            || origin.is_some()
            || ship.is_some()
            || seed.is_some()
            || solo
            || save_dir_given
            || !save_actions.is_empty()
            || confirm_delete
            || resume_slot.is_some()
            || !panes.is_empty()
            || frame_stats
            || mod_pack_dir.is_some()
            || test_output.is_some()
            || meter_microphone.is_some()
            || preview_camera.is_some()
            || !log_spec.is_empty()
            || !log_entity.is_empty()
        {
            return Err("Workshop is offline and cannot combine with simulation, crew, setup or bridge-profile flags".into());
        }
        // The ordinary CLI default is LAN delivery. An offline Workshop never
        // inherits that default, and cannot expose an authored root over LAN.
        addr = "127.0.0.1:0".into();
    } else if workshop_open.is_some() {
        return Err("--workshop-open requires --workshop-project or --workshop-mod".into());
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
    if test_output.is_some() && (!setup || profile.is_none()) {
        return Err("--test-output needs --setup and --profile".to_string());
    }
    for (flag, supplied) in [
        ("--meter-microphone", meter_microphone.is_some()),
        ("--preview-camera", preview_camera.is_some()),
    ] {
        if supplied && (!setup || profile.is_none()) {
            return Err(format!("{flag} needs --setup and --profile"));
        }
    }
    if [
        test_output.is_some(),
        meter_microphone.is_some(),
        preview_camera.is_some(),
    ]
    .into_iter()
    .filter(|given| *given)
    .count()
        > 1
    {
        return Err("choose one media test per setup invocation".into());
    }
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
            ("--fleet-code", fleet_code.is_some()),
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
    if fleet_code.is_some()
        && (world.is_none() || rendezvous.is_none() || client_dir.is_none() || solo)
    {
        return Err("--fleet-code needs --world, --rendezvous, --origin and --client-dir, and refuses --solo".into());
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

    // A lobby is the native front door, so it always has a real mod-pack
    // shelf. Keep the default out of delivery-only and direct --world runs:
    // neither has a landing on which the shelf could be opened.
    if lobby && !mod_pack_dir_given {
        mod_pack_dir = Some(DEFAULT_MOD_PACK_DIR.to_string());
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
            mod_pack_dir_is_default: lobby && !mod_pack_dir_given,
            panes,
            frame_stats,
            rendezvous,
            fleet_code,
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
        workshop,
        setup,
        test_output,
        meter_microphone,
        preview_camera,
        profile,
    })))
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod tests;
