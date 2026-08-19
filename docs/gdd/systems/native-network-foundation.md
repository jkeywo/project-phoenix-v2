# Project Phoenix — Native and Network Foundation

| Field | Value |
|---|---|
| Status | Accepted for Band A2 — Bridge Foundations |
| Scope | Join codes, rendezvous, WebSocket/RTC transport, host recovery, native Windows hosting, Ultralight consoles, multi-monitor/touch and local media-device setup |
| Audience | Product, networking, native client, UI, simulation, operations and test |

Band A2 front-loads the technical foundation needed for reliable multi-ship play and optional physical bridges. The zero-setup browser host and phone-console route remains complete. The native route uses the same authoritative simulation, commands, station ownership, projections and content rather than becoming a separate edition of the game.

Related documents: [Game and Session Lifecycle](../foundation/game-lifecycle.md), [Future Modes and Optional Bridge Extensions](../future/future-modes.md), [Command and Crew Control](../mechanics/command-and-crew-control.md), [Onboarding and Accessibility](../foundation/onboarding-accessibility.md), [AI and Backfill](./ai-and-backfill.md), and [Planned but Not Scheduled](../future/planned-not-scheduled.md).

## Join-code model

Phoenix uses human-readable structured join identifiers in the form `PROJECT_GUID_VERSION_GUID_CODE`, where `CODE` is five letters. Normal operators type only the five-letter suffix; a client or host setup screen supplies its own project and version GUIDs. The full form remains available for copying, QR codes, diagnostics and launchers.

Client joining and server joining use separate project GUIDs and therefore separate namespaces. A compatible release uses the same version GUID in both. Every player-ship host issues its own client code, which is transport metadata never shared with the simulation or other ship hosts. A multi-ship session has one privileged server code used to admit ship hosts and recover them.

Suffixes use uppercase letters, reject offensive or misleading words and are collision-checked within their typed project/version namespace. Input is case-insensitive; `0` normalises to `O`, and `1` or `L` normalises to `I`. `J` remains distinct. Entering the wrong code type produces a specific error without revealing further session information.

Codes remain stable for the multi-ship session, including mission transitions, disconnects and host replacement. A ship host may regenerate its client code between missions. The fleet may regenerate the server code only while no mission is running. Ending the session releases every code.

The rendezvous service can detect a suffix registered under another active version of the same project and return a useful version-mismatch result. The eventual host handshake still performs the authoritative protocol/content version check, so friendly discovery errors cannot weaken compatibility admission.

## Ship-host admission and recovery

Before a mission, the server code admits new ship hosts while server joining is open. The lead host may close or regenerate admission once the fleet is assembled without disconnecting admitted ships. Mission start freezes player-ship slots and, when customisation exists, their loadouts. After that point the server code cannot create another player-ship slot, but it can be entered on any replacement machine to claim a currently disconnected slot. Connected ships cannot be displaced; the first synchronised valid claim wins if replacements race.

A disconnected player ship is operated through ordinary station/system Backfill on the remaining host machines after the disconnect transition is agreed for one logical tick. The ship does not disappear, freeze or switch to a second simplified simulation. A replacement host restores the fixed ship slot from the shared snapshot and command history, then resumes authority after agreement.

Each ship's existing client code is rebound to its replacement host through the rendezvous layer. Other hosts and the simulation do not learn it. Existing phones and local clients can reconnect without the crew distributing a new code during combat.

## Transport replacement

Phoenix replaces PeerJS with a project-owned transport split. A secure WebSocket connection to the rendezvous service handles typed join-code lookup, presence, admission, WebRTC signalling and reconnection coordination. Direct WebRTC DataChannels carry gameplay traffic between machines, retaining reliable ordered commands and a lossy unordered snapshot path where appropriate. WebRTC media tracks are reserved for later ship-to-ship voice/video. Native local station panes use an in-process adapter behind the same logical connection interface.

The transport retains ICE, STUN and managed TURN. It must support same-LAN play, restrictive public Wi-Fi, multiple devices on one mobile hotspot and bridges on separate mobile networks without inbound port configuration. Relay availability, candidate types, connection attempts, escalating timeouts and degraded fallback remain visible in diagnostics. A secure WebSocket relay may carry game messages when direct RTC fails; ordinary signalling sockets are not a media relay.

The host-to-host protocol remains distinct from player `ClientMessage`/`ServerMessage` traffic. Lockstep command ordering, snapshot join/recovery, hash diagnostics and synchronised Backfill transitions use the existing deterministic tick, command-log and snapshot foundations rather than introducing parallel state paths.

## Native Windows host

The first true native target is Windows. One native process runs the authoritative Bevy simulation, renders the shared viewscreen through native Bevy/wgpu and creates local Ultralight station surfaces. It continues to accept ordinary browser and phone clients. The current native delivery server remains useful for bundle/content publication but no longer defines the ceiling of native hosting.

Every local station pane is an isolated Ultralight view with its own player identity, document, client state, permissions, input queue and lifecycle. It joins, names its player, claims a station, readies, disconnects and goes AFK through the same rules as a phone. In-process delivery may avoid network serialisation but cannot bypass command admission or projection boundaries. A failed or reloaded pane follows ordinary disconnect and Backfill behaviour without pausing the simulation or another pane.

The web and Ultralight console implementations should match as closely as possible. Both consume the shared Hero Bar and complete station surfaces defined in [Command and Crew Control](../mechanics/command-and-crew-control.md). Native-only controls belong to host setup, not to a privileged station UI.

## Displays, touch and bridge profiles

Every configured monitor is covered by a borderless full-screen Phoenix surface. A monitor assigned to stations displays either one full station or two fixed station panes. More than two stations per monitor is unsupported. A viewscreen monitor presents the native shared view; a station monitor presents its assigned pane layout. The system mouse may traverse the complete Windows extended desktop and interact with every surface for local testing.

Each touchscreen manages its own contacts. Physical display coordinates map into that monitor's pane layout, and every contact ID remains captured by the pane where it began until lift. A drag cannot leak into a neighbouring station or another monitor. Simultaneous touches on separate screens remain independent; mouse and keyboard focus follow explicit interaction.

A native setup screen detects displays and saves reusable bridge profiles. A profile records viewscreen/station roles, one- or two-pane layouts, station assignments, window placement, touch mapping and media devices. Missing monitors produce a recoverable setup warning; their stations are not silently moved to another display. Loss of a station display during play disconnects that local identity and invokes Backfill until the pane returns or the seat is claimed normally.

Accessibility preferences belong to each pane's private player profile, not to the physical bridge profile. Web and native panes use the same Accessibility settings surface, take OS text/contrast/motion preferences as initial defaults and permit explicit overrides. Text scaling must preserve supported one- and two-pane layouts, and every touchscreen setup action has a keyboard/mouse route. Only a derived functional eligibility or assistance request may cross the client boundary; diagnostic or medical reasons do not.

## Local media devices

Band A2 establishes multi-device routing without shipping inter-ship calls. Every viewscreen or station surface can author or select its own camera, microphone and audio output. Shared devices require explicit configuration rather than automatic reuse. The setup screen warns about conflicting or unsupported claims, persists assignments and provides camera preview, microphone level/loopback and output tests. A missing device disables only that media path, never the station or simulation.

This boundary permits a private Comms endpoint and a separate bridge-wide viewscreen endpoint inside one native process. The planned ship-to-ship media feature can later hand an established call between them without redesigning native device ownership.

## Band A2 acceptance criteria

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
