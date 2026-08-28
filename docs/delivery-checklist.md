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
      single-threaded build nothing and would break the cross-origin rendezvous
      socket and TURN fetches. It becomes a requirement only if the worker-thread spike in
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

Since `e407bfd9` a client whose credential worker is unreachable falls back to
the free shared OpenRelay TURN and says so (`relaySource: 'openrelay'`), so a
broken worker is now a *degraded* connection rather than no connection at all.
That is a safety net, not a fix: OpenRelay is shared, unmetered and nobody's
promise. Everything below still needs doing.

- [ ] **Redeploy the dev worker** to push the current CORS allowlist:

      ```
      cd worker && npx wrangler deploy
      ```

      Until this is done `pp-dev` has no worker relay and every player is on the
      shared fallback.
- [ ] **Give each worker at least one working credential source.** The worker
      now tries Metered.ca and Cloudflare Realtime TURN concurrently and needs
      only one to succeed; the demo worker was last seen failing its Metered
      source. Secrets are per worker name, so set them for each config
      separately:

      ```
      cd worker
      npx wrangler secret put METERED_KEY                                  # dev
      npx wrangler secret put CF_TURN_KEY_ID
      npx wrangler secret put CF_TURN_API_TOKEN
      npx wrangler secret put METERED_KEY       --config wrangler.demo.toml
      npx wrangler secret put CF_TURN_KEY_ID    --config wrangler.demo.toml
      npx wrangler secret put CF_TURN_API_TOKEN --config wrangler.demo.toml
      ```

      Configuring both sources is the cheap redundancy: either alone is enough,
      and a source that starts failing then costs a header line rather than the
      relay.
- [ ] **Verify each worker by hand after any domain, origin or secret change.**
      The failure is silent from the page's side, so check it from outside. The
      whole recipe — this worker and the rendezvous one in §3a — is now a script
      (issue #1113), which makes the judgements for you and exits non-zero when
      one fails:

      ```
      node scripts/check-rendezvous.mjs \
        --turn   https://phoenix-turn-credentials-demo.project-phoenix.workers.dev \
        --origin https://pp-demo.kiwigamedesign.co.uk
      ```

      The `curl` it replaces, for when you want to look at the raw headers:

      ```
      curl -D - -o /dev/null -H "Origin: https://pp-demo.kiwigamedesign.co.uk" \
        https://phoenix-turn-credentials-demo.project-phoenix.workers.dev
      ```

      Expect **`200`**, an `Access-Control-Allow-Origin` **equal to the Origin
      you sent**, and **no `X-Turn-Source-Errors` header**. A `502` means every
      credential source failed. A `200` *with* `X-Turn-Source-Errors` means one
      source is down and the other is carrying it — worth fixing before it is
      the only one. A different origin echoed means the CORS allowlist is stale,
      which is the 2026-08 failure exactly.
- [ ] **Keep the two `ALLOWED_ORIGIN` lists in step with reality.**
      `worker/wrangler.toml` (dev) lists `pp-dev`, the `github.io` origin and
      `localhost:3911`; `worker/wrangler.demo.toml` (demo) lists `pp-demo` only.
      Adding a hostname to either file does nothing until that worker is
      redeployed.
- [ ] **Record what you deployed.** Note the date, the `ALLOWED_ORIGIN` value
      and which credential sources were configured, for each worker, here or in
      the deploy notes. The deployed values are otherwise invisible.

---

## 3a. The rendezvous workers — the same trap, worse consequences

> **BLOCKING SINCE ISSUE #1112.** This service is not deployed, and PeerJS —
> which used to be underneath it — is gone. Until the boxes below are ticked,
> a deployed build has **no join path at all**: the viewscreen shows no code,
> the diagnostics row says the service was lost, and no phone can reach the
> host by any route. This is no longer an opt-in extra; it is the transport.

`worker-rendezvous/` (issues #1111/#1112) is a **sibling** of `worker/`: same two-config
pattern (`wrangler.toml` = `phoenix-rendezvous`, `wrangler.demo.toml` =
`phoenix-rendezvous-demo`), same `ALLOWED_ORIGIN` var, same repo secrets for the
deploy itself, and **no secrets of its own**. It is not a route on the TURN
worker because it holds live state — the join-code registry, presence and the
signalling relay live in a Durable Object.

**Read §3 first, then read this sentence: for TURN a stale `ALLOWED_ORIGIN`
degrades the connection; for rendezvous it means nobody can join at all.** There
is no OpenRelay-shaped safety net here — a rendezvous service is not something a
client can silently fall back to a free public copy of. Verification is the
mitigation, so do not skip the health check.

- [ ] **Decide the Durable Objects tier.** They are an account-plan decision, and
      this is the repository's first use of any Cloudflare primitive beyond
      Workers and Pages. Nothing deploys until the account allows them. Both
      configs declare the class with `new_sqlite_classes` — the SQLite-backed
      storage class, which works on the free tier as well as the paid one and is
      Cloudflare's current default for a new class — so nothing here commits the
      account to the paid path while this box is unticked. `new_classes`
      (key-value-backed, paid-only) is the deliberate edit to make if the tier
      decision goes the other way; changing it after a deploy is a migration,
      not a config tweak.
- [ ] **Deploy the dev worker** — manual, like the dev TURN worker, and no CI
      step deploys it, which is half of how §3's drift happened:

      ```
      cd worker-rendezvous && npx wrangler deploy
      ```
- [ ] **Deploy the demo worker** alongside the demo build:

      ```
      cd worker-rendezvous && npx wrangler deploy --config wrangler.demo.toml
      ```

      Add the matching `cloudflare/wrangler-action@v3` step to
      `.github/workflows/deploy-demo.yml` (`workingDirectory: worker-rendezvous`)
      when the demo build starts using this route, together with a
      sweep-and-verify patch of the service URL literal — the same treatment
      `DEV_TURN_URL` already gets, for the same reason: the literal is baked into
      more than one built file, so a hardcoded file list would miss one.

      **That sweep does not exist yet**, and until it does a demo build's
      rendezvous route points at the DEV worker. `deploy-demo.yml` patches
      `DEV_TURN_URL` only. `gui/rendezvous-transport.js`'s comment above
      `DEV_RENDEZVOUS_URL` says so; update both in the same change, or the next
      agent trusts a CI guard that is not there.
- [ ] **Verify each worker by hand after any origin change.** This service has a
      health endpoint precisely because a WebSocket upgrade is awkward to curl
      and an origin refusal is otherwise invisible from the page's side. Since
      issue #1113 the whole contract — both workers — is a script, and it is
      what a field session's preconditions ask you to run:

      ```
      node scripts/check-rendezvous.mjs \
        --rendezvous https://phoenix-rendezvous-demo.project-phoenix.workers.dev \
        --turn       https://phoenix-turn-credentials-demo.project-phoenix.workers.dev \
        --origin     https://pp-demo.kiwigamedesign.co.uk
      ```

      Exit 0 means both contracts hold; 1 is a real finding; 2 means something
      was unreachable. It asserts more than the eye does: TLS, the protocol
      revision against this checkout's, the bundled join-code table version,
      that a socket endpoint demands an upgrade, and — the one a human never
      thinks to check — that an origin that is NOT ours is refused, because an
      `ALLOWED_ORIGIN` of `"*"` passes every other test while letting anybody's
      page register a host here.

      The `curl` it replaces, for when you want the raw body:

      ```
      curl -s -H "Origin: https://pp-demo.kiwigamedesign.co.uk" \
        https://phoenix-rendezvous-demo.project-phoenix.workers.dev/v1/health
      ```

      Expect `{"ok":true,…,"origin_allowed":true}`, with the `origin` field
      echoing what you sent. `"origin_allowed":false` is the 2026-08 failure
      class, caught before a player meets it.
- [ ] **Run the check before every field session, not only after a deploy.** The
      deployed value is invisible and drifts with nothing to notice; the whole
      point of §3's story is that the repository was right and the edge was
      wrong for weeks. `docs/acceptance/1113-networks.md` makes this its first
      precondition for exactly that reason.
- [ ] **Keep the two `ALLOWED_ORIGIN` lists in step with reality**, and **record
      what you deployed** — date and value, per worker. Same reasoning as §3: a
      worker only picks up `[vars]` on `wrangler deploy`.
- [ ] **Do not look for an opt-in flag: there is not one any more.** #1111
      shipped this route behind `?rendezvous`; #1112 retired PeerJS and with it
      the flag. Both pages now reach for the service on every load.
      `?rendezvous=<url>` survives as a service OVERRIDE — point a dev build at
      a local `wrangler dev` — and the retired spellings (`?rendezvous`, `=on`,
      `=off`) are ignored rather than honoured, so an old bookmark still opens
      the game instead of dialling a host called "on".
- [ ] **Expect a fresh code after a service blip.** A host that loses its record
      re-registers on a backoff and is issued a NEW code, because the old record
      really is gone and the letters on screen resolve to nothing. Anyone reading
      a code aloud across the room has to re-read it. Keeping the SAME code
      across a host drop needs persistence in the service and is issue #1115.

---

## 4. Native host — packaging and hosting

`phoenix-host` builds and runs today, and the tests cover it, but nothing about
*distributing* it has been decided. These are decisions, not chores.

```
cargo build --release --features host --bin phoenix-host
./target/release/phoenix-host --client-dir dist
```

- [x] **Decide whether a binary is published at all**, and where — a GitHub
      release asset is the cheap answer and needs no new infrastructure.
      **Decided 2026-08-17:** yes, a GitHub Release. `deploy-demo.yml`'s
      `package-native-demo` job builds and publishes it automatically on every
      demo deploy, under the rolling `demo-latest` tag — no manual step.
- [x] **Decide the bundle shape.** The host needs a `--client-dir` (a built
      `dist/`) and a `--content-dir` (the `assets/` tree the manifest and worlds
      are read from). A release archive that carries both, with the binary,
      makes the version pin trivially satisfiable; two separate downloads make
      it the user's problem.
      **Decided 2026-08-17:** one archive — `phoenix-host.exe`, `dist/`, and
      `assets/` together, plus a `README.txt` with run instructions.
- [x] **Code signing.** An unsigned binary is a SmartScreen warning on Windows
      and a Gatekeeper refusal on macOS. Signing needs a paid certificate and an
      identity decision; until it is made, the honest answer is "Windows only,
      unsigned, with instructions", and the release notes should say so rather
      than let a player discover it.
      **Decided 2026-08-17:** staying unsigned for now. Windows only; the
      release's `README.txt` documents the SmartScreen click-through. Revisit
      once there's a real distribution need beyond people who can be walked
      through it.
- [x] **Decide the LAN story.** The default bind was `127.0.0.1:8080`, correct
      for a single machine and useless for a bridge crew. `--addr 0.0.0.0:8080`
      is the LAN form and will prompt a Windows Firewall dialogue on first run;
      whether that is documented or pre-approved by an installer is a packaging
      decision.
      **Decided 2026-08-17:** the default is now `0.0.0.0:8080` — LAN-reachable
      out of the box, no flag needed. Trade-off accepted deliberately: every
      run, including solo play, now triggers the Windows Firewall prompt on
      first launch, and the listening surface is broader by default. Pass
      `--addr 127.0.0.1:8080` to restrict to this machine only. No installer or
      pre-approved firewall rule — manual flag/prompt is still the whole story,
      just inverted from what used to be the default.
- [ ] **Nothing here is a public-internet server.** `phoenix-host` speaks plain
      HTTP, has no TLS, no authentication and no rate limiting. It is for a LAN
      or a machine behind something else. Do not put it on a public address; if
      that is ever wanted, it is a new decision with its own security review, not
      a flag.

### What the native host does NOT do yet

State this in any release notes, because the gap is not obvious from the name:

- With no `--world` it serves assets, the content manifest, the catalogue and
  the version pin, and nothing else — the authoritative sim is then still the
  browser host (`server.html`) or `phoenix-headless`. With `--world` (issue
  #1121) it *is* the authoritative host and draws the viewscreen itself.
- Since issue #1113 it **can** carry a crew of its own: `--rendezvous <URL>
  --origin <URL>` registers it with the rendezvous service and browser clients
  join over the service's WebSocket game relay, because a native process has no
  WebRTC. It prints the five-letter code at startup. A `--world` host can
  therefore be crewed by phones over the relay, by `--solo` (all Backfill), or
  by local Ultralight panes (§4a) — and without the rendezvous flags nobody can
  join and the host says so at boot, which is what `--solo` is for.
- What it still does **not** do is WebRTC. Every crew member on a native host is
  relayed, so the service carries their traffic for the whole mission rather
  than only introducing them. That is a real cost difference from a browser
  host, and the reason the relay's bounds are authored in
  `assets/join/join-codes.toml` rather than assumed.
- Snapshot save/restore works (`tests/native_host_snapshot.rs`); there is no
  operator-facing session surface for it yet.

---

## 4a. Ultralight local Station panes (issue #1122)

`--pane <NAME>` opens a local bridge station in an embedded browser view inside
the host process. It needs a build with the `ultralight` cargo feature, which
**no CI job sets and none may ever set**: `ul-next-sys`'s build script downloads
a proprietary ~100 MB SDK archive at build time.

Keeping that true takes an arrangement, not just a default-off feature.
`--all-features` enables it regardless, and the `test` job's clippy step asked
for exactly that until issue #1122's review caught it. That step now names its
features — every one `Cargo.toml` declares that **gates code**, except
`ultralight` — and `AGENTS.md`'s local gate command mirrors the same list.
`falling-skyway-sim-tests` is left out of both and costs nothing: its only use
is `#[cfg_attr(not(feature = …), ignore)]` in `tests/headless_runner.rs`, so it
flips 55 tests between run and ignored and compiles not one extra line. **A new
cargo feature that gates code belongs in both lists**, or it is a feature
nothing lints. Vellum does the same thing a different way: its engine job
selects the workspace with `--exclude vellum-ultralight`, and type-checks the
SDK half in a manual, non-required job.

```
node scripts/build-client.mjs            # a pane loads the BUILT bundle's page
cargo build --release --features ultralight --bin phoenix-host
./target/release/phoenix-host --client-dir dist \
    --world assets/worlds/combat_test.toml --pane Ada --pane Grace
```

- [ ] **REPOINT THE `vellum-ultralight` DEPENDENCY BEFORE THIS BATCH MERGES.**
      `Cargo.toml`'s `[workspace.dependencies]` currently reads
      `vellum-ultralight = { git = "file:///C:/Coding/vellum", branch =
      "phoenix-ultralight" }` — a **local clone on one Windows machine**, because
      the vellum branch that carries the crate is not pushed yet. **CI cannot
      resolve that line**: cargo fetches every git dependency during resolution,
      optional or not, feature-gated or not, so an `ubuntu-latest` runner fails
      at `cargo metadata` before it compiles anything. Expected on the issue
      branch; a merge blocker.
      The fix, once the vellum branch is pushed: change it to
      `{ git = "https://github.com/jkeywo/vellum", rev = "<merged rev>" }` **and
      bump the other six vellum revs to that same rev in the same diff**, because
      issue #1184's rule is one vellum revision for the whole repository. A
      comment in `Cargo.toml` says the same thing; this line exists because a
      comment is not a gate.
- [ ] **Decide whether a packaged release ships the Ultralight build at all.**
      Today `deploy-demo.yml`'s `package-native-demo` builds
      `--features host`, which has no Ultralight in it, so the published archive
      is unaffected by any of this. Shipping panes means adding the SDK
      download to that job and the redistributables to the archive — a
      decision with a licence question attached (below), not a flag.
- [ ] **Ultralight licence and redistributables.** The SDK is proprietary. The
      download is a public, unauthenticated URL (`ul-next-sys`'s build script);
      **no credential exists and none is checked in**, and nothing about the SDK
      is vendored into this repository or into vellum. The terms are the
      **Ultralight Free License Agreement V1**, shipped inside the download at
      `<ul-sdk>/license/LICENSE.txt`, alongside `EULA.txt` and `NOTICES.md`;
      `vellum_ultralight::staging::licence_files` prints their paths from a
      checkout, and `phoenix-host` logs them at startup. Read them before
      putting `Ultralight.dll`, `UltralightCore.dll`, `WebCore.dll` or
      `AppCore.dll` in a distributed archive.
- [ ] **Runtime staging is automatic but local.** Cargo links the SDK and stages
      nothing, so `panes::ultralight::stage_sdk` copies the four libraries
      beside the executable and the SDK's `resources/` (CA bundle, ICU table)
      into the working directory at startup, from
      `target/<profile>/build/ul-next-sys-*/out/ul-sdk`. **That path only exists
      in a build tree.** A packaged archive has to carry them itself — decide
      that with the packaging decision above. On Windows a missing DLL is a
      process that exits with an OS error code and *no message at all*, so the
      operator log names each one.
- [ ] **`--pane` needs `--client-dir`.** A pane loads the client bundle this same
      process serves, from this process's own address; the argument parser
      refuses the combination at the prompt rather than opening a blank view.

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
      entry points, which blocks every cross-origin subresource — the rendezvous
      service and the TURN credential worker included. Both would need a same-origin path or a
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
      a relay is available **from the worker**, not the shared OpenRelay
      fallback: the lobby shows a "using free fallback relay" notice when
      `relaySource` is `openrelay`, and seeing it means §3 is not finished.
- [ ] Load the public build twice from a cold cache and a warm one, and confirm
      the second load does not re-download the WASM.
