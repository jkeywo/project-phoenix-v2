// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest';
import { parse as tomlParse } from 'smol-toml';
import { mountModMode, MOD_DIRTY_KEY } from '../mod-mode-view.js';
import { ModeShell } from '../mode-shell.js';
import { readStoreZip, MANIFEST_PATH } from '../mod-pack-export.js';
import { canonicalTemplatePath } from '../entity-includes.js';

// Issue #989 — the MOD-mode DOM view over the pure workspace. jsdom: the view
// owns the DOM; IO (base-file reads for classification/stale, fragment
// resolution) is injected; export/import run the real mod-pack-export code, and
// the download seam is captured so no real file is written.

const WORLD_PATH = 'assets/worlds/default.toml';
const WORLD_TEXT = '[global]\n[anchors]\n';

/** A mutable in-memory disk. `set` mutates so a test can drift a base file. */
function makeIo(initial = {}) {
  const disk = new Map(Object.entries(initial));
  return {
    disk,
    io: {
      readFile: async (p) => {
        if (disk.has(p)) return disk.get(p);
        throw new Error(`ENOENT ${p}`);
      },
      listDirectory: async () => [],
    },
  };
}

/** Capture the last download instead of touching the DOM/URL APIs. */
function makeDownload() {
  const calls = [];
  return { calls, download: (bytes, filename) => calls.push({ bytes, filename }) };
}

function goodMeta(view) {
  const ws = view.getWorkspace();
  ws.setPack({
    id: 'test-pack',
    version: '1.0.0',
    name: 'Test Pack',
    requires: { content_id: 'phoenix-base', content_epoch: 1 },
  });
}

function mount(opts = {}) {
  const host = document.createElement('div');
  document.body.appendChild(host);
  const modeShell = new ModeShell();
  const { io } = opts.ioBundle || makeIo();
  const dl = opts.dlBundle || makeDownload();
  const view = mountModMode({
    host,
    modeShell,
    io,
    resolveEntityConfig: opts.resolveEntityConfig || (async () => ({ ok: false, sources: [] })),
    download: dl.download,
  });
  return { host, modeShell, view, download: dl };
}

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('mountModMode DOM shell', () => {
  it('renders the MOD-pack sections and the [pack] metadata inputs', () => {
    const { host } = mount();
    expect(host.querySelector('.mod-mode')).toBeTruthy();
    expect(host.querySelector('.mod-meta-form')).toBeTruthy();
    expect(host.querySelector('.mod-input-id')).toBeTruthy();
    expect(host.querySelector('.mod-member-list')).toBeTruthy();
    expect(host.querySelector('.mod-export-btn')).toBeTruthy();
    expect(host.querySelector('.mod-import-input')).toBeTruthy();
  });
});

describe('metadata form writes the [pack] header + dirties MOD mode', () => {
  it('a form input updates the workspace and marks MOD dirty; export emits [pack]', async () => {
    const { host, modeShell, view, download } = mount();
    const idInput = host.querySelector('.mod-input-id');
    idInput.value = 'aurora';
    idInput.dispatchEvent(new Event('input'));
    expect(view.getWorkspace().getPack().id).toBe('aurora');
    expect(modeShell.hasAnyDirty()).toBe(true);
    expect(modeShell.isDirty('MOD', MOD_DIRTY_KEY)).toBe(true);

    // Fill the rest and export.
    view.getWorkspace().setPack({
      version: '1.0.0',
      name: 'Aurora',
      requires: { content_id: 'phoenix-base', content_epoch: 1 },
    });
    view.getWorkspace().addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    view.getWorkspace().addScenario({ id: 'default', world: WORLD_PATH });

    const result = await view._internal.exportPackNow();
    expect(result.ok).toBe(true);
    expect(download.calls).toHaveLength(1);
    const files = readStoreZip(download.calls[0].bytes);
    const manifest = tomlParse(files[MANIFEST_PATH]);
    expect(manifest.pack.id).toBe('aurora');
    // A successful export clears the dirty bit.
    expect(modeShell.hasAnyDirty()).toBe(false);
  });
});

describe('member classification: patch vs new', () => {
  it('a path under the project root classifies as patch; an absent path is new', async () => {
    const bundle = makeIo({ [WORLD_PATH]: WORLD_TEXT });
    const { host, view } = mount({ ioBundle: bundle });

    await view._internal.addMemberByPath(WORLD_PATH); // exists on disk → patch
    await view._internal.addMemberByPath('assets/worlds/brand_new.toml'); // absent → new

    const rows = host.querySelectorAll('.mod-member-row');
    expect(rows.length).toBe(2);
    const patch = view.getWorkspace().getMember(WORLD_PATH);
    expect(patch.classification).toBe('patch');
    expect(patch.baseDigest).toBeTruthy();
    expect(view.getWorkspace().getMember('assets/worlds/brand_new.toml').classification).toBe('new');

    // Badges reflect classification in the DOM.
    expect(host.querySelector('.mod-member-badge-patch')).toBeTruthy();
    expect(host.querySelector('.mod-member-badge-new')).toBeTruthy();
  });
});

describe('export refusal on incomplete metadata (partitionFindings gate)', () => {
  it('refuses with a visible message and no download when [pack] is incomplete', async () => {
    const { host, view, download } = mount();
    view.getWorkspace().addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    view.getWorkspace().addScenario({ id: 'default', world: WORLD_PATH });

    const result = await view._internal.exportPackNow();
    expect(result.ok).toBe(false);
    expect(download.calls).toHaveLength(0);
    const errBox = host.querySelector('.mod-messages-errors');
    expect(errBox).toBeTruthy();
    expect(errBox.textContent).toMatch(/\[pack\]/);
  });
});

describe('stale-patch warning on export (never blocking)', () => {
  it('warns when a patch base drifted since it was added, but still exports', async () => {
    const bundle = makeIo({ [WORLD_PATH]: WORLD_TEXT });
    const { host, view, download } = mount({ ioBundle: bundle });
    goodMeta(view);
    await view._internal.addMemberByPath(WORLD_PATH); // patch, digest of v1 recorded
    view.getWorkspace().addScenario({ id: 'default', world: WORLD_PATH });

    // The base file on disk changes AFTER the member was added.
    bundle.disk.set(WORLD_PATH, `${WORLD_TEXT}# balance tweak\n`);

    const result = await view._internal.exportPackNow();
    expect(result.ok).toBe(true); // NEVER blocking
    expect(download.calls).toHaveLength(1);
    expect(result.staleWarnings.some((w) => w.category === 'stale-patch')).toBe(true);
    const warnBox = host.querySelector('.mod-messages-warnings');
    expect(warnBox).toBeTruthy();
    expect(warnBox.textContent).toMatch(/stale|since changed/i);
  });
});

describe('#910 fragment members auto-populate', () => {
  it('adding a composed hull pulls its include fragments in as members', async () => {
    const HULL = 'assets/entities/hull.toml';
    const FRAGMENT = 'assets/entities/base.toml';
    const bundle = makeIo({
      [HULL]: 'includes = ["base.toml"]\n',
      [FRAGMENT]: 'tags = ["ship"]\n[shape]\nkind = "sphere"\nradius = 1\n',
    });
    const resolveEntityConfig = async (p) => {
      if (p !== HULL) return { ok: false, sources: [] };
      return { ok: true, sources: [canonicalTemplatePath(HULL), FRAGMENT] };
    };
    const { view } = mount({ ioBundle: bundle, resolveEntityConfig });

    await view._internal.addMemberByPath(HULL);

    expect(view.getWorkspace().hasMember(HULL)).toBe(true);
    expect(view.getWorkspace().hasMember(FRAGMENT)).toBe(true);
  });
});

describe('#988 .rhai members supported', () => {
  it('adds a sibling .rhai and exports it verbatim alongside its world', async () => {
    const SCRIPT_WORLD = 'assets/worlds/combat.toml';
    const SCRIPT = 'assets/worlds/combat.rhai';
    const scriptText = 'fn on_alarm(ctx) { 2 + 2 }\n';
    const bundle = makeIo({
      [SCRIPT_WORLD]: 'script = "combat.rhai"\n[global]\n[anchors]\n',
      [SCRIPT]: scriptText,
    });
    const { view, download } = mount({ ioBundle: bundle });
    goodMeta(view);
    await view._internal.addMemberByPath(SCRIPT_WORLD);
    await view._internal.addMemberByPath(SCRIPT);
    view.getWorkspace().addScenario({ id: 'combat', world: SCRIPT_WORLD });

    const result = await view._internal.exportPackNow();
    expect(result.ok).toBe(true);
    const files = readStoreZip(download.calls[0].bytes);
    expect(files[SCRIPT]).toBe(scriptText);
  });
});

describe('round trip: import → re-export is byte-identical', () => {
  it('importing an exported pack and re-exporting without edits yields identical bytes', async () => {
    const bundle = makeIo();
    const { view, modeShell, download } = mount({ ioBundle: bundle });
    goodMeta(view);
    view.getWorkspace().addMember({ path: WORLD_PATH, text: WORLD_TEXT }, {});
    view.getWorkspace().addMember(
      { path: 'assets/entities/cruiser.toml', text: 'tags = ["ship"]\n[shape]\nkind = "sphere"\nradius = 1\n' },
      {},
    );
    view.getWorkspace().addScenario({ id: 'default', world: WORLD_PATH, label: 'Default' });

    const first = await view._internal.exportPackNow();
    expect(first.ok).toBe(true);
    const firstBytes = download.calls[0].bytes;

    // Import those exact bytes back into the workspace, then re-export.
    const reopened = await view._internal.importArchiveBytes(firstBytes);
    expect(reopened).toBeTruthy();
    expect(modeShell.hasAnyDirty()).toBe(false); // a fresh import is clean

    const second = await view._internal.exportPackNow();
    expect(second.ok).toBe(true);
    const secondBytes = download.calls[1].bytes;

    expect(Array.from(secondBytes)).toEqual(Array.from(firstBytes));
  });
});
