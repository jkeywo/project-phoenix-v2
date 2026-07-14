# Wiki Schema

The wiki is a small, current-state orientation layer for Project Phoenix.

## Authority

1. Code and `assets/` are runtime truth.
2. `pasm/spec/` is the in-repository design and architecture authority.
3. GitHub PRDs/issues are planning truth.
4. `AGENTS.md` is the agent operating manual; `CONTEXT.md` is stable vocabulary; `README.md` is onboarding.
5. The wiki summarises current code navigation only. It is not a design archive.

## Layout

- `entities/`: current domain nouns.
- `concepts/`: current cross-cutting implementation concepts.
- `index.md`: the entry point.

Every page has frontmatter with a title, type, tags, and current code or PASM sources. Do not create historical PRD, ADR, roadmap, or changelog pages here. Git and GitHub preserve history.

## Workflow

Read `index.md` first for non-trivial work. Update a wiki page only when current code navigation changes. Record intended behavior in PASM rather than prose wiki pages. Remove pages that only describe superseded architecture.

## Lint

Check that file references resolve, index links exist, and pages agree with code and PASM.
