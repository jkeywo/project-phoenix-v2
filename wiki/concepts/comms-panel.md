---
title: CommsPanelPlugin
---

# CommsPanelPlugin

Client-side Comms console: two-panel inbox + chat room layout.

## Location

- `gui/comms-console.html` — current pure-HTML phone Comms console iframe.
- `gui/comms-state.js` — current pure-JS Comms state model used by `client.html`.

- `src/console/comms/client.rs` — `CommsPanelPlugin` (Bevy, client feature)
- `src/client_comms.rs` — `ClientCommsState` (pure, Bevy-free, unit-tested)
- `src/console/comms/inbox.rs` — `CommsInbox` (pure server-side, no Bevy)

## Layout

The current HTML iframe keeps a two-column inbox/chat layout in landscape. In
portrait, selecting an inbox message adds `portrait-message` to the main panel so
the message replaces the list; the Back button clears the local `_selectedId`,
removes `portrait-message`, and re-renders the list with no selected row.

```
┌──────────────────┬───────────────────────────┐
│  Contacts strip  │                           │
│  (hail buttons)  │   CommsChatPanel          │
├──────────────────┤   ─ Back / On Screen      │
│  INBOX  Clear All│   ─ Contact messages      │
├──────────────────┤     (sender name + body)  │
│  CommsInboxList  │   ─ "You: …" reply bubbles│
│  (thread rows)   │   ─ Response buttons      │
│                  ├───────────────────────────┤
│                  │  CommsObjectivesFooter    │
└──────────────────┴───────────────────────────┘
```

## Threading model

All messages that belong to the same hail/dialogue tree share a `thread_id` UUID.
The server generates `thread_id` when the first message is injected; follow-up nodes
inherit the same id. Auto-triggered messages (no hail) each get their own `thread_id`
(single-message threads). Old wire payloads without `thread_id` default to `""` and
the client treats that as "own thread" (`effective_thread_id` falls back to `msg.id`).

The pure-HTML phone console (`gui/comms-console.html`) also uses `thread_id` as
the local selection key. Its inbox renders one row per thread, the chat panel
renders every message in the selected thread, and response buttons target the
latest unanswered message in that thread. This is what keeps the Before the Fire
Research Outpost handoff and delayed Dr. Myst briefing in one conversation while
still allowing the operator to reply to Dr. Myst.

### Auto-chained root follow-up (`[comms.follow_up]`)

A root `[[comms]]` block can declare a sibling `[comms.follow_up]` that fires
automatically after the root message is injected — no player response click
required. Used for one-way broadcasts that promise more dialogue
("Stand by — patching you through..."). The chained node:

- inherits the parent's `thread_id` (same conversation),
- supports its own `speaker` override (see "Multi-speaker channels" below),
- supports its own `trigger` (e.g. `on_timer` with `after_secs` for a delayed
  reveal — silent during the wait, no `...` placeholder for root chains),
- supports its own nested `[[comms.follow_up.response]]` tree (so the chained
  message can become a branching dialogue),
- inherits the parent template's `urgent` flag.

`[comms.follow_up]` is **mutually exclusive** with `[[comms.response]]` on the
root node: the parser hard-errors when both are present so authors pick one
shape per node — branching dialogue (responses) OR a chained monologue
(follow_up).

Worked example — Before the Fire Research Outpost (`assets/worlds/before_the_fire.toml`):

```toml
[[comms]]
thread_id = "research-scholar"
from      = "Research Outpost"
trigger   = "on_hailed"
entity    = "Research Outpost"
urgent    = true
message   = "...Stand by — patching you through to Dr. Myst now."

  [comms.follow_up]
  speaker    = "Dr. Myst"
  trigger    = "on_timer"
  after_secs = 3.0
  message    = "Ardent, this is Dr. Myst..."

    [[comms.follow_up.response]]
    text = "What happens if it fires?"

      [comms.follow_up.response.follow_up]
      speaker    = "Dr. Myst"
      trigger    = "on_timer"
      after_secs = 2.0
      message    = "If it fires at full charge..."
```

Implementation: scheduled in `handle_hail` (`src/world/server.rs:719`) and
`handle_ai_events` (`src/world/server.rs:1786`) by pushing a
`PendingFollowUp` onto `runtime.pending_follow_ups` whose `placeholder_id =
None`. The `tick_pending_follow_ups` system evaluates each pending
follow-up's `trigger` each tick against current world state (region
membership, flag store, live entity UUIDs) plus this tick's pending events;
ready follow-ups are injected and replace any `...` placeholder.

### Multi-speaker channels

Comms threads can contain multiple displayed speakers while staying anchored to
one physical or synthetic channel. In world TOML, top-level `from` is the radio
endpoint used for hailing, range checks, contact lookup, and synthetic broadcast
identity. Optional `speaker` on the root comms node or a follow-up changes only
the delivered `CommsMessage.sender_name`. Legacy follow-up `from` is still
accepted as a display-speaker alias, but new content should use `speaker`.

Example: Before the Fire keeps `from = "Research Outpost"` and
`thread_id = "research-scholar"` for the channel, while Dr. Myst's entries set
`speaker = "Dr. Myst"`. The Comms inbox can remain labelled as the Research
Outpost channel, while the chat transcript shows Dr. Myst as the speaker for
the relevant messages.

### Triggered follow-ups (proximity, flags, events)

Any `[comms.response.follow_up]` (or chained `[comms.follow_up]`) can carry an
optional `trigger` field that gates injection until a world condition is met.
All `TriggerCondition` variants are supported, mirroring the `[[trigger]]`
block:

- **Time-based** — `on_timer` + `after_secs` (queue-relative; counts from when
  the follow-up is queued, not from world load). Replaces the legacy
  `delay_secs` shortcut, which was removed.
- **State-based** — `on_entered_region`, `on_exited_region`, `on_flag_set`,
  `on_flag_cleared`, `on_destroyed`, `on_all_destroyed`, `on_world_loaded`.
  These fire on the next tick if the condition is **already true** at queue
  time (e.g. the ship is currently inside the named region, the flag is
  already set, the entity is already destroyed). This means a player who
  picks "we are proceeding to your location" while already at the dock
  receives the follow-up immediately rather than having to leave and
  re-enter the region.
- **Event-only** — `on_attacked`, `on_hailed`. These require a fresh event in
  `pending_world_events` and have no "already-happened" short-circuit.

Response follow-ups show a `...` placeholder in the chat while the trigger is
pending; chained root follow-ups stay silent. The placeholder is removed and
replaced with the real message when the trigger fires.

Worked example — Axiom Station acknowledges arrival when the player ship
enters its dock region (`assets/worlds/before_the_fire.toml`):

```toml
[[comms.response]]
text = "Understood, Axiom Station. We are proceeding to your location."

  [comms.response.follow_up]
  trigger = "on_entered_region"
  entity  = "Axiom Station Dock"
  message = "Ardent, we have you on the dock approach. Welcome to Axiom..."
```

Implementation: the pure evaluator `follow_up_trigger_holds` in
`src/world/server.rs` returns true when the condition is met (or
already-true at queue time). `tick_pending_follow_ups` runs in
`SimSet::Physics` (ordered `.before(handle_ai_events)` so it observes
`pending_world_events` before they are drained) and snapshots region
membership + live UUIDs + flag store for the evaluator.

### Delayed messages

Use `trigger = "on_timer"` + `after_secs` on any follow-up for time-based
delays. The clock is **queue-relative** for follow-ups (counts from when the
follow-up is queued — i.e. when the response is picked or the parent message
is injected), and **world-relative** for root `[[comms]]` blocks (counts from
world load, matching the existing `on_timer` trigger semantics for top-level
triggers).

### Inbox list — one row per thread

`sorted_threads()` groups `messages` by `effective_thread_id`, then sorts unread
threads first. Each `ThreadSummary` carries metadata from the **latest** message in
the thread:

| Field | Source |
|---|---|
| `sender_name` | latest message; in `gui/comms-state.js`, matching contact name wins for channel-anchored multi-speaker threads |
| `subject` | latest message |
| `any_unread` | any message in thread has `!is_read` |
| `latest_out_of_range` | latest message `!sender_in_range` |
| `latest_orphaned` | latest message `is_orphaned` |

Row styling mirrors the old per-message style:
- out-of-range → alert-red `(35,25,25)` bg / `rgb(1.0,0.2,0.267)` fg
- all-read → dim `(30,30,40)` / `0.4,0.4,0.5`
- any-unread → bright `(40,40,55)` / `0.9,0.9,1.0`

### Chat panel — chat-room view

When the operator opens a thread, all messages are displayed in chronological order:

1. **Contact bubble** — sender name (purple) + body text.
2. **Player reply bubble** — green, right-aligned, "You: \<text\>" rendered
   immediately after each message that has a `selected_response` set.
3. **Response buttons** — shown below the last contact message only if there is an
   *active message* (latest message with non-empty `responses`, no
   `selected_response`, not orphaned, `sender_in_range`).
4. **Status text** — if no active message, shows "Transmission ended" (orphaned)
   or "OUT OF RANGE" based on latest message state.

The **On Screen** button targets the latest message in the thread.

## ClientCommsState (pure)

`src/client_comms.rs`

| Method | Description |
|---|---|
| `apply(&ServerMessage)` | Replaces messages/objectives/contacts from `CommsState`; drops `selected_thread_id` if thread no longer present. |
| `select_thread(thread_id)` | Opens a thread in the chat panel. |
| `thread_messages(thread_id)` | All messages in thread, insertion order. |
| `active_message_for_thread(tid)` | Last message with pending responses. |
| `sorted_threads()` | `Vec<ThreadSummary>` for the inbox list. |
| `response_buttons_enabled()` | True when selected thread has an active message. |
| `clear_selection()` | Back button / deselect thread. |
| `can_hail(uuid)` | True if contact exists and `in_range`. |

## Wire types

`src/core/messages.rs`

```rust
pub struct CommsMessage {
    pub id: String,
    pub sender_uuid: String,
    pub sender_name: String,
    pub subject: String,
    pub body: String,
    pub responses: Vec<String>,
    pub selected_response: Option<usize>,
    pub is_read: bool,
    pub is_orphaned: bool,
    pub sender_in_range: bool,   // #[serde(default = "default_true")]
    pub thread_id: String,       // #[serde(default)]  "" = own thread
}
```

## Marker components

| Component | Purpose |
|---|---|
| `CommsPanel` | Root node; visibility target. |
| `CommsContactsStrip` | Horizontal contacts strip. |
| `CommsInboxList` | Vertical scrollable thread list. |
| `CommsChatPanel` | Chat view (right/bottom). |
| `CommsObjectivesFooter` | Objectives strip (right/bottom). |
| `CommsClearButton` | "Clear All" button. |
| `CommsBackButton` | Back button in chat view. |
| `CommsOnScreenButton { message_id }` | On Screen; carries latest message id in thread. |
| `CommsContactPill { target_uuid }` | Hail button in contacts strip. |
| `CommsMessageRow { thread_id }` | Inbox row; carries thread id. |
| `CommsResponseButton { response_index, message_id }` | Response option button. |

## Systems (CommsPanelPlugin)

| System | Trigger | Responsibility |
|---|---|---|
| `spawn_comms_ui` | once (no `CommsPanelSpawned`) | Spawns ConsoleShell with two panes. |
| `toggle_comms_panel_visibility` | every frame | Shows/hides `CommsPanel` via `comms_panel_visible`. |
| `respawn_comms_on_orientation_change` | `DeviceOrientation` changed | Despawns panel; clears `CommsPanelSpawned`. |
| `refresh_all_comms_ui` | every frame, if dirty | Rebuilds contacts, inbox, chat, objectives from `ClientCommsState`. |
| `detect_comms_clicks` | every frame | Routes `Interaction::Pressed` → outbound `ClientMessage`s + state mutations. |

### Visibility rules (`comms_panel_visible`)

1. Phase must be `InProgress`.
2. Local player must hold the `"comms"` station (`Player.station == "comms"`).
3. One-console player → always visible.
4. Multi-console player → visible only when `ActiveConsole == Some("comms")`.

## Tests

Pure unit tests in `src/client_comms.rs` and `src/console/comms/client.rs`. Run with:

```bash
cargo test comms
```

## Sources

- `src/console/comms/client.rs`
- `src/client_comms.rs`
- `src/console/comms/inbox.rs`
- `src/core/messages.rs`
- `src/world/config.rs` (`speaker` parsing, legacy follow-up `from` alias, root-level `[comms.follow_up]` parsing + mutual-exclusion validation)
- `src/world/server.rs` (thread_id generation in handle_hail, handle_respond_to_message, auto-triggered comms; root_follow_up scheduling at inject time)
- `src/world/content.rs` (ActiveDialogue.thread_id; `FiredCommsTemplate.root_follow_up`)
- `assets/worlds/before_the_fire.toml` (Research Outpost handoff → Dr. Myst chain via `[comms.follow_up]`)
