# Project Phoenix — Comms and Commitments

| Field | Value |
|---|---|
| Status | Current mechanic |
| Scope | Hails, scripted dialogue, response authority, range and jamming, promises and settlement |
| Audience | Design, narrative, content, UI, simulation and playtest |

Comms is where other actors become people rather than contacts. It carries conversations shaped by physical reach, lets the crew commit to terms, and records whether those terms were later kept or broken without labelling the choice for them.

Related documents: [Scenario Authoring](../systems/scenario-authoring.md), [Sensors and Epistemics](./sensors-epistemics.md), [Navigation and Relative Motion](./navigation-relative-motion.md), [Campaign Continuity](../foundation/campaign-continuity.md), and [Thin Margin Setting](../foundation/thin-margin-setting.md).

## Experience goals

- Make dialogue an operational part of the scenario, not a pause outside the simulation.
- Let range, jamming and actor identity affect who can be reached.
- Give one operator a clear response action while encouraging whole-crew discussion.
- Turn promises into durable game state that later scenes and missions can test.
- Present terms and consequences plainly without assigning moral labels to the player's choice.

## Threads and messages

Scenario Rhai opens and advances dialogue threads. A thread has stable identity, a sender resolved from world content, chronological messages and a current set of authored responses. Physical senders correspond to entities in the world; synthetic or system senders may remain readable without a physical endpoint.

Messages are host-authored and replicated to the shared Comms surface. Responses are final actions, not local draft state. Once a response is accepted, its scripted effects execute through the scenario boundary and the conversation advances according to the authored node.

The inbox and hail roster are authoritative projections. Ordering must be deterministic so players do not chase moving entries. Display names and dialogue text come from the string catalogue.

## Hailing and reachability

The hail roster is derived from live hailable entities. A physical endpoint must be known, in range and not blocked by the current jamming rules. Synthetic senders do not pretend to occupy a reachable location.

An unavailable response remains visible when that helps the crew understand the situation, but it is disabled and explains the physical reason. If conditions change between display and selection, the host rejects the attempt and the client shows the current reason.

Range failure, jamming, an unknown endpoint and a scripted refusal are different outcomes. The first three mean the channel did not deliver the action; the last means another actor received it and chose not to comply.

## Response authority

The Comms station owns the button press, while the crew owns the conversation around it. Important authored responses use a two-step confirmation to prevent a thumb slip on a small screen. There is no hidden vote, captain-only override or reversal after acceptance unless the scenario explicitly offers another in-fiction response.

This supports both straightforward tests and complex drama. A simple scenario may offer one obvious acknowledgement; a richer scenario may ask the crew to choose terms under time pressure. Neither is forbidden by the game's pillars.

## Commitments

A commitment exists only when the scenario records a promise the crew actually made. It has a stable id, the party it was made to, string-catalogue terms, a description of what would resolve it, open/kept/broken state, and the ticks at which it was created and settled.

Scenario script uses `ctx.commitments.record`, `keep`, `break_promise` and `state`. Duplicate ids are authoring errors because silently replacing a promise would erase terms the player accepted. Keeping or breaking an already settled promise is a no-op.

The ledger does not schedule or judge fulfillment. A timed promise composes with a scenario deadline whose handler settles it. `resolves_when` is declared for players and authors; the script resolves at the dramatic beat where the promise is actually tested.

Settlement writes the ordinary `commitment.<id>.kept` or `commitment.<id>.broken` campaign flag. Later dialogue can offer options based on the live state, and campaign projection can carry the settled promise into a later mission.

## Dossiers and continuity

Promises appear on the dossier of the party they were made to when that party has a dossier surface. They are matched by authored party identity rather than a run-specific UUID because a political actor can outlive the particular ship or station used for the conversation.

Campaign continuity preserves the declared promise and settlement, not the whole transient inbox. A later scenario may react to the fact and terms; it should not depend on reconstructing every previous message bubble.

## Intelligence database

Band C6's database/library work gives Comms the primary Intelligence search and correlation surface. It organises known entities, registry codes, dossiers, reports, pursuit estimates and mission history over the shared evidence model. Other station databases remain focused projections and do not obsolete Intelligence.

Comms-specialist Away Missions may acquire sources, records or other durable entries for the Intelligence database. Those missions follow the unscheduled Operations and Duty Team framework rather than adding an espionage mode to ordinary dialogue.

Surrender is accepted but unscheduled. Comms may offer or demand authored terms; the receiving actor may accept, refuse, counteroffer or feign compliance. Acceptance changes objectives, hostility and vessel orders without automatically despawning or transferring ownership.

## AI and backfill

AI Comms uses authored policy to choose whom to hail and which offered response to select. It submits ordinary admitted commands and receives the same range, jamming and confirmation constraints appropriate to its control surface.

Backfill should keep low-stakes traffic moving, but scenarios may reserve consequential commitments for a human by authoring policy that holds rather than chooses. AI must not infer unspoken moral priority from response wording.

## Authoring principles

Write response labels as actions or terms: “Commit the repair team,” “Refuse priority,” or “Ask for evidence.” Avoid “good choice,” “selfish choice” or other authorial verdicts. Put consequences in world reactions, resources, trust and later commitments.

Every important response should have a distinct operational meaning, a clear actor and a tested continuation. Rhai owns branching and effects; TOML owns hailable entities and physical parameters. Text remains in `assets/strings/strings.csv`.

## Playtest questions

- Can the crew tell whether a failed action was undelivered, jammed or refused?
- Does the Comms operator feel responsible without excluding the rest of the crew from the decision?
- Are important confirmations protective rather than tedious?
- Can players restate the terms and resolution condition of each open promise?
- Do later consequences feel connected to what was said without the UI moralising?
- Does AI backfill avoid making irreversible promises that policy did not explicitly authorise?

## Canonical sources

- [Comms design](../../../pasm/spec/design/comms.yaml)
- [Comms architecture](../../../pasm/spec/architecture/comms.yaml)
- [Scenario scripting architecture](../../../pasm/spec/architecture/scenario-scripting.yaml)
- [Comms panel wiki](../../../wiki/concepts/comms-panel.md)
