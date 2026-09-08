# Typed reconnect projection proof

The #1400 reconnect change replaces the exclusive refresh callback with one
ordinary Bevy `PipeSystem`: collect targeted Welcome occurrences, capture every
owner's projection coherently, then deliver by request ordinal and lexical
owner key. The actual system retains the old callback's full name and boundary
after Lobby and before the Lobby drain and Broadcast. There is one replacement
instance, with the union of its concrete accesses; no diagnostic alias or
ambiguity exemption is added.

Owners register concrete `ReadOnlySystemParam` inputs. The capture holds their
whole read union throughout the cohort, preventing a writer from splitting
Hull and Repair observations. Duplicate request tokens and payload order are
retained. Empty requests do not invoke projectors, and targeted projection does
not mutate live recipient caches. Registration is sealed after plugin finish.

`ReconnectRequests` is an exact physical alias of the existing
`ReplicationLifecycleRegistry` Cache owner. The enumeration guard checks this
identity alongside the seven RNG and four mint aliases, rejects an undeclared
generic instantiation, and verifies that aliases add no canonical census rows.
The authoritative digest and original ambiguity allowance remain unchanged.

## Focused checks

The ten framework regressions cover owner keys and markers, cardinality,
duplicates, privacy, ordering, coherent read access, sealed registration and
idle projection cost. Owner and lifecycle tests cover existing resets and
per-recipient behavior. The advancing Shields and Weapons fixtures compare
complete 12-frame traces in two fresh default processes and one pinned process,
including the unaffected recipient, underlying state and caches.

`tests/reconnect_repair_boundary.rs` supplies an actual correlated Repair
command through normal ingress and a real Identify on its admitted tick, then
through team travel and arrival. Both Apps explicitly use a two-tick transport
delay and must observe two pre-due idle boundaries. Authored team count, travel
duration and repair rate remain unchanged. It requires actual repair events,
HP improvement and coherent Hull/Repair visibility, comparing complete state,
recipient caches and raw messages with an otherwise identical host that does
not reconnect. Three fresh child processes retain their actual completion
summaries and complete reports. This is software validation, not physical
console or human GM acceptance.

The graph diagnostic finishes and cleans ready plugins before inspection, so
finish-registered systems cannot disappear from the captured schedule. Four
generic lifecycle regressions check real late resource conflicts without
executing Startup or a frame, and retain refusal of unready plugins and an
already initialized graph.

## Evidence and limits

The first native attempt compiled and passed all 26 selected library tests and
all six reconnect observations internally. It exposed a missing exact cache
alias in the enumeration test and an incomplete diagnostic capture before
plugin finalization. Its 1,509-row capture is therefore **not** a valid runtime
debt measurement.

Comparing full traces with the earlier passing baseline also found two
Viewscreen changes delivered one tick late. The underlying state and all other
observations matched. The aggregate broadcaster now explicitly follows the
Viewscreen publisher within PublishAggregate, and the existing Weapons fixture
requires both changed snapshots and the delta cache to update in that tick.
The complete original trace comparison remains intact.

Corrected execution is recorded locally under
`.t2-batch/typed-reconnect-native-r2/`, with base `ed0990c9` and runtime source
inventory SHA-256
`BDFE7EE38A28BD0D2AA93C659C1B8FBCBBA4E7A3072440784C97896BF6BEE22F`.
The configuration is `host,headless,perf,ultralight`, locked offline dependencies
and six build jobs. The build, all 26 focused library tests, all four state
enumeration tests, Lobby outbox tests and the Repair/Identify parent with three
fresh children passed. Shields and Weapons each passed three fresh observations.
Their complete 12-frame traces also match the earlier R4 baseline in all three
roles, without normalizing or dropping fields. The publisher parent and twelve
children and foreign-consumer parent and nine children passed on this source.

The finalized capture contains 1,543 ambiguity rows, down from R4's 1,685. Its
SHA-256 is
`c272a76bccbf05829049a731cab0cb40cbb8a39e8273fb62d250537a2d257c4d`.
The live debt/history guard passed against the unchanged original allowance.
This measures remaining scheduler debt, not proof that those conflicts commute.

Independent graph comparison accounts for every change: 194 former exclusive
reconnect rows are replaced by 54 concrete conflict vectors; two other rows
disappear through the intended Viewscreen publication order. All 286 system
instances remain, including exactly one callback with its original full name.
Across all 69,169 ordered pairs of unique system names, only the two intended
publication paths change, and no path is lost. All 21 deferred-boundary
signatures retain their complete predecessor/successor name multisets. These
checks preserve repeated-instance counts without claiming stable numeric graph
IDs or inferring complete read/write access from conflict rows.

The diagnostic suite initially passed 21 tests and failed one obsolete
expectation that an inert Input probe must conflict with an exclusive system.
The typed cohort removes that last unordered exclusive callback. The corrected
test independently derives the exact expected probe rows from the actual graph,
including repeated instances, and retains full production-census equality and
digest checks. This test-only correction passed independent review and the
targeted R3 run: 22 tests passed, with the two explicit capture tests left ignored
because both captures had already run separately. R3 rebuilt the library from
this worktree, verified its source/dependency record, and used the pre-change
commit `ed0990c9` as the ambiguity base. Its receipt is under
`.t2-batch/typed-reconnect-invariants-r3/`. All other runtime inputs still match
the passing R2 source inventory. The original failed run is retained.
PASM validate, scan and traceability passed. Ordinary headless, the full pre-push
matrix and broader #1400 acceptance remain separate.
