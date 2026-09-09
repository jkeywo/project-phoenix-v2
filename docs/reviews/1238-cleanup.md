# Shallow-module checklist reconciliation (#1238)

The #1449 continuation removes `MessageCodec` and its test-only adapter.
`JsonCodec` retains the same four encode/decode methods as inherent methods.
The exhaustive tests still exercise compact and pretty JSON through the
production decoder. Thirteen importing files no longer import the trait.

Both settings adapters now pass visible tab descriptors and body callbacks to
`renderSettingsOverlay`. That shared renderer owns popup, heading, tab strip,
body container, and active-tab dispatch. The host retains polling and the phone
retains state-push updates inside their adapters. Existing CSS hooks, order,
focus-trap ownership, tab visibility, and per-tab arguments remain unchanged.

| Checklist item | Disposition |
| --- | --- |
| Redundant codec trait | Removed in this change |
| Coordination no-op | Removed by `bdb2dff2` |
| Dead damage-bar helpers and standalone ship picker | Removed by `642f544f` |
| Export shared wireText | `e5892a11` |
| One descriptor-driven settings overlay | Shared shell completed by this change's descriptor renderer |
| Retire compatibility aliases | `75707964`; remaining phoenix_math exports are current APIs |
| Rhai host_fn declaration macro | `e2801f03` |
| SpawnSection | `0ec4ac25` |
| SimFixture and removal of test-only HeadlessArgs fields | `7df6ae65` |

Focused validation on this change over `d24bf437`:

- `cargo test --lib --features headless core::codec::tests`: **179 passed,
  0 failed, 0 ignored**, exit 0. Five multiline helper calls initially retained
  an obsolete argument; review caught them and they were fixed before this pass.
- `npx vitest run tests/client/settings-panel.test.js tests/client/server-settings.test.js tests/client/native-settings.test.js`:
  **191 passed**, three files, exit 0.

The entity-config split is a separate #1241 change and is not included here.
Final integration gates and push remain separate.
