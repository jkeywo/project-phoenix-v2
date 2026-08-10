// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest';
import { mountEntityMode } from '../entity-mode-view.js';
import { ModeShell } from '../mode-shell.js';

// Issue #910, AC6 — a broken include (missing fragment, cycle, malformed
// declaration) must SURFACE in the editor naming the declaring file, not fall
// back silently to an uncomposed view. This exercises the view-level render of
// a resolution failure: the resolve dependency is stubbed to return
// { ok: false, error }, and the centre pane must render a located error banner
// while STILL opening the hull uncomposed.

const HULL = 'assets/entities/hull.toml';

/** A minimal saveFlow — the view only calls setContent during a load. */
function makeSaveFlow() {
  return { setContent() {} };
}

/** Stub io whose `resolve` is caller-supplied; everything else is inert. */
function makeIo({ resolve, readFile } = {}) {
  return {
    readFile: readFile || (async () => 'tags = ["ship"]\nincludes = ["broken.toml"]\n'),
    listDirectory: async () => [],
    preload: async () => {},
    resolve,
    onCacheInvalidate: () => ({ unsubscribe() {} }),
    getProjectRoot: async () => '/root',
    discover: async () => ({ factionMap: new Map(), complexityPaths: [] }),
    // Konva omitted — the preview pane renders a "not available" note, no throw.
  };
}

async function flush() {
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
}

describe('mountEntityMode include-resolution failures (issue #910 AC6)', () => {
  let host;
  let modeShell;

  beforeEach(() => {
    document.body.innerHTML = '';
    host = document.createElement('div');
    document.body.appendChild(host);
    modeShell = new ModeShell();
  });

  it('renders a located error banner naming the declaring file, and still opens the hull uncomposed', async () => {
    const error = {
      category: 'include-missing',
      message: 'included template not found: assets/entities/broken.toml',
      file: HULL,
      chain: [HULL, 'assets/entities/broken.toml'],
    };
    const io = makeIo({ resolve: async () => ({ ok: false, error }) });
    const view = mountEntityMode({ host, modeShell, saveFlow: makeSaveFlow(), io });
    await flush();

    await view._internal.loadEntity(HULL);

    const banner = host.querySelector('.entity-include-error');
    expect(banner).toBeTruthy();
    const text = banner.textContent;
    expect(text).toContain('include-missing'); // category
    expect(text).toContain(error.message); // message
    expect(text).toContain(HULL); // NAMES the declaring file
    expect(text).toContain('assets/entities/broken.toml'); // the chain

    // Not a silent omission: the hull still opened (uncomposed), so the
    // component stack is present alongside the error.
    expect(host.querySelector('.entity-component-stack')).toBeTruthy();
  });

  it('supports IncludeError instances via chainDisplay()', async () => {
    const error = {
      category: 'include-cycle',
      message: 'include cycle detected',
      file: 'assets/entities/b.toml',
      chain: [HULL, 'assets/entities/b.toml', HULL],
      chainDisplay() {
        return this.chain.join(' -> ');
      },
    };
    const io = makeIo({ resolve: async () => ({ ok: false, error }) });
    const view = mountEntityMode({ host, modeShell, saveFlow: makeSaveFlow(), io });
    await flush();

    await view._internal.loadEntity(HULL);

    const banner = host.querySelector('.entity-include-error');
    expect(banner).toBeTruthy();
    expect(banner.textContent).toContain('assets/entities/b.toml');
    expect(banner.textContent).toContain(' -> ');
  });

  it('shows no banner when resolution succeeds (composed)', async () => {
    const io = makeIo({
      resolve: async () => ({
        ok: true,
        isComposed: true,
        config: { tags: ['ship'], includes: ['broken.toml'] },
        provenance: { fields: new Map(), order: [] },
      }),
    });
    const view = mountEntityMode({ host, modeShell, saveFlow: makeSaveFlow(), io });
    await flush();

    await view._internal.loadEntity(HULL);

    expect(host.querySelector('.entity-include-error')).toBeNull();
    expect(host.querySelector('.entity-component-stack')).toBeTruthy();
  });

  it('clears a prior banner when a later file resolves cleanly', async () => {
    const error = {
      category: 'include-missing',
      message: 'included template not found',
      file: HULL,
      chain: [HULL],
    };
    let failNext = true;
    const io = makeIo({
      resolve: async () =>
        failNext
          ? { ok: false, error }
          : { ok: true, isComposed: false, config: { tags: ['ship'] }, provenance: null },
    });
    const view = mountEntityMode({ host, modeShell, saveFlow: makeSaveFlow(), io });
    await flush();

    await view._internal.loadEntity(HULL);
    expect(host.querySelector('.entity-include-error')).toBeTruthy();

    failNext = false;
    await view._internal.loadEntity('assets/entities/other.toml');
    expect(host.querySelector('.entity-include-error')).toBeNull();
  });
});
