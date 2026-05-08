# Wiki Schema

This wiki is an LLM-maintained knowledge base for **Project Phoenix v2**, the browser-based bridge simulator. It follows the pattern from [Karpathy's LLM Wiki gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f): a persistent, compounding artifact that an LLM agent reads and writes, with humans curating the sources and asking questions.

## Three layers

1. **Raw sources** — immutable. Live *outside* `wiki/`:
   - The codebase (`src/`, `server.html`, `client.html`, `tests/`)
   - PRDs on the GitHub issue tracker (label `PRD`)
   - Draft design notes (`docs/`)
   - `README.md`, `AGENTS.md`, `CONTEXT.md`
   The wiki **summarises and links** to these. It never duplicates them as canonical truth.

2. **The wiki** — `wiki/`. Markdown files written by the LLM, organised as:
   - `entities/` — one page per noun in the game/system (Console, Ship, Asteroid, Player, Session, …).
   - `concepts/` — one page per cross-cutting idea (Architecture, Networking, Codec, Game Loop, Testing, …).
   - `sources/` — one page per ingested source (PRD-001, design-doc-01, …). Each is a faithful summary of one external document with a backlink.
   - `roadmap/` — synthesis pages about future work, drawn from open PRDs and `docs/` drafts.
   - `index.md` — catalog of every page in the wiki, by category.
   - `log.md` — chronological append-only record of ingests, queries, and lint passes.

3. **The schema** — this file (`SCHEMA.md`). The LLM reads this at the start of every session.

## Page conventions

Every page starts with YAML frontmatter:

```yaml
---
title: Helm Console
type: entity            # entity | concept | source | roadmap
tags: [console, helm, input, ship]
sources: [PRD-022, src/server/simulation.rs, src/client/helm_plugin.rs]
updated: 2026-05-08
---
```

Body structure:

- **Summary** — 1–3 sentences. What this page is about.
- **Sections** — H2 headings. Pick whatever fits.
- **Code references** — use `path/to/file.rs:LINE` so they're clickable in editors.
- **Cross-links** — wiki-style `[Helm Console](../entities/helm-console.md)`. Prefer relative links so the wiki works on disk and in any markdown viewer.
- **Open questions** — if the source is silent or contradictory, capture it under a `## Open questions` heading rather than guessing.

## Source pages

Each source page summarises **one** external artifact:

- Filename matches the source: `prd-022-helm-and-game-world.md`, `design-04-combat-update.md`.
- Frontmatter `source_url` (issue link) or `source_path` (repo path).
- Body sections: `## Status`, `## Problem`, `## Solution`, `## Key decisions`, `## Open user stories`, `## Cross-references`.
- Out-of-scope items from the source are captured verbatim — they're often where future PRDs come from.

## Workflows

### Ingest a new source
1. Drop the new artifact into the raw layer (commit a PRD, add a `docs/*.md`, land code).
2. Read it. Discuss takeaways with the user.
3. Create or update the matching `sources/` page.
4. Update every `entities/` and `concepts/` page the source touches.
5. Append a one-line entry to `log.md`:
   `## [YYYY-MM-DD] ingest | <Source title> | touched: page-a, page-b, page-c`
6. Update `index.md` if new pages were created.

### Answer a query
1. Read `index.md` first to find candidate pages.
2. Read those pages. If they cite a raw source, read the raw source too when precision matters.
3. Synthesise an answer. If the answer is non-trivial, **file it back into the wiki** as a new concept or roadmap page.
4. Append a log entry: `## [YYYY-MM-DD] query | <one-line question> | filed: <new pages>`

### Lint pass
Run periodically. Look for:
- Pages that reference a `path/to/file.rs:LINE` that no longer exists.
- Entity pages that don't appear in any source page.
- Source pages without backlinks from any entity/concept page.
- Contradictions between two pages (e.g. two different "max ship speed" values).
- Open questions that have since been resolved by a new source.
- `roadmap/` pages whose backing PRD has shipped.

## Naming

- Files: kebab-case (`helm-console.md`, `prd-022-helm-and-game-world.md`).
- Use the canonical names from `CONTEXT.md` (Console, Session, Captain, Helm Input, Red Alert, View Mode, Radar, World Data, Lobby Phase, In-Progress Phase). Do **not** invent synonyms.

## What this wiki is not

- Not a replacement for `README.md` (user-facing) or `AGENTS.md` (agent operating manual). Those are raw sources for *this* wiki.
- Not auto-generated API docs (Rust has `cargo doc` for that).
- Not a place to store secrets, transient debugging notes, or chat transcripts.
