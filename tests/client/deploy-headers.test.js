/**
 * tests/client/deploy-headers.test.js — the deployed header/caching contract
 * (PRD #855, story 5), over canned responses.
 *
 * `scripts/deploy-headers.mjs` is pure by design so this suite never touches
 * the network: every case here is a fabricated `{ path, status, headers }`,
 * which is also what lets it assert the failures — a live site cannot be asked
 * to serve a wrong header on demand.
 */

import { describe, expect, it } from 'vitest';
import {
  bundlePathsFromHtml,
  cachePolicyFor,
  checkAll,
  checkProbe,
  expectedContentType,
  isHashedAsset,
  normaliseHeaders,
} from '../../scripts/deploy-headers.mjs';

const IMMUTABLE = 'public, max-age=31536000, immutable';
const NOSNIFF = { 'x-content-type-options': 'nosniff' };

const probe = (path, headers, status = 200) => ({
  path,
  status,
  headers: { ...NOSNIFF, ...headers },
});

const errors = (findings) => findings.filter((f) => f.level === 'error');
const messages = (findings) => findings.map((f) => f.message).join(' | ');

describe('classifying a deployed path', () => {
  it('recognises a trunk content-addressed bundle', () => {
    expect(isHashedAsset('project-phoenix-6f3a91b2c4d5e607.js')).toBe(true);
    expect(isHashedAsset('project-phoenix-6f3a91b2c4d5e607_bg.wasm')).toBe(true);
    expect(isHashedAsset('project-phoenix-6f3a91b2c4d5e607_bg.wasm.gz')).toBe(true);
  });

  it('does not mistake an authored name that merely contains a dash', () => {
    expect(isHashedAsset('alliance-destroyer.glb')).toBe(false);
    expect(isHashedAsset('helm-console.html')).toBe(false);
    // Too short to be a content hash.
    expect(isHashedAsset('thing-abc123.js')).toBe(false);
  });

  it('puts entry points and authored manifests on revalidate, bundles on immutable', () => {
    expect(cachePolicyFor('/')).toBe('revalidate');
    expect(cachePolicyFor('/client/')).toBe('revalidate');
    expect(cachePolicyFor('/index.html')).toBe('revalidate');
    expect(cachePolicyFor('/assets/scenarios.toml')).toBe('revalidate');
    expect(cachePolicyFor('/project-phoenix-6f3a91b2c4d5e607.js')).toBe('immutable');
    expect(cachePolicyFor('/assets/models/alliance_destroyer.glb')).toBe('short');
    expect(cachePolicyFor('/gui/helm-console.js')).toBe('short');
  });

  it('demands application/wasm, because streaming instantiation refuses anything else', () => {
    expect(expectedContentType('/project-phoenix-6f3a91b2c4d5e607_bg.wasm'))
      .toBe('application/wasm');
    expect(expectedContentType('/project-phoenix-6f3a91b2c4d5e607_bg.wasm.gz'))
      .toBe('application/gzip');
    expect(expectedContentType('/')).toBe('text/html');
    expect(expectedContentType('/assets/models/x.glb')).toBe(null);
  });
});

describe('checking one deployed response', () => {
  it('accepts an entry point that revalidates', () => {
    const findings = checkProbe(probe('/', {
      'content-type': 'text/html; charset=utf-8',
      'cache-control': 'no-cache',
    }));
    expect(findings).toEqual([]);
  });

  it('accepts max-age=0 as revalidation, which is what Pages sends by default', () => {
    const findings = checkProbe(probe('/', {
      'content-type': 'text/html',
      'cache-control': 'public, max-age=0, must-revalidate',
    }));
    expect(findings).toEqual([]);
  });

  it('rejects an entry point cached for a year — a deploy could never reach a player', () => {
    const findings = checkProbe(probe('/', {
      'content-type': 'text/html',
      'cache-control': IMMUTABLE,
    }));
    expect(errors(findings)).toHaveLength(1);
    expect(messages(findings)).toMatch(/must revalidate/);
  });

  it('accepts a content-addressed bundle cached for a year', () => {
    const findings = checkProbe(probe('/project-phoenix-6f3a91b2c4d5e607.js', {
      'content-type': 'text/javascript; charset=utf-8',
      'cache-control': IMMUTABLE,
    }));
    expect(findings).toEqual([]);
  });

  it('rejects a content-addressed bundle that is not cached — the whole point of the hash', () => {
    const findings = checkProbe(probe('/project-phoenix-6f3a91b2c4d5e607.js', {
      'content-type': 'text/javascript',
      'cache-control': 'public, max-age=0, must-revalidate',
    }));
    expect(errors(findings)).toHaveLength(1);
    expect(messages(findings)).toMatch(/at least 31536000s/);
  });

  it('warns when a cached-for-a-year bundle is not marked immutable', () => {
    const findings = checkProbe(probe('/project-phoenix-6f3a91b2c4d5e607.js', {
      'content-type': 'text/javascript',
      'cache-control': 'public, max-age=31536000',
    }));
    expect(errors(findings)).toHaveLength(0);
    expect(messages(findings)).toMatch(/immutable/);
  });

  it('rejects a NON-hashed asset cached for a year, which a deploy could not evict', () => {
    const findings = checkProbe(probe('/assets/models/alliance_destroyer.glb', {
      'content-type': 'model/gltf-binary',
      'cache-control': IMMUTABLE,
    }));
    expect(errors(findings)).toHaveLength(1);
    expect(messages(findings)).toMatch(/name does not change/);
  });

  it('rejects wasm served as octet-stream', () => {
    const findings = checkProbe(probe('/project-phoenix-6f3a91b2c4d5e607_bg.wasm', {
      'content-type': 'application/octet-stream',
      'cache-control': IMMUTABLE,
    }));
    expect(errors(findings)).toHaveLength(1);
    expect(messages(findings)).toMatch(/application\/wasm/);
  });

  it('rejects the gzipped wasm arriving with Content-Encoding, which the page cannot then decompress', () => {
    const findings = checkProbe(probe('/project-phoenix-6f3a91b2c4d5e607_bg.wasm.gz', {
      'content-type': 'application/gzip',
      'cache-control': IMMUTABLE,
      'content-encoding': 'gzip',
    }));
    expect(errors(findings)).toHaveLength(1);
    expect(messages(findings)).toMatch(/Content-Encoding/);
  });

  it('reports a non-200 and stops, rather than judging headers of a body nobody got', () => {
    const findings = checkProbe(probe('/assets/scenarios.toml', {}, 404));
    expect(findings).toHaveLength(1);
    expect(messages(findings)).toMatch(/expected 200, got 404/);
  });

  it('warns about a missing nosniff without failing the deploy over it', () => {
    const findings = checkProbe({
      path: '/',
      status: 200,
      headers: { 'content-type': 'text/html', 'cache-control': 'no-cache' },
    });
    expect(errors(findings)).toHaveLength(0);
    expect(messages(findings)).toMatch(/nosniff/);
  });
});

describe('cross-origin isolation', () => {
  const entry = (extra) => probe('/', {
    'content-type': 'text/html',
    'cache-control': 'no-cache',
    ...extra,
  });

  it('is not required by the current single-threaded build', () => {
    expect(checkProbe(entry({}))).toEqual([]);
  });

  it('rejects COEP set without COOP — it isolates nothing and blocks PeerJS and TURN', () => {
    const findings = checkProbe(entry({ 'cross-origin-embedder-policy': 'require-corp' }));
    expect(errors(findings)).toHaveLength(1);
    expect(messages(findings)).toMatch(/isolates nothing/);
  });

  it('accepts a fully isolated pair even though it is not required', () => {
    expect(checkProbe(entry({
      'cross-origin-opener-policy': 'same-origin',
      'cross-origin-embedder-policy': 'require-corp',
    }))).toEqual([]);
  });

  it('demands both halves once the worker-thread spike turns the switch on', () => {
    const findings = checkProbe(entry({}), { requireIsolation: true });
    expect(errors(findings)).toHaveLength(2);
    expect(messages(findings)).toMatch(/Cross-Origin-Opener-Policy/);
    expect(messages(findings)).toMatch(/Cross-Origin-Embedder-Policy/);
  });

  it('does not ask an asset to be isolated — only the entry points can be', () => {
    const findings = checkProbe(probe('/project-phoenix-6f3a91b2c4d5e607.js', {
      'content-type': 'text/javascript',
      'cache-control': IMMUTABLE,
    }), { requireIsolation: true });
    expect(findings).toEqual([]);
  });
});

describe('rolling a whole run up', () => {
  it('is ok only when nothing errored, and counts both levels', () => {
    const result = checkAll([
      probe('/', { 'content-type': 'text/html', 'cache-control': 'no-cache' }),
      probe('/project-phoenix-6f3a91b2c4d5e607.js', {
        'content-type': 'text/javascript',
        'cache-control': 'public, max-age=31536000',
      }),
    ]);
    expect(result.ok).toBe(true);
    expect(result.errors).toBe(0);
    expect(result.warnings).toBe(1);

    const bad = checkAll([probe('/assets/scenarios.toml', {
      'content-type': 'text/plain',
      'cache-control': IMMUTABLE,
    })]);
    expect(bad.ok).toBe(false);
    expect(bad.errors).toBe(1);
  });
});

describe('finding the content-addressed bundle in a deployed page', () => {
  it('reads the loader and preload targets out of index.html', () => {
    const html = `
      <link rel="preload" href="./project-phoenix-6f3a91b2c4d5e607_bg.wasm" as="fetch">
      <script type="module">
        import init from './project-phoenix-6f3a91b2c4d5e607.js';
        init({ module_or_path: './project-phoenix-6f3a91b2c4d5e607_bg.wasm' });
      </script>`;
    expect(bundlePathsFromHtml(html)).toEqual([
      '/project-phoenix-6f3a91b2c4d5e607_bg.wasm',
      '/project-phoenix-6f3a91b2c4d5e607.js',
    ]);
  });

  it('finds the gzipped wasm the demo deploy substitutes', () => {
    const html = `fetch('./project-phoenix-6f3a91b2c4d5e607_bg.wasm.gz')`;
    expect(bundlePathsFromHtml(html)).toEqual([
      '/project-phoenix-6f3a91b2c4d5e607_bg.wasm.gz',
    ]);
  });

  it('finds nothing in a page with no bundle, rather than inventing one', () => {
    expect(bundlePathsFromHtml('<html><body>hello</body></html>')).toEqual([]);
  });
});

describe('normalising fetched headers', () => {
  it('lowercases names from a Headers object and from a plain one alike', () => {
    expect(normaliseHeaders({ 'Cache-Control': 'no-cache' }))
      .toEqual({ 'cache-control': 'no-cache' });
    const headers = new Headers({ 'Content-Type': 'application/wasm' });
    expect(normaliseHeaders(headers)['content-type']).toBe('application/wasm');
    expect(normaliseHeaders(null)).toEqual({});
  });
});
