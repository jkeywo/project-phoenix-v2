# GM Objective instance controls

## Authoring

An instance activation must be authored in the ordinary GM Objective palette:

```toml
[[gm_objective_palette]]
id = "escort"
instance_id = "lead"
label = "objective.escort"
text = "objective.escort"
recipient_ship_slots = ["lead"]
```

Other instances may use `recipient_factions = ["Alliance"]` or
`all_player_ships = true`. These selectors have the same live membership and
precedence as scripted Objectives. Legacy `recipients` must not accompany an
instance row. A definition can have several uniquely named palette instances.
Complete and fail also operate on retained script-created instances.

The GM panel displays each instance's progress, current membership, status and
fixed completion credit. “Activate all instances” operates on every authored
palette instance of that Objective. “Complete all instances” and “Fail all
instances” operate on every retained instance of that Objective. These actions
never depend on the map's selected ship. They preview in stable key order and
commit atomically: one invalid selector, ambiguity or incompatible terminal
state refuses the entire operation. Repeating an already satisfied operation
is a no-op. Completion credit is recorded once per instance.

The ordinary GM authority, agreed application tick, correlation and equal-GM
attribution apply. Results retain a typed instance scope (`{"instance":"lead"}`
or `"all"`) beside the Objective ID. Instance ambiguity diagnostics use the
same bounded GM diagnostics panel as Workshop Test. Crew replicas continue to
receive only their selected ship's Objective projection.

## Bounded human acceptance (not yet run)

Use a two-ship mission with the palette above plus a faction-addressed and an
all-player instance. Allow ten minutes, with two GM identities and one console
per crew. Record build, browser/native host, input devices and locale.

1. Tab to a named instance action and activate it. Check the confirmation names
   the instance, the result names the acting GM, and each crew sees only its
   selected instance. Repeat with the configured gamepad focus/activate controls.
2. Activate the separately labelled all-instance action. Confirm the instance
   count and inspect progress/current membership independently for each row.
3. Change a ship's faction through authored scenario behavior. Confirm the
   explicit slot wins over faction, faction wins over all, and leaving an
   otherwise unassigned instance preserves its last progress.
4. Complete one instance as the second GM. Check attribution and completion
   membership. Join another ship to that completed instance: it sees Completed
   without another completion credit or reward.
5. Attempt a conflicting activation and an all-instance Fail containing an
   already Completed instance. Check refused feedback and no partial changes.
6. Reconnect each GM, switch locale, and repeat keyboard/gamepad navigation.
   Check translated labels, visible focus, retained progress and distinct bulk
   labels. Record any overflow at the actual console resolution.

Automated evidence lives in `tests/gm_objective.rs` and
`tests/client/gm-objective-panel.test.js`. It does not replace this device pass.
