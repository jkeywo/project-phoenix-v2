---
title: "Issue #509: phone-density layout review + layout-hint/aggregation spec"
kind: source
issue: 509
status: decided
---

The design question in issue #509 — whether to invest in a generalised layout-hint / fragment-aggregation engine or to keep the current per-console HTML pattern — was resolved in favour of **per-console HTML supported by a shared authoring library**. Each console owns its own hand-authored HTML file that handles layout, aggregation, and render logic; the engine provides no declarative HTML generation. This pattern is supported by `gui/console-ui.js`, a reusable DOM-primitives library (a #510 deliverable) that eliminates the boilerplate currently copy-pasted across console files. See [wiki/concepts/console-ui-library.md](../concepts/console-ui-library.md) for the full decision record, library API, and migration strategy.
