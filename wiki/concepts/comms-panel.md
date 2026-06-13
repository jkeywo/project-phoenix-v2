---
title: CommsPanelPlugin
---

# CommsPanelPlugin

Client-side Comms console: two-panel inbox + chat room layout.

## Location

- `src/console/comms/client.rs` — `CommsPanelPlugin` (Bevy, client feature)
- `src/client_comms.rs` — `ClientCommsState` (pure, Bevy-free, unit-tested)
- `src/console/comms/inbox.rs` — `CommsInbox` (pure server-side, no Bevy)

## Layout

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

### Delayed messages

`delay_secs` works on both root `[[comms]]` messages and
`[comms.response.follow_up]` nodes. Root/template delays are silent until the
timer expires, so a delayed character introduction does not appear as an
immediate `...` row. Response follow-ups still show a `...` placeholder inside
the active thread while the reply is pending.

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
2. Local player must hold `Console::Comms`.
3. One-console player → always visible.
4. Multi-console player → visible only when `ActiveConsole == Some(Comms)`.

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
- `src/world/config.rs` (`speaker` parsing, legacy follow-up `from` alias)
- `src/world/server.rs` (thread_id generation in handle_hail, handle_respond_to_message, auto-triggered comms)
- `src/world/content.rs` (ActiveDialogue.thread_id)
