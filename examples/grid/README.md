# Grid: a second game using the shared layers

A small authoritative game: console buttons move a marker on an 8 by 8 grid.
It uses Phoenix runtime ordering, lockstep readiness, digest history and recovery
chunks, transport admission/relay and platform atomic file replacement. It has
no dependency on Phoenix game rules, model, content or presentation.

## Run locally

From the repository root, with Rust's `wasm32-unknown-unknown` target and
`wasm-bindgen-cli` matching Cargo.lock (currently 0.2.120):

```sh
npm ci
cargo build -p phoenix-grid
node scripts/build-grid.mjs
node scripts/rendezvous-dev-server.mjs --port 8788 --codes examples/grid/join-codes.json
```

Serve the repository over HTTP in another terminal (for example
`python -m http.server 8000`). Open
`http://127.0.0.1:8000/examples/grid/`. Choose **Host in this browser** to run the
WASM host, or start a native host in another terminal:

```sh
cargo run -p phoenix-grid -- http://127.0.0.1:8788 http://127.0.0.1:8000
```

An optional third argument is the native checkpoint file. Open a second page,
enter the host's join code and join as a console. Console pages download no WASM.
Browser-host checkpoints survive reload in localStorage. The Recover button
requests the host checkpoint over reliable delivery; normal state uses snapshots.
`GRID_WASM_BINDGEN` may select an exact-version executable for the build script.

## Verify

```sh
cargo test -p phoenix-grid
npm ci --prefix tests/smoke
npx --prefix tests/smoke playwright install chromium
node tests/layers/grid-smoke.mjs
```

The smoke test starts its own static server, rendezvous and native host, drives
native and browser-host movement/reconnect/reload, checks console requests for
WASM, and stops its services. Build the native binary and browser package first.
Unit tests cover peer ordering with delayed/duplicate delivery, digest divergence,
checkpoint restoration and rejection without partial state mutation.

The browser smoke distinguishes three contracts: console checkpoint delivery
(the console paints the checkpoint), native file persistence across process
restart, and host continuation from the same checkpoint on native and WASM.
`phoenix-grid --verify-continuation <checkpoint-file>` is the finite comparison
mode used by the smoke: it restores the checkpoint, queues right/down moves,
advances five ticks and prints the resulting checkpoint and State envelope.
The direct Rust chunk-transfer tests separately prove in-memory runtime recovery;
ordinary console connections do not implement networked host-to-host lockstep.
