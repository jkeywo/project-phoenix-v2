# Increment 4: common observations and reports

The user instructed “Do 4” on 14 October 2026. Prior human/device acceptance
remains open; this authorises advancing without marking it complete.

## Contract

Adopt `pasm/spec/design/common-observations.yaml` before runtime changes.
Introduce a shared Model contract for observation subject, provenance, logical
sample/receipt ticks and supported uncertainty. Recorded Scan readings, dossier
evidence, delayed Sensors reports and repair reports are real consumers. Keep
existing canonical sample/evidence state; metadata projections borrow or derive
from it. Existing wire fields may remain derived presentation mirrors.

Do not label quantized measurements as probability, claims as verified truth,
or unknown confidence as certainty. Keep current observation access, sample
cadence, quantization, routing and gameplay. Ordinary continuously refreshed
blackboards remain live; no every-tick metadata churn on held readings.

Expose metadata through the existing Console owned/feed view and publication
path. Human and individual System AI read the same permitted records, with
current diagnosis/visibility restrictions preserved. Co-hosting adds no knowledge.
Feeds grant information only. Handoff, reconnect and forwarding preserve sample
time; clients derive age from logical time, never wall clocks or receipt time.

Use the existing RepairRequestQueue as the shared outstanding-report record.
Preserve aggregation/ranking, human/AI routing, damage visibility, team assignment,
travel and repair outcomes. Keep contributor attribution and observation/receipt
sequence. Make raised/updated/resolved outcomes explicit. A delayed older report
or completion cannot replace newer evidence. Completed work identifies its System;
recall/Returning alone is not success. Existing operational-group diagnosis becomes
an explicit recovery observation feeding reduction at the existing schedule point,
without expanding current information access or adding a job executor.

New durable report state joins existing capture/restore and deterministic digest.
Use existing replication and transport. Advance compatibility dimensions where
supported state changes; refuse unsupported old state explicitly. Workshop's
existing typed feed editing/validation/Preview/disposable Test remains the authoring
surface; metadata is produced by capabilities, not an editable truth/confidence field.

## Verification

Test held sample age/precision, provenance, delayed-report concealment, explicit
feed isolation/revocation, repair attribution/coalescing, AI-only senders, late
reports/completions, completion versus recall, human handoff and save/replay.
Exercise human Scan/Repair details and real Workshop feed editing/Test. Run targeted
owner tests during development and the required integrated gates once on the
combined result. Compare Cruiser gameplay/performance and unchanged balance
thresholds with Increment 3; keep physical/device acceptance separate.

Advanced sensing is Increment 14; shared jobs/team execution is Increment 5.
Do not migrate unrelated System AI, repair rates, resource models or networking.
