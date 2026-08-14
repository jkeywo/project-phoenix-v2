---
title: CommsPanelPlugin
---

# CommsPanelPlugin

Client-side Comms console: two-panel inbox + chat room layout.

## Intended decision contract

Comms owns the ship's shared dialogue channel. A selected response is an
immediate, irreversible bridge decision; the host remains authoritative for
acceptance and authored consequences. The intended PASM design adds an optional
`important: true` response marker for a future clear warning and confirmation
in the Comms UI before it sends that normal response. It is presentation only:
it does not introduce a vote, another station's approval, deferred execution,
or a reversal path.

Physical sender reachability should grey out, rather than remove, response
buttons. If a stale or otherwise forced attempt reaches the host and is
rejected, the attempted response must briefly flash red. The current server
rejects such submissions correctly but does not yet provide that feedback
signal, so this remains future work.

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
latest unanswered message in that thread. This is what keeps a handoff and a
delayed follow-up briefing in one conversation while still allowing the operator
to reply to the later speaker.

### Authoring a thread

A comms thread is authored in Rhai, in the world's `[script]` block. Issue #985
deleted the declarative `[[comms]]` front-end this page used to document at
length — the root template, `[[comms.response]]`, the recursive
`[comms.response.follow_up]` tree, the auto-chained `[comms.follow_up]`, and the
per-follow-up `trigger` that gated injection behind a world condition. See
`docs/toml-authoring-guide.md` §1.6 for the current form; in outline:

```rhai
on_hailed("Research Outpost", "on_outpost_hailed");

fn on_outpost_hailed(ctx) {
    ctx.effects.open_comms(#{ from: "Research Outpost", node_fn: "outpost_hail",
                              thread_id: "research-scholar", urgent: true });
}

fn outpost_hail(ctx) {
    #{ message: "…Stand by — patching you through to Dr. Myst now.",
       responses: [ #{ text: "Patch them through.", on_pick: "on_patch" } ] }
}

fn on_patch(ctx) { #{ message: "Ardent, this is Dr. Myst…", responses: [] } }
```

One fn is one node. A response's `on_pick` names the fn that runs when it is
picked; that fn buffers the response's effects and returns the follow-up node,
or `()` to end the thread. A node with an empty `responses` array is the one-way
broadcast a template with no responses used to be.

A **delayed** reply is `ctx.schedule.after(n, |ctx| ctx.effects.open_comms(#{ thread_id: "…", node_fn: "next" }))` — an ordinary deferred callback that
opens into the same thread. The `PendingFollowUp` queue, its `…` placeholder row
and the `follow_up_trigger_holds` evaluator that decided when to swap it went
with the declarative front-end; a delayed reply now shows nothing until it
arrives, like a chained root always did.

### Multi-speaker channels

A thread stays anchored to one physical or synthetic channel while different
characters speak on it. `open_comms`' `from` is the radio endpoint used for
hailing, range checks, contact lookup and synthetic broadcast identity;
`display_name` is the player-facing label, and `thread_id` is what keeps
successive opens in one conversation.

The per-NODE `speaker` override the declarative form carried is gone with it
(issue #985): a scripted node is body and responses, and who is speaking is
metadata on the OPEN. To change the visible speaker mid-thread, open again into
the same `thread_id` with a different `display_name`.

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

## Server-side handlers (#608)

Comms conversation handlers were relocated from `src/world/server.rs` into `src/console/comms/server.rs` (issue #608): `handle_hail`, `handle_respond_to_message`, `handle_clear_comms`, `handle_show_on_screen`, `handle_comms_channel2`, and `current_sender_in_range`, along with their 32 unit tests. Behaviour-preserving — only cross-module visibility (`pub(crate)`) was widened where the relocated handlers now call world-module helpers. The source page reference below is now the primary location.

## Sources

- `src/console/comms/client.rs`
- `src/console/comms/server.rs` (handle_hail, handle_respond_to_message, handle_clear_comms, handle_show_on_screen, handle_comms_channel2, current_sender_in_range — migrated from `src/world/server.rs` in #608)
- `src/client_comms.rs`
- `src/console/comms/inbox.rs`
- `src/core/messages.rs`
- `src/comms/server.rs` (CommsRuntime, broadcast/range/roster systems — consolidated in #816)
- `src/comms/content.rs` (CommsDialogueNode/CommsResponse, ActiveDialogue, ScriptedDialogue, OpenCommsRequest)
- `src/comms/scripted.rs` (`open_scripted_comms_threads` — the one thread-opening path since #985)
- `src/world/script/comms.rs` (`enter_node`, `project_node` — script meets wire shape)
- `src/world/server.rs` (dispatch appliers called by `handle_respond_to_message`)
- `assets/worlds/default.toml` (the reference scripted dialogue tree)
