# Increment 4: common observations and reports

Contract: [increment 4](../gameplay-redesign/implementation/increment-4.md).

## Implemented scope

Scan readings, dossier evidence, delayed Sensors reports and repair contributors
use common subject, provenance, logical sample/receipt time and uncertainty.
Existing canonical records remain authoritative. Quantization names the measured
field; unknown precision stays unknown. Forwarding and hosting changes preserve
sample time. Continuation generation matching keeps old packets from mis-ageing
records after a restore.

Console owned/feed views expose borrowed metadata through existing permissions;
co-hosting grants no additional knowledge or machinery authority. The shared
details disclosure uses only delivered data. Ordinary live gauges retain their
current publications rather than gaining new sample history.

The existing Repair queue accepts ordinary AI and human-routed reports, retains
contributors and rejects stale samples/outcomes. Actual team work contributes
completion evidence; recall alone does not. The existing operational diagnosis
produces an explicit recovery observation at its current schedule point. Same-tick
new damage remains admissible. Priorities, dispatch, repair rates and outcomes
remain unchanged; shared jobs are later work.

Snapshot 40 and simulation rules 0.12 reject incompatible earlier continuation
state explicitly. Protocol 6 and the existing transport/delivery classes remain.
Workshop edits existing typed feeds; it does not author truth or confidence.

## Evidence

Runtime implementation: `0e331b4f` plus RON continuation fix `faf88fb6`, following
the common contract, client and browser commits. Independent reviews passed for
producers, Console access, repair reduction/visibility/continuation, client clock
ordering and documentation.
The [validation receipt](../gameplay-redesign/implementation/observations-validation.json)
records commands, configurations, counts and raw-log hashes. The
[baseline comparison](../gameplay-redesign/implementation/observations-baseline.md)
compares the same bounded scenarios and unchanged Cruiser thresholds.

The full workspace suite passes 9,101 tests (102 existing ignored); the
renderer-independent Simulation suite passes 4,682, and JavaScript passes 8,719.
Integration exposed and corrected named-System RON decoding and historical
payload fixture migrations without weakening assertions or changing gameplay.
All required static/PASM gates pass. Fifteen observation/repair browser cases
and both rendered Workshop cases pass. The separate viewer bundle was rebuilt
before Preview acceptance; no stale artifact or relaxed renderer assertion was used.

Both bounded scenario reports match Increment 3 byte for byte. Seeded repeats,
replay and continuation agree; Cruiser balance remains 20/20 victories, all 16
unchanged thresholds pass, and merged matrix results are identical. Timings are
similar in the same-machine diagnostic. Automated acceptance is complete;
physical/device and human-workload acceptance remains open.

The new Chromium case renders the shipped Scan and Repair components with
already-projected records. It verifies held/forwarded sample age, disclosure
continuity, unknown or concealed precision, contributor attribution, absent
concealed deficit and revocation. It does not establish server permissions by
itself. Native owner/integration tests cover those boundaries. Existing repair
control browser cases also pass. The details screenshot was inspected.

The rendered Workshop journey authors typed feeds, validates, exercises history,
export/reload and disposable Test. The separate Preview case renders a draft-only
model through the real WASM path. Browser transport uses the existing shim.

## Human/device acceptance

Prior physical phone/native multi-monitor and human-workload acceptance remains
pending. The user explicitly authorised advancing to Increment 4. Increment 5
has not started. The ledger retains `in_progress` until this acceptance is complete.

On a mixed human/AI Cruiser, take a Scan reading and leave it held while the world
changes. Open details and confirm age advances without renewing the sample. Hand
off or reconnect the Console and confirm the same reading remains. Inspect a
dossier claim and a delayed Sensors report without confusing their provenance
with verified truth.

Damage a System operated by ordinary AI, inspect its repair report and dispatch
a team. Recall before completion, then complete actual work. Confirm recall is
not reported as recovery and newer damage remains outstanding. In Workshop edit
a typed Console feed, undo/redo, save/reload, Preview and enter/stop disposable
Test. Judge the disclosures on physical phones and native Station displays;
assess attention cost and clarity separately from automated checks.
