# Addressing script effects and Comms

Scripts can select receiving player ships when an effect executes. The same
fields work on `ctx.effects.addressed(#{ ... })` and
`ctx.effects.open_comms(#{ ... })`:

| Field | Value |
| --- | --- |
| `recipient_ship_slots` | Array of authored `[[ship_slots]]` ids |
| `recipient_factions` | Array of authored faction names |
| `all_player_ships` | Boolean |
| `recipient_objective_instances` | Array of maps with `objective_id` and `instance_id` |

Selections form a union, deduplicated and applied in stable ship UUID order.
An Objective selector reads the instance's effective current membership, after
its ordinary specificity and conflict rules. It does not include former members
merely because their consoles retain Objective history. A declared but absent
ship slot or an inactive instance can therefore select no ships.
Delivery excludes retained zero-hull crew ships without erasing their identity
or Objective membership/history.

```rhai
on_world_loaded("escort_orders");
fn escort_orders(ctx) {
    ctx.effects.add_objective(#{ id: "escort", instance_id: "pair",
        text: "world.example.escort", recipient_ship_slots: ["wing"] });
    ctx.effects.addressed(#{ type: "apply_modifier", slot: "MaxSpeed",
        tag: "escort_boost", bonus: flt("1.5"), recipient_ship_slots: ["lead"],
        recipient_objective_instances: [#{ objective_id: "escort", instance_id: "pair" }] });
    ctx.effects.open_comms(#{ from: "control", node_fn: "escort_hail",
        recipient_objective_instances: [#{ objective_id: "escort", instance_id: "pair" }] });
}
fn escort_hail(ctx) { #{ text: "world.example.escort_orders", responses: [] } }
```

Use `flt("1.5")` for a fractional modifier: Rhai's deterministic grammar has no
native float literal. `addressed` accepts presentation, contact-information,
NPC-doctrine, AI-state, modifier/flag/integer-modifier and entity-removal action
types through the existing typed action parser. It substitutes only the receiving
ship; other action fields retain their ordinary meaning and validation. A
world-global action is refused. Do not add an `entity` field beside selectors.
Ordinary action gates still apply; selecting a player ship does not make it a
compatible NPC doctrine target or grant it otherwise hidden information.

`addressed` requires an explicit selector field. To intentionally do nothing,
use `recipient_ship_slots: []` (or `all_player_ships: false`). A valid empty
selection performs no action, delivers no message, and reports a source-located
diagnostic in Workshop Test and the GM mission panel. In particular, an empty
Comms selection does not enter the root function or run its effects.

Comms evaluates its root once, then creates a private ordinary thread for each
selected compatible live receiver. Replies, range checks, clear and Viewscreen
selection keep their existing ship-specific admission. Later dialogue nodes stay
bound to that receiver; changing membership affects future opens only. Authored
`thread_id` values are scoped by receiver, so a repeated open can
reprice the same private thread without another ship superseding its reply.
Pending selectors survive snapshot restore and resolve when that queue is drained.
Omitting every selector from `open_comms` preserves its existing audience.

Unknown slot, faction or Objective-instance names refuse the whole selection,
even when another selector would match. They never become an all-ship broadcast.
Workshop Save checks literal selectors in the resolved script set and reports
the script and line. Instance declarations come from literal `add_objective`
maps in that set. Computed selector values are checked when the effect runs;
use Workshop Test to exercise those paths. Runtime diagnostics retain only the
latest 64 rows and are operational feedback, not authoritative saved state.
