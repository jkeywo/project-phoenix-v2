// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, it, expect, beforeEach } from 'vitest';
import { parse as tomlParse } from 'smol-toml';
import { mountModMode, MOD_DIRTY_KEY } from '../mod-mode-view.js';
import { ModeShell } from '../mode-shell.js';
import {
  buildManifestToml,
  createStoreZip,
  readStoreZip,
  readStoreZipArchive,
  MANIFEST_PATH,
} from '../mod-pack-export.js';
import { canonicalTemplatePath } from '../entity-includes.js';
import {
  MOD_ACTION_CONTEXT,
  MOD_EXPORT_ACTION_ID,
  MOD_IMPORT_ACTION_ID,
  MOD_VALIDATE_ACTION_ID,
} from '../mod-actions.js';
import { OPERATOR_PROFILE_KEY } from '../../gui/operator-profile.js';

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
    exportPack: opts.exportPack,
    readArchive: opts.readArchive,
    feedbackRoot: opts.feedbackRoot,
    profileStorage: opts.profileStorage ?? window.localStorage,
  });
  return { host, modeShell, view, download: dl };
}

beforeEach(() => {
  document.body.innerHTML = '';
  window.localStorage.clear();
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

describe('#1321 semantic import lifecycle and validation focus', () => {
  function validManifestText(scenarioPatch = {}) {
    return buildManifestToml(
      [{ id: 'default', world: WORLD_PATH, label: 'Default', ...scenarioPatch }],
      {
        id: 'imported-pack',
        version: '1.0.0',
        name: 'Imported Pack',
        requires: { content_id: 'phoenix-base', content_epoch: 1 },
      },
    );
  }

  function archiveWithManifest(manifestText, extraEntries = [], worldText = WORLD_TEXT) {
    return createStoreZip([
      { path: MANIFEST_PATH, text: manifestText },
      { path: WORLD_PATH, text: worldText },
      ...extraEntries,
    ]);
  }

  function validArchive(extraEntries = []) {
    return archiveWithManifest(validManifestText(), extraEntries);
  }

  async function chooseBytes(view, bytes) {
    const { importInput } = view._internal.elements;
    Object.defineProperty(importInput, 'files', {
      configurable: true,
      value: [{
        arrayBuffer: async () => bytes.buffer.slice(
          bytes.byteOffset,
          bytes.byteOffset + bytes.byteLength,
        ),
      }],
    });
    importInput.dispatchEvent(new Event('change'));
    await new Promise((resolve) => setTimeout(resolve, 0));
  }

  it('imports a valid real archive from the keyboard action and reports Applied', async () => {
    const { host, view } = mount();
    const key = new KeyboardEvent('keydown', {
      code: 'KeyI',
      key: 'i',
      bubbles: true,
      cancelable: true,
    });

    expect(view.dispatchKeyboardEvent(key)).toMatchObject({
      claimed: true,
      actionId: MOD_IMPORT_ACTION_ID,
      handled: true,
    });
    expect(key.defaultPrevented).toBe(true);
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Pending');

    await chooseBytes(view, validArchive());

    expect(view.getWorkspace().getPack().id).toBe('imported-pack');
    expect(view.getWorkspace().getMember(WORLD_PATH).text).toBe(WORLD_TEXT);
    expect(host.querySelector('.mod-import-success')).toBeTruthy();
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Applied');
    expect(document.activeElement).toBe(view._internal.fields.id);
  });

  it('cancels a pending chooser without a terminal result and restores control focus', () => {
    const { view } = mount();
    const result = view.semanticActions.activate(MOD_IMPORT_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
    });
    expect(result.handled).toBe(true);
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Pending');

    view._internal.elements.importInput.dispatchEvent(new Event('cancel'));

    expect(view._internal.elements.feedbackStatus.textContent).toBe('');
    expect(view._internal.elements.feedbackStatus.dataset.state).toBeUndefined();
    expect(document.activeElement).toBe(view._internal.elements.importBtn);
  });

  it('keeps a readable invalid source loaded and focuses non-colour findings', async () => {
    const disallowed = 'assets/secret/keys.toml';
    const manifestText = [
      '# Keep this operator note and the noncanonical key order.',
      '[pack]',
      'name = "Invalid Pack"',
      // Rust accepts this ignored extension i64, despite it exceeding JS's
      // safe Number range. Import must retain its exact token/source bytes.
      'custom_big_integer = 9007199254740992',
      'id = "invalid-pack"',
      'version = "1.0.0"',
      'format = 1',
      '',
      '[pack.requires]',
      'content_epoch = 1',
      'content_id = "phoenix-base"',
      '',
      '[[scenario]]',
      `world = "${WORLD_PATH}"`,
      'custom_scenario_key = "retain-this-too"',
      'id = "default"',
      '',
    ].join('\n');
    const worldText = '# Exact member source uses CRLF.\r\n[global]\r\n[anchors]\r\n';
    const bytes = createStoreZip([
      { path: MANIFEST_PATH, text: manifestText },
      { path: WORLD_PATH, text: worldText },
      { path: disallowed, text: 'secret = true\n' },
    ]);
    const { host, view, download } = mount();
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });

    await chooseBytes(view, bytes);

    const findings = host.querySelector('.mod-import-findings');
    expect(findings).toBeTruthy();
    expect(findings.getAttribute('role')).toBe('alert');
    expect(findings.textContent).toMatch(/source bundle remains loaded and editable/i);
    expect(findings.textContent).toMatch(/not a supported authored path/i);
    expect(document.activeElement).toBe(findings);
    expect(view.getWorkspace().getMember(disallowed).text).toBe('secret = true\n');
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Refused');

    const source = view.getWorkspace().getSourceArchive();
    expect(Array.from(source.bytes)).toEqual(Array.from(bytes));
    expect(view.getWorkspace().getSourceEntry(MANIFEST_PATH).text).toBe(manifestText);
    expect(Array.from(view.getWorkspace().getSourceEntry(WORLD_PATH).bytes)).toEqual(
      Array.from(new TextEncoder().encode(worldText)),
    );

    // Repair the evidenced member finding through the existing editable list.
    // The untouched manifest and member source then export byte-for-byte; the
    // removed entry remains available as immutable import provenance.
    host.querySelector(`.mod-member-row[data-path="${disallowed}"] .mod-member-remove`).click();
    expect(view.getWorkspace().hasMember(disallowed)).toBe(false);
    expect(view.getWorkspace().getSourceEntry(disallowed).text).toBe('secret = true\n');
    const repaired = await view._internal.exportPackNow();
    expect(repaired.ok).toBe(true);
    const exported = readStoreZipArchive(download.calls[0].bytes);
    expect(exported.files[MANIFEST_PATH]).toBe(manifestText);
    expect(Array.from(exported.source.entries.find((entry) => entry.path === MANIFEST_PATH).bytes))
      .toEqual(Array.from(new TextEncoder().encode(manifestText)));
    expect(Array.from(exported.source.entries.find((entry) => entry.path === WORLD_PATH).bytes))
      .toEqual(Array.from(new TextEncoder().encode(worldText)));
    expect(exported.files[disallowed]).toBeUndefined();
  });

  it.each([
    ['minimum', '-9223372036854775808'],
    ['maximum', '9223372036854775807'],
  ])('imports and byte-identically exports the signed i64 %s content_epoch', async (
    _label,
    token,
  ) => {
    const manifestText = validManifestText().replace(
      'content_epoch = 1',
      `content_epoch = ${token}`,
    );
    const bytes = archiveWithManifest(manifestText);
    const { view, download } = mount();
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });

    await chooseBytes(view, bytes);

    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Applied');
    expect(view.getWorkspace().getPack().requires.content_epoch).toBe(BigInt(token));
    expect(view._internal.fields.content_epoch.value).toBe(token);
    const exported = await view._internal.exportPackNow();
    expect(exported.ok).toBe(true);
    expect(Array.from(download.calls[0].bytes)).toEqual(Array.from(bytes));
  });

  it('refuses but retains a real archive whose unsupported member is named __proto__', async () => {
    const memberText = 'untrusted member source\r\n';
    const bytes = validArchive([{ path: '__proto__', text: memberText }]);
    const { host, view } = mount();
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });

    await chooseBytes(view, bytes);

    expect(view.getWorkspace().getPack().id).toBe('imported-pack');
    expect(view.getWorkspace().getMember(WORLD_PATH).text).toBe(WORLD_TEXT);
    expect(view.getWorkspace().getMember('__proto__').text).toBe(memberText);
    const findings = host.querySelector('.mod-import-findings');
    expect(findings).toBeTruthy();
    expect(findings.textContent).toMatch(/"__proto__" is not a supported authored path/i);
    expect(findings.textContent).toMatch(/source bundle remains loaded and editable/i);
    expect(document.activeElement).toBe(findings);
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Refused');

    const source = view.getWorkspace().getSourceArchive();
    expect(Array.from(source.bytes)).toEqual(Array.from(bytes));
    expect(source.entries.map((entry) => entry.path)).toEqual([
      MANIFEST_PATH,
      WORLD_PATH,
      '__proto__',
    ]);
    expect(view.getWorkspace().getSourceEntry('__proto__').text).toBe(memberText);
  });

  it('carries a valid scenario ships list through import, view, and byte-identical export', async () => {
    const offered = 'assets/entities/alliance_destroyer.toml';
    const worldText = [
      '[global]',
      '[anchors]',
      '[[available_ships]]',
      `template_path = "${offered}"`,
      '',
    ].join('\n');
    const manifestText = validManifestText({ ships: [offered] });
    const bytes = archiveWithManifest(manifestText, [], worldText);
    const { host, view, download } = mount();
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });

    await chooseBytes(view, bytes);

    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Applied');
    expect(view.getWorkspace().getScenarios()[0].ships).toEqual([offered]);
    expect(host.querySelector('.mod-scenario-row-ships').textContent).toBe(offered);
    const exported = await view._internal.exportPackNow();
    expect(exported.ok).toBe(true);
    expect(Array.from(download.calls[0].bytes)).toEqual(Array.from(bytes));
    expect(tomlParse(readStoreZip(download.calls[0].bytes)[MANIFEST_PATH]).scenario[0].ships)
      .toEqual([offered]);
  });

  it('refuses but retains a real archive that curates a ship its world does not offer', async () => {
    const offered = 'assets/entities/alliance_destroyer.toml';
    const absent = 'assets/entities/not_offered.toml';
    const worldText = [
      '[global]',
      '[anchors]',
      '[[available_ships]]',
      `template_path = "${offered}"`,
      '',
    ].join('\n');
    const manifestText = validManifestText({ ships: [absent] });
    const bytes = archiveWithManifest(manifestText, [], worldText);
    const { host, view } = mount();
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });

    await chooseBytes(view, bytes);

    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Refused');
    expect(view.getWorkspace().getScenarios()[0].ships).toEqual([absent]);
    const findings = host.querySelector('.mod-import-findings');
    expect(findings.textContent).toMatch(/curates ship.*does not offer/i);
    expect(findings.textContent).toContain(absent);
    expect(view.getWorkspace().getSourceEntry(MANIFEST_PATH).text).toBe(manifestText);
    expect(Array.from(view.getWorkspace().getSourceArchive().bytes)).toEqual(Array.from(bytes));
  });

  it.each([
    [
      'a future pack format',
      validManifestText().replace('format = 1', 'format = 2'),
      /format 2.*supports at most 1/i,
      'imported-pack',
    ],
    [
      'a missing mandatory format',
      validManifestText().replace('format = 1\n', ''),
      /\[pack\] format is required/i,
      'keep-me',
    ],
    [
      'a string content epoch',
      validManifestText().replace('content_epoch = 1', 'content_epoch = "1"'),
      /content_epoch.*integer token/i,
      'keep-me',
    ],
    [
      'a float-token pack format',
      validManifestText().replace('format = 1', 'format = 1.0'),
      /\[pack\] format.*integer token/i,
      'keep-me',
    ],
    [
      'an exponent-token pack format',
      validManifestText().replace('format = 1', 'format = 1e0'),
      /\[pack\] format.*integer token/i,
      'keep-me',
    ],
    [
      'a float-token content epoch',
      validManifestText().replace('content_epoch = 1', 'content_epoch = 1.0'),
      /content_epoch.*integer token/i,
      'keep-me',
    ],
    [
      'an exponent-token content epoch',
      validManifestText().replace('content_epoch = 1', 'content_epoch = 1e0'),
      /content_epoch.*integer token/i,
      'keep-me',
    ],
    [
      'a missing content epoch',
      validManifestText().replace('content_epoch = 1\n', ''),
      /content_epoch.*integer/i,
      'imported-pack',
    ],
    [
      'a missing required version',
      validManifestText().replace('version = "1.0.0"\n', ''),
      /version is required/i,
      'imported-pack',
    ],
  ])('settles Refused for %s before raw source can be reused', async (
    _label,
    manifestText,
    findingPattern,
    expectedPackId,
  ) => {
    const { host, view } = mount();
    view.getWorkspace().setPack({ id: 'keep-me' });
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });

    await chooseBytes(view, archiveWithManifest(manifestText));

    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Refused');
    const findings = host.querySelector('.mod-import-findings');
    expect(findings).toBeTruthy();
    expect(findings.textContent).toMatch(findingPattern);
    expect(document.activeElement).toBe(findings);
    expect(view.getWorkspace().getPack().id).toBe(expectedPackId);
  });

  it.each([
    ['non-ZIP bytes', new TextEncoder().encode('this is not a ZIP archive')],
    [
      'the committed CRC-corrupt archive',
      new Uint8Array(readFileSync(path.resolve(
        process.cwd(),
        'tests/fixtures/mod-packs/corrupt-crc.zip',
      ))),
    ],
    [
      'CRC-valid archive content with invalid UTF-8',
      createStoreZip([{ path: MANIFEST_PATH, bytes: Uint8Array.of(0xff) }]),
    ],
  ])('preserves the previous workspace when the real reader rejects %s', async (_label, bytes) => {
    const { host, view } = mount();
    view.getWorkspace().setPack({ id: 'keep-me' });
    view.getWorkspace().addMember({ path: WORLD_PATH, text: 'keep this source\n' }, {});
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });

    await chooseBytes(view, bytes);

    expect(view.getWorkspace().getPack().id).toBe('keep-me');
    expect(view.getWorkspace().getMember(WORLD_PATH).text).toBe('keep this source\n');
    const findings = host.querySelector('.mod-import-findings');
    expect(findings.getAttribute('role')).toBe('alert');
    expect(findings.textContent).toMatch(/previous MOD workspace was not changed/i);
    expect(document.activeElement).toBe(findings);
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Refused');
  });

  it('#1322 edits, validates, remaps, and exports one imported pack without losing source guarantees', async () => {
    const manifestText = `# retain this manifest note\n${validManifestText()}`;
    const worldText = '# retain this world note\r\n[global]\r\n[anchors]\r\n';
    const editedWorld = worldText.replace(
      '[global]\r\n',
      '[global]\r\nsim_tick_hz = 60\r\n',
    );
    const bytes = archiveWithManifest(manifestText, [], worldText);
    const ioBundle = makeIo({ [WORLD_PATH]: worldText });
    const { host, view, modeShell, download } = mount({ ioBundle });
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });
    await chooseBytes(view, bytes);

    const before = view.getWorkspace().getMember(WORLD_PATH);
    host.querySelector(`.mod-member-row[data-path="${WORLD_PATH}"] .mod-member-edit`).click();
    const sourceInput = view._internal.elements.memberEditorInput;
    expect(document.activeElement).toBe(sourceInput);
    sourceInput.value = editedWorld;
    sourceInput.dispatchEvent(new Event('input'));
    expect(view.getWorkspace().getMember(WORLD_PATH)).toMatchObject({
      text: editedWorld,
      classification: 'patch',
      baseDigest: before.baseDigest,
    });
    expect(modeShell.isDirty('MOD', MOD_DIRTY_KEY)).toBe(true);

    const validateKey = new KeyboardEvent('keydown', {
      code: 'KeyV', key: 'v', bubbles: true, cancelable: true,
    });
    expect(view.dispatchKeyboardEvent(validateKey)).toMatchObject({
      claimed: true,
      actionId: MOD_VALIDATE_ACTION_ID,
      handled: true,
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Applied');
    expect(document.activeElement).toBe(view._internal.elements.exportBtn);
    expect(modeShell.isDirty('MOD', MOD_DIRTY_KEY)).toBe(true);
    expect(download.calls).toHaveLength(0);

    const capture = host.querySelector(
      `[data-control="semantic-binding-${MOD_EXPORT_ACTION_ID}-0"]`,
    );
    capture.focus();
    capture.dispatchEvent(new KeyboardEvent('keydown', {
      code: 'KeyX', key: 'x', bubbles: true, cancelable: true,
    }));
    const privateProfile = JSON.parse(window.localStorage.getItem(OPERATOR_PROFILE_KEY));
    expect(privateProfile.bindings[MOD_EXPORT_ACTION_ID][0].code).toBe('KeyX');
    expect(privateProfile.bindings['captain.red-alert']).toBeTruthy();
    expect(privateProfile).not.toHaveProperty('identity');
    expect(privateProfile).not.toHaveProperty('station');
    expect(privateProfile).not.toHaveProperty('saves');

    const exportKey = new KeyboardEvent('keydown', {
      code: 'KeyX', key: 'x', bubbles: true, cancelable: true,
    });
    expect(view.dispatchKeyboardEvent(exportKey)).toMatchObject({
      claimed: true,
      actionId: MOD_EXPORT_ACTION_ID,
      handled: true,
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Applied');
    expect(document.activeElement).toBe(view._internal.elements.exportBtn);
    expect(modeShell.isDirty('MOD', MOD_DIRTY_KEY)).toBe(false);
    expect(download.calls).toHaveLength(1);

    const exported = readStoreZipArchive(download.calls[0].bytes);
    expect(exported.files[MANIFEST_PATH]).toBe(manifestText);
    expect(exported.files[WORLD_PATH]).toBe(editedWorld);
    expect(exported.files[WORLD_PATH]).toContain('# retain this world note\r\n');
    expect(view.getWorkspace().getSourceEntry(WORLD_PATH).text).toBe(worldText);
    expect(Array.from(view.getWorkspace().getSourceArchive().bytes)).toEqual(Array.from(bytes));
  });

  it('#1322 Reset All restores Captain, Helm, and editor bindings in one persisted profile', () => {
    const { host, view } = mount();
    const registry = view.semanticActions;
    const actionIds = ['captain.red-alert', 'helm.steering', MOD_EXPORT_ACTION_ID];
    const defaults = Object.fromEntries(actionIds.map((id) => [
      id,
      registry.action(id).bindings,
    ]));
    const keyX = {
      type: 'keyboard',
      code: 'KeyX',
      ctrlKey: false,
      shiftKey: false,
      altKey: false,
      metaKey: false,
    };
    const keyZ = { ...keyX, code: 'KeyZ' };

    expect(registry.setBinding('captain.red-alert', 1, keyZ).status).toBe('applied');
    expect(registry.setBinding('helm.steering', 0, null).status).toBe('applied');
    expect(registry.setBinding(MOD_EXPORT_ACTION_ID, 0, keyX).status).toBe('applied');
    for (const id of actionIds) expect(registry.action(id).bindings).not.toEqual(defaults[id]);

    host.querySelector('[data-control="semantic-binding-reset-all"]').click();

    for (const id of actionIds) expect(registry.action(id).bindings).toEqual(defaults[id]);
    const stored = JSON.parse(window.localStorage.getItem(OPERATOR_PROFILE_KEY));
    for (const id of actionIds) expect(stored.bindings[id]).toEqual(defaults[id]);
  });

  it('#1322 keeps overlapping action feedback keyed while Validate is pending', async () => {
    let releaseBase;
    const baseGate = new Promise((resolve) => { releaseBase = resolve; });
    const ioBundle = {
      io: {
        readFile: async (requestedPath) => {
          if (requestedPath !== WORLD_PATH) throw new Error('not found');
          await baseGate;
          return WORLD_TEXT;
        },
      },
    };
    const { host, view } = mount({ ioBundle });
    goodMeta(view);
    view.getWorkspace().addMember(
      { path: WORLD_PATH, text: WORLD_TEXT },
      { [WORLD_PATH]: WORLD_TEXT },
    );
    view.getWorkspace().addScenario({ id: 'default', world: WORLD_PATH });
    view.render();

    expect(view.semanticActions.activate(MOD_VALIDATE_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
    })).toMatchObject({ claimed: true, handled: true });
    expect(host.querySelector(
      `.mod-action-feedback-row[data-action-id="${MOD_VALIDATE_ACTION_ID}"]`,
    ).dataset.state).toBe('Pending');

    // A different operation is explicitly refused, but both states remain in
    // the aggregate live region and the refusal keeps focus.
    expect(view.semanticActions.activate(MOD_EXPORT_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
    })).toMatchObject({ claimed: true, handled: true });
    expect(host.querySelector(
      `.mod-action-feedback-row[data-action-id="${MOD_VALIDATE_ACTION_ID}"]`,
    ).dataset.state).toBe('Pending');
    expect(host.querySelector(
      `.mod-action-feedback-row[data-action-id="${MOD_EXPORT_ACTION_ID}"]`,
    ).dataset.state).toBe('Refused');
    expect(document.activeElement).toBe(host.querySelector(
      '.mod-operation-result[data-outcome="refused"]',
    ));

    expect(view.semanticActions.activate(MOD_IMPORT_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
    })).toMatchObject({ claimed: true, handled: true });
    expect(host.querySelector(
      `.mod-action-feedback-row[data-action-id="${MOD_VALIDATE_ACTION_ID}"]`,
    ).dataset.state).toBe('Pending');
    expect(host.querySelector(
      `.mod-action-feedback-row[data-action-id="${MOD_IMPORT_ACTION_ID}"]`,
    ).dataset.state).toBe('Refused');
    const busyAlert = host.querySelector('.mod-operation-result[data-outcome="refused"]');
    expect(busyAlert.getAttribute('role')).toBe('alert');
    expect(document.activeElement).toBe(busyAlert);

    // A duplicate Validate is cancelled by the adapter. The shared lifecycle
    // promotes the original correlation and the aggregate restores Pending.
    expect(view.semanticActions.activate(MOD_VALIDATE_ACTION_ID, {
      context: MOD_ACTION_CONTEXT,
    })).toMatchObject({ claimed: true, handled: false });
    expect(host.querySelector(
      `.mod-action-feedback-row[data-action-id="${MOD_VALIDATE_ACTION_ID}"]`,
    ).dataset.state).toBe('Pending');
    expect(host.querySelectorAll('.mod-action-feedback-row')).toHaveLength(3);
    expect(document.activeElement).toBe(busyAlert);

    releaseBase();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(host.querySelector(
      `.mod-action-feedback-row[data-action-id="${MOD_VALIDATE_ACTION_ID}"]`,
    ).dataset.state).toBe('Applied');
    expect(document.activeElement).toBe(view._internal.elements.exportBtn);
  });

  it('#1322 refuses an invalid member edit with focused accessible feedback and no download', async () => {
    const bytes = validArchive();
    const { host, view, modeShell, download } = mount();
    view.semanticActions.activate(MOD_IMPORT_ACTION_ID, { context: MOD_ACTION_CONTEXT });
    await chooseBytes(view, bytes);
    host.querySelector(`.mod-member-row[data-path="${WORLD_PATH}"] .mod-member-edit`).click();
    const sourceInput = view._internal.elements.memberEditorInput;
    sourceInput.value = 'not_valid = [';
    sourceInput.dispatchEvent(new Event('input'));

    view.semanticActions.activate(MOD_VALIDATE_ACTION_ID, { context: MOD_ACTION_CONTEXT });
    await new Promise((resolve) => setTimeout(resolve, 0));

    const refusal = host.querySelector('.mod-operation-result[data-outcome="refused"]');
    expect(refusal).toBeTruthy();
    expect(refusal.getAttribute('role')).toBe('alert');
    expect(document.activeElement).toBe(refusal);
    expect(view._internal.elements.feedbackStatus.dataset.state).toBe('Refused');
    expect(modeShell.isDirty('MOD', MOD_DIRTY_KEY)).toBe(true);
    expect(download.calls).toHaveLength(0);
    expect(Array.from(view.getWorkspace().getSourceArchive().bytes)).toEqual(Array.from(bytes));
  });

  it('proves the bounded T2 surface and dependencies instead of placeholder selectors', () => {
    const { host, view } = mount();
    expect(view._internal.elements.scopeBoundary.textContent).toMatch(/M6/i);
    const inventory = (selector) => Array.from(
      host.querySelector(selector).children,
      (node) => `${node.tagName.toLowerCase()}.${Array.from(node.classList).join('.')}`,
    );
    expect(inventory('.mod-mode-body')).toEqual([
      'section.mod-section.mod-meta',
      'section.mod-section.mod-scenarios',
      'section.mod-section.mod-members',
      'section.mod-section.mod-actions',
    ]);
    expect(inventory('.mod-actions')).toEqual([
      'button.mod-import-btn',
      'button.mod-validate-btn',
      'button.mod-export-btn',
      'input.mod-import-input.mod-file-input',
      'div.mod-action-feedback',
      'div.mod-messages',
      'details.mod-private-settings',
      'p.mod-scope-boundary',
    ]);

    const source = readFileSync(path.resolve(process.cwd(), 'editor/mod-mode-view.js'), 'utf8');
    const dependencies = new Set([
      ...Array.from(source.matchAll(/\bfrom\s+['"]([^'"]+)['"]/g), (match) => match[1]),
      ...Array.from(source.matchAll(/^\s*import\s+['"]([^'"]+)['"]/gm), (match) => match[1]),
    ]);
    expect([...dependencies].sort()).toEqual([
      '../gui/action-feedback.js',
      '../gui/operator-profile.js',
      '../gui/semantic-controls-remapper.js',
      '../gui/strings-boot.js',
      '../gui/strings.js',
      './entity-cache.js',
      './entity-includes.js',
      './mod-actions.js',
      './mod-pack-export.js',
      './mod-pack-workspace.js',
      './project-root.js',
    ]);
    expect(source).toContain("import { readFile as defaultReadFile } from './project-root.js';");
    expect(source).not.toMatch(/\bwriteFile\b|\.\/save-flow\.js|models-mode-view|workshop|inspector/i);
  });
});
