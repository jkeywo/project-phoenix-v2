---
title: Reusable Layers
type: concept
tags: [architecture, packages, reuse, testing]
sources: [Cargo.toml, scripts/check-layers.mjs, src/lib.rs, src/boot/mod.rs, crates/phoenix-runtime/src/lib.rs, crates/phoenix-transport/src/lib.rs, crates/phoenix-platform/src/lib.rs, crates/phoenix-model/src/lib.rs, crates/phoenix-content/src/lib.rs, crates/phoenix-simulation/src/lib.rs, crates/phoenix-sim-contracts/src/lib.rs, crates/phoenix-sim-gameplay/src/lib.rs, crates/phoenix-sim-world/src/lib.rs, crates/phoenix-sim-session/src/lib.rs, crates/phoenix-presentation/src/lib.rs, examples/grid/src/lib.rs, packages/transport/src/rendezvous-transport.js, packages/session/src/session-token.js, gui/action-map.js, editor/workshop-provider.js, worker-rendezvous/src/index.js, worker/src/index.js, pasm/spec/architecture/reusable-layers.yaml, src/server/fleet_staging.rs, src/server_app/mod.rs, crates/phoenix-simulation/src/world/mod_pack.rs]
updated: 2026-10-05
---

# Reusable Layers

This guide explains what each module owns, what calls it, and what it calls.
Use it to choose where a change belongs and how much of the project you need to
understand for that change. The [interactive dependency graph](../../docs/architecture/layers.html)
shows the same package boundaries; select a node to inspect individual imports.

The root host assembles a running application. Simulation assembles the game
from Gameplay, World and Session. Presentation draws the resulting state.
Shared packages provide the vocabulary, content loading, networking and machine
services these layers need.

## Reading the dependency descriptions

**Uses** and **Used by** list direct internal package dependencies for Rust.
They include target-specific and optional dependencies; a particular build may
use fewer. Test-only dependencies are marked. Third-party libraries are mentioned
only where they explain the responsibility. Transitive users are not repeated:
Presentation can reach Gameplay through Simulation without importing Gameplay.

For browser code and services, the guide distinguishes JavaScript imports from
messages, HTTP requests and asset reads. A phone communicating with a Rust host
is a runtime connection, not a Cargo dependency.

The graph's tree is a map of composition. Shared foundations have several
callers, which appear when a node is selected. Gameplay, World and Session
have **no sibling or parent imports**, including in their tests. The simulation
parent joins them through shared contracts and live inputs.

Short names below refer to the corresponding `phoenix-*` crate, except
**Host** (`project-phoenix`), **Contracts** (`phoenix-sim-contracts`), and the
three simulation branches (`phoenix-sim-gameplay`, `phoenix-sim-world`,
`phoenix-sim-session`). Browser packages are named explicitly.

| Area | Start here | Main question it answers |
|---|---|---|
| Application | [Host composition](#host-composition) | Which capabilities does this executable or page install? |
| Game assembly | [Simulation](#simulation-composition-and-adapters) | How do the game domains form one running mission? |
| Game domains | [Gameplay](#simulation-gameplay), [World](#simulation-world), [Session](#simulation-session) | How do entities act, what happens in the scenario, and who controls what? |
| Shared game definitions | [Contracts](#simulation-contracts), [Model](#model) | Which types and rules cross a boundary? |
| Content | [Content preparation](#content-preparation), [Authored assets](#authored-assets) | How are files loaded, and where are tunable values authored? |
| Screens | [3D presentation](#3d-presentation), [Browser UI and Workshop](#browser-ui-and-workshop) | How does a person see and operate the game? |
| Reusable foundations | [Runtime](#runtime), [Transport](#rust-transport), [Platform](#platform), [Maths](#maths) | How do peers coordinate, bytes travel and hosts use their machine? |
| Browser foundations | [Transport](#browser-transport), [Session](#browser-session) | How does a browser connect and retain its identity? |
| Services and verification | [Cloud services](#cloud-services), [Grid](#grid-reuse-example), [Tests](#cross-module-tests) | How is joining supported, reuse demonstrated and integration checked? |

## Host composition

**Owner:** root `project-phoenix` crate in [`src/`](../../src/lib.rs).

**Responsible for:** building the application for browser, native, headless and
Workshop hosts. It chooses the Boot Profile, installs plugins, connects
presentation inputs to simulation, exposes browser exports, and integrates
native windows, local Station panes and delivery. Its compatibility re-exports
preserve public paths after code has moved into another crate.

**Uses:** Simulation, Presentation, Maths, Model, Content, Platform, Runtime and
Transport. Native and browser adapters also connect to the JavaScript surfaces
through their respective bridges.

**Used by:** the root package's binaries and generated WASM entry points. No
other workspace crate imports the root package. Integration tests use its public
API to construct complete hosts.

**Work here for:** a new boot option, native host lifecycle, or a change to which
capabilities a host installs. Start with [`boot`](../../src/boot/mod.rs),
[`native_host`](../../src/native_host/), or
[`presentation_adapters.rs`](../../src/presentation_adapters.rs).
Follow a re-export to its owning crate before changing game behavior.

`server_app::compose_live_host` owns Asteroid, Modifier, Lobby, simulation, RNG
and World registration in that order. The Boot Profile selects presentation;
headless registration perturbations stay explicit options. Native and headless
hull preparation resolves and parses once, returning ledger records for the
adapter to apply after its admission checks. Browser fleet callbacks stage
joins, leaves, control projections and completions through
[`FleetStaging`](../../src/server/fleet_staging.rs).


Accepted pack installation consumes `ValidatedModPack::into_active_pack` from
Simulation. The semantic validator retains the parsed identity and hands off
content, assets and source archive together; browser and native adapters do not
reconstruct an accepted pack.

## Simulation composition and adapters

**Owner:** [`phoenix-simulation`](../../crates/phoenix-simulation/src/lib.rs).

**Responsible for:** assembling the simulation schedule and connecting the
independent game domains. It owns live adapters, spawning and materialization,
validation that combines entity definitions with scenario definitions, command
application, state publication, snapshots, digest traversal and recovery
integration. Systems that still combine several domains live here, so this is
still a substantial implementation package.

**Uses:** Gameplay, World, Session, Contracts, Maths, Model, Content, Platform,
Runtime and Transport. Bevy supplies ECS and scheduling. The independent
simulation build contains no renderer, window stack or Ultralight.

**Used by:** Host and Presentation directly. Native tests and headless hosts
exercise the same simulation through the Host composition layer.

**Work here for:** a change that joins domains, changes schedule ordering, loads
an entity into the live world, or captures/restores a complete mission. The
parent supplies live borrowed inputs and applies script effects in their
scheduled order; the branches do not keep copied mission-input caches or call
one another through a generic event bus.

Shared summary-control permission and Objective visibility live in Contracts.
World owns ship-effective legacy/instance Objective reads. Runtime owns guarded
continuation transitions; adapters apply game ownership effects before consuming
its prepared completion. Readiness and bounded histories validate their
invariants at decode and aggregation boundaries.

## Simulation gameplay

**Owner:** [`phoenix-sim-gameplay`](../../crates/phoenix-sim-gameplay/src/lib.rs).

**Responsible for:** what ships and other entities can do. This includes the
complete `EntityConfig` parser, entity state and validation, physics and damage
rules, weapons, control and rating rules, policy execution, Helm AI operators,
and the mechanics of capabilities such as docking, tractors and transporters.
Live systems that need other domains are composed in Simulation.

**Uses:** Contracts for shared simulation vocabulary and rules; Model for shared
game data; Content for content preparation mechanisms; Maths for deterministic
calculations. It also uses Bevy and physics libraries where its implementation
requires them.

**Used by:** Simulation directly. Other application layers reach these mechanics
through Simulation's public surface.

**Work here for:** a damage calculation, weapon rule, steering operator or entity
configuration field. Tune an authored weapon value in `assets/`; change its
meaning here. Gameplay does not import World or Session.

## Simulation world

**Owner:** [`phoenix-sim-world`](../../crates/phoenix-sim-world/src/lib.rs).

**Responsible for:** what a scenario declares, requests and records. It owns the
complete `WorldConfig` parser, script machinery, Objectives, flags, delayed work,
deadlines, commitments, task lifecycles, narrative and report state, and related
GM decision models. Scenario effects that touch live gameplay are applied by the
parent's adapters.

**Uses:** Contracts, Model, Content and Maths. Rhai and the script support library
provide script execution machinery.

**Used by:** Simulation directly.

**Work here for:** Objective lifecycle semantics, script vocabulary or scenario
state rules. Change the scenario's actual timing and content in its authored
files and design model. World does not import Gameplay or Session.

## Simulation session

**Owner:** [`phoenix-sim-session`](../../crates/phoenix-sim-session/src/lib.rs).

**Responsible for:** who participates and who controls each Station. It owns crew
Session records, Station tenure and lobby decisions, connection lifecycle
bookkeeping, GM roster state, command-log support and authenticated fleet
protocol state.

**Uses:** Contracts for authority and command vocabulary; Model for messages and
game identities; Runtime for multiplayer coordination; Rust Transport for
connection and delivery abstractions.

**Used by:** Simulation directly.

**Work here for:** who may claim a Station, what happens when a participant
reconnects, or how fleet participation is tracked. A change to browser token
storage belongs in Browser session. Session does not import Gameplay or World.

## Simulation contracts

**Owner:** [`phoenix-sim-contracts`](../../crates/phoenix-sim-contracts/src/lib.rs).

**Responsible for:** the common simulation language required by more than one
branch: identities, logical ticks, seeded RNG, schedule labels, authority rules,
command envelopes, shared authored vocabulary, effect-queue primitives and
presentation readiness/input contracts. These definitions allow the parent to
join branches without introducing sibling imports.

**Uses:** Model, Content, Maths, Runtime and Rust Transport. Its simulation-facing
types can use Bevy ECS without pulling in a renderer.

**Used by:** Gameplay, World, Session and Simulation.

**Work here for:** a shared command or scheduling contract whose meaning must be
identical across domains. State or a rule owned by one domain stays with that
domain; Contracts is not a destination for every reusable helper.

## 3D presentation

**Owner:** [`phoenix-presentation`](../../crates/phoenix-presentation/src/lib.rs).

**Responsible for:** the shared Viewscreen and rendered previews: cameras,
meshes/materials, lighting, visual effects, HUD, debug drawing and Workshop model
preview. It reads simulation state and supplies presentation readiness or input
through the host adapters and shared contracts.

**Uses:** Simulation, Model and Maths, plus the Bevy rendering stack. It also
has a **test-only** dependency on Content.

**Used by:** Host, which decides whether a boot profile installs real rendering.

**Work here for:** the appearance of a beam, camera behavior, a rendered HUD or
model preview. Simulation cannot import Presentation. Renderer readiness is
communicated through a narrow contract so headless execution remains possible.

## Model

**Owner:** [`phoenix-model`](../../crates/phoenix-model/src/lib.rs).

**Responsible for:** shared Phoenix data vocabulary: identities, messages, wire
representations, entity tags, texture/rig declarations and debug surface data.
These types describe what a payload or declaration means without running the
simulation or drawing a screen.

**Uses:** Rust Transport directly for transport-related shared types. Its
optional `ecs` feature supplies the existing ECS traits on relevant game types;
the default package needs neither ECS nor rendering.

**Used by:** Host, Content, Contracts, Gameplay, World, Session, Simulation and
Presentation.

**Work here for:** a shared wire payload, identity type or visual declaration.
A rule for advancing game state belongs in a simulation domain. A shared
simulation tick or authority contract belongs in Contracts.

## Content preparation

**Owner:** [`phoenix-content`](../../crates/phoenix-content/src/lib.rs).

**Responsible for:** turning authored bytes into prepared content: archives,
manifests, includes and overrides, overlays, the loaded-content ledger, asset
validation, string catalogues, sound cues and rig parsing. It can decode images
for admission without creating a GPU or window.

**Uses:** Model for common declarations, Maths for shared calculations and
Platform for machine-level file mechanisms.

**Used by:** Host, Contracts, Gameplay, World and Simulation at runtime;
Presentation in tests.

**Work here for:** include resolution, pack admission, manifest handling or
content version tracking. The complete entity parser belongs to Gameplay and
the complete world parser belongs to World. Validation requiring both domains
stays in Simulation. Content accepts data or injected resolvers to keep these
dependencies pointing downwards.

## Runtime

**Owner:** [`phoenix-runtime`](../../crates/phoenix-runtime/src/lib.rs).

**Responsible for:** deterministic multiplayer coordination independent of game
rules and I/O: command ordering, tick readiness, continuation, digest history,
recovery planning and chunk transfer. The game provides admitted commands and
its own state capture/application.

**Uses:** no other workspace crate directly. Serialization and digest libraries
support its implementation.

**Used by:** Host, Contracts, Session, Simulation and Grid.

**Work here for:** ordering commands consistently across peers, deciding whether
a tick can advance, or transferring a recovery checkpoint. Whether a player is
allowed to fire a weapon remains a game-level decision.

## Rust transport

**Owner:** [`phoenix-transport`](../../crates/phoenix-transport/src/lib.rs).

**Responsible for:** physical connections and delivery: connection generations,
logical client routing, delivery classes, paired transports, rendezvous frames,
and relay/socket state machines. Its native socket adapter is feature- and
target-gated. It carries game frames without interpreting Phoenix commands.

**Uses:** no other workspace crate directly; native builds select the relevant
socket/network support.

**Used by:** Host, Model, Contracts, Session, Simulation and Grid. Browser
transport is its JavaScript counterpart on the wire, not a Rust crate importer.

**Work here for:** native connection handling, relay framing or how two transport
implementations are paired. A payload's game meaning belongs in Model and the
simulation domains.

Native redialling sockets queue ordered Opened/Closed/Text events. The protocol
consumes a missed disconnect even if redial completed between two frame polls,
retiring old crew and registration before handling a new Ready.

## Platform

**Owner:** [`phoenix-platform`](../../crates/phoenix-platform/src/lib.rs).

**Responsible for:** machine-facing mechanisms: atomic files, monitor identity,
pane geometry, input routing, document surfaces, bounded frame pools and frame
lifetimes. Optional features provide GPU upload, Ultralight integration and
audio decoding.

**Uses:** no other workspace crate directly. Hosts select its optional platform
and rendering libraries through features.

**Used by:** Host, Content, Simulation and Grid.

**Work here for:** a surface allocation, frame upload, file replacement or input
coordinate calculation. Native host code still chooses which Station occupies
a pane and orders its game bridge; Platform supplies the underlying surfaces.
The pane loop configures its frame sink only after successful surface creation
or resize, replacing the bounded pool generation together with the surface.


## Maths

**Owner:** [`phoenix-math`](../../crates/phoenix-math/src/lib.rs).

**Responsible for:** deterministic numerical helpers, shared transcendental
functions, composite-key random value derivation, bounded histories, audio
configuration calculations and the pure LOD tuning calculations.

**Uses:** no other workspace crate; its external dependencies are `libm` and
`serde`. It has no Bevy dependency.

**Used by:** Host, Content, Contracts, Gameplay, World, Simulation and
Presentation directly.

**Work here for:** a numerical operation that must produce the same result on
native and WASM, or a generic bounded-history calculation. The simulation RNG
resource and tick ownership live in Contracts.

## Browser UI and Workshop

**Owners:** [`gui/`](../../gui/), [`editor/`](../../editor/), and root HTML entry
points such as [`client.html`](../../client.html) and
[`server.html`](../../server.html). These are application areas rather than one enforced package boundary.

**Responsible for:** phone Consoles, host lobby and settings, GM controls,
state-to-screen projection, input action mapping, shared web components and
localisation. Workshop adds document editing, validation feedback, asset and
pack management, undo, model previews and test-run controls. `gui/` supplies
screens and adapters; `editor/` contains much of the document and authoring logic.
The host page also connects the Rust/WASM host to browser networking and chrome.

**Uses:** Browser transport and Browser session through Phoenix adapters; shared
UI and editor helpers; authored assets and the String Table. The host page calls
Rust/WASM exports. Consoles exchange commands and state with the authoritative
host over the transport. Workshop uses browser/native provider bridges and
preview/test hosts. The phone Console itself loads no WASM.

**Used by:** browser entry pages and embedded native documents, including local
Station panes, the lobby and Workshop. Tests exercise both helpers and complete
pages. These application areas also import one another's helpers. The graph's
browser UI node covers `gui/` and root HTML imports; `editor/` is described here
but is not enumerated as a separate graph node.

**Work here for:** a Console control or layout, how received state is shown, or
an authoring workflow. Useful entry points are
[`action-map.js`](../../gui/action-map.js),
[`sim-state.js`](../../gui/sim-state.js) and
[`workshop-provider.js`](../../editor/workshop-provider.js).
Authoritative game rules remain in Rust.

## Browser transport

**Owner:** [`packages/transport`](../../packages/transport/src/).

**Responsible for:** reusable JavaScript connection machinery: join-code
parsing, rendezvous signalling, WebRTC setup, reconnect, delivery classes and
WebSocket relay. It also owns the service registry and relay mechanisms that
the rendezvous Worker composes. Game identity and configuration are supplied by
its callers.

**Uses:** its own modules and injected/browser networking APIs. It has no imports
from Phoenix UI, Browser session or the Rust game packages.

**Used by:** Phoenix adapters in `gui/`, the Grid console/host adapter, and the
rendezvous Worker. Local rendezvous tooling and tests also consume its service
mechanisms. Native hosts interoperate through the protocol.

**Work here for:** WebRTC establishment, generic reconnect or relay behavior.
Phoenix-specific join configuration and UI behavior belong with their adapters
and authored configuration.

## Browser session

**Owner:** [`packages/session`](../../packages/session/src/).

**Responsible for:** browser identity and continuation mechanisms: per-tab
session-token selection, persistence and tab liveness, fleet continuation
journals and their wire helpers. Callers inject application-specific identity
and configuration.

**Uses:** its own modules and supplied browser storage/window facilities. It has
no imports from Browser transport or Phoenix application modules.

**Used by:** Phoenix browser adapters and the Grid example.

**Work here for:** preserving an identity across reloads without making two tabs
claim the same identity, or generic continuation reconciliation. Whether that
identity can hold a Station is decided by the authoritative simulation Session.

## Cloud services

Two independently deployed JavaScript services support joining and connection
establishment.

**Rendezvous Worker** — [`worker-rendezvous/src/`](../../worker-rendezvous/src/index.js)
owns the Cloudflare Worker/Durable Object adapter, join-code lookup, signalling
and fallback frame relay. **Uses:** Browser transport's service registry/relay
and the authored join-code configuration, plus Cloudflare's runtime APIs.
**Used by:** browser hosts and Consoles and native hosts that enable the cloud
connection, through WebSocket messages. Change this service for deployment and
socket lifecycle concerns; change the shared package for reusable registry rules.

**TURN credential Worker** — [`worker/src/index.js`](../../worker/src/index.js)
returns temporary ICE server credentials, keeping provider secrets on the
service. **Uses:** configured Metered and/or Cloudflare TURN credential APIs and
its origin configuration; it imports no internal game package. **Used by:**
browser networking setup over HTTP. It supports connection establishment and
does not execute game rules.

## Grid reuse example

**Owners:** [`phoenix-grid`](../../examples/grid/src/lib.rs) and its
[JavaScript page adapter](../../examples/grid/app.js).

**Responsible for:** a separate small authoritative grid game demonstrating
that the foundation packages work without Phoenix's game domains. It exercises
command ordering, readiness, digest history, checkpoint transfer, reconnection
and native/browser hosting with a pure JavaScript console.

**Uses:** Rust Runtime, Transport and Platform directly. Its JavaScript uses
Browser transport and Browser session. It imports no Phoenix Model, Content,
Simulation or Presentation package.

**Used by:** developers checking reuse and [`tests/layers/grid-smoke.mjs`](../../tests/layers/grid-smoke.mjs).
Phoenix does not need Grid to run.

**Work here for:** demonstrating a foundation capability independently of the
spaceship game. See the [Grid guide](../../examples/grid/README.md) for launch
and verification commands.

Grid native and WASM adapters share typed application admission in
`protocol::apply` and reusable connection ownership. The browser adapter owns
physical peer handles and terminal callbacks. The smoke suite exercises actual
native checkpoint-file restart and exact native/WASM continuation from the same
checkpoint, including pending moves and issuer sequence.

## Cross-module tests

**Owners:** [`tests/`](../../tests/), plus owner-local Rust test siblings and
[`editor/tests/`](../../editor/tests/).

**Responsible for:** proving behavior at the appropriate boundary. Unit tests
sit with their owner; composition tests stay in Simulation; root Rust
integration tests cover complete hosts and deterministic missions. Vitest
covers browser helpers and Console behavior. Playwright drives real browser
hosts and pages. Grid smoke tests check reuse across native and browser hosts.

**Uses:** the public APIs, fixtures, assets and host entry points needed by each
case. Tests for Gameplay, World and Session obey the same ban on sibling/parent
imports as production code.

**Used by:** developers and CI. Production layers do not depend on the test
suite. Tests spanning several owners stay at their integration boundary.

**Work here for:** a regression involving more than one layer, protocol
integration, deterministic replay or a user journey. Add a narrow owner-local
test when only that owner's behavior is involved.

## Authored assets

**Owner:** [`assets/`](../../assets/). Authored data is a separate part of the
architecture alongside the Rust and JavaScript packages.

**Responsible for:** authored hulls, components, worlds, tunable game values,
models, rigs, sounds, strings and join-code configuration.

**Uses:** the schemas and vocabulary understood by Model, Gameplay and World;
includes and asset references link data files. These are data relationships,
not package imports.

**Used by:** Content during preparation, Gameplay and World during parsing,
Simulation during materialization, Presentation and browser UI when displaying
assets, and Workshop while editing them.

**Work here for:** scenario content, balance values, hull definitions or display
text. The String Table at `assets/strings/strings.csv` owns player-visible text.
Before scenario design work, run `uv run pasm design digest`; use the design
writeback workflow where possible, as described in [AGENTS.md](../../AGENTS.md).

## Following a change across layers

A weapon change illustrates the separation. The authored hull supplies its
values. Content resolves its includes, Gameplay parses the complete entity
configuration, and Simulation creates the live entity. A Console sends an input
through Browser transport; the host's admission path uses Session participation
state and Contracts authority rules. Gameplay supplies the weapon mechanics,
with Simulation scheduling and connecting the live systems. Presentation draws
the result while browser Consoles receive state updates.

Changing only the weapon's damage value starts in Assets. Changing the damage
rule starts in Gameplay. Changing who can fire it starts in authority/Session
and its admission adapter. Changing how the shot looks starts in Presentation.
A feature spanning those concerns legitimately touches several owners, with
the integration remaining in the parent.

## Maintaining the guide and graph

Code and manifests define the current dependency edges. The
[PASM architecture slice](../../pasm/spec/architecture/reusable-layers.yaml)
records the intended boundaries; this guide explains how to navigate their
current implementation. The [layer policy](../../scripts/layer-policy.mjs)
sets permitted edges, which can be broader than the edges actually declared.

The HTML graph shows workspace crates and JavaScript layer imports. External
libraries and authored data are outside its graph. Solid arrows include runtime
dependencies; dashed arrows are test/build-only. Regenerate it with
`npm run layers:graph` after changing Cargo declarations or JavaScript imports;
`node scripts/generate-layers-graph.mjs --check` checks for drift. Update this
guide when ownership or its direct dependency lists change.

## Focused checks

```sh
npm run layers:check
node scripts/check-layers.mjs --dependency-tree
cargo test -p phoenix-runtime -p phoenix-transport -p phoenix-model -p phoenix-content
cargo test -p phoenix-sim-contracts -p phoenix-sim-gameplay -p phoenix-sim-world -p phoenix-sim-session --lib
cargo test -p phoenix-simulation --lib --no-default-features
cargo test -p phoenix-grid
```

The dependency check covers target-specific, dev and build Cargo dependencies,
JavaScript static/dynamic imports, and independently selected dependency
graphs for the parent and every simulation branch. Workspace feature unification is not evidence of independence.

[Grid](../../examples/grid/README.md) exercises the shared command, digest,
recovery and network code through a native host and a WASM browser host. Its
console is pure JavaScript. `tests/layers/grid-smoke.mjs` drives both with a real
browser against the local rendezvous service.
