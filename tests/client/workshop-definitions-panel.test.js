// @vitest-environment jsdom
import { beforeEach, afterEach, it, expect, vi } from 'vitest';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopDefinitions } from '../../gui/workshop-definitions-panel.js';
import { t } from '../../gui/strings.js';

const A = 'aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa';
const B = 'bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb';
const C = 'cccccccc-3333-4333-8333-cccccccccccc';
const D = 'dddddddd-4444-4444-8444-dddddddddddd';
const MINE = 'assets/factions/mine.toml', ALLIANCE = 'assets/factions/alliance.toml', HULL = 'assets/entities/hull.toml';
const mineSource = `# mine\r\nuuid = "${A}"\r\nname = "Mine"\r\nenemies = [\r\n    "${B}", # Alliance\r\n]\r\nbanner = "keep"\r\n`;
const hullSource = '[[system]]\nid = "helm-lateral-thrust"\nkind = "thruster"\nstation = "helm"\n\n[[station]]\nid = "helm"\n'
  + 'name = "Helm"\nhuman_seeking = true\nvisiting_rating = "Std"\n\n[[station.rating]]\nname = "Std"\nautomated_systems = []\n\n'
  + '[[station.rating]]\nname = "Simplified"\nautomated_systems = ["helm-lateral-thrust", "weapons-phaser"]\n'
  + '[station.rating.ai_tuning]\ntorpedo_auto_fire = {}\n';
const catalog = () => ({
  factions: [
    { path: MINE, origin: 'draft', uuid: A, uuid_line: 2, name: 'Mine', name_line: 3, display_name: null, display_name_line: null,
      enemies: [{ uuid: B, line: 5, name: 'Alliance' }], compliance: null, unknown_keys: ['banner'] },
    { path: ALLIANCE, origin: 'base', uuid: B, uuid_line: 1, name: 'Alliance', name_line: 2, display_name: 'faction.alliance.display_name',
      display_name_line: 3, enemies: [], compliance: null, unknown_keys: [] },
    { path: 'assets/factions/pirate.toml', origin: 'pack:raiders', uuid: C, uuid_line: 1, name: 'Pirate', name_line: 2,
      display_name: null, display_name_line: null, enemies: [{ uuid: A, line: 3, name: 'Mine' }], compliance: null, unknown_keys: [] },
  ],
  hulls: [{ path: HULL, origin: 'draft', stations: [{ id: 'helm', name: 'Helm', line: 6, human_seeking: true,
    visiting_rating: { source: '"Std"', line: 10 }, systems: [{ id: 'helm-lateral-thrust', kind: 'thruster', line: 2 }],
    ratings: [{ index: 0, name: 'Std', line: 12, automated_systems: [], ai_tuning: [] },
      { index: 1, name: 'Simplified', line: 16, automated_systems: [{ id: 'helm-lateral-thrust', line: 18, owned: true },
        { id: 'weapons-phaser', line: 18, owned: false }], ai_tuning: [{ rule: 'torpedo_auto_fire', line: 20 }] }] },
  // A station that seats no human, authored with a visiting rating the runtime would refuse.
  { id: 'tactical', name: 'Tactical', line: 24, human_seeking: false, visiting_rating: { source: '"Std"', line: 26 }, systems: [],
    ratings: [{ index: 0, name: 'Std', line: 28, automated_systems: [], ai_tuning: [] }] }] }],
  choices: { factions: [{ uuid: A, name: 'Mine', origin: 'draft' }, { uuid: B, name: 'Alliance', origin: 'base' },
    { uuid: C, name: 'Pirate', origin: 'pack:raiders' }], order_responses: ['comply', 'refuse'], ai_rules: ['torpedo_auto_fire'] },
  defaults: { compliance: { ack_secs: 2, decide_secs: 3, hold: 'comply', divert: 'comply', dock: 'comply' } },
  findings: [{ severity: 'error', category: 'rating-unowned-system', message: 'weapons-phaser is not a helm system', file: HULL, line: 18 }],
});

let draft, runtime, panel, held, changed, win;
const byId = id => document.getElementById(`workshop-definitions-${id}`);
const section = () => document.getElementById('workshop-definitions');
const change = node => node.dispatchEvent(new Event('change'));
async function read() {
  byId('refresh').click();
  await vi.waitFor(() => expect(byId('name')).not.toBeNull());
}
beforeEach(() => {
  document.body.innerHTML = '<main id="root"></main>';
  draft = new WorkshopDocument(createStoreZip([{ path: MINE, text: mineSource }, { path: HULL, text: hullSource }]), { kind: 'project' });
  held = false;
  runtime = { definitions: vi.fn(async () => catalog()),
    edit: vi.fn(async (source, request) => `${source}# edited ${request.edits.length}\r\n`),
    newFaction: vi.fn(async (name, uuid) => `uuid = "${uuid}"\nname = "${name}"\nenemies = []\n`) };
  changed = vi.fn();
  win = { confirm: vi.fn(() => true), crypto: { randomUUID: () => D } };
  panel = mountWorkshopDefinitions({ root: document.getElementById('root'), runtime, draft: () => draft, win,
    busy: () => held, setBusy(value) { held = value; panel?.refresh(); }, changed });
});
afterEach(() => panel.dispose());

it('reads the draft through the runtime, edits draft factions and shows base and pack ones read-only', async () => {
  expect(byId('status').textContent).toBe(t('workshop.definitions.empty'));
  expect(byId('name')).toBeNull();
  await read();
  expect(runtime.definitions).toHaveBeenCalledExactlyOnceWith({ [MINE]: mineSource, [HULL]: hullSource });
  expect(byId('status').textContent).toBe(t('workshop.definitions.refreshed'));
  expect([...byId('faction').options].map(option => option.value)).toEqual([MINE, ALLIANCE, 'assets/factions/pirate.toml']);
  expect(byId('faction').options[2].textContent).toBe('Pirate (pack:raiders)');
  expect(byId('name').value).toBe('Mine');
  expect(byId('name').disabled).toBe(false);
  expect(byId('apply-faction').disabled).toBe(false);
  expect([...byId('enemies').querySelectorAll('li')].map(row => row.firstChild.textContent)).toEqual([`Alliance (${B})`]);
  // Itself and the enemy it already lists are not offered.
  expect([...byId('add-enemy').options].map(option => option.value)).toEqual([C]);
  expect([...byId('unknown-keys').querySelectorAll('li')].map(row => row.textContent)).toEqual(['banner']);
  expect(byId('materialise-compliance')).not.toBeNull();
  expect([...byId('findings').querySelectorAll('li')].map(row => row.textContent))
    .toEqual([`${HULL}:18 — ${t('workshop.severity.error')}: weapons-phaser is not a helm system`]);
  byId('faction').value = ALLIANCE; change(byId('faction'));
  expect(byId('name').value).toBe('Alliance');
  expect(byId('name').disabled).toBe(true);
  expect(byId('display-name').value).toBe('faction.alliance.display_name');
  expect(byId('apply-faction').disabled).toBe(true);
  expect(byId('delete-faction').disabled).toBe(true);
  expect(section().querySelector('.workshop-definitions-origin').textContent)
    .toBe(t('workshop.definitions.read_only_origin', { origin: 'base' }));
  byId('apply-faction').click(); byId('delete-faction').click();
  expect(runtime.edit).not.toHaveBeenCalled(); expect(win.confirm).not.toHaveBeenCalled();
});

it('adds an enemy as ONE runtime edit holding one insert and ONE history entry that undo reverts', async () => {
  await read();
  byId('add-enemy').value = C; byId('add-enemy-button').click();
  expect(byId('enemies').querySelectorAll('li')).toHaveLength(2);
  expect(byId('add-enemy').options).toHaveLength(0);
  expect(draft.canUndo()).toBe(false); // Nothing reaches the draft before Apply.
  byId('apply-faction').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(MINE));
  expect(runtime.edit).toHaveBeenCalledExactlyOnceWith(mineSource, { document_path: MINE, expected_source: mineSource,
    edits: [{ op: 'insert', path: ['enemies'], index: 1, value_source: `"${C}"` }] });
  expect(draft.read(MINE)).toBe(`${mineSource}# edited 1\r\n`);
  // The applied source is read again so the forms show what was written.
  await vi.waitFor(() => expect(runtime.definitions).toHaveBeenCalledTimes(2));
  expect(runtime.definitions.mock.calls[1][0][MINE]).toBe(`${mineSource}# edited 1\r\n`);
  expect(draft.undo()).toBe(MINE);
  expect(draft.read(MINE)).toBe(mineSource);
  expect(draft.canUndo()).toBe(false);
});

it('removes an enemy and renames in the same single edit, and reports no-op forms without calling the runtime', async () => {
  await read();
  byId('apply-faction').click();
  expect(byId('status').textContent).toBe(t('workshop.definitions.unchanged'));
  expect(runtime.edit).not.toHaveBeenCalled();
  byId('name').value = 'Mine Two'; byId('name').dispatchEvent(new Event('input'));
  byId('enemy-remove-0').click();
  expect(byId('enemies').querySelectorAll('li')).toHaveLength(0);
  expect([...byId('add-enemy').options].map(option => option.value)).toEqual([B, C]);
  byId('apply-faction').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(MINE));
  expect(runtime.edit.mock.calls[0][1].edits).toEqual([
    { op: 'set', path: ['name'], value_source: '"Mine Two"' }, { op: 'remove', path: ['enemies', 0] }]);
});

it('materialises compliance from the runtime defaults and applies it as one put', async () => {
  await read();
  byId('materialise-compliance').click();
  expect(byId('materialise-compliance')).toBeNull();
  expect(byId('ack-secs').value).toBe('2');
  expect(byId('decide-secs').value).toBe('3');
  expect([...byId('hold').options].map(option => option.value)).toEqual(['comply', 'refuse']);
  byId('hold').value = 'refuse'; change(byId('hold'));
  byId('apply-faction').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(MINE));
  expect(runtime.edit.mock.calls[0][1].edits).toEqual([{ op: 'put', path: ['compliance'],
    value_source: '{ ack_secs = 2, decide_secs = 3, hold = "refuse", divert = "comply", dock = "comply" }' }]);
});

it('leaves the draft untouched when the runtime refuses and when the draft moved during the edit', async () => {
  await read();
  runtime.edit.mockRejectedValueOnce(new Error('refused'));
  byId('add-enemy').value = C; byId('add-enemy-button').click();
  byId('apply-faction').click();
  await vi.waitFor(() => expect(byId('status').getAttribute('role')).toBe('alert'));
  expect(byId('status').textContent).toBe(t('workshop.inspector_refused'));
  expect(draft.canUndo()).toBe(false);
  expect(changed).not.toHaveBeenCalled();
  expect(byId('apply-faction').disabled).toBe(false); // The reading is still current.
  let complete;
  runtime.edit.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  byId('apply-faction').click();
  await vi.waitFor(() => expect(held).toBe(true));
  draft.edit(HULL, `${hullSource}# moved\n`);
  complete(`${mineSource}# late\r\n`);
  await vi.waitFor(() => expect(held).toBe(false));
  expect(draft.read(MINE)).toBe(mineSource);
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
  expect(changed).not.toHaveBeenCalled();
});

it('disables the forms once any text member changed and hides the panel under a Test hold', async () => {
  await read();
  draft.edit(HULL, `${hullSource}# newer\n`); panel.refresh();
  expect(byId('status').textContent).toBe(t('workshop.inspector_stale'));
  for (const id of ['name', 'display-name', 'add-enemy-button', 'apply-faction', 'delete-faction', 'faction', 'hull', 'station',
    'visiting-rating', 'apply-hull', 'new-faction', 'create']) expect(byId(id).disabled).toBe(true);
  expect(byId('refresh').disabled).toBe(false);
  byId('apply-faction').click(); byId('apply-hull').click(); byId('create').click();
  expect(runtime.edit).not.toHaveBeenCalled(); expect(runtime.newFaction).not.toHaveBeenCalled();
  draft.undo(); panel.refresh();
  expect(byId('apply-faction').disabled).toBe(false);
  held = true; panel.refresh({ hidden: true });
  expect(section().hidden).toBe(true);
  for (const id of ['refresh', 'name', 'apply-faction', 'apply-hull', 'create']) expect(byId(id).disabled).toBe(true);
  byId('apply-faction').click(); byId('refresh').click();
  expect(runtime.edit).not.toHaveBeenCalled(); expect(runtime.definitions).toHaveBeenCalledTimes(1);
  held = false; panel.refresh({ hidden: false });
  expect(section().hidden).toBe(false);
  expect(byId('apply-faction').disabled).toBe(false);
});

it('does not display a reading that belongs to an older draft', async () => {
  let complete;
  runtime.definitions.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  byId('refresh').click();
  draft.edit(MINE, `${mineSource}# changed during read\r\n`);
  complete(catalog());
  await vi.waitFor(() => expect(held).toBe(false));
  expect(byId('name')).toBeNull();
  expect(byId('apply-faction').disabled).toBe(true);
});

it('creates a faction at its slug path from the runtime skeleton and refuses an existing path', async () => {
  await read();
  byId('new-faction').value = 'Mine'; byId('create').click();
  expect(byId('status').getAttribute('role')).toBe('alert');
  expect(byId('status').textContent).toBe(t('workshop.definitions.faction_exists'));
  expect(runtime.newFaction).not.toHaveBeenCalled();
  byId('new-faction').value = '!!!'; byId('create').click();
  expect(byId('status').textContent).toBe(t('workshop.definitions.invalid_name'));
  byId('new-faction').value = 'Harrow Navy'; byId('create').click();
  const path = 'assets/factions/harrow-navy.toml';
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(path));
  expect(runtime.newFaction).toHaveBeenCalledExactlyOnceWith('Harrow Navy', D);
  expect(draft.read(path)).toBe(`uuid = "${D}"\nname = "Harrow Navy"\nenemies = []\n`);
  expect(byId('new-faction').value).toBe('');
  expect(draft.undo()).toBe(path);
  expect(draft.paths()).not.toContain(path);
});

it('deletes a draft faction only after confirmation, as one reversible history entry', async () => {
  await read();
  win.confirm.mockReturnValueOnce(false);
  byId('delete-faction').click();
  expect(win.confirm).toHaveBeenCalledWith(t('workshop.definitions.delete_confirm', { path: MINE }));
  expect(draft.paths()).toContain(MINE);
  expect(changed).not.toHaveBeenCalled();
  byId('delete-faction').click();
  expect(draft.paths()).not.toContain(MINE);
  expect(changed).toHaveBeenCalledWith(HULL);
  await vi.waitFor(() => expect(held).toBe(false));
  expect(draft.undo()).toBe(MINE);
  expect(draft.read(MINE)).toBe(mineSource);
});

it('adds a rung through append_table and refuses a duplicate rung name before the runtime', async () => {
  await read();
  expect(byId('hull').value).toBe(HULL);
  expect(byId('station').value).toBe('0');
  expect(byId('human-seeking').checked).toBe(true);
  expect(byId('human-seeking').disabled).toBe(true);
  byId('new-rung').value = 'Std'; byId('add-rung').click();
  expect(byId('status').textContent).toBe(t('workshop.definitions.rung_exists'));
  byId('new-rung').value = 'Expert'; byId('add-rung').click();
  expect(byId('rung-2-name').value).toBe('Expert');
  expect([...byId('visiting-rating').options].map(option => option.value)).toEqual(['', 'Std', 'Simplified', 'Expert']);
  byId('rung-2-system-0').checked = true; change(byId('rung-2-system-0'));
  byId('apply-hull').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(HULL));
  expect(runtime.edit).toHaveBeenCalledExactlyOnceWith(hullSource, { document_path: HULL, expected_source: hullSource,
    edits: [{ op: 'append_table', path: ['station', 0, 'rating'],
      fields: [['name', '"Expert"'], ['automated_systems', '["helm-lateral-thrust"]']] }] });
  expect(draft.undo()).toBe(HULL);
});

it('clears the visiting rating with a remove and drops an unowned automated system from its checked row', async () => {
  await read();
  const unowned = byId('rung-1-unowned-0');
  expect(unowned.checked).toBe(true);
  const label = section().querySelector(`label[for="${unowned.id}"]`).textContent;
  expect(label).toContain('weapons-phaser');
  expect(label).toContain(t('workshop.definitions.unavailable_system'));
  expect(label).toContain('weapons-phaser is not a helm system');
  expect(byId('rung-1-rule-0').checked).toBe(true);
  expect(byId('rung-0-rule-0').checked).toBe(false);
  unowned.checked = false; change(unowned);
  expect(byId('rung-1-unowned-0')).toBeNull();
  byId('visiting-rating').value = ''; change(byId('visiting-rating'));
  byId('rung-0-rule-0').checked = true; change(byId('rung-0-rule-0'));
  byId('rung-1-remove').click();
  expect(byId('rung-1-name')).toBeNull();
  byId('apply-hull').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(HULL));
  expect(runtime.edit.mock.calls[0][1].edits).toEqual([
    { op: 'remove', path: ['station', 0, 'visiting_rating'] },
    { op: 'put', path: ['station', 0, 'rating', 0, 'ai_tuning', 'torpedo_auto_fire'], value_source: '{}' },
    { op: 'remove', path: ['station', 0, 'rating', 1] },
  ]);
});

it('offers a station that seats no human only the clearing of a visiting rating it already has', async () => {
  await read();
  byId('station').value = '1'; change(byId('station'));
  expect(byId('human-seeking').checked).toBe(false);
  expect([...byId('visiting-rating').options].map(option => option.value)).toEqual(['', 'Std']);
  expect(byId('visiting-rating').disabled).toBe(false);
  byId('visiting-rating').value = ''; change(byId('visiting-rating'));
  byId('apply-hull').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(HULL));
  expect(runtime.edit.mock.calls[0][1].edits).toEqual([{ op: 'remove', path: ['station', 1, 'visiting_rating'] }]);
  // With nothing to clear there is nothing to choose, so the control is read-only.
  const cleared = catalog();
  cleared.hulls[0].stations[1].visiting_rating = null;
  runtime.definitions.mockResolvedValueOnce(cleared);
  byId('refresh').click();
  await vi.waitFor(() => expect(byId('status').textContent).toBe(t('workshop.definitions.refreshed')));
  expect(byId('station').value).toBe('1');
  expect([...byId('visiting-rating').options].map(option => option.value)).toEqual(['']);
  expect(byId('visiting-rating').disabled).toBe(true);
  byId('station').value = '0'; change(byId('station'));
  expect(byId('visiting-rating').disabled).toBe(false);
  expect([...byId('visiting-rating').options].map(option => option.value)).toEqual(['', 'Std', 'Simplified']);
});

it('keeps keyboard focus inside the form after every removal', async () => {
  await read();
  byId('enemy-remove-0').focus();
  byId('enemy-remove-0').click();
  expect(document.activeElement).toBe(byId('add-enemy'));
  byId('rung-1-unowned-0').focus();
  byId('rung-1-unowned-0').checked = false; change(byId('rung-1-unowned-0'));
  expect(document.activeElement).toBe(byId('rung-1-system-0'));
  byId('new-rung').value = 'Expert'; byId('add-rung').click();
  byId('rung-1-remove').focus();
  byId('rung-1-remove').click();
  expect(byId('rung-1-name')).toBeNull();
  expect(document.activeElement).toBe(byId('rung-2-name'));
  byId('rung-2-remove').click();
  expect(document.activeElement).toBe(byId('rung-0-name'));
  byId('rung-0-remove').click();
  expect(document.activeElement).toBe(byId('new-rung'));
});

it('keeps the unapplied edits of one form while the other form is applied', async () => {
  await read();
  byId('rung-0-name').value = 'Novice'; byId('rung-0-name').dispatchEvent(new Event('input'));
  byId('name').value = 'Mine Two'; byId('name').dispatchEvent(new Event('input'));
  byId('apply-faction').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(MINE));
  await vi.waitFor(() => expect(runtime.definitions).toHaveBeenCalledTimes(2));
  // The faction form shows what was written; the hull member did not move, so its form keeps the typing.
  expect(byId('name').value).toBe('Mine');
  expect(byId('rung-0-name').value).toBe('Novice');
  byId('display-name').value = 'faction.mine.display_name'; byId('display-name').dispatchEvent(new Event('input'));
  byId('apply-hull').click();
  await vi.waitFor(() => expect(changed).toHaveBeenCalledWith(HULL));
  await vi.waitFor(() => expect(runtime.definitions).toHaveBeenCalledTimes(3));
  expect(runtime.edit.mock.calls[1][1].edits).toEqual([{ op: 'set', path: ['station', 0, 'rating', 0, 'name'], value_source: '"Novice"' }]);
  expect(byId('rung-0-name').value).toBe('Std');
  expect(byId('display-name').value).toBe('faction.mine.display_name');
  // A member that changed under the form is read afresh, typing and all.
  draft.edit(HULL, `${hullSource}# elsewhere\n`);
  byId('rung-0-name').value = 'Lost'; byId('rung-0-name').dispatchEvent(new Event('input'));
  byId('refresh').click();
  await vi.waitFor(() => expect(runtime.definitions).toHaveBeenCalledTimes(4));
  expect(byId('rung-0-name').value).toBe('Std');
  expect(byId('display-name').value).toBe('faction.mine.display_name');
});

it('labels every control, gives every button text and every group a legend, and keeps ids unique', async () => {
  await read();
  byId('materialise-compliance').click();
  byId('new-rung').value = 'Expert'; byId('add-rung').click();
  const controls = [...section().querySelectorAll('input, select')];
  expect(controls.length).toBeGreaterThan(12);
  for (const control of controls) {
    expect(control.id).toBeTruthy();
    expect(section().querySelector(`label[for="${control.id}"]`), control.id).not.toBeNull();
    expect(control.tabIndex).toBeGreaterThanOrEqual(0);
  }
  const buttons = [...section().querySelectorAll('button')];
  expect(buttons.length).toBeGreaterThan(8);
  for (const button of buttons) {
    expect(button.getAttribute('type')).toBe('button');
    expect(button.textContent.trim()).not.toBe('');
  }
  for (const group of section().querySelectorAll('fieldset')) {
    expect(group.firstElementChild.tagName).toBe('LEGEND');
    expect(group.firstElementChild.textContent.trim()).not.toBe('');
  }
  for (const list of section().querySelectorAll('ul')) expect([...list.children].every(child => child.tagName === 'LI')).toBe(true);
  const ids = [...section().querySelectorAll('[id]')].map(element => element.id);
  expect(new Set(ids).size).toBe(ids.length);
  expect(byId('status').getAttribute('tabindex')).toBe('-1');
});
