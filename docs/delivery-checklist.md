# Delivery checklist — the manual half of PRD #855

Everything in this repository that needs a Cloudflare account, a secret, a
domain, or a signing decision. Nothing here can be done from CI or by an agent:
each item needs credentials or console access, so each one is a step for the
repository owner.

The code half is done and tested. This file is the list of things the code
cannot do for itself, in the order they matter.

**How to read a checkbox here.** Unticked means "nobody has done this"; there is
no state in this file that CI keeps up to date. Tick one when you have done it
and, where the item says so, record the value you used — several of these are
invisible once set and drift silently, which is exactly how the 2026-08 TURN
outage happened.

---

## 0. What the code now does on its own

Context for everything below, so the manual steps are not read as the whole
picture.

| Concern | Handled in repo by | Verified by |
| --- | --- | --- |
| Native PC host serving client + manifest + catalogue | `phoenix-host` (`src/delivery/`, `--features host`) | `tests/native_host.rs`, `src/delivery/*` unit tests |
| Version pin (protocol + content id/epoch) | `delivery::stamp`; startup pin against the bundle, request-time pin against a client | `tests/native_host.rs`, `src/delivery/stamp.rs` |
| Native and browser hosts publishing the same catalogue | `delivery::payload` — one field list, walked by the wasm bridge and by the JSON encoder | `the_native_hosts_catalogue_is_the_browser_hosts_catalogue_with_no_packs_applied` |
| Curated public catalogue | `assets/scenarios.demo.toml`, selected by `--manifest` (native) / `?manifest=` (browser) | `the_curated_public_manifest_really_does_restrict_what_the_native_host_publishes` |
| No runtime widening of the curated catalogue | `wasm_add_mod_pack` absent from a demo build; upload control removed | `build_flags::a_demo_build_that_curates_its_catalogue_offers_no_mod_pack_upload`, deploy-demo.yml's verify step |
| Deployed caching/headers | `deploy/cloudflare/_headers`, installed by deploy-demo.yml | `tests/client/deploy-headers.test.js`; `scripts/check-deploy-headers.mjs` against a real URL |

---

## 1. Cloudflare Pages — the restricted public build

The `Deploy Demo` workflow (`.github/workflows/deploy-demo.yml`) is
`workflow_dispatch` only and creates the Pages project on first run, so most of
this is already automated. What is not:

- [ ] **Repository secrets exist.** `Settings → Secrets and variables → Actions`
      must carry `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`. The token
      needs `Account → Cloudflare Pages → Edit` and `Account → Workers Scripts →
      Edit` (the same workflow deploys the demo TURN worker). Without both, the
      workflow fails at its first `wrangler-action` step.
- [ ] **Confirm the Pages project name.** The workflow creates and deploys
      `project-phoenix-demo` with `--production-branch=main`. If the account
      already has a project by that name owned by something else, rename it in
      the workflow rather than reusing it.
- [ ] **Custom domain.** `pp-demo.kiwigamedesign.co.uk` is what
      `worker/wrangler.demo.toml`'s `ALLOWED_ORIGIN` names, so the Pages project
      must actually serve that hostname or TURN is CORS-blocked (see §3). Add it
      under `Pages → project-phoenix-demo → Custom domains` and wait for the
      certificate before testing.
- [ ] **Leave the dev host alone.** `pp-dev.kiwigamedesign.co.uk` is GitHub
      Pages, published by `ci.yml`'s `deploy` job, and keeps its debug tooling
      on purpose. Nothing in this checklist should be applied to it.

### Caching rules

The rules ship in the repository as `deploy/cloudflare/_headers` and are copied
to `dist/_headers` by the deploy workflow. Pages reads that file from the root
of the uploaded directory, so no dashboard configuration is needed for them.

- [ ] **Do not add dashboard Cache Rules that also set `Cache-Control`.** Pages
      applies every matching `_headers` rule, and this repository has no way to
      test precedence when two sources set the same header — which is why
      `_headers` itself is written so no two of its patterns overlap. A
      dashboard rule would reintroduce exactly the ambiguity the file avoids.
- [ ] **After the first deploy, purge the cache once.** Anything already held
      from a deploy that predates `_headers` keeps its old policy until it
      expires. `Caching → Configuration → Purge Everything`, once.

---

## 2. Verifying a deploy's headers

- [ ] **Run the check after every public deploy.** Either from a laptop (needs
      only Node 20 — the script has no dependencies):

      ```
      node scripts/check-deploy-headers.mjs https://pp-demo.kiwigamedesign.co.uk/
      ```

      or by dispatching the `Check Deploy Headers` workflow with that URL as its
      `url` input. Exit 0 means the contract holds; 1 means a real finding; 2
      means something was unreachable.

- [ ] **Leave `require_isolation` off.** Cross-origin isolation buys the current
      single-threaded build nothing and would break the cross-origin PeerJS and
      TURN fetches. It becomes a requirement only if the worker-thread spike in
      §5 says yes.

The check is deliberately not a push gate: it talks to a live origin, so as a
blocking step it would turn someone else's uptime into a red branch. The half
that can be checked offline runs on every push already.

---

## 3. The TURN credential workers — the known trap

**Read this before touching a domain.** On 2026-08-15 phones could not join a
host on a phone hotspot. Root cause: the *deployed* `phoenix-turn-credentials`
worker still carried `ALLOWED_ORIGIN = https://jkeywo.github.io` from before the
custom domain existed. Every browser fetch from `pp-dev.kiwigamedesign.co.uk`
was CORS-blocked, clients silently fell back to a TURN list that no longer
exists, and no relay meant no CGNAT/hotspot connection at all. The repository's
`wrangler.toml` had been correct the whole time — **a worker only picks up
`[vars]` on `wrangler deploy`**, and worker deploys are out-of-band from the
Pages deploy, so deployed configuration drifts from the file that describes it
with nothing to notice.

- [ ] **Redeploy the dev worker** to push the current CORS allowlist:

      ```
      cd worker && npx wrangler deploy
      ```

      Until this is done `pp-dev` has no TURN at all.
- [ ] **Fix the demo worker's Metered credentials.**
      `phoenix-turn-credentials-demo` currently answers **401** from Metered.ca,
      i.e. a wrong `METERED_KEY` or `METERED_APP` for that target, so `pp-demo`
      has no TURN either. The key is a secret and must be set per worker name:

      ```
      cd worker && npx wrangler secret put METERED_KEY --config wrangler.demo.toml
      ```

- [ ] **Verify each worker by hand after any domain or origin change.** The
      failure is silent from the page's side, so check it from outside:

      ```
      curl -D - -o /dev/null -H "Origin: https://pp-demo.kiwigamedesign.co.uk" \
        https://phoenix-turn-credentials-demo.project-phoenix.workers.dev
      ```

      Expect `200` and an `Access-Control-Allow-Origin` **equal to the Origin you
      sent**. Anything else — a different origin echoed, a 401, a 502 — means
      clients on mobile networks will fail to connect while the host page looks
      healthy.
- [ ] **Keep the two `ALLOWED_ORIGIN` lists in step with reality.**
      `worker/wrangler.toml` (dev) lists `pp-dev`, the `github.io` origin and
      `localhost:3911`; `worker/wrangler.demo.toml` (demo) lists `pp-demo` only.
      Adding a hostname to either file does nothing until that worker is
      redeployed.
- [ ] **Record what you deployed.** Note the date and the `ALLOWED_ORIGIN` value
      each worker was last deployed with, here or in the deploy notes. The
      deployed value is otherwise invisible.

---

## 4. Native host — packaging and hosting

`phoenix-host` builds and runs today, and the tests cover it, but nothing about
*distributing* it has been decided. These are decisions, not chores.

```
cargo build --release --features host --bin phoenix-host
./target/release/phoenix-host --client-dir dist --addr 0.0.0.0:8080
```

- [ ] **Decide whether a binary is published at all**, and where — a GitHub
      release asset is the cheap answer and needs no new infrastructure.
- [ ] **Decide the bundle shape.** The host needs a `--client-dir` (a built
      `dist/`) and a `--content-dir` (the `assets/` tree the manifest and worlds
      are read from). A release archive that carries both, with the binary,
      makes the version pin trivially satisfiable; two separate downloads make
      it the user's problem.
- [ ] **Code signing.** An unsigned binary is a SmartScreen warning on Windows
      and a Gatekeeper refusal on macOS. Signing needs a paid certificate and an
      identity decision; until it is made, the honest answer is "Windows only,
      unsigned, with instructions", and the release notes should say so rather
      than let a player discover it.
- [ ] **Decide the LAN story.** The default bind is `127.0.0.1:8080`, which is
      correct for a single machine and useless for a bridge crew.
      `--addr 0.0.0.0:8080` is the LAN form and will prompt a Windows Firewall
      dialogue on first run; whether that is documented or pre-approved by an
      installer is a packaging decision.
- [ ] **Nothing here is a public-internet server.** `phoenix-host` speaks plain
      HTTP, has no TLS, no authentication and no rate limiting. It is for a LAN
      or a machine behind something else. Do not put it on a public address; if
      that is ever wanted, it is a new decision with its own security review, not
      a flag.

### What the native host does NOT do yet

State this in any release notes, because the gap is not obvious from the name:

- It serves assets, the content manifest, the catalogue and the version pin. It
  does **not** run the simulation — the authoritative sim is still the browser
  host (`server.html`) or `phoenix-headless`.
- It does **not** do PeerJS signalling. Clients still reach the host through the
  PeerJS cloud broker exactly as they do today.
- It has no snapshot, save, or session surface.

---

## 5. Worker threads — explicitly out of scope

PRD #855 puts cross-origin isolation and worker threads behind a benchmark
spike, and this batch wrote **no code** for either. The gate, so the decision is
not quietly skipped later:

- [ ] **Run the spike before writing any of it.** Measure the current
      single-threaded frame cost with the existing tooling
      (`phoenix-perf`'s `browser` scenario and the committed baselines) and
      state what a multi-threaded build would have to beat.
- [ ] **Price the cost, not just the win.** Isolation means COOP/COEP on both
      entry points, which blocks every cross-origin subresource — PeerJS and the
      TURN credential worker included. Both would need a same-origin path or a
      CORP header from their side before isolation is even possible.
- [ ] **Only then** flip `require_isolation` on in the header-check workflow and
      add the COOP/COEP rules to `deploy/cloudflare/_headers`. The checker
      already refuses the half-applied combination (COEP without COOP), which is
      the failure mode to expect.

---

## 6. Multi-device manual test

PRD #855's testing decisions ask for a manual multi-device pass; it needs real
phones on a real network, so it stays here.

- [ ] Host on the native `phoenix-host` over LAN, join from two phones on the
      same Wi-Fi, confirm the catalogue shows the curated scenario and hull only.
- [ ] Host on the deployed public build, join from a phone on **mobile data**
      (not Wi-Fi) — this is the case the TURN relay exists for and the one the
      2026-08 outage broke. Confirm the host page's connection diagnostics report
      a relay is available.
- [ ] Load the public build twice from a cold cache and a warm one, and confirm
      the second load does not re-download the WASM.
