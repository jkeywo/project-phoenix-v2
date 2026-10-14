# Project Phoenix — Native and Network Foundation

| Field | Value |
|---|---|
| Status | Current native/network implementation with Plan completion and acceptance |
| Last updated | 2026-10-02 |
| Scope | Join codes, rendezvous, WebSocket/RTC transport, host recovery, native Windows hosting, Ultralight consoles, multi-monitor/touch and local media-device setup |
| Audience | Product, networking, native client, UI, simulation, operations and test |

T1 front-loads the technical foundation needed for reliable multi-ship play and optional physical bridges. The zero-setup browser host and phone-console route remains complete. The native route uses the same authoritative simulation, commands, station ownership, projections and content rather than becoming a separate edition of the game.

## Current delivery and Plan boundary

**Current:** Phoenix codes/rendezvous/WebRTC and same-frame WebSocket fallback; two-host lockstep/snapshot/recovery code; native Windows simulation/Viewscreen; isolated Ultralight panes, runtime lobby/picks/QR, monitor movement and one/two-console Station screens; host-owned Station reservations, input routing and saved per-class layouts. Native hosts accept phones directly over LAN and may add cloud joining. PeerJS is retired.

**Plan — T1 completion/acceptance:** media setup and hardware proof #1126, private Accessibility/setup acceptance #1127/#1128, real-monitor run #1335 and direct-LAN burst defect #1354 remain open in the PASM audit. The OS preference reader now uses Windows UISettings/AccessibilitySettings; the old kit's stub statement is stale, but this does not establish human acceptance.

**Plan — T5:** larger-session recovery, scaling, performance and real-network/long-session/human acceptance remain open despite implemented fixed slots, six-peer tests and reference worlds. Protocol/setup descriptions below include intended contracts rather than a completed field-test matrix. See [Current Baseline](../foundation/current-baseline.md) and [Plan Baseline](../future/roadmap-baseline.md).

Related documents: [Game and Session Lifecycle](../foundation/game-lifecycle.md), [Future Modes and Optional Bridge Extensions](../future/future-modes.md), [Command and Crew Control](../mechanics/command-and-crew-control.md), [Onboarding and Accessibility](../foundation/onboarding-accessibility.md), [AI and Backfill](./ai-and-backfill.md), and [Planned but Not Scheduled](../future/planned-not-scheduled.md).

## Join-code model

Phoenix uses human-readable structured join identifiers in the form `PROJECT_GUID_VERSION_GUID_CODE`, where `CODE` is five letters. Normal operators type only the suffix; a client or host setup screen supplies its own project and version GUIDs. The full form remains available for copying, QR codes, diagnostics and launchers.

Client joining and server joining use separate project GUIDs and therefore separate namespaces. A compatible release uses the same version GUID in both. Every player-ship host issues its own client code, which is transport metadata never shared with the simulation or other ship hosts. A multi-ship session has one privileged server code used to admit ship hosts and recover them.

Suffixes use uppercase letters, reject offensive or misleading words and are collision-checked within their typed project/version namespace. Input is case-insensitive; `0` normalises to `O`, and `1` or `L` normalises to `I`. `J` remains distinct. Entering the wrong code type produces a specific error without revealing further session information.

Codes remain stable for the multi-ship session, including mission transitions, disconnects and host replacement. A ship host may regenerate its client code between missions. The fleet may regenerate the server code only while no mission is running. Ending the session releases every code.

The rendezvous service can detect a suffix registered under another active version of the same project and return a useful version-mismatch result. The eventual host handshake still performs the authoritative protocol/content version check, so friendly discovery errors cannot weaken compatibility admission.

## Ship-host admission and recovery

Before a mission, the server code admits new ship hosts while server joining is open. The lead host may close or regenerate admission once the fleet is assembled without disconnecting admitted ships. Mission start freezes player-ship slots and, when customisation exists, their loadouts. After that point the server code cannot create another player-ship slot, but it can be entered on any replacement machine to claim a currently disconnected slot. Connected ships cannot be displaced; the first synchronised valid claim wins if replacements race.

A disconnected player ship is operated through ordinary station/system Backfill on the remaining host machines after the disconnect transition is agreed for one logical tick. The ship does not disappear, freeze or switch to a second simplified simulation. A replacement host restores the fixed ship slot from the shared snapshot and command history, then resumes authority after agreement.

Each ship's existing client code is rebound to its replacement host through the rendezvous layer. Other hosts and the simulation do not learn it. Existing phones and local clients can reconnect without the crew distributing a new code during combat.

## Current Phoenix transport

Phoenix has replaced PeerJS. Secure WebSocket owns rendezvous/code lookup/signalling; browser WebRTC carries reliable commands and a separate lossy snapshot class. If no direct link forms, the same rendezvous socket carries the game's own frames with delivery classes and Admission preserved. Native hosts use that cloud relay or their direct LAN listener; local panes use an in-process adapter. **Plan — unscheduled:** ship-to-ship voice/video.

The transport retains ICE, STUN and managed TURN. It must support same-LAN play, restrictive public Wi-Fi, multiple devices on one mobile hotspot and bridges on separate mobile networks without inbound port configuration. Relay availability, candidate types, connection attempts, escalating timeouts and degraded fallback remain visible in diagnostics. A secure WebSocket relay may carry game messages when direct RTC fails; ordinary signalling sockets are not a media relay.

The host-to-host protocol remains distinct from player `ClientMessage`/`ServerMessage` traffic. Lockstep command ordering, snapshot join/recovery, hash diagnostics and synchronised Backfill transitions use the existing deterministic tick, command-log and snapshot foundations rather than introducing parallel state paths.

## Native Windows host

The first true native target is Windows. One native process runs the authoritative Bevy simulation, renders the shared viewscreen through native Bevy/wgpu and creates local Ultralight station surfaces. It continues to accept ordinary browser and phone clients. The current native delivery server remains useful for bundle/content publication but no longer defines the ceiling of native hosting.

Each local pane has an ordinary session token and isolated document/state/input. Manually opened participant panes use ordinary claim/readiness. Lobby-assigned Station consoles have host-owned reservations: competing claims, switching and voluntary release are refused; Host Off releases them. Rebuilds retain token/reservation and Backfill covers disconnection. In-process delivery cannot bypass Admission or projection boundaries.

The web and Ultralight console implementations should match as closely as possible. Both consume the shared Hero Bar and complete station surfaces defined in [Command and Crew Control](../mechanics/command-and-crew-control.md). Native-only controls belong to host setup, not to a privileged station UI.

## Displays, touch and bridge profiles

Every configured monitor is covered by a borderless full-screen Phoenix surface. A monitor assigned to stations displays either one full station or two fixed station panes. More than two stations per monitor is unsupported. A viewscreen monitor presents the native shared view; a station monitor presents its assigned pane layout. The system mouse may traverse the complete Windows extended desktop and interact with every surface for local testing.

Each touchscreen manages its own contacts. Physical display coordinates map into that monitor's pane layout, and every contact ID remains captured by the pane where it began until lift. A drag cannot leak into a neighbouring station or another monitor. Simultaneous touches on separate screens remain independent; mouse and keyboard focus follow explicit interaction.

The current native lobby detects displays, moves the Viewscreen and opens/moves/closes Station consoles at runtime. One Station window tiles whole occupancy, including authored participant reservations, at at most two consoles. Authored split/order is preserved and saved per-class layouts reuse assignments. Missing monitors invoke reconciliation/Backfill; assigned reservations survive until Host Off or bounded repair failure surrenders the seat. Recovery cannot place a rebuilt console over the Viewscreen. Touch routing exists; real multi-touch/display acceptance remains open.

Accessibility preferences belong to each pane's private player profile, not to the physical bridge profile. Web and native panes use the same Accessibility settings surface, take OS text/contrast/motion preferences as initial defaults and permit explicit overrides. Text scaling must preserve supported one- and two-pane layouts, and every touchscreen setup action has a keyboard/mouse route. Only a derived functional eligibility or assistance request may cross the client boundary; diagnostic or medical reasons do not.

## Plan — local media completion and hardware acceptance

T1's intended setup lets each surface select camera, microphone and output, warn about conflicting claims, persist assignments and run local tests. Existing routing and T4 audio/output capabilities are Current; complete media setup and real-hardware proof remain **Plan — #1126**. Human inter-ship calls remain **Plan — unscheduled**. A missing device must disable only its media path.

This boundary permits a private Comms endpoint and a separate bridge-wide viewscreen endpoint inside one native process. The planned ship-to-ship media feature can later hand an established call between them without redesigning native device ownership.

## T1 contracts and remaining acceptance targets

- Typed client/server codes resolve within separate project namespaces, give specific wrong-type/version errors and survive supported reconnect/recovery paths.
- Mission start freezes ship slots and loadouts; the server code restores only disconnected fixed slots after start.
- The PeerJS replacement works through direct and TURN-relayed WebRTC on the supported network setups without inbound port configuration.
- A disconnected ship transitions to synchronised Backfill and can be restored on a replacement machine without resetting the mission.
- The native Windows host runs the ordinary authoritative simulation, renders the viewscreen natively and supports mixed native/web participants.
- Every local pane has an isolated player identity and the same authority/projection boundaries as a web client.
- Full-screen monitor profiles support one or two station panes, cross-screen mouse testing and independently captured touchscreen contacts.
- Bridge profiles persist display, touch and per-surface media-device assignments and recover visibly from missing hardware.
- Browser-host play remains complete and is not demoted to a compatibility mode.

## Out of scope

- macOS or Linux native packaging in the first native release.
- More than two stations on one physical monitor.
- Exclusive native-only gameplay commands or information.
- Public internet hosting without TLS, TURN, admission controls and deployment diagnostics.
- Ship-to-ship voice/video, Discord integration and recording; their accepted design is planned but unscheduled.

## Canonical sources

- [PRD #1093 — Band A2 Bridge Foundations: Native and Network Foundation](https://github.com/jkeywo/project-phoenix-v2/issues/1093)
- [Phoenix delivery roadmap](../../../pasm/spec/roadmap/phoenix-delivery-roadmap.yaml)
- `pasm/spec/design/p2p-design-deltas.yaml`
- `pasm/spec/architecture/native-delivery.yaml`
- [Command and Crew Control](../mechanics/command-and-crew-control.md)
