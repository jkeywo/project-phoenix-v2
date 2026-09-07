/**
 * gui/host-catalog-templates.js — deliver the catalogue's hull templates to
 * Rust BEFORE the picker's ship cards are drawn.
 *
 * `wasm_get_scenario_catalog()` publishes each `[[available_ships]]` entry
 * through `delivery::payload::ship_payload`, whose `class`, `hull_id`, `mass`,
 * `power_rating` and `name` are enrichment read from the CACHED entity
 * template. `server.html`'s `buildScenarioCatalog()` pushed the scenario
 * manifest and every referenced World TOML and then read the catalogue
 * immediately, so that cache was empty for every hull: each card fell back to
 * `template_path` + `label`, badged `component.ship_picker.class.unknown` and
 * carried no registry, mass or power rating. The hull templates were fetched
 * only later, in `preloadAndStart`, long after the picker had been drawn.
 *
 * The decisions in that fix — which paths to fetch, in what order, how many
 * rounds an include closure is allowed, and what a failure means — live here
 * rather than in `server.html` so vitest can reach them; `server.html` keeps
 * the `fetch` and the `wasmBindings` calls, injected as the two callbacks.
 *
 * Two rules this module exists to hold:
 *
 * 1. **Concurrency.** Every path in a round is fetched at once, exactly like
 *    the World-TOML fetch beside it. The picker is on the critical path to
 *    first interaction and a serial walk over four hulls plus their fragments
 *    would be felt.
 * 2. **A failure is never fatal.** A hull that 404s, will not parse, or whose
 *    include fragments never arrive leaves that ONE card label-only — which is
 *    what every card looked like before this existed. It never fails the
 *    catalogue, and it never rejects.
 * 3. **A failed fetch is still a delivery.** The path is pushed with an empty
 *    body, exactly as `handleConfigRequest` pushes `wasm_load_config(path, '')`
 *    on a 404, so Rust can substitute a mod pack's own copy from the session
 *    overlay — the only place a PACK hull's text ever lives, since it has no
 *    URL to fetch. Without that, the mod-pack half of this module would be
 *    dead code.
 */

/**
 * How many delivery rounds an include closure gets.
 *
 * Round 1 is the hulls themselves; each further round is one level deeper into
 * `includes`. Shipped hulls are one level deep (`fragments/ai/*.toml`), so two
 * rounds is the live case and the rest is headroom. The cap is what stops a
 * pathological or hostile chain walking for ever on the critical path — the
 * resolver's own cycle detection refuses a true cycle, but a very deep legal
 * chain would otherwise keep the picker waiting.
 */
export const MAX_INCLUDE_ROUNDS = 8;

/**
 * Every distinct hull template path a catalogue references, in first-seen
 * order.
 *
 * Read out of the catalogue's own first pass rather than by re-parsing the
 * World TOMLs in JS: the paths come from the worlds' `[[available_ships]]`
 * entries, and `build_merged_catalog` has already resolved those — including
 * for a mod-supplied World, whose own text lives in the session overlay and
 * never came from a URL at all. Only the PATH is settled here; the hull's text
 * is fetched (or, for a pack's own hull, taken from the overlay) by
 * {@link deliverCatalogTemplates}.
 *
 * @param {Iterable<object>} catalog rows from `wasm_get_scenario_catalog()`
 * @returns {string[]} distinct `template_path` values
 */
export function catalogTemplatePaths(catalog) {
  const out = [];
  const seen = new Set();
  for (const entry of Array.from(catalog || [])) {
    const ships = (entry && entry.ships) || [];
    for (const ship of Array.from(ships)) {
      const path = ship && ship.template_path;
      if (typeof path !== 'string' || path === '' || seen.has(path)) continue;
      seen.add(path);
      out.push(path);
    }
  }
  return out;
}

/**
 * Fetch and deliver `paths` and their include closure, round by round.
 *
 * `pushTemplate(path, toml, isRoot)` returns the fragment paths still missing
 * across every root delivered so far; those become the next round. A path is
 * fetched at most once, and a round ends only when all of its fetches have
 * settled, so each round is one wave of concurrent requests.
 *
 * Never rejects. `pushTemplate` is allowed to throw for one path — a template
 * Rust refuses — and that path is simply dropped.
 *
 * A path whose FETCH brought nothing is still delivered, as the empty string.
 * That is the contract `server.html`'s own `handleConfigRequest` keeps with
 * `wasm_load_config`, and it is the whole reason a mod pack's content works: a
 * pack's own hull lives in the session overlay and has NO URL, so its fetch
 * 404s every time and the overlay is the only place its text has ever been.
 * Rust consults the overlay first and treats an empty delivery with no overlay
 * copy as a no-op, so a genuinely missing fragment stays "still to fetch"
 * rather than composing as present-and-empty.
 *
 * @param {object} opts
 * @param {string[]} opts.paths root template paths (from `catalogTemplatePaths`)
 * @param {(path: string) => Promise<string>} opts.fetchText fetch one path's text
 * @param {(path: string, toml: string, isRoot: boolean) => Iterable<string>} opts.pushTemplate
 *   deliver one template, returning the fragment paths still needed
 * @param {number} [opts.maxRounds] override for {@link MAX_INCLUDE_ROUNDS}
 * @returns {Promise<{delivered: string[], unfetched: string[], failed: string[],
 *   rounds: number, truncated: boolean}>} a report for the host page's console
 *   line: `delivered` came off the wire, `unfetched` was offered to the overlay
 *   instead (normal for a pack hull, a 404 for a shipped one), and `failed` is
 *   what Rust refused outright.
 */
export async function deliverCatalogTemplates(opts) {
  const options = opts || {};
  const fetchText = options.fetchText;
  const pushTemplate = options.pushTemplate;
  const maxRounds = options.maxRounds != null ? options.maxRounds : MAX_INCLUDE_ROUNDS;
  const delivered = [];
  const unfetched = [];
  const failed = [];
  const attempted = new Set();
  let round = Array.from(new Set(options.paths || [])).filter((p) => typeof p === 'string' && p !== '');
  let isRoot = true;
  let rounds = 0;

  while (round.length > 0 && rounds < maxRounds) {
    rounds += 1;
    const rootRound = isRoot;
    const wanted = [];
    await Promise.all(round.map(async function deliverOne(path) {
      attempted.add(path);
      let text;
      try {
        text = await fetchText(path);
      } catch (_) {
        text = null;
      }
      // `null` and a non-string alike mean "nothing came off the wire", which
      // is NOT the end of this path: the overlay may carry it, and only Rust
      // can answer that.
      const fetched = typeof text === 'string';
      let missing;
      try {
        missing = pushTemplate(path, fetched ? text : '', rootRound);
      } catch (_) {
        failed.push(path);
        return;
      }
      (fetched ? delivered : unfetched).push(path);
      for (const p of Array.from(missing || [])) {
        if (typeof p === 'string' && p !== '') wanted.push(p);
      }
    }));
    isRoot = false;
    round = Array.from(new Set(wanted)).filter((p) => !attempted.has(p));
  }

  return { delivered, unfetched, failed, rounds, truncated: round.length > 0 };
}

// Expose for the classic-script consumer (server.html is not a module).
if (typeof window !== 'undefined') {
  window.hostCatalogTemplates = { catalogTemplatePaths, deliverCatalogTemplates, MAX_INCLUDE_ROUNDS };
}
