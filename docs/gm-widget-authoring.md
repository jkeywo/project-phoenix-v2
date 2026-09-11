# Authoring GM widgets

Issue #1439, PRD #1419 story 13. Companion to `docs/gm-comms-authoring.md`.

A **role preset** is personal presentation a Game Master selects for
themselves (issue #1319). A **widget** is one card that preset composes onto
their desk. Widgets are typed: you choose among four surfaces this build
already draws and hand them defaults. There is no field in which markup, a
script, a style, a new GM action or a new permission can arrive, so a widget
can never give one Game Master authority another does not have.

Every rule below is enforced by `parse_world`, and every refusal names the
preset and the widget by index and id:

```
[[gm_role_preset]] #0 'tactical' [[gm_role_preset.widget]] #3 declares unknown
widget type 'scoreboard'; this build draws 'attention', 'workload', 'actions',
'note'
```

## Shape

```toml
[[gm_role_preset]]
id = "tactical"
label = "world.my_world.gm_role_preset.tactical.label"

[[gm_role_preset.widget]]
id = "urgent-traffic"      # stable, unique within THIS preset
type = "attention"         # one of the four below
label = "world.my_world.gm_widget.urgent_traffic.label"   # a strings.csv id
```

`id` and `label` are required on every widget. Two presets may reuse a widget
id — only one preset is ever effective on a desk — but one preset may not.

## `type = "attention"`

Shows the GM attention queue (#1433) narrowed by the operator's own filters,
and seeds those filters with the defaults you author.

| key | meaning |
| --- | --- |
| `band` | `urgent`, `attention` or `background` |
| `category` | `pending_comms`, `eligible_beat`, `idle_npc`, `station_health`, `quiet_time` |
| `ship` | a world entity `name`, the way `contacts` names one |

These are **defaults**, not a second filter. They are applied when the operator
selects this preset — a live switch is them asking for your view — and never on
a reconnect, where whatever they changed afterwards, and whatever snooze minute
is still running, wins. A facet you omit is reset to "All" on a switch.

`ship` is resolved against the live queue when the default is applied, because
the queue's ship facet is a runtime entity id no author can know. A hull this
session has never seen leaves that facet unnarrowed rather than emptying the
card.

## `type = "workload"`

Summarises the Station-workload advisory (#1438): one line per Station, its
level as a **word**, from the rows the advisory already published.

| key | meaning |
| --- | --- |
| `ship` | show only this hull's Stations (a world entity `name`) |

## `type = "actions"`

Repeats existing permitted GM action buttons where this role can reach them.

| key | meaning |
| --- | --- |
| `actions` | ids from the registry below, in the order you want them |

```toml
actions = ["gm-session-pause", "gm-session-resume"]
```

The registry is `GM_WIDGET_ACTION_IDS` in `src/world/config.rs`. Anything else
fails the world load, naming the id and listing what is permitted.

A widget button **activates the shipped control**; it does not issue an action
of its own. The confirmation, the admission check and the action feedback are
the ones that button already had. A control the desk is not offering right now
is drawn disabled, with a sentence saying how many are unavailable.

## `type = "note"`

One authored sentence.

| key | meaning |
| --- | --- |
| `text` | a `strings.csv` id |

`text` is an id, never prose and never markup: it must be ASCII
alphanumerics, `.`, `_` and `-`. The page renders it with `textContent`, so
there is no path from an authored note to anything a browser parses.

## Keys belong to one type

A key on the wrong type fails the world load. That is deliberate: a `text` on
an `attention` widget is a note you will never see, and a world that loaded
anyway would look, on a live desk, exactly like the feature being broken.

## What widgets are not

- not authority — nothing here reaches `GmOperator`, a `GmAction`, a snapshot
  or the digest, and two Game Masters on different presets have identical
  action availability;
- not shared — another operator never sees your selection, your filters or your
  snoozes;
- not a second queue — the attention and workload cards read the surfaces
  beside them rather than parsing or narrowing anything of their own.

## Worked example

`assets/worlds/probe_gm_widgets.toml` authors all four types across two
presets. `tests/fixtures/gm-widgets-presets.json` is exactly what that file
publishes to the browser; `tests/gm_widgets.rs` asserts the two agree, and
`tests/client/gm-widgets-panel.test.js` drives the desk from the same fixture.
