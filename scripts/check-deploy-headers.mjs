/**
 * scripts/check-deploy-headers.mjs — assert a DEPLOYED site's caching and
 * header contract (PRD #855, story 5).
 *
 *   node scripts/check-deploy-headers.mjs https://pp-demo.example.co.uk/
 *   node scripts/check-deploy-headers.mjs <url> --require-isolation
 *   node scripts/check-deploy-headers.mjs <url> --json
 *
 * Exits 0 when the contract holds (warnings do not fail), 1 on any error
 * finding, 2 on a usage or network problem.
 *
 * Deliberately NOT part of the push pipeline. It talks to a live origin, so it
 * would make every push depend on someone else's uptime and on a deploy having
 * already happened; `.github/workflows/check-deploy-headers.yml` runs it on
 * `workflow_dispatch` with the URL as an input instead.
 *
 * All the judgement lives in `scripts/deploy-headers.mjs` and is unit-tested
 * over canned fixtures (`tests/client/deploy-headers.test.js`). This file only
 * decides WHAT to fetch and how to print the answer.
 */

import { bundlePathsFromHtml, checkAll, normaliseHeaders } from './deploy-headers.mjs';

/** Paths probed on every run, whatever the bundle happens to be called. */
const FIXED_PATHS = [
  // The two entry points.
  '/',
  '/client/',
  // The catalogue the host reads before anything loads. Cached wrongly, a host
  // keeps offering scenarios a deploy has already removed.
  '/assets/scenarios.toml',
  // An ordinary non-hashed asset, so the "must not be cached for a year" half
  // of the contract is exercised too. Chosen because it is present in every
  // build and small.
  '/assets/logo.png',
];

function usage(message) {
  process.stderr.write(`${message}\n\n`);
  process.stderr.write(
    'usage: node scripts/check-deploy-headers.mjs <url> [--require-isolation] [--json]\n',
  );
  process.exit(2);
}

/** Join a site root and an absolute site path. */
function urlFor(root, path) {
  return new URL(path.replace(/^\//, ''), root.endsWith('/') ? root : `${root}/`).toString();
}

/** Fetch one path and reduce it to a probe. */
async function probe(root, path) {
  const url = urlFor(root, path);
  let response;
  try {
    // GET, not HEAD: a CDN can (and Cloudflare does) answer HEAD from a
    // different path than the one that serves bodies, so a HEAD-only check can
    // pass while real traffic is cached wrongly. `redirect: 'follow'` mirrors a
    // browser, which is whose experience this is asserting.
    response = await fetch(url, { redirect: 'follow' });
  } catch (e) {
    return { path, status: 0, headers: {}, error: String(e && e.message ? e.message : e) };
  }
  // Drain the body so the socket closes rather than being held open by a
  // half-read response.
  try { await response.arrayBuffer(); } catch { /* a truncated body is not a header fact */ }
  return { path, status: response.status, headers: normaliseHeaders(response.headers) };
}

async function main() {
  const args = process.argv.slice(2);
  const flags = new Set(args.filter((a) => a.startsWith('--')));
  const positional = args.filter((a) => !a.startsWith('--'));
  if (positional.length !== 1) usage('exactly one URL is required');

  const root = positional[0];
  try {
    const parsed = new URL(root);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      usage(`unsupported scheme ${parsed.protocol}`);
    }
  } catch {
    usage(`not a URL: ${root}`);
  }

  const requireIsolation = flags.has('--require-isolation');
  const asJson = flags.has('--json');

  // The bundle is content-addressed, so its real names come from the deployed
  // index rather than from a guess about what trunk emitted.
  let indexHtml = '';
  try {
    const response = await fetch(urlFor(root, '/'), { redirect: 'follow' });
    if (response.ok) indexHtml = await response.text();
  } catch {
    // The probe of '/' below reports the failure properly; nothing to add here.
  }
  const bundlePaths = bundlePathsFromHtml(indexHtml, '/');

  const paths = [...FIXED_PATHS, ...bundlePaths];
  const probes = [];
  for (const path of paths) {
    // Serially: this is a handful of requests against a live origin, and a
    // burst of parallel ones is exactly the shape a CDN rate-limits.
    probes.push(await probe(root, path));
  }

  const unreachable = probes.filter((p) => p.error);
  const result = checkAll(
    probes.filter((p) => !p.error),
    { requireIsolation },
  );

  if (asJson) {
    process.stdout.write(`${JSON.stringify({
      url: root,
      requireIsolation,
      probed: paths,
      unreachable: unreachable.map((p) => ({ path: p.path, error: p.error })),
      ...result,
    }, null, 2)}\n`);
  } else {
    process.stdout.write(`checked ${root}\n`);
    if (bundlePaths.length === 0) {
      process.stdout.write(
        '  note: no content-addressed bundle found in the served index.html — ' +
        'the immutable-caching half of the contract was not exercised\n',
      );
    }
    for (const p of unreachable) {
      process.stdout.write(`  unreachable ${p.path}: ${p.error}\n`);
    }
    for (const f of result.findings) {
      process.stdout.write(`  ${f.level.toUpperCase()} ${f.path}: ${f.message}\n`);
    }
    process.stdout.write(
      `${paths.length} paths probed; ${result.errors} error(s), ${result.warnings} warning(s)\n`,
    );
  }

  // `process.exitCode` rather than `process.exit()`: after `fetch`, Node's
  // connection pool still holds sockets, and forcing the process down on
  // Windows trips a libuv assertion (`!(handle->flags & UV_HANDLE_CLOSING)`)
  // that replaces the real exit code with 127. Setting the code and letting the
  // event loop drain gives the same answer on every platform.
  process.exitCode = unreachable.length > 0 ? 2 : (result.ok ? 0 : 1);
}

main().catch((e) => {
  process.stderr.write(`check-deploy-headers: ${e && e.stack ? e.stack : e}\n`);
  process.exitCode = 2;
});
