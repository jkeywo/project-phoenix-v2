# Project Phoenix Domain Vocabulary

Use these names consistently. Runtime detail belongs in code/wiki; intended behavior belongs in PASM.

- **Station**: a fixed player seat identified by `StationId` and authored by a ship.
- **System**: a fine-grained operable capability identified by `SystemId`.
- **Station Rating**: an authored station mode that selects which owned systems are AI-operated. `Backfill` operates every owned system.
- **Control Source**: `Human`, `Ai`, or `Offline` authority for a system on a tick.
- **Session**: host-owned player identity and connection record, keyed by session token.
- **World**: authored TOML content and its layered runtime; root worlds may load supporting worlds.
- **Objective**: authored player-facing goal and AI directive.
- **Viewscreen**: the shared host display.
- **Waypoint**: Navigation-owned shared destination.
- **Coordination**: inter-system advisory messages; it does not bypass system authority.
- **PASM**: Phoenix Architecture & System Model, the repository's design and architecture record.
- **String Table**: `assets/strings/strings.csv`, the single source of all display text, keyed by String Id; `[bracketed]` English marks unreviewed agent-drafted copy.
- **String Id**: stable dotted key (`console.common.no_target`) that code and TOML carry instead of prose; the client resolves it at render time.
- **Radar**: a per-station System kind (`helm-radar`, `tactical-radar`, `sensor-radar`) that owns contact projection and target selection for its station and publishes both in its own blackboard.
- **Combat Lock**: the ship-wide authoritative target, selected on the tactical radar and lifted onto the Viewscreen blackboard by the aggregator; weapons, helm, shields, and comms read it there.
- **Science Target**: the sensor radar's selected contact, lifted alongside the Combat Lock; advisory to Tactical via Coordination, never overriding Tactical authority.
- **Admission**: the seam where commands from any Control Source are validated against station/system authority and become anonymous per-entity commands; nothing downstream branches on who issued them.
- **Host Channel**: a named host-page-local outbound channel from the sim to `server.html` chrome (HUD, audio, shake); never sent to peers.
