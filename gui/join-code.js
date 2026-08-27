/**
 * gui/join-code.js — Typed human-readable join identifiers (issue #1111).
 *
 * A Phoenix join identifier is `PROJECT_GUID_VERSION_GUID_CODE`:
 *
 *     2f6b0a11-9c4e-4d7a-8f31-5b90c2d47e18_5c0a3e91-…-9e61af07c25b_QUART
 *     └── project (which KIND of join) ──┘ └── compatible release ──┘ └ typed
 *
 * Normal entry types only the five-letter suffix; the page supplies its own
 * project and version GUIDs from local context. The full form exists for
 * pasting, QR fragments, diagnostics and launchers.
 *
 * Everything here is pure — no DOM, no network, no transport — so the same
 * functions run in the phone, in the host page, in the Cloudflare rendezvous
 * Worker (which imports this file directly) and under vitest. The format
 * itself is NOT written here: it is authored in
 * `assets/join/join-codes.toml` and read from the JSON generated beside it
 * (`assets/join/join-codes.json` — see scripts/join-codes.mjs), passed in as
 * `data` or installed once with `setJoinCodeData()`.
 *
 * Every failure is reported as a stable machine reason string. The map from
 * reason to `assets/strings/strings.csv` id lives at the bottom of this file,
 * so the phone and the host page cannot describe the same failure differently.
 */

/**
 * Wire/format revision this module implements. Checked against the authored
 * table's own `format_version` whenever one is installed or loaded — an
 * unchecked constant is a comment wearing a keyword.
 */
export const JOIN_CODE_FORMAT_VERSION = 1;

/** The two typed namespaces. `server` exists so wrong-type is answerable. */
export const NAMESPACE_CLIENT = 'client';
export const NAMESPACE_SERVER = 'server';

/** Separator between the two GUIDs and the suffix. GUIDs never contain `_`. */
const PART_SEPARATOR = '_';

// ── The authored format table ───────────────────────────────────────────────
// Mirrors gui/strings.js's setTable/getTable: a module-level table that page
// code installs once, while every function still accepts an explicit table so
// tests and the Worker never depend on global state.

let installed = null;

/**
 * Refuse a table this module does not implement, rather than reading fields
 * that may have moved. Throws; there is no partial answer worth giving from a
 * format revision we do not know.
 *
 * @param {object} data parsed authored table
 * @returns {object} the same table, when it is one we speak
 */
export function checkJoinCodeFormat(data) {
  const found = data && data.format_version;
  if (found !== JOIN_CODE_FORMAT_VERSION) {
    throw new Error(
      `join-code: format_version ${found} is not the ${JOIN_CODE_FORMAT_VERSION} this build reads`,
    );
  }
  return data;
}

/** Install the authored format table (parsed assets/join/join-codes.json). */
export function setJoinCodeData(next) {
  installed = next ? checkJoinCodeFormat(next) : null;
}

/** The installed format table, or null if nothing has been installed yet. */
export function getJoinCodeData() {
  return installed;
}

function table(data) {
  const d = data || installed;
  if (!d) throw new Error('join-code: no format data installed (setJoinCodeData)');
  return d;
}

/**
 * Fetch and install the authored table. Impure by necessity; kept here rather
 * than in a boot module because nothing renders during module evaluation from
 * a join code, so an ordinary async load is safe (unlike gui/strings-boot.js,
 * which must be synchronous).
 *
 * @param {string|URL} [url] defaults to assets/join/join-codes.json beside gui/
 * @returns {Promise<object>} the installed table
 */
export async function loadJoinCodeData(url) {
  const target = url || new URL('../assets/join/join-codes.json', import.meta.url);
  const res = await fetch(target);
  if (!res.ok) throw new Error(`join-code: HTTP ${res.status} loading ${target}`);
  const data = await res.json();
  // Throws on a revision this build does not implement; the page reports the
  // failure as "cannot reach the join service" rather than joining on a table
  // it half understands.
  setJoinCodeData(data);
  return data;
}

// ── Canonicalisation ────────────────────────────────────────────────────────

/**
 * Fold typed input to the one spelling the registry stores.
 *
 * Uppercases, drops the authored `strip` characters (spaces, dashes and
 * underscores a player adds while reading a code aloud), then applies the
 * authored confusable map — `0`→`O`, `1`→`I`, `L`→`I`, with `J` deliberately
 * left alone. Characters outside the alphabet survive unchanged so the caller
 * can tell "you typed a digit" apart from "you typed too few letters"; use
 * {@link validateSuffix} for the judgement.
 *
 * @param {string} raw
 * @param {object} [data]
 * @returns {string}
 */
export function canonicaliseSuffix(raw, data) {
  const { suffix } = table(data);
  const strip = new Set((suffix.strip || '').split(''));
  const map = suffix.normalise || {};
  let out = '';
  for (const ch of String(raw == null ? '' : raw).toUpperCase()) {
    if (strip.has(ch)) continue;
    out += Object.prototype.hasOwnProperty.call(map, ch) ? map[ch] : ch;
  }
  return out;
}

/** The canonicalised deny-list, so `HELLO`, `HEIIO` and `he110` all refuse. */
export function deniedSuffixes(data) {
  const d = table(data);
  const out = new Set();
  for (const entry of d.deny || []) {
    const word = typeof entry === 'string' ? entry : entry && entry.word;
    if (!word) continue;
    out.add(canonicaliseSuffix(word, d));
  }
  return out;
}

/** True when a canonical suffix is on the authored deny-list. */
export function isDenied(suffix, data) {
  return deniedSuffixes(data).has(canonicaliseSuffix(suffix, data));
}

/**
 * Validate typed suffix input.
 *
 * @returns {{ok: true, suffix: string} | {ok: false, reason: string, suffix: string}}
 *   reason ∈ 'empty' | 'length' | 'charset' | 'denied'
 */
export function validateSuffix(raw, data) {
  const d = table(data);
  const suffix = canonicaliseSuffix(raw, d);
  if (suffix.length === 0) return { ok: false, reason: 'empty', suffix };
  if (suffix.length !== d.suffix.length) return { ok: false, reason: 'length', suffix };
  const alphabet = new Set(d.suffix.alphabet.split(''));
  for (const ch of suffix) {
    if (!alphabet.has(ch)) return { ok: false, reason: 'charset', suffix };
  }
  if (deniedSuffixes(d).has(suffix)) return { ok: false, reason: 'denied', suffix };
  return { ok: true, suffix };
}

// ── Namespaces ──────────────────────────────────────────────────────────────

/** Project GUID for a namespace name, or null. */
export function projectGuidFor(namespace, data) {
  const d = table(data);
  return (d.namespaces && d.namespaces[namespace]) || null;
}

/** Namespace name for a project GUID, or null when it belongs to no namespace. */
export function namespaceOf(projectGuid, data) {
  const d = table(data);
  const guid = String(projectGuid || '').toLowerCase();
  for (const name of [NAMESPACE_CLIENT, NAMESPACE_SERVER]) {
    const known = d.namespaces && d.namespaces[name];
    if (known && known.toLowerCase() === guid) return name;
  }
  return null;
}

/** This build's compatible-release version GUID. */
export function versionGuid(data) {
  const d = table(data);
  return (d.version && d.version.guid) || null;
}

// ── Composing and parsing the full form ─────────────────────────────────────

/** `PROJECT_VERSION_SUFFIX` from its three parts. */
export function composeJoinCode({ project, version, suffix }) {
  return [project, version, suffix].join(PART_SEPARATOR);
}

/**
 * The full code for a suffix typed into a namespace, using this build's own
 * project/version context. Returns `{ok:false, reason}` on a bad suffix so a
 * caller never composes a code around input it should have refused.
 */
export function joinCodeForSuffix(raw, namespace, data) {
  const d = table(data);
  const checked = validateSuffix(raw, d);
  if (!checked.ok) return checked;
  const project = projectGuidFor(namespace, d);
  if (!project) return { ok: false, reason: 'unknown-namespace', suffix: checked.suffix };
  return {
    ok: true,
    suffix: checked.suffix,
    namespace,
    project,
    version: versionGuid(d),
    full: composeJoinCode({ project, version: versionGuid(d), suffix: checked.suffix }),
  };
}

/**
 * Is this part of a split input one of a full code's two GUID heads, rather
 * than a piece of a five-letter suffix a player punctuated?
 *
 * Deliberately looser than a GUID regex — hosts and tests do register
 * identifier-shaped versions that are not canonical GUIDs — but long enough
 * that nothing a five-letter code can be broken into ever matches it.
 */
const CODE_HEAD = /^[0-9a-z][0-9a-z-]{7,}$/i;

/**
 * Parse a pasted/QR'd full code, or a bare suffix.
 *
 * The SHAPE is decided before any canonicalisation, because `_` is both the
 * part separator and an authored strip character: `QU_ARK` is a player spacing
 * out five letters, while `<guid>_<guid>_QUARK` is a pasted identifier, and
 * splitting first would report the former as malformed. So a three-part string
 * with two identifier-shaped heads is taken at its word — including a project
 * GUID this build does not recognise (reported as `unknown-project`, never
 * silently retyped) — and everything else goes down the suffix path, where
 * canonicalisation drops the punctuation. An input that is *shaped* like a
 * paste but does not have three parts is `malformed`: it was not typed.
 *
 * `fallbackNamespace` is the namespace the page supplies for suffix input.
 *
 * @returns {{ok:true, project, version, suffix, namespace, full, typed:'full'|'suffix'}
 *          |{ok:false, reason:string}}
 */
export function parseJoinCode(raw, fallbackNamespace, data) {
  const d = table(data);
  const text = String(raw == null ? '' : raw).trim();
  if (!text) return { ok: false, reason: 'empty' };

  // A URL (QR entry) carries the code in its fragment.
  const fragment = text.includes('#') ? text.slice(text.indexOf('#') + 1) : text;
  // Trimmed per part: a code read off a screen and retyped with spaces around
  // the separators is still a paste of that code.
  const parts = fragment.split(PART_SEPARATOR).map((part) => part.trim());
  const structured = parts.length === 3 && CODE_HEAD.test(parts[0]) && CODE_HEAD.test(parts[1]);

  if (!structured) {
    if (parts.length > 1 && parts.some((part) => CODE_HEAD.test(part))) {
      return { ok: false, reason: 'malformed' };
    }
    const composed = joinCodeForSuffix(fragment, fallbackNamespace, d);
    return composed.ok ? { ...composed, typed: 'suffix' } : composed;
  }

  const [project, version, rawSuffix] = parts;
  const checked = validateSuffix(rawSuffix, d);
  if (!checked.ok) return checked;
  const namespace = namespaceOf(project, d);
  if (!namespace) return { ok: false, reason: 'unknown-project', suffix: checked.suffix };
  return {
    ok: true,
    typed: 'full',
    project,
    version,
    suffix: checked.suffix,
    namespace,
    full: composeJoinCode({ project, version, suffix: checked.suffix }),
  };
}

// ── Minting ─────────────────────────────────────────────────────────────────

/**
 * Draw a fresh suffix that is neither denied nor already taken in its
 * namespace. `randomInt(n)` returns an integer in `[0, n)`; `isTaken(suffix)`
 * is the caller's collision check — the registry's, in the rendezvous service.
 *
 * Returns `{ok:false, reason:'exhausted'}` rather than looping forever, so a
 * saturated namespace surfaces as a refusal instead of a hung host.
 */
export function mintSuffix(data, isTaken, randomInt, maxAttempts = 64) {
  const d = table(data);
  const alphabet = d.suffix.alphabet;
  const denied = deniedSuffixes(d);
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    let suffix = '';
    for (let i = 0; i < d.suffix.length; i += 1) {
      suffix += alphabet[randomInt(alphabet.length)];
    }
    if (denied.has(suffix)) continue;
    if (isTaken && isTaken(suffix)) continue;
    return { ok: true, suffix };
  }
  return { ok: false, reason: 'exhausted' };
}

// ── Reason → display string id ──────────────────────────────────────────────

/**
 * The single map from a machine reason to a `strings.csv` id, shared by the
 * phone entry field and the host page so one failure never gets two wordings.
 *
 * Three sources feed it, and all three must be covered or a real refusal
 * renders as the misleading `unknown` fallback:
 *
 *   1. this module — `validateSuffix`, `parseJoinCode`;
 *   2. the rendezvous service's `error` frames (worker-rendezvous/src/registry.js);
 *   3. the HOST's authoritative compatibility verdict, whose codes are Rust's
 *      `StampMismatch::code()` (src/delivery/stamp.rs) relayed verbatim through
 *      `JoinRefused`. tests/client/join-code.test.js pins that list, so adding
 *      a variant there without a row here fails the editor-test job.
 */
const REASON_STRING_IDS = {
  empty: 'client.join.error_empty',
  length: 'client.join.error_length',
  charset: 'client.join.error_charset',
  denied: 'client.join.error_denied',
  malformed: 'client.join.error_malformed',
  // A GUID belonging to no Phoenix namespace at all is NOT "the other typed
  // namespace" — saying "that is a fleet code" about it would be a false
  // statement about somebody else's identifier.
  'unknown-project': 'client.join.error_not_phoenix',
  'unknown-namespace': 'client.join.error_not_phoenix',
  unknown: 'client.join.error_unknown',
  'wrong-type': 'client.join.error_wrong_type',
  'version-mismatch': 'client.join.error_version',
  'not-joinable': 'client.join.error_wrong_type',
  'admission-closed': 'client.join.error_closed',
  'host-gone': 'client.join.error_host_gone',
  exhausted: 'client.join.error_exhausted',
  unreachable: 'client.join.error_unreachable',
  'too-many-attempts': 'client.join.error_too_many',
  'unsupported-protocol': 'client.join.error_version',
  // ── The host's own verdict (StampMismatch::code()) ────────────────────────
  // A protocol difference and a content difference have the same fix for the
  // player — reload this page against the ship they are joining — but they are
  // two different sentences, because "different version" is a lie when the
  // build is right and the mission content is not.
  'protocol-mismatch': 'client.join.error_version',
  'content-id-mismatch': 'client.join.error_content',
  'content-epoch-mismatch': 'client.join.error_content',
  'bundle-content-missing': 'client.join.error_content',
  'client-stamp-missing': 'client.join.error_stamp_missing',
};

/** String id for a machine reason; a reason with no row falls back to unknown. */
export function reasonStringId(reason) {
  return REASON_STRING_IDS[reason] || REASON_STRING_IDS.unknown;
}

/** Every reason this build can put on screen. Read by the coverage tests. */
export function knownReasons() {
  return Object.keys(REASON_STRING_IDS);
}

// Expose for classic-script consumers (client.html / server.html are not modules).
if (typeof window !== 'undefined') {
  window.joinCode = {
    JOIN_CODE_FORMAT_VERSION,
    NAMESPACE_CLIENT,
    NAMESPACE_SERVER,
    checkJoinCodeFormat,
    setJoinCodeData,
    getJoinCodeData,
    loadJoinCodeData,
    canonicaliseSuffix,
    deniedSuffixes,
    isDenied,
    validateSuffix,
    projectGuidFor,
    namespaceOf,
    versionGuid,
    composeJoinCode,
    joinCodeForSuffix,
    parseJoinCode,
    mintSuffix,
    reasonStringId,
    knownReasons,
  };
}
