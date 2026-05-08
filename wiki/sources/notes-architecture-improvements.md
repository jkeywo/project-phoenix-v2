---
title: Architecture Improvement Notes
type: source
tags: [draft, design, architecture, messaging, subscription]
source_path: docs/Architecture Improvement Notes.md
status: draft (note)
updated: 2026-05-08
---

# Architecture Improvement Notes

> "Network messaging needs breaking up into type (i.e. general ship, nearby entities, console specific). Each console determines which messages it needs. A client's consoles are combined to find which messages it needs."

A scaling concern, not a feature. Currently every client receives every broadcast. As consoles multiply ([PRD #66](./prd-066-weapons-and-engineering.md), Drafts 3–8), each phone gets bombarded with messages it has no use for.

## Proposed model

- Categorise messages by **scope**:
  - Ship-wide (Red Alert, view mode, hull integrity)
  - Nearby-entity (asteroid positions in radar range)
  - Console-specific (target lock for Weapons, breakdown queue for Engineering)
- Each console declares a **subscription** to message categories.
- The client merges its consoles' subscriptions.
- The server filters outbound messages against the client's subscription set.

## Today's mechanism

`OutboundMessage` already carries a routing target (`All` / `Token` / `AllExcept`). The proposal is finer-grained — *what kind* of message, not just *who*.

PRD #66 partially anticipates this: it routes per-console payloads with `Target::One(token)` at 10 Hz so Weapons/Engineering get their own slice. Generalising to "categories the client subscribes to" would let one server message satisfy two clients without duplication.

## Open questions

- Subscription declaration in code (per Console Plugin?) or in data (config file)?
- Does the server keep a "client → subscription set" map, or does each client filter incoming messages itself? Latter is simpler but wastes bandwidth.
- Interaction with [Draft 2's "asteroids only spawn near the ship"](./design-02-game-map.md) — both are about reducing what each client cares about.

## Cross-references

- Concept: [Message Flow](../concepts/message-flow.md), [Console Plugin Pattern](../concepts/console-plugin-pattern.md)
- Roadmap: [Open Architectural Questions](../roadmap/open-architectural-questions.md)
