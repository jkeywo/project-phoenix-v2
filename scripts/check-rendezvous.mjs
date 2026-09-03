#!/usr/bin/env node
/**
 * scripts/check-rendezvous.mjs — run the §3/§3a deploy contract against LIVE
 * workers (issue #1113).
 *
 *   node scripts/check-rendezvous.mjs \
 *     --rendezvous https://phoenix-rendezvous.project-phoenix.workers.dev \
 *     --turn https://phoenix-turn-credentials.project-phoenix.workers.dev \
 *     --origin https://pp-dev.kiwigamedesign.co.uk
 *
 * Exits 0 when both contracts hold (warnings do not fail), 1 on any error
 * finding, 2 on a usage or network problem — the same three exits
 * `check-deploy-headers.mjs` uses, for the same reason.
 *
 * Dependency-free (Node 20's global `fetch`, nothing installed) so it can be
 * run from a laptop in the middle of a deploy session, which is the only moment
 * it matters. Deliberately NOT a push gate: it talks to live origins, and as a
 * blocking step it would turn someone else's uptime into a red branch.
 *
 * Every judgement lives in `scripts/rendezvous-checks.mjs` and is unit-tested
 * over fixtures; this file only decides what to fetch and how to print.
 */

import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  checkOriginIsRefused,
  checkRendezvousHealth,
  checkTurnWorker,
  checkUpgradeRequired,
  summarise,
} from './rendezvous-checks.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

/**
 * An origin no deployment of ours should ever allow. Used as the control probe:
 * an allowlist that says yes to this says yes to everybody.
 */
const CONTROL_ORIGIN = 'https://not-a-phoenix-origin.invalid';

function usage(message) {
  process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    'usage: node scripts/check-rendezvous.mjs --origin <site-origin>\n'
    + '         [--rendezvous <worker-url>] [--turn <worker-url>] [--json]\n\n'
    + '  --origin      the site origin the workers must allow, e.g.\n'
    + '                https://pp-dev.kiwigamedesign.co.uk\n'
    + '  --rendezvous  the rendezvous worker (docs/delivery-checklist.md §3a)\n'
    + '  --turn        the TURN credential worker (§3)\n'
    + '  At least one of --rendezvous / --turn is required.\n',
  );
  process.exit(2);
}

function parseArgs(argv) {
  const out = { json: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const value = () => {
      const v = argv[i + 1];
      if (v === undefined || v.startsWith('--')) usage(`${arg} needs a value`);
      i += 1;
      return v;
    };
    if (arg === '--json') out.json = true;
    else if (arg === '--origin') out.origin = value();
    else if (arg === '--rendezvous') out.rendezvous = value();
    else if (arg === '--turn') out.turn = value();
    else usage(`unknown argument ${arg}`);
  }
  return out;
}

const join = (base, p) =>
  new URL(p.replace(/^\//, ''), base.endsWith('/') ? base : `${base}/`).toString();

/** GET one URL with an Origin, reducing it to the shape the checks read. */
async function probe(url, origin, { json = true } = {}) {
  let response;
  try {
    response = await fetch(url, { headers: { Origin: origin }, redirect: 'follow' });
  } catch (e) {
    process.stderr.write(`cannot reach ${url}: ${e.message}\n`);
    process.exit(2);
  }
  let body = null;
  if (json) {
    try {
      body = await response.json();
    } catch {
      body = null;
    }
  }
  return { url, origin, status: response.status, headers: response.headers, body };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!args.origin) usage('--origin is required');
  if (!args.rendezvous && !args.turn) usage('give at least one of --rendezvous / --turn');

  const findings = [];

  if (args.rendezvous) {
    // The committed join-code table's version, so a service bundled from
    // another checkout is caught rather than merely suspected.
    const table = JSON.parse(
      await readFile(path.join(root, 'assets/join/join-codes.json'), 'utf8'),
    );
    const health = await probe(join(args.rendezvous, '/v1/health'), args.origin);
    findings.push(...checkRendezvousHealth({ ...health, formatVersion: table.format_version }));

    // The control: an origin nothing of ours should allow.
    const control = await probe(join(args.rendezvous, '/v1/health'), CONTROL_ORIGIN);
    findings.push(...checkOriginIsRefused(control));

    // Both socket endpoints must demand an upgrade rather than answering a GET.
    for (const endpoint of ['/v1/host', '/v1/join']) {
      const plain = await probe(join(args.rendezvous, endpoint), args.origin);
      findings.push(...checkUpgradeRequired({ ...plain, path: endpoint }));
    }
  }

  if (args.turn) {
    findings.push(...checkTurnWorker(await probe(args.turn, args.origin)));
  }

  const verdict = summarise(findings);
  if (args.json) {
    process.stdout.write(`${JSON.stringify({ ...verdict, findings }, null, 2)}\n`);
  } else {
    for (const f of findings) {
      process.stdout.write(`${f.level === 'error' ? 'ERROR' : 'warn '} ${f.id}: ${f.detail}\n`);
    }
    process.stdout.write(
      verdict.ok
        ? `\nOK — ${verdict.warnings.length} warning(s), no errors\n`
        : `\nFAILED — ${verdict.errors.length} error(s), ${verdict.warnings.length} warning(s)\n`,
    );
  }
  process.exit(verdict.ok ? 0 : 1);
}

main();
