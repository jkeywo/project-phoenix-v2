# PRD: Red Alert Glow Fix & Multi-Console Player Support

## Problem Statement

The Red Alert border glow never appears on the view screen despite Red Alert being toggled. The root cause is `ship.is_changed()` in `sync_red_alert_border` — Bevy's resource change-detection is consumed by the first reader and never fires for subsequent systems in the frame, so the glow is stuck `Hidden`.

Additionally, the current `Player.console: Option<Console>` model prevents a single player from holding multiple roles. On small crews or solo testing, a player must pick one console and can't access others — wasting available stations and creating a poor experience.

---

## Solution

### Red Alert Glow Fix

Replace the broken `is_changed()` guard with a proper reactive dependency: read `ShipState` via `Res` without the change-detection short-circuit. Update the Red Alert overlay every frame during `InProgress` by comparing the *actual* `red_alert` boolean rather than relying on Bevy's volatile change flag.

Enhance the visual effect from thin 8px solid border strips to an actual glow using `BoxShadow` with red color, inward-facing blur, and a subtle darkened background on a full-canvas overlay node.

### Multi-Console Player Support

Change `Player.console: Option<Console>` to `Player.consoles: Vec<Console>` across the entire stack:

| Layer | Change |
|---|---|
| `messages.rs` | `Player.consoles: Vec<Console>`, `ServerMessage::ConsoleSelected` gains full `consoles` list |
| `session.rs` | `toggle_console()` — add if absent, remove if present, conflict if another connected player holds it; `clear_consoles()` — remove all |
| `lobby.rs` | `ClientMessage::ToggleConsole { console }` replaces `ClientMessage::SelectConsole`; existing `ClearConsole` maps to `clear_consoles()` |
| `simulation.rs` | `session.0.console_owner(Console::CaptainChair)` and `session.0.console_owner(Console::Helm)` for authority checks |
| `renderer.rs` | Player list text displays each player's full console set |
| `client.html` | Tab system for multiple console views; toggle buttons; Drop All button |

---

## User Stories

### Red Alert Glow

1. As a player watching the view screen, I want a red glow effect around the edges of the 3D display when Red Alert is active, so the emergency state is visually obvious and immersive.
2. As a player, I want the glow to appear immediately when the captain toggles Red Alert and disappear immediately when toggled off, so there is no visual lag.

### Multi-Console Lobby

3. As a player in the lobby, I want to toggle console buttons (click to assign, click again to unassign), so that I can hold multiple roles.
4. As a player, I want to see a "Drop All" button when I hold at least one console, so I can quickly clear all my assignments.
5. As a player watching the lobby player list, I want to see all of a player's assigned consoles next to their name, so I know who is operating what.

### Multi-Console In-Game

6. As a player holding multiple consoles, I want each console's UI to appear as a tab I can switch between, so I can operate all my roles during the game.
7. As a player holding no consoles, I want the game view to show a "Stand by" message, so I know I need to assign consoles in the lobby.
8. As a player who disconnects, I want all of my previously assigned consoles released (not just one), so they become available immediately.
9. As a reconnecting player, I want all of my previously assigned consoles restored (if still free), so my station assignments persist across crashes.

---

## Implementation Decisions

### Red Alert Glow — `renderer.rs`

```
Old approach:
  - 4 thin Node strips with BackgroundColor(Color::RED), 8px height
  - Visibility toggled by `sync_red_alert_border` gated on `ship.is_changed()`

New approach:
  - Single full-canvas Node with:
    - BackgroundColor: dark red tint (semi-transparent)
    - BoxShadow: red inward-facing glow
  - Visibility toggled every frame by comparing current `red_alert` value
```

The glow node uses `PositionType::Absolute`, full width/height, and `BoxShadow` applied to all four sides. The red color and blur radius are tuned for a soft emissive glow rather than a harsh border.

**Systems:**

```rust
fn update_red_alert_glow(
    ship: Res<ShipState>,
    phase: Res<CurrentPhase>,
    mut query: Query<&mut Visibility, With<RedAlertGlow>>,
) {
    if phase.0 != GamePhase::InProgress { return; }
    let show = ship.red_alert; // direct field read, no is_changed()
    for mut vis in query.iter_mut() {
        *vis = if show { Visibility::Inherited } else { Visibility::Hidden };
    }
}
```

The old `sync_red_alert_border` system is replaced entirely. No `is_changed()` — just read the boolean every frame. Performance cost is negligible for a single entity query.

### Player & Console Data — `messages.rs`

```rust
// Before
pub struct Player {
    pub console: Option<Console>,
    // ...
}

// After  
pub struct Player {
    pub consoles: Vec<Console>,
    // ...
}
```

`ServerMessage::ConsoleSelected` is updated to broadcast the **full** console list for the player:

```rust
ServerMessage::ConsoleSelected {
    token: String,
    consoles: Vec<Console>,   // was: console: Console
},
```

This avoids incremental patching — the client replaces the player's console list atomically on each `ConsoleSelected` message.

### Toggle Console — `session.rs`

```rust
pub fn toggle_console(&mut self, token: &str, console: Console) -> Result<bool, ConflictError> {
    // Returns true if console was added, false if removed
    let taken = self.players.iter().any(|p|
        p.connected && p.token != token && p.consoles.contains(&console));
    if taken { return Err(ConflictError::ConsoleTaken); }
    
    let idx = self.idx(token).ok_or(ConflictError::ConsoleTaken)?;
    let player = &mut self.players[idx];
    
    if player.consoles.contains(&console) {
        player.consoles.retain(|c| c != &console);
        Ok(false) // removed
    } else {
        player.consoles.push(console);
        Ok(true)  // added
    }
}

pub fn clear_consoles(&mut self, token: &str) {
    if let Some(idx) = self.idx(token) {
        self.players[idx].consoles.clear();
    }
}

pub fn console_owner(&self, console: Console) -> Option<&str> {
    self.players.iter()
        .find(|p| p.connected && p.consoles.contains(&console))
        .map(|p| p.token.as_str())
}
```

### Session Reconnect

The `last_consoles` map stores `Vec<Console>` per token. On reconnect, each previously-held console is individually checked for availability:

```rust
pub fn reconnect(&mut self, token: &str) -> Option<&mut Player> {
    let idx = self.idx(token)?;
    self.players[idx].connected = true;
    if let Some(last) = self.last_consoles.get(token).cloned() {
        for console in last {
            let free = self.players.iter()
                .all(|p| !p.connected || p.token == token || !p.consoles.contains(&console));
            if free {
                self.players[idx].consoles.push(console);
            }
        }
        self.last_consoles.remove(token);
    }
    Some(&mut self.players[idx])
}
```

### Lobby — `lobby.rs`

`ClientMessage::ToggleConsole { console }` maps to `toggle_console()`. On success:

```rust
let consoles = sessions.0.players()[idx].consoles.clone();
outbound.write(OutboundMessage {
    target: Target::All,
    msg: ServerMessage::ConsoleSelected {
        token: ev.token.clone(),
        consoles,
    },
});
```

`ClientMessage::ClearConsole` maps to `clear_consoles()` and broadcasts `ConsoleCleared`.

### Simulation Authority — `simulation.rs`

Red Alert toggle authority:

```rust
// Before: sessions.0.captain_token() == Some(ev.token.as_str())
// After:
sessions.0.console_owner(Console::CaptainChair) == Some(ev.token.as_str())
```

Helm input authority:

```rust
// Before: sessions.0.helm_token()
// After:
let helm_token = sessions.0.console_owner(Console::Helm);
```

Multiple players holding the same console: the **first** connected player listed at that console wins. If two players both hold Helm, both can send `HelmInput` — the game uses the last valid helm input received each tick.

### Client UI Tabs — `client.html`

The game section (`#game-ui`) is replaced by a tab bar + content area:

```
Tab Bar: ┌──────────────┬──────────────┐
         │ Captain Chair│ Helm         │
Content: └──────────────┴──────────────┘
         ┌──────────────────────────────┐
         │  [Red Alert: OFF]           │  ← shown when Captain tab active
         └──────────────────────────────┘
```

```html
<div id="tab-bar" style="display:none; display:flex; gap:0; margin-bottom:0.5rem;">
  <button class="tab-btn active" data-console="CaptainChair">Captain's Chair</button>
  <button class="tab-btn" data-console="Helm">Helm</button>
</div>
<div id="tab-content"></div>
```

Tab buttons are styled with `border: 1px solid #446; border-bottom: none;` in non-active state, and `border-color: #8af; color: #8af; background: #335;` when active. The active tab visually connects to the content area below.

The content area renders the console UI for the currently active tab:
- **Captain tab:** Red Alert toggle button
- **Helm tab:** thrust slider, steering slider, joystick area

A "Drop All" button appears below the console button list in the lobby when the player holds at least one console.

### Client Message Handler

```javascript
case 'ConsoleSelected': {
  const p = state.players.find(p => p.token === msg.data.token);
  if (p) p.consoles = msg.data.consoles;   // atomic replace
  break;
}
case 'ConsoleCleared': {
  const p = state.players.find(p => p.token === msg.data.token);
  if (p) p.consoles = [];
  break;
}
```

---

## Testing Decisions

### Extended Tests

**Session Manager:**
- Toggle console: empty → add → `[Console]`
- Toggle console: has it → remove → `[]`  
- Toggle console: held by another connected player → `ConflictError::ConsoleTaken`
- Clear consoles: has 2 → clear → `[]`
- Reconnect with 2 consoles, both free → restore both
- Reconnect with 2 consoles, 1 taken → restore only the free one
- Reconnect with 2 consoles, both taken → restore none

**Lobby System:**
- ToggleConsole with no prior assignment → adds console, broadcasts full list
- ToggleConsole with existing assignment → removes console, broadcasts empty list
- ClearConsole → removes all consoles, broadcasts ConsoleCleared

**Codec:**
- Round-trip test for `ConsoleSelected` with `consoles: Vec<Console>` field

### Modules Not Tested
- Glow visual effect (manual testing)
- Tab UI interaction (manual testing)
- Multi-tab console input routing (manual testing)

---

## Out of Scope

- Console role conflict resolution beyond simple "first connected wins"
- Dynamic authority transfer mid-game (e.g., helm hands off to captain)
- Tab swipe gestures for mobile (future enhancement)
- Per-console notification system (e.g., blinking tab when action needed)
- Sound effects for tab switching or console assignment changes

---

## Migration Notes

- `Player.console` → `Player.consoles` is a **breaking change** to the wire format. All connected clients must connect fresh or receive a full `Welcome` state refresh. Since the existing `Welcome` message already sends the full state, reconnecting clients get the new format automatically.
- The `ToggleConsole` message type is a new variant on an existing enum — old clients sending `SelectConsole` will receive a deserialization error. Old clients must be updated.
- The `ConsoleSelected.data.console` field becomes `ConsoleSelected.data.consoles` — clients must read the array, not a single value.
