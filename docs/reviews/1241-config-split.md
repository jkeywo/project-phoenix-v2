# Entity schema split and cleanup reconciliation (#1241)

The #1449 change moves subsystem schema declarations from `entities/config.rs`
into thirteen leaves under `entities/config/`: AI, visual/LOD, celestial,
hull/collider, Helm, weapons, console metadata, power, shields, repair, Comms,
sensors/navigation, and scene shapes. `EntityConfig::from_toml`, integration
validation, and the existing test-module path remain in `config.rs`.

Parent re-exports preserve all existing `entities::config::*` import paths.
Independent source comparison confirmed each extracted range is unchanged
apart from imports, whitespace, and the parent visibility needed for
`reject_relocated_mesh_lod`. Serde attributes, defaults, diagnostics, and
composition logic are preserved.

`cargo test --lib --features headless -- entities::config::tests entities::include_resolve world::spawn_origin world::dispatch_tests`
passed **415 tests, 0 failed, 0 ignored**, exit 0, on the schema-split working
tree above `828393d6`. Independent read-only review: PASS.

The other seven checklist items were already complete; each commit below was
verified to be an ancestor of this batch:

| Item | Landed commit / current evidence |
| --- | --- |
| Blaster feature-only bindings | `323b915e`, explicit server-feature gates |
| Broadcast file size | `83108927`, 778-line broadcast plus 1,311-line publish module |
| Banner encoding | `f5c60425`, clean torpedo and module-test banners |
| Placeholder PASM IDs | `e5985bf0`, four definitions resolve; later debug IDs added by `08202345` |
| Connection diagnostics nullguard | `c3384dec`, both pages use optional page-chrome adapter |
| Script ledger returned as data | `bb1f8193`, LedgerPlan is applied by the caller |
| Nonexistent test citation | `26e22954`, hull comment acknowledges deletion and historical validation gap |

No gameplay tuning or new validator was added under the citation cleanup.
Final PASM/wiki checks, integration gates, and push remain separate.
