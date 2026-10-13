# Increment 1: explicit Consoles

Status: implementation under verification. This record does not claim browser,
native, performance or human acceptance. PRD: [#1562](https://github.com/jkeywo/project-phoenix-v2/issues/1562).

## Playable exit

Use the six-seat Alliance Cruiser first, then the Destroyer for visiting tabs.
Record the tested commit, build mode, hull, scenario and participating devices.

| Journey | Required observation | Result |
|---|---|---|
| Join Cruiser from phone | Existing seats and screens; Console tabs match authored hosting | Pending |
| Operate and delegate a System | Commands retain System identity; one human or ordinary AI controller | Pending |
| Disconnect, AFK and reconnect | Backfill continues work; returning operator receives current state | Pending |
| Move a Destroyer visiting Console | One current host; previous host loses control and projections | Pending |
| Return to the same host after no human host | Unchanged machinery values reappear correctly | Pending |
| Co-host two Consoles using the same screen | Distinct tabs, working views and permitted information | Pending |
| Zoom, filter and inspect, then hand off | Supported working view follows Console; accessibility stays personal | Pending |
| Save/resume with visiting Console and active AI work | Hosting, views, intent and memory survive without restarting work | Pending |
| GM Station operation and Viewscreen controls | Current hosting governs control; viewing a feed grants no authority | Pending |
| Courier/Destroyer/Cruiser/Battleship | Supported 2/4/6/8-seat journeys remain available | Pending |
| Native Station display | Window remains assigned to Station and shows its current Console tabs | Pending |
| Workshop composition | Create/order/assign Consoles, set System ownership and feeds; undo/redo and save/reload | Pending |
| Workshop validation and Test | Invalid ownership/feed/hosting refused; valid composition previews and runs in disposable Test | Pending |

## Automated evidence

Owner tests cover content loading, stale-host rejection, feed isolation,
working-view admission, snapshots/digests, scoped reconnect publication,
repair visibility and Workshop definitions. Each result must identify its
revision and command before this increment is marked complete. Historical
increment 0 results are comparison evidence, not proof of this migration.

The final integration gate and representative gameplay/performance comparison
remain outstanding. Keep existing balance thresholds. Do not infer native
multi-monitor acceptance from browser tests or Rust unit tests.
