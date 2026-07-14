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
