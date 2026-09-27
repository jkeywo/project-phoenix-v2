# Mixed browser/native recovery observations (#1534)

`scripts/fleet-mixed-recovery.mjs` extends the real mixed matrix after its healthy
six-peer workload gate. The composition is browser ships 1/2 and GM 1, native
ships 3/4 and GM 2, with twelve active Station documents. No runtime pass is
claimed by the runner or its fixture tests alone.

## Invocation

Use the integrator's source-matched native binary and browser/native bundles,
and both build receipts. Keep them unchanged for the run. For example:

```powershell
node scripts/fleet-mixed-recovery.mjs --failure ship --routes ws-relay --render --seconds 3 --deadline 295 --binary <phoenix-host.exe> --bundle <native-dist> --dist <browser-dist> --source <source-checkout> --build-receipt <native-receipt.json> --wasm-build-receipt <wasm-receipt.json> --out <new-artifact-directory>
```

Repeat with `--failure gm`, `leader`, or `replacement` in new output directories.
The `ship` and `gm` cases terminate native ship 3 or native GM 2; `leader` closes
the browser owner's simulation page. `replacement` first verifies native ship 3
loss, then starts a fresh native process whose ordinary fleet join requests the
same technical slot. The observer supplies the existing `join` API's claim
argument; it does not change admission or write simulation state. Each process
has a private identity-store directory. No reconnect credential is logged.

The healthy matrix's deadline includes recovery. A fault has at most 90 seconds
and cannot extend the overall 295-second cap; startup or automatic fallback can
consume that budget and produce an explicit failure. Each native probe also has
its own 300-second lifetime and bounded, confirmed process cleanup.

## Required evidence

- Six distinct admitted baseline slots and the existing healthy workload gate.
- Actual victim process/page exit, stable survivor slots, exactly one identical
  applied HostLoss per survivor, and crew removal/Backfill for a lost ship.
- Browser simulation progress and the reduced wait set. Native progress is
  measured by actual outgoing digest checkpoints; no synthetic phase or mesh
  tick is substituted for absent native diagnostics.
- Two exact 16-digit hexadecimal matching post-loss checkpoints from all five
  survivors; contradictory repeated checkpoints fail.
- For owner loss, committed continuation and the same loss watermark in every
  browser/native survivor's authoritative status.
- No repeated outgoing command order or observer/runtime error. Outgoing order
  uniqueness does not independently count every simulation side effect.
- Replacement additionally requires all six peers to report one restore boundary
  and claim sequence, one authoritative SlotClaim, the original technical slot,
  the old ship entity ID in a new post-restore command, and two fresh matching
  six-peer checkpoints.

Raw native events, process manifests, source/bundle/binary hashes, browser
observations, relay counters and recovery verdicts remain in the ignored output
directory. Overflow is a failure. A missing observation never becomes a pass.

## Limits

This is one-machine loopback, with rendered browser WASM and real native
Bevy/Ultralight. The native replacement case has one candidate; the separate
browser replacement helper owns the simultaneous race and connected-holder
challenge. This runner does not claim native owner promotion (its original
owner is a browser), arbitrary packet-loss tolerance, divergence restoration,
endurance, physical-network behavior or complete effect-count auditing.
An old-owner suffix retained only by a non-successor has no transferable origin
proof and intentionally remains held/refused. A failed owner-continuation cell
must retain that evidence rather than weaken the provenance gate.
