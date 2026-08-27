# Project Phoenix Domain Vocabulary

Use these names consistently. Runtime detail belongs in code/wiki; intended behavior belongs in PASM.

- **Station**: an authored operable surface identified by `StationId`, owning a coherent UI and set of Systems. A primary Station may be a fixed player seat; an auxiliary Station may exist only as a hosted tab. A human-seeking Station retains its identity while being presented by another directly player-held Station.
- **System**: a fine-grained operable capability identified by `SystemId`.
- **Console Family**: the presentation taxonomy declared by a System-kind descriptor and projected per System instance to select client payload builders and dirty-console routing. It is separate from Station ownership and System command authority.
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
- **Host Spine**: the single Admission-facing AI host module (`ai::host`): one `decide` gate→declare→resolve step behind `AiHostEnv`, emitting through `AiEmitter`. Every fine-system AI host runs on it; it invents no second evaluator and never branches on who is operating a System.
- **Boot Profile**: the seam that selects app composition — `Headless`, `BrowserHost`, `BrowserAutomation` — so the plugin/asset/message/world-ingestion inventory is registered once in `boot::build` instead of hand-listed per entry point. `BrowserAutomation` and `Headless` share the render surrogate; only `BrowserHost` gets the real render stack.
- **State Census**: the declared digest-boundary classification of every authoritative type (`authoritative::StateCensus`), populated by `declare_state` at each owning plugin. It is inert to the digest and is what the authoritative-state enumeration guard reads instead of a hand-transcribed list.
- **PhElement**: the shared base for console web components; owns shadow/style/template setup, synchronous state-render, and live `sendAction` wiring so no component re-implements the ritual or needs a DOM-walk to be reachable.
