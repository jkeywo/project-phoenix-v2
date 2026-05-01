---
type: AFK
priority: infrastructure
---

# 011 — Build Configuration

Trunk.toml configurations and GitHub Actions CI/CD.

## What to build

- `Trunk.toml` — default config targeting `server.html`
- `client-trunk.toml` — config targeting `client.html`
- `.github/workflows/deploy.yml` — builds both with Trunk, merges outputs into `dist/`, deploys to `gh-pages` branch via `peaceiris/actions-gh-pages` on push to `main`
- `package.json` scripts for local build: `build:server`, `build:client`, `build:all`

## Acceptance Criteria

- [ ] `cargo check` passes (CI gate)
- [ ] GitHub Actions workflow file is valid YAML
- [ ] `dist/` would contain both `server.html` and `client.html` after a full build
