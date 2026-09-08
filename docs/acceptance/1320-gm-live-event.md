# Acceptance kit — #1320, the M3 two-GM live event

**Status: preparation only. No human session has been run or accepted by this document.**

Issue #1320 is the human exit gate for GM console M1–M3 under #930. Two different people must operate equal GM identities while a real crew plays the scenario. Automated browser clients, two tabs operated by one person, and the #1316 automated M2 report are useful preparation; they do not complete this gate.

Run this kit on the integrated, validated build after #1294, #1316, #1317, #1318 and #1319 are ready. The coordinator records one explicit Accept or Reject decision against the issue. Until the required observations and evidence exist, the decision stays **Pending** and #1320 stays open.

## 1. Prepare the event

Nominate GM A, GM B, a crew coordinator and an evidence recorder. The recorder may also coordinate a crew, but each GM must be a separate human participant. Use two real Fleet ships for the recipient-isolation comparison. Each ship needs its own host and real crew; assign an actual human to Comms on both ships and cover the other Stations needed by the loaded hull's readiness controls. Record the actual arrangement instead of counting unused test clients as crew.

Both GMs use their own browser profiles on the same integrated build. Keep those profiles for the reconnect exercise. Use the supported browser host/GM Fleet join path and ordinary crew join codes; record browser, OS, input device and network arrangement for every participant. A GM is a host-class participant and consumes no player-ship or Station seat.

Before inviting the crew, fill this readiness table. Missing content or evidence tooling means the event is not ready; do not improvise missing production controls during the human run.

| Required input | Record before starting |
| --- | --- |
| Integrated build | Full commit, build/content stamp, build URL, date and owner |
| Automated baseline | #1316 report/artifacts and its explicit result on the integrated revision; relevant #1317 browser/native results |
| Scenario | Exact world path, content revision/hash, seed, selected hulls and intended ending |
| Role presets | Two distinct scenario-authored preset ids and displayed labels; what each visibly filters; built-in All available |
| Fictional Comms | Existing live hailable speaker, selected-ship route, Fleet route, authored hail and expected crew response/continuation |
| Knowledge comparison | A named contact with a known Truth/crew visibility difference and the observing ship; intended Objective/Comms scope |
| Directing choices | Authored event and spawn choices, a removable target, Objective choice, supported NPC doctrine, scoped System/effect targets, compatible player/NPC Stations |
| Evidence capture | Recorder, destination, a demonstrated read-only command/result and same-tick digest capture method from #1316, snapshot/export method and review owner |

The event uses the generated `assets/worlds/prepared/gm_live_event_two_ship.toml` with two Alliance Cruiser hulls. Ordinary `assets/worlds/combat_test.toml` has only one GameStart ship slot. Its additive GM authoring from #1316/#1317 must first be present in the integrated checkout. Prepare the two-slot variant there, before building the event bundle:

```sh
node scripts/prepare-gm-live-event.mjs
node scripts/prepare-gm-live-event.mjs --check
```

Keep the command's source, slot and output hashes with the event record. The tool preserves Combat Test's comments and complete eight-wave script, appends the authored second-ship fixture, and binds the authored hail to the generated world's own setup script. It leaves the ordinary world and base catalogue unchanged. The generated world and `assets/scenarios.gm-live-event.toml` are local preparation outputs, excluded from Git; regenerate them after source authoring changes. Generation verifies content shape, not the simulation or human workflow.

Build and serve the integrated bundle using the normal Trunk/client path after generation; Trunk copies these ordinary assets. Open both hosts at `/?manifest=assets/scenarios.gm-live-event.toml` on that bundle's origin and choose the offered scenario. The manifest selects Alliance Cruiser hulls. Before inviting the human crew, verify the served generated world/manifest match the recorded hashes, run the queued two-host runtime proof, and record **two distinct live Fleet hulls** with their own identities and crew projections. A second joined host alone does not satisfy that precheck. Missing build/runtime evidence keeps the event unready.

Run the opt-in native topology precheck on that integrated source, with local Cargo work coordinated sequentially:

```sh
cargo test --features headless --test gm_live_event_precheck prepared_event_has_two_authored_fleet_hulls_and_equal_peer_digests -- --ignored --exact --nocapture
```

Record a true exit 0 and **one passing test**, not a zero-match or normal ignored result. It boots the generated world on two ordinary headless peers, compares their shared digests, and checks the two authored spawns and different LocalShip projections. It deliberately uses Backfill crews: served browser/crew checks and the real human Comms exercise remain separate prerequisites.

Retain the `GM_LIVE_EVENT_PRECHECK` JSON line printed only after those assertions pass. It records the actual per-peer initial spawn identities/positions, LocalShip, final tick/digest, comparison-round count and loaded hail binding alongside the source/generated asset hashes. A queued command or prepared JSON schema is not execution evidence.

Use these actual choices when filling the readiness table:

| Exercise | Combat Test choice |
| --- | --- |
| Role presets | `directing` and `observer`; switch to All when a required panel is hidden |
| Fictional speaker and routes | Starbase Alpha; `starbase-selected` for one ship and `starbase-fleet` for both |
| Authored hail | `defence-briefing` on `starbase-selected`; acknowledge through the crew's Comms interface and observe the follow-up |
| Spawn/removal | `relief-cruiser`, variant `removable` |
| Optional Objectives | `gm-relief-rendezvous` and `gm-relief-cover` |
| NPC doctrine | `raider-regroup` and `raider-assault` for a live named wave ship |
| Events | `release_wave_1` through `release_wave_8` expose Fire and Pause; `report_wave_2` also exposes Skip |

Keep both player ships within Starbase Alpha's ordinary Comms range for the routing comparison. The wave releases retain the scenario's eight-wave victory accounting; only the second-wave report can be skipped. Choose a live wave contact for the knowledge comparison and restore Normal afterward. Record the actual target identities and translated labels rather than assuming an entity index remains valid.

The #1317 `assets/worlds/probe_gm_comms.toml` remains useful for a separate Comms diagnostic. It has no complete M2 directing palette and does not substitute for this event. Prepared authoring and a runbook do not establish that the combined build or human event has passed.

Keep one scenario and seed for the event. A separate diagnostic probe can explain a defect, but record it separately and do not substitute its result for an uncompleted event step. If the integrated build changes, record the new revision and identify which observations require another run.

## 2. Start with two equal GMs

1. Open the scenario on Ship A's host and open its Fleet. Join Ship B through the Fleet code using the same content. Join the two human crews to their respective ships using those ships' crew codes.
2. GM A and GM B join that Fleet through the **Game Master** role control, each with a distinct display name. Record their public operator ids and the two ship identities/ordinals. Do not put private reconnect capabilities or transport credentials in the public issue report.
3. Have each crew confirm that both GMs appear in the separate GM roster, that neither occupies a Station, and that neither is presented as a higher-authority GM. A technical Fleet owner is not an extra product permission tier.
4. Ready both crews and both GMs through their ordinary controls. Observe the shared transition into play and the same scenario on both ships. Record any readiness confusion or inability to launch.
5. GM A applies Pause, then GM B applies Resume. Each checks the other's attributed result in Activity. Record the actual result and simulation tick; a click or Pending indication alone is not evidence of application.

Evidence: roster and Station screenshots, launch observation, both operator ids, two attributed action results, and a first shared digest checkpoint captured by the prepared collector.

## 3. Use different and duplicate role presets

1. GM A selects the first authored **Role preset**; GM B selects the second. Each describes which panels, quick actions or contacts changed in their own browser. Check that the other GM's view did not change.
2. Both select the same authored preset at once. Neither choice should evict, reserve or lock that preset for the other person. Record both selections.
3. Each switches to **All** and performs one available directing action from section 6. Both must retain the same authority. A hidden control under a presentation preset is not an authorization refusal.
4. Choose different presets again and record each selection for the reconnect check. Keep the controls needed by the next step visible, or deliberately switch to All and record that choice.

Observe how readily a GM finds a hidden tool again and whether the preset names help divide the human workload. Record confusion, missed actions and extra coordination, including cases where the software behaves correctly but the workflow is difficult.

## 4. Perform in-character Comms

The speaking identity must be an existing offered entity. In **Comms Studio**, select the authored route, **Speak as**, and **Recipient ships** before writing. Do not invent a speaker name or script function through a developer console.

1. GM A sends a short in-character transmission to Ship A only. The human on Ship A reads it from the authentic Comms interface and replies or acknowledges as the content allows. Ship B's human checks their own inbox and history: that private copy must not appear there. The fictional message names its speaker; GM identity and action result belong to the operational surface.
2. Copy `server.gm.comms.heading` into **Exact transmission text** and send it to Ship A. Its displayed body must remain that literal text, rather than becoming a UI heading. Then send a short line containing accented/Unicode characters, markup characters and deliberate whitespace, for example `  <b>🌒</b> {sender}  `. Record the authored and received strings. Inspect the exact stored body through the prepared read-only collector for whitespace that the browser's layout cannot show clearly.
3. GM B uses the Fleet route for an in-character announcement. Both Comms operators read one copy addressed to their own ship. Record sender, recipients, text, correlation, operator and result. A repeated new human send is a new intent; do not confuse it with a transport retry of the same correlation.
4. GM A selects the approved **Authored scripted hail** for Ship A and presses **Initiate scripted hail**. The human opens the ordinary hail, chooses the authored response and reads the follow-up. Ship B checks that neither the private offer nor its continuation arrived there. The GM result means the hail was queued; the appearance of the real node and reply effect is separate evidence.
5. On an agreed spoken cue, both GMs send different in-character lines through valid routes. Record both terminal results and both crew observations. Preserve each actual apply tick/order; if the commands landed on different ticks, report that fact. Do not manufacture a same-tick claim from a simultaneous countdown.

Measure the observable delay from press to terminal result and from press to the crew reading the message. Record the measurement method, sample count, worst observed delay, any lost/duplicate text and whether timing disrupted play. Agree the event's acceptable interaction delay before starting, and retain that value with the accept/reject decision rather than choosing it after seeing the results.

## 5. Compare knowledge with the real crew

1. In **Knowledge comparison**, choose Ship A. Identify the prepared contact in **Truth**, **Crew Knowledge** and **Difference**. Ask Ship A's crew to inspect its authentic Sensors/Navigation picture without first telling it the hidden fact. Record what the crew could actually see.
2. Choose Ship B and repeat. Compare the selected ship's own picture, not the host page's currently projected ship. Use the authored contact or the ordinary contact Reveal/Conceal/Normal controls to exercise a real visible difference. Record the target and observing-ship identities with each action and restore Normal afterward.
3. Compare an Objective and the Comms exchange from section 4 against the chosen ship's actual crew interface. Record identity, scope and visible state. Equal Objective/Comms rows alone are not proof of independent sources: the original #1318 comparison derives those categories from the same selected-ship projection. The integrated routing and Objective changes must be assessed against actual recipients' screens as well.
4. Verify that the comparison does not expose Station-private detail as general crew knowledge. If a GM needs such detail, open that ship's authentic compatible Station interface through the ordinary GM control and record that separate operation.

Evidence: paired GM and crew screenshots at identified checkpoints, selected ship/target ids, observed Difference labels, associated contact/Objective results and any discrepancy. A GM saying what the crew ought to know is not a crew observation.

## 6. Direct a complete scenario together

Use the same loaded scenario's authored choices and ordinary GM controls. Rotate who performs the action so both GMs exercise authority. The event recorder fills one row per actual action; #1316 already supplies exhaustive automated family invariants, while this pass records how people use the combined surfaces during play.

| Action family | Human observation to retain |
| --- | --- |
| Event/mission control | Selected authored event or lever, crew-visible consequence, attributed result |
| Spawn and despawn | Offered authored choice, map placement, real spawned target, removal confirmation and removal result |
| Objective | Intended recipient scope, ordinary crew display and completion/failure result |
| NPC doctrine | Offered compatible authored choice, actual visible NPC intent/movement/behavior |
| Contact knowledge | Correct observer/target, actual crew visibility, return to Normal |
| Damage/heal and disable/restore | Captured target and scope, preview, real affected Systems and recovery |
| Player Station control | Authentic interface, crew disclosure, ordinary effect, release back to the current crew/AI ownership |
| NPC Station control | Only an offered compatible interface, actual effect, release and resumed ordinary NPC behavior |

Ask both GMs to apply the same absolute choice to a suitable target on a spoken cue, then to apply two competing authored choices. Preserve all canonical outcomes and the final state; never choose a winner by browser arrival order. Both people remain eligible operators. Record whether the activity feed and confirmations explain what happened well enough to continue play.

Change one confirmation category in GM A's **Settings → Controls** while GM B retains its own choice. Exercise the category and confirm that A's dialog describes its captured target/intent, cancel once, then accept a fresh request. Cancellation must not create an action. Confirm B's preference and role preset remain unchanged. Record any unexpected repeated confirmation during authentic Station control.

Have the affected crew read the operational disclosure for a GM's direct intervention or Station takeover. Separately record the fictional consequence they observed. End the scenario through its intended ordinary ending and ask each GM and crew for their experience from join through completion.

## 7. Lose and reconnect one GM during play

Schedule this before the scenario ending, with a live Comms conversation available to check afterward.

1. Record GM B's public operator id, current preset and confirmation preference. If B is controlling a compatible Station, record whether GM A also holds it and who its ordinary human holder is. Capture a pre-loss checkpoint.
2. Abruptly close GM B's GM tab/process while preserving its browser profile. Record exactly what was closed. This exercises abrupt process loss; it is not evidence of a spontaneous browser crash unless a real crash occurred. Keep the other hosts, GM and crews running.
3. Observe the disconnection and any synchronized pause/host-loss status. Check that B's Station control is retired, that A's surviving control remains if present, and that a Station with no remaining GM returns to its ordinary current ownership. Record crew-visible status and any interruption to play.
4. Reopen the same build and reconnect through B's retained profile and the ordinary Fleet flow. The returning participant must retain B's public operator id; a newly created operator is not a successful reconnect. Record the restore status and digest proof from the prepared collector. Existing peers should not receive a first-time-GM Accept/Reject prompt for this known identity.
5. Wait for a matching-state completion. Reconnection must not silently Resume the mission. Have either GM explicitly Resume, record that attributed result, then let B perform a fresh Comms or directing action. Confirm the recorded preset/private preference is restored and A's settings are unchanged.
6. Continue the earlier private conversation from the intended crew's authentic interface. Check its recipient and history, the other ship's absence of the private copy, retained operational history and ability to keep playing.

If restoration is refused or times out, retain the visible reason and collector output. Do not replace the failed reconnect with a new operator and mark it passed. Record recovery work separately and decide which event observations remain usable.

## 8. Preserve evidence and decide

Use the existing snapshot/save export UI to retain the event state and the prepared #1316 collector to retain authoritative commands, GM journal/results and digest evidence. Capture shared digest checkpoints at the same agreed simulation boundary; matching screenshots or numbers from different ticks do not prove convergence.

An exported browser save and a headless `ReplayArtifact` are different records. Keep the format and capture method with each file. Do not pass a browser save to `--replay` or rename one into a replay artifact. If the reviewed collector produces a replayable event artifact, record the verifier command, tested build, true exit code and verdict; otherwise record the replay evidence actually supplied by #1316 and disclose the limit on verification of the human event's command record.

The Activity ring is bounded, so collect required results during the run rather than relying on the final visible list to contain every earlier action. Preserve human observations even when a later focused retry passes. List defects and follow-ups with the triggering step, affected people and artifact references; do not silently remove a failed attempt from the event record.

Copy and complete the report below in #1320 or an attached report linked from it. Publication is a separate authorized action. The coordinator marks **Accept** only when both GMs and the real crew completed the required run, all three issue criteria have evidence, defects have an explicit disposition, and the humans judge the M1–M3 workflow usable for the intended event. Missing mandatory exercise/evidence remains Pending; a completed run that fails the gate is Reject. A software fix or an automated PASS does not change a human Reject without the relevant human rerun.

```markdown
# #1320 live event record

Status: Pending / Accept / Reject
Decision owner and date:
Session start/end and timezone:
Integrated commit/build/content stamp:
Build URL and network arrangement:
Scenario path/hash, seed and hulls:
Automated #1316/#1317 baseline evidence:

## People and devices
| Role | Participant | Public operator/ship identity | Device/OS/browser/input | Crew Station |
| --- | --- | --- | --- | --- |
| GM A | | | | |
| GM B | | | | |
| Ship A crew | | | | |
| Ship B crew | | | | |
| Evidence recorder | | | | |

## Exercises
| Step | Actual action/observation | Result | Artifact/time/tick | Defect or follow-up |
| --- | --- | --- | --- | --- |
| Equal join/readiness and Pause/Resume | | Pending | | |
| Different and duplicate presets | | Pending | | |
| Private literal and Fleet Comms | | Pending | | |
| Authored hail, crew response, continuation | | Pending | | |
| Two-GM simultaneous actions | | Pending | | |
| Knowledge compared with real crew | | Pending | | |
| All directing families and disclosures | | Pending | | |
| Abrupt GM loss and same-identity reconnect | | Pending | | |
| Complete scenario ending and debrief | | Pending | | |

Comms delay criterion agreed before play:
Measured samples, method and worst observed delay:
GM A assessment:
GM B assessment:
Crew assessments (identify ship):

## Authoritative evidence
Collector and exact capture method/revision:
Command/GM journal and terminal-result files:
Snapshot/export filenames and formats:
Same-boundary checkpoint ticks and per-peer digest values:
Replay artifact/verifier/exit/verdict, or explicit limit:
Final state and scenario ending:
Evidence not obtained and reason:

## Decision against #1320
AC1 — two human GMs, real crew, different/duplicate presets:
AC2 — simultaneous actions, reconnect, Comms, knowledge, directing, disclosures:
AC3 — people/devices, observations, defects, command/digest evidence, decision:
Accept/Reject rationale, or missing work keeping Pending:
Defect dispositions and required human reruns:
```

## Source pointers for the coordinator

- [GM operator identity, role presets and reconnect](../../wiki/entities/gm-operator.md)
- [M1–M3 design and exit boundaries](../../pasm/spec/design/gm-console-t2.yaml)
- [Role-preset controller](../../gui/gm-role-presets.js) and [knowledge comparison scope](../../gui/gm-knowledge-compare.js)
- [Private confirmation controller](../../gui/gm-confirmation.js)
- [Save catalogue UI](../../gui/save-slots.js), [export format](../../src/snapshot.rs) and [headless replay arguments](../../src/headless/args.rs)

After #1317 and the #1316 scenario preparation are integrated, use `docs/gm-comms-authoring.md` for the route and authored hail contract and the generated two-ship world for this event's choices. The generation inputs are [the preparation tool](../../scripts/prepare-gm-live-event.mjs), [second-ship fixture](fixtures/1320-second-ship.toml), and the integrated `assets/worlds/combat_test.toml`. Integrated runtime evidence and human acceptance remain prerequisites; this kit does not supply them.
