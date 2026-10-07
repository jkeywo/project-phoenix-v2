# Simulation composition integration

The five structural stages are ported from `codex/simulation-composition` onto
main baseline `f14895ca87379b0a4823054f90ba1c32ee62b5b4`, preserving the gameplay
redesign through increment 5. No gameplay or content tuning is included.

Preparation owns offline cross-domain validation and Workshop authoring. World
execution, snapshot continuity and GM responsibilities have explicit internal
owners. Delivery owns HTTP serving over injected content sources. Compatibility
facades preserve callers; no new wire or save format is introduced.

Snapshot format 41, simulation rules 0.13 and protocol 6 remain unchanged.
Console ownership, composed Dock equipment, AI state trees, common observations
and canonical repair jobs retain main's implementation.

## Validation

- Preparation owner tests: 300 passed.
- Independent Simulation suite after World/snapshot extraction: 4,385 passed,
  including one subsequently removed duplicate relocated test.
- Source comparison: all 48 World and 114 snapshot function bodies matched
  the pre-change main revision, disregarding whitespace.
- Final integrated validation is pending; earlier branch receipts are not
  evidence for this integrated revision.

Physical/device and human-workload acceptance remain pending as recorded by
the gameplay redesign ledger. This refactor does not close those exits.
