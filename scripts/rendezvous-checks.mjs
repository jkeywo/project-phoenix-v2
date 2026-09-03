/**
 * scripts/rendezvous-checks.mjs — the §3/§3a curl contract, as code
 * (issue #1113).
 *
 * `docs/delivery-checklist.md` asks an operator to run two `curl` commands
 * after every worker deploy and to read the answers carefully. That is the
 * mitigation for the sharpest operational trap this repository has: **a Worker
 * only picks up `[vars]` on `wrangler deploy`**, so the repo file can be
 * perfectly correct while the live edge silently disagrees, with nothing to
 * notice. It cost a whole field session on 2026-08-15.
 *
 * A recipe a human reads is only as good as the human's attention at the end of
 * a deploy. This module is the same judgements written down so a machine makes
 * them, and `scripts/check-rendezvous.mjs` is the thing that runs it against a
 * live URL.
 *
 * Pure and network-free by design — the same three-way split
 * `deploy-headers.mjs` / `check-deploy-headers.mjs` /
 * `tests/client/deploy-headers.test.js` already uses — so every rule here is
 * covered by fixtures in `tests/client/rendezvous-checks.test.js` and a live
 * run needs no test infrastructure at all.
 *
 * ## What "a finding" means
 *
 *   error    the deployment is broken for players. A field session run against
 *            it will fail, and probably fail invisibly.
 *   warn     degraded but usable, or something an operator should look at.
 */

import { RENDEZVOUS_PROTOCOL } from '../gui/rendezvous-protocol.js';

const error = (id, detail) => ({ level: 'error', id, detail });
const warn = (id, detail) => ({ level: 'warn', id, detail });

/** Lower-case every header name, so a check never depends on the casing. */
export function normaliseHeaders(headers) {
  const out = {};
  if (!headers) return out;
  const entries = typeof headers.entries === 'function'
    ? [...headers.entries()]
    : Object.entries(headers);
  for (const [k, v] of entries) out[String(k).toLowerCase()] = String(v);
  return out;
}

/**
 * The rendezvous service's `/v1/health`, checked against what the origin the
 * probe CLAIMED to come from should have got back.
 *
 * @param {object} probe
 * @param {string} probe.url the URL that was fetched
 * @param {number} probe.status
 * @param {object} probe.headers
 * @param {object|null} probe.body parsed JSON, or null when it would not parse
 * @param {string} probe.origin the Origin header that was sent
 * @param {number} [probe.formatVersion] the committed join-code table's version
 */
export function checkRendezvousHealth(probe) {
  const findings = [];
  const headers = normaliseHeaders(probe.headers);

  // TLS. Not a nicety: the page is https, so a ws:// service would be blocked
  // as mixed content before a single frame crossed, and nothing on screen would
  // say why.
  if (!String(probe.url || '').startsWith('https://')) {
    findings.push(error('tls', `${probe.url} is not https — a secure page cannot open a ws:// socket`));
  }

  if (probe.status !== 200) {
    findings.push(error('health-status', `/v1/health answered ${probe.status}, not 200`));
    return findings;
  }
  if (!probe.body || probe.body.ok !== true) {
    findings.push(error('health-body', 'the service did not answer {"ok":true}'));
    return findings;
  }

  // THE trap, caught before a player meets it. A stale ALLOWED_ORIGIN refuses
  // every upgrade, and unlike the TURN worker's version of the same fault
  // (which merely stripped relay) this one means nobody can join at all.
  if (probe.body.origin_allowed !== true) {
    findings.push(error(
      'origin-not-allowed',
      `the deployed worker does not allow ${probe.origin} — every upgrade from it is refused, `
      + 'so nobody can join. Update ALLOWED_ORIGIN and REDEPLOY the worker: '
      + 'a worker only picks up [vars] on `wrangler deploy`',
    ));
  }
  if (probe.body.origin && probe.body.origin !== probe.origin) {
    findings.push(warn('origin-echo', `the service echoed ${probe.body.origin}, not ${probe.origin}`));
  }

  // A frame with a `v` neither end recognises is hard-refused, so a service
  // deployed from another checkout is a total outage rather than a degradation.
  if (probe.body.protocol !== RENDEZVOUS_PROTOCOL) {
    findings.push(error(
      'protocol',
      `the deployed service speaks rendezvous v${probe.body.protocol} and this checkout speaks `
      + `v${RENDEZVOUS_PROTOCOL} — every frame between them is refused`,
    ));
  }
  if (probe.formatVersion !== undefined && probe.body.format_version !== probe.formatVersion) {
    findings.push(error(
      'format-version',
      `the deployed service bundles join-code table v${probe.body.format_version}; this checkout `
      + `has v${probe.formatVersion}`,
    ));
  }

  // Time-limited liveness must never be cached, or a health check answers for
  // a deploy that has since been replaced.
  if (headers['cache-control'] !== 'no-store') {
    findings.push(warn('health-cache', `Cache-Control is ${headers['cache-control'] || '(absent)'}, not no-store`));
  }
  return findings;
}

/**
 * The other half of the origin check: a bogus origin must be REFUSED.
 *
 * An allowlist that says yes to everything passes the check above while being
 * exactly as broken as one that says no to everything — `ALLOWED_ORIGIN = "*"`
 * is a plausible way to "fix" a CORS problem under time pressure, and it turns
 * an unauthenticated public endpoint into one anybody's page can register
 * against.
 */
export function checkOriginIsRefused(probe) {
  const findings = [];
  if (probe.status !== 200 || !probe.body) {
    findings.push(warn('control-unreachable', 'could not ask about an unknown origin'));
    return findings;
  }
  if (probe.body.origin_allowed === true) {
    findings.push(error(
      'origin-allowlist-open',
      `the deployed worker allows ${probe.origin}, which is not one of ours — ALLOWED_ORIGIN is `
      + 'probably "*", which lets anybody\'s page register a host against this service',
    ));
  }
  return findings;
}

/** A WebSocket endpoint must refuse a plain GET rather than answering one. */
export function checkUpgradeRequired(probe) {
  if (probe.status === 426) return [];
  return [error(
    'upgrade',
    `${probe.path} answered ${probe.status} to a plain GET; the endpoint should demand a `
    + 'WebSocket upgrade (426)',
  )];
}

/**
 * The TURN credential worker — §3's curl contract, with the same three
 * judgements the checklist asks a human to make.
 */
export function checkTurnWorker(probe) {
  const findings = [];
  const headers = normaliseHeaders(probe.headers);

  if (!String(probe.url || '').startsWith('https://')) {
    findings.push(error('turn-tls', `${probe.url} is not https`));
  }
  if (probe.status === 502) {
    findings.push(error('turn-sources', 'every credential source failed (502) — there is no relay at all'));
    return findings;
  }
  if (probe.status !== 200) {
    findings.push(error('turn-status', `the credential worker answered ${probe.status}, not 200`));
    return findings;
  }

  // The 2026-08 failure, exactly: a stale allowlist echoes SOMETHING (the first
  // entry) rather than the origin asked, the browser blocks the fetch, and every
  // client silently drops to the free shared fallback.
  const allow = headers['access-control-allow-origin'];
  if (allow !== probe.origin) {
    findings.push(error(
      'turn-cors',
      `Access-Control-Allow-Origin is ${allow || '(absent)'}, not the ${probe.origin} that was `
      + 'sent — every browser fetch from that origin is CORS-blocked and every player silently '
      + 'falls back to the free shared relay',
    ));
  }
  if (headers['x-turn-source-errors']) {
    findings.push(warn(
      'turn-source-degraded',
      `one credential source is down (${headers['x-turn-source-errors']}) and the other is `
      + 'carrying it — worth fixing before it is the only one',
    ));
  }
  if (headers['cache-control'] !== 'no-store') {
    findings.push(warn('turn-cache', 'time-limited credentials should be Cache-Control: no-store'));
  }

  const servers = Array.isArray(probe.body) ? probe.body : [];
  if (servers.length === 0) {
    findings.push(error('turn-empty', 'the worker returned no ICE servers'));
    return findings;
  }
  const relays = servers.filter((s) => {
    const urls = Array.isArray(s?.urls) ? s.urls : [s?.urls];
    return urls.some((u) => typeof u === 'string' && (u.startsWith('turn:') || u.startsWith('turns:')));
  });
  if (relays.length === 0) {
    findings.push(error(
      'turn-no-relay',
      'the worker answered 200 but with no TURN entry — players on mobile networks cannot connect',
    ));
  }
  return findings;
}

/** Fold every finding into one exit-worthy verdict. */
export function summarise(findings) {
  const errors = findings.filter((f) => f.level === 'error');
  const warnings = findings.filter((f) => f.level === 'warn');
  return { ok: errors.length === 0, errors, warnings };
}
