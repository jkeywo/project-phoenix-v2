/**
 * gui/build-flags.js — "which build am I?" for the pages (issue #939).
 *
 * The host settings menu has to hide its Debug/Cheat tab in the public demo
 * build, so the pages need a runtime answer to "am I the demo?".
 *
 * The flag is `PHOENIX_DEMO_BUILD`, set only by `deploy-demo.yml`, and it is
 * deliberately NOT `TRUNK_BUILD_RELEASE`: that one is the wasm-opt size switch
 * and `ci.yml` sets it too, so gating on it would strip the debug tooling from
 * the GitHub Pages build — which is the DEV host and needs to keep it while
 * staying size-optimised.
 *
 * The two pages learn the flag differently:
 *
 *   - **server.html** compiles it in. `crate::build_flags::is_demo_build()`
 *     bakes `option_env!("PHOENIX_DEMO_BUILD")` into the WASM binary and
 *     `wasm_is_demo_build()` reads it back. That is the authoritative source
 *     and this module prefers it whenever it is available. It is NOT available
 *     for the whole WASM download+instantiate window, though, which is why the
 *     server page also carries the meta tag below — the getter answering
 *     "nothing there" must not be allowed to mean "dev build" on the demo.
 *
 *   - **client.html** has no WASM at all — `scripts/build-client.mjs` is a
 *     deterministic file copy with no compile step to inject anything into.
 *     So the client half (issue #940) uses one of the other two readers below:
 *     either the `<meta name="phoenix-build-demo" content="true">` stamped
 *     into `dist/client/index.html` by the demo workflow (which already
 *     rewrites that file for the worker URL swap), or an explicit
 *     `setBuildFlags({ demo })` call from whatever told it.
 *
 * Resolution: an explicit override wins outright; otherwise EITHER source
 * saying "demo" makes it a demo build. That is an OR rather than a priority
 * ladder on purpose, because each source is silent at a different moment: the
 * getter does not exist until WASM boots (so on the demo it would read as a
 * dev build for the whole download+instantiate window), and the tag is only
 * stamped by the demo workflow (so a locally-compiled `PHOENIX_DEMO_BUILD=true`
 * carries no tag). Neither silence may be read as "dev". "Dev" is what you get
 * when nothing at all says otherwise — an unknown build must not silently hide
 * the debug tools during development.
 *
 * DOM/window-free at import time so vitest can import it in Node.
 */

const META_NAME = 'phoenix-build-demo';

/** Explicit override set by `setBuildFlags`, or `null` when unset. */
let _override = null;

/**
 * Force the answer, for a page that learns its build some other way (the
 * pure-JS client, issue #940) or for a test.
 *
 * @param {{ demo?: boolean|null }} flags — `demo: null` clears the override
 *   and falls back to the detection ladder.
 */
export function setBuildFlags({ demo } = {}) {
  _override = demo === null || demo === undefined ? null : !!demo;
}

/** The current override, or `null` when detection is in charge. */
export function buildFlagOverride() {
  return _override;
}

/**
 * Read `<meta name="phoenix-build-demo" content="...">`, treating exactly
 * `"true"` as the demo build — the same literal `crate::build_flags` compares
 * against, so a stamped page and a compiled one cannot disagree by accident.
 *
 * @param {Document|null|undefined} doc
 * @returns {boolean|null} null when the page carries no such tag.
 */
export function demoFromMeta(doc) {
  if (!doc || typeof doc.querySelector !== 'function') return null;
  const meta = doc.querySelector('meta[name="' + META_NAME + '"]');
  if (!meta) return null;
  const content = typeof meta.getAttribute === 'function' ? meta.getAttribute('content') : null;
  return content === 'true';
}

/**
 * True when this page was built by the public demo deploy.
 *
 * @param {{ win?: object, doc?: Document }} [env] — injectable for tests.
 * @returns {boolean}
 */
export function isDemoBuild(env = {}) {
  if (_override !== null) return _override;

  const doc = env.doc !== undefined
    ? env.doc
    : (typeof document !== 'undefined' ? document : null);
  if (demoFromMeta(doc) === true) return true;

  const win = env.win !== undefined
    ? env.win
    : (typeof window !== 'undefined' ? window : null);
  if (win && typeof win.wasm_is_demo_build === 'function') {
    try {
      if (win.wasm_is_demo_build()) return true;
    } catch (_) {
      // WASM not up yet, or torn down mid-call — the tag above already had
      // its say, and neither silence promotes this to a dev build.
    }
  }

  return false;
}
