# Project Phoenix — Product Requirements Document

## Problem Statement

Running a tabletop-style spaceship bridge simulator requires either dedicated hardware/software per player station, or players crowding around a single screen. Existing solutions (e.g. Artemis SBS) require each player to install a desktop client and be on the same local network, creating significant setup friction before play can begin. There is no mainstream bridge simulator that lets a group of players join instantly from their own phones — the device they already have in their pocket.

---

## Solution

Project Phoenix is a browser-based bridge simulator where one browser tab (the "view screen") drives the game and displays a shared 3D view of space, while each player connects from their phone by scanning a QR code. No installation required on any device. Players pick their station, the captain hits "Engage", and the game begins.

The game is built in Rust/Bevy compiled to WebAssembly for the view screen, with phone consoles served as plain HTML pages. Networking uses PeerJS (WebRTC) in a star topology — the view screen is the authoritative host, phones are spokes.

---

## User Stories

### Joining a Game

1. As a player, I want to join a game by scanning a QR code with my phone, so that I can connect without installing anything or typing a URL manually.
2. As a player, I want to be assigned a random space-themed name when I first open the client page, so that I can get started quickly without having to think of one.
3. As a player, I want to edit my name before the game starts, so that I can personalise my character.
4. As a player, I want my name and session to be remembered across browser refreshes and reconnects, so that I can recover from crashes without losing my spot.
5. As a player, I want to see all available consoles and which ones are already taken, so that I can choose a role that fits the team.
6. As a player, I want to select a console and have my choice reflected immediately on the view screen lobby, so that other players can see who is sitting where.
7. As a player, I want to be able to deselect my console and pick a different one before the game starts, so that the team can reorganise freely.
8. As a player, I want to see other players' names appear on the view screen lobby as they connect, so that everyone can see who has joined.

### Starting the Game

9. As the captain, I want to see an "Engage" button in my lobby, so that I can start the game when the crew is ready.
10. As a player, I want my screen to automatically transition from the lobby to my console when the captain starts the game, so that I don't have to do anything manually.
11. As the host, I want the view screen to automatically transition from the lobby to the 3D view when the game starts, so that the shared display updates for everyone in the room.

### Playing the Game

12. As the captain, I want a Red Alert toggle button on my console, so that I can signal an emergency to the crew.
13. As a player, I want to see the view screen react visually when Red Alert is activated, so that the shared display reinforces the game state.
14. As a player, I want my console to reflect the current Red Alert status, so that I know the ship's condition at a glance.
15. As a player watching the view screen, I want to see a 3D representation of space rendered in real time, so that the game feels immersive for the whole room.

### Reconnecting and Late Joining

16. As a player who lost connection, I want to reconnect and be automatically placed back at my previous console, so that I can resume play without interrupting the game.
17. As a late joiner, I want to see a list of available consoles even after the game has started, so that I can pick a role and join in progress.
18. As a late joiner, I want a clear "Join In Progress" button, so that I understand how to enter an already-running game.
19. As a player swapping devices mid-game, I want my session token on the new device to reclaim my old console, so that I can switch hardware without losing my role.
20. As a player, I want a disconnected player's console to become available immediately, so that another player can take over if needed.

### Hosting

21. As the host, I want the view screen to display a QR code in the lobby, so that players can join without me needing to share a URL manually.
22. As the host, I want the view screen to show each player's name next to their chosen console in the lobby, so that I can see at a glance who is ready.
23. As the host, I want the game to run entirely in a browser tab with no server to maintain, so that I can run a session anywhere with an internet connection.

---

## Implementation Decisions

### Modules

**1. Serialization Abstraction Layer**
A Rust trait (e.g. `MessageCodec`) that encodes and decodes `ClientMessage` and `ServerMessage` values to/from byte buffers. The production implementation wraps `serde_json`. The interface is the only surface the rest of the codebase touches — `serde_json` is never called directly outside this module. Designed so the implementation can be swapped to a binary format (e.g. MessagePack) by changing one module.

**2. Message Type Definitions**
A pure-data Rust module defining the canonical `ClientMessage` and `ServerMessage` enums, the `Console` enum, `GamePhase`, `GameState`, `Player`, and `SimSnapshot` types. No logic, no I/O — only data shapes and their derivations (`Clone`, `Debug`, `serde` traits). Shared across all other Rust modules.

**3. Session Manager**
A Rust component/resource tracking session tokens → player records (name, console assignment, connection status). Responsible for: registering new players, recognising returning tokens on reconnect, auto-assigning previous console, and releasing consoles on disconnect. Exposes a clean interface that the lobby system queries and mutates — it has no knowledge of Bevy systems or PeerJS.

**4. Lobby System (Bevy)**
Bevy systems and resources managing the pre-game phase: handling `Identify`, `SetName`, `SelectConsole`, `ClearConsole`, and `StartGame` messages from clients; validating authorship (e.g. only captain can start); producing outbound `ServerMessage` events in response. Transitions game phase from `Lobby` to `InProgress`.

**5. Game Simulation (Bevy)**
Bevy systems running the in-game simulation. For the PoC this is minimal: a `RedAlertState` resource toggled by captain input. Grows to encompass ship movement, power, combat, and sensors in later milestones. Produces `SimState` snapshots broadcast at 10Hz.

**6. View Screen Renderer (Bevy)**
Bevy rendering systems for the server page `<canvas>`: lobby display (player list, QR placeholder), and the 3D view screen (PoC: rotating cube that reacts to `RedAlertState`). Driven by game state — no direct PeerJS knowledge.

**7. WASM/JS Bridge**
The `wasm-bindgen`-exposed API surface between Bevy and the JS layer:
- `wasm_receive_message(json: &str)` — JS calls this to deliver an inbound message into Bevy.
- `set_message_callback(fn)` — JS registers a callback; Bevy calls it with outbound JSON strings.
- `wasm_init()` — entry point called by JS on page load.

**8. Server Page JS Shell**
JavaScript running on `server.html`. Owns the PeerJS host peer lifecycle: generates UUID peer ID, creates QR code via `qrcode.js`, accepts incoming connections, routes inbound messages to WASM, routes outbound WASM messages to the correct peer(s), self-loopbacks host messages back into WASM, fires `ConsoleCleared` when a connection closes.

**9. Client Page Application**
Standalone HTML + JS on `client.html`. Manages: reading the peer ID from the URL fragment, generating/loading the session token from `localStorage`, generating a random space-themed name, rendering lobby UI (name input, console picker, Engage / Join In Progress button), rendering console UI (PoC: Red Alert toggle), connecting to the host peer via PeerJS, sending `ClientMessage` values, and updating UI in response to `ServerMessage` events.

**10. Build Configuration**
Two `Trunk.toml` configurations (one per HTML entry point) and a GitHub Actions workflow that builds both, merges their outputs into `dist/`, and deploys to the `gh-pages` branch via `peaceiris/actions-gh-pages` on every push to `main`.

### Architectural Decisions

- **Star topology:** The server page is the sole authoritative host peer. Clients never communicate directly with each other.
- **JS owns networking:** PeerJS is handled entirely in JavaScript. Bevy/WASM is never aware of PeerJS; it only sees decoded message values.
- **Event stream + connect snapshot:** Clients receive a full `GameState` snapshot on connect, then receive discrete events. Continuous simulation data is pushed at 10Hz as `SimState` snapshots.
- **Session tokens for identity:** Each client stores a UUID v4 session token in `localStorage`. The server maps tokens to player records. PeerJS peer IDs are ephemeral and not used for identity.
- **Console vacancy on disconnect:** Disconnection immediately releases the console in all game phases. Players coordinate console ownership themselves.
- **Hybrid tick rate:** Discrete events fire immediately. Continuous simulation state (`SimState`) broadcasts at 10Hz.
- **Single Rust crate, two entry points:** One `wasm-pack`/`trunk` build produces a single WASM binary. Both `server.html` and `client.html` can load it. The client page is pure HTML/JS for the PoC but retains the option to load WASM for future 3D consoles.
- **WebGL2 rendering:** Chosen for broad browser support including mobile, sufficient for all foreseeable PoC and near-term game rendering needs.
- **PeerJS cloud broker:** Used for WebRTC signalling. Adequate for low-player-count sessions. Self-hosting deferred post-PoC.

---

## Testing Decisions

### What Makes a Good Test

Tests should verify external behaviour through a module's public interface, not its internal implementation. A good test: sets up state, performs an action, and asserts on observable output. Tests must not assert on private fields, internal call counts, or implementation-specific side effects.

### Modules to Test

**Session Manager** — the highest-value test target. It is a pure logic module with no Bevy or network dependencies. Tests should cover: new player registration, duplicate token detection, console assignment and clearing, returning-token auto-assignment, disconnection vacancy, and querying available consoles.

**Message Type Definitions + Serialization Abstraction** — round-trip tests: serialize a `ClientMessage` / `ServerMessage` value, deserialize it, assert equality. One test per message variant. These act as a regression guard if the wire format changes and also validate that the codec abstraction works correctly.

**Lobby System** — tested via Bevy's `App`-level test harness. Spin up a minimal Bevy app with only the lobby systems registered, inject `ClientMessage` events, assert on the resulting `ServerMessage` events and game state changes. Tests should cover: console selection, deselection, name changes, start game validation (captain only), and phase transition.

### Modules Not Tested in Automated Tests (PoC)

- View Screen Renderer — visual output; manual testing is sufficient for PoC.
- WASM/JS Bridge — integration boundary; tested via end-to-end browser testing in a later milestone.
- Server Page JS Shell — PeerJS integration; end-to-end only.
- Client Page Application — UI; manual testing for PoC.
- Build Configuration — validated by CI passing.

---

## Out of Scope

- Additional consoles beyond Captain's Chair (Helm, Weapons, Comms, Science, Engineering).
- Actual ship simulation: movement, navigation, combat, sensors, power management.
- WASM loading on the client page (reserved for consoles that require 3D rendering).
- Self-hosted PeerJS signalling server.
- Binary wire format (architecture supports swapping; not implemented in PoC).
- Console UI for late-join auto-reconnect (server-side session token logic is implemented; client UI to display returning-player state is deferred).
- Spectator / observer mode.
- Multiple simultaneous game rooms.
- Authentication or access control.
- Mobile-native apps.

---

## Further Notes

- The QR code encodes the full client URL with the host's PeerJS peer ID in the URL fragment (e.g. `https://user.github.io/repo/client.html#abc123`). The fragment is never sent to a server.
- The PoC deliberately touches every layer of the stack (Rust/Bevy WASM → JS bridge → PeerJS → client HTML → user input → server state → rendered output) to surface integration issues early.
