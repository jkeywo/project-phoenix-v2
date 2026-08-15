/**
 * scripts/deploy-headers.mjs — the deployed caching/header contract, as pure
 * functions over already-fetched responses (PRD #855, story 5).
 *
 * The network half lives in `check-deploy-headers.mjs`; everything that decides
 * whether a header is right is here, so the contract is unit-testable against
 * canned fixtures instead of against a live site.
 *
 * This is the SAME contract `src/delivery/http.rs` serves from the native host:
 * hashed bundles are immutable for a year, entry points and authored manifests
 * always revalidate, everything else is short-lived, and `.wasm` is
 * `application/wasm` because a browser's streaming instantiation refuses
 * anything else. Two implementations, one contract — which is exactly why this
 * file states it in one place and the checker only applies it.
 *
 * Cross-origin isolation is deliberately NOT required. The shipped build is
 * single-threaded (`Cargo.toml`'s wasm stanza; rapier's `parallel` is off on
 * both targets since #896), so COOP/COEP would buy nothing and would break the
 * cross-origin PeerJS and TURN fetches the page depends on. PRD #855 puts
 * worker threads behind a benchmark spike; `requireIsolation` is the switch that
 * turns this into a requirement on the day that spike says yes.
 */

/** One year — the value an immutable asset must be cached for, at minimum. */
export const IMMUTABLE_MIN_MAX_AGE = 31536000;

/**
 * How a path may be cached. Mirrors `delivery::http::CachePolicy`.
 * @typedef {'immutable'|'revalidate'|'short'} CachePolicy
 */

/**
 * Is this file name content-addressed? Mirrors
 * `delivery::http::is_hashed_asset`: trunk emits `<stem>-<hex>.<ext>` (and
 * `<stem>-<hex>_bg.wasm`) with at least 8 hex digits, and an authored name that
 * merely contains a dash must not be cached for a year.
 *
 * @param {string} fileName
 * @returns {boolean}
 */
export function isHashedAsset(fileName) {
  let stem = fileName.includes('.') ? fileName.slice(0, fileName.lastIndexOf('.')) : fileName;
  // `foo-<hash>_bg.wasm.gz` — strip the compression suffix's extension too.
  if (stem.endsWith('.wasm')) stem = stem.slice(0, -'.wasm'.length);
  if (stem.endsWith('_bg')) stem = stem.slice(0, -'_bg'.length);
  const dash = stem.lastIndexOf('-');
  if (dash < 0) return false;
  const last = stem.slice(dash + 1);
  return last.length >= 8 && /^[0-9a-fA-F]+$/.test(last);
}

/** The path's file name, or '' for a directory request. */
function fileNameOf(path) {
  const withoutQuery = path.split('?')[0];
  const last = withoutQuery.split('/').pop();
  return last || '';
}

/** The extension of a path's file name, lowercased, or null. */
function extensionOf(path) {
  const name = fileNameOf(path);
  if (!name.includes('.')) return null;
  return name.slice(name.lastIndexOf('.') + 1).toLowerCase();
}

/**
 * The caching policy a path must be served under. Mirrors
 * `delivery::http::cache_policy_for`.
 *
 * @param {string} path
 * @returns {CachePolicy}
 */
export function cachePolicyFor(path) {
  if (isHashedAsset(fileNameOf(path))) return 'immutable';
  const ext = extensionOf(path);
  if (ext === null) return 'revalidate'; // a directory request → its index.html
  if (ext === 'html' || ext === 'toml' || ext === 'csv' || ext === 'json') return 'revalidate';
  return 'short';
}

/**
 * The `Content-Type` a path must be served with, or null when the contract
 * does not care. Mirrors the entries of `delivery::http::content_type_for`
 * that actually matter over the wire.
 *
 * @param {string} path
 * @returns {string|null}
 */
export function expectedContentType(path) {
  const ext = extensionOf(path);
  if (ext === null || ext === 'html') return 'text/html';
  switch (ext) {
    case 'js':
    case 'mjs': return 'text/javascript';
    case 'wasm': return 'application/wasm';
    case 'json': return 'application/json';
    case 'css': return 'text/css';
    // deploy-demo.yml ships the WASM gzipped under the Pages 25 MiB file cap and
    // decompresses it in the page, so Pages must serve it VERBATIM as a gzip
    // body — not as a `Content-Encoding` the browser would silently unwrap,
    // which would hand DecompressionStream already-decompressed bytes.
    case 'gz': return 'application/gzip';
    default: return null;
  }
}

/** Parse `max-age=<n>` out of a Cache-Control value. */
function maxAgeOf(value) {
  const m = /(?:^|[,\s])max-age\s*=\s*(\d+)/i.exec(value || '');
  return m ? Number(m[1]) : null;
}

/** Does this Cache-Control force a revalidation on every use? */
function revalidates(value) {
  const v = (value || '').toLowerCase();
  if (v.includes('no-store') || v.includes('no-cache')) return true;
  if (v.includes('must-revalidate') && maxAgeOf(v) === 0) return true;
  return maxAgeOf(v) === 0;
}

function finding(level, path, message) {
  return { level, path, message };
}

/**
 * @typedef {{ path: string, status: number, headers: Record<string,string>,
 *             ok?: boolean }} Probe
 * A fetched response reduced to what the contract reads. Header names must
 * already be lowercased — `normaliseHeaders` does that.
 */

/**
 * Lowercase every header name of a fetched response (or a plain object).
 * @param {Headers|Record<string,string>} headers
 * @returns {Record<string,string>}
 */
export function normaliseHeaders(headers) {
  const out = {};
  if (!headers) return out;
  if (typeof headers.forEach === 'function' && typeof headers.get === 'function') {
    headers.forEach((value, name) => { out[String(name).toLowerCase()] = value; });
    return out;
  }
  for (const [name, value] of Object.entries(headers)) {
    out[String(name).toLowerCase()] = value;
  }
  return out;
}

/**
 * Check one probe against the contract.
 *
 * @param {Probe} probe
 * @param {{ requireIsolation?: boolean }} [opts]
 * @returns {{level:string, path:string, message:string}[]}
 */
export function checkProbe(probe, opts = {}) {
  const findings = [];
  const { path, status } = probe;
  const headers = probe.headers || {};

  if (status !== 200) {
    findings.push(finding('error', path, `expected 200, got ${status}`));
    // Everything below is a statement about a body that was not served.
    return findings;
  }

  const wanted = expectedContentType(path);
  const actual = (headers['content-type'] || '').toLowerCase();
  if (wanted && !actual.startsWith(wanted)) {
    findings.push(finding(
      'error',
      path,
      `Content-Type must be ${wanted}, got ${actual || '(absent)'}`,
    ));
  }

  const policy = cachePolicyFor(path);
  const cache = headers['cache-control'] || '';
  if (!cache) {
    findings.push(finding('error', path, 'no Cache-Control header at all'));
  } else if (policy === 'immutable') {
    const age = maxAgeOf(cache);
    if (age === null || age < IMMUTABLE_MIN_MAX_AGE) {
      findings.push(finding(
        'error',
        path,
        `content-addressed asset must be cached at least ${IMMUTABLE_MIN_MAX_AGE}s, ` +
        `got ${cache !== '' ? cache : '(absent)'}`,
      ));
    }
    if (!/\bimmutable\b/i.test(cache)) {
      findings.push(finding(
        'warn',
        path,
        `content-addressed asset should be marked immutable, got ${cache}`,
      ));
    }
  } else if (policy === 'revalidate') {
    if (!revalidates(cache)) {
      findings.push(finding(
        'error',
        path,
        `entry point / authored manifest must revalidate, got ${cache}`,
      ));
    }
  } else if (maxAgeOf(cache) !== null && maxAgeOf(cache) >= IMMUTABLE_MIN_MAX_AGE) {
    findings.push(finding(
      'error',
      path,
      `non-hashed asset cached for a year (${cache}) — its name does not change ` +
      'when its bytes do, so a deploy could not evict it',
    ));
  }

  if ((headers['x-content-type-options'] || '').toLowerCase() !== 'nosniff') {
    findings.push(finding('warn', path, 'X-Content-Type-Options: nosniff is not set'));
  }

  // A `.gz` asset the page decompresses itself must NOT arrive with
  // Content-Encoding: gzip — the browser would unwrap it and DecompressionStream
  // would then be handed plain WASM bytes.
  if (extensionOf(path) === 'gz' && /gzip/i.test(headers['content-encoding'] || '')) {
    findings.push(finding(
      'error',
      path,
      'served with Content-Encoding: gzip, so the browser unwraps it before the ' +
      'page can — the page decompresses this asset itself',
    ));
  }

  findings.push(...checkIsolation(probe, opts));
  return findings;
}

/**
 * Cross-origin isolation, checked in the direction the current build needs.
 * @param {Probe} probe
 * @param {{ requireIsolation?: boolean }} opts
 */
function checkIsolation(probe, opts) {
  // Only the entry points can be isolated; an asset's own COOP is meaningless.
  if (cachePolicyFor(probe.path) !== 'revalidate' || expectedContentType(probe.path) !== 'text/html') {
    return [];
  }
  const headers = probe.headers || {};
  const coop = (headers['cross-origin-opener-policy'] || '').toLowerCase();
  const coep = (headers['cross-origin-embedder-policy'] || '').toLowerCase();

  if (opts.requireIsolation) {
    const out = [];
    if (coop !== 'same-origin') {
      out.push(finding('error', probe.path, `Cross-Origin-Opener-Policy must be same-origin, got ${coop || '(absent)'}`));
    }
    if (coep !== 'require-corp' && coep !== 'credentialless') {
      out.push(finding('error', probe.path, `Cross-Origin-Embedder-Policy must be require-corp or credentialless, got ${coep || '(absent)'}`));
    }
    return out;
  }

  // Not required — but half-applied isolation is worse than none: COEP without
  // COOP does not isolate anything and does block every cross-origin
  // subresource, which on this page is PeerJS and the TURN credential worker.
  if (coep && coop !== 'same-origin') {
    return [finding(
      'error',
      probe.path,
      `Cross-Origin-Embedder-Policy is set (${coep}) without Cross-Origin-Opener-Policy: ` +
      'same-origin — that isolates nothing and blocks the cross-origin PeerJS and ' +
      'TURN requests the page needs',
    )];
  }
  return [];
}

/**
 * Check every probe and roll the findings up.
 *
 * @param {Probe[]} probes
 * @param {{ requireIsolation?: boolean }} [opts]
 * @returns {{ findings: {level:string,path:string,message:string}[],
 *             errors: number, warnings: number, ok: boolean }}
 */
export function checkAll(probes, opts = {}) {
  const findings = [];
  for (const probe of probes) findings.push(...checkProbe(probe, opts));
  const errors = findings.filter((f) => f.level === 'error').length;
  const warnings = findings.filter((f) => f.level === 'warn').length;
  return { findings, errors, warnings, ok: errors === 0 };
}

/**
 * The bundle paths a deployed `index.html` references, so the checker probes
 * the real content-addressed names instead of guessing them.
 *
 * Reads the wasm-bindgen loader's `module_or_path` and the trunk `<link
 * rel="preload">`/`<script type="module">` targets. Returns absolute site
 * paths, deduplicated, in document order.
 *
 * @param {string} html
 * @param {string} [basePath] — the directory the HTML was served from, e.g. '/'
 * @returns {string[]}
 */
export function bundlePathsFromHtml(html, basePath = '/') {
  const found = [];
  const push = (raw) => {
    if (!raw) return;
    const cleaned = raw.replace(/^\.\//, '');
    const abs = cleaned.startsWith('/') ? cleaned : basePath.replace(/\/?$/, '/') + cleaned;
    if (!found.includes(abs)) found.push(abs);
  };
  // `./project-phoenix-<hash>_bg.wasm`, `.wasm.gz`, and `./project-phoenix-<hash>.js`
  const asset = /['"]([^'"\s]*?[-_][0-9a-fA-F]{8,}(?:_bg)?\.(?:js|wasm|wasm\.gz))['"]/g;
  for (let m = asset.exec(html); m; m = asset.exec(html)) push(m[1]);
  return found;
}
