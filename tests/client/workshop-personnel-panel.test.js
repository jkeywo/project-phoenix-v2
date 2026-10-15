/** @vitest-environment jsdom */
import { expect, it, vi } from 'vitest';
import fs from 'node:fs';
import pathModule from 'node:path';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { inspectPersonnel, validatePersonnel, preparePersonnelOperation, applyPersonnelOperation, guardPersonnelReferences } from '../../editor/workshop-personnel.js';
import { mountWorkshopPersonnel } from '../../gui/workshop-personnel-panel.js';
const path = 'assets/entities/test.toml';
const entity = () => ({ name: 'Hull', system: [{ id: 'repair', kind: 'repair' }], repair: { repair_team_count: 2 },
  interior: { team_homes: ['engineering', 'engineering'], compartment: [{ id: 'engineering' }] }, damage: {} });
const profile = () => ({ repair_team: [{ id: 'alpha', label: 'Alpha', team_idx: 0, competencies: ['mechanical_repair'] }],
  officer: [{ id: 'chief', label: 'Chief', home_compartment: 'engineering', competencies: ['mechanical_repair'], repair_work_multiplier: 1.25 }] });
const documentFor = source => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
const dependencies = { base_files: {}, packs: [] };
const settle = async () => { for (let i = 0; i < 20; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
const control = id => document.getElementById('workshop-personnel-' + id);

it('validates real bindings and refuses unknown competencies, impossible homes and nonfinite work', () => {
  expect(() => validatePersonnel(profile(), entity())).not.toThrow();
  for (const change of [p => p.repair_team.push({ ...p.repair_team[0] }), p => p.repair_team[0].team_idx = 2,
    p => p.officer[0].home_compartment = 'bridge', p => p.officer[0].competencies = ['magic'],
    p => p.officer[0].repair_work_multiplier = Infinity, p => p.officer[0].repair_work_multiplier = 0.5,
    p => p.officer[0].other = 1]) {
    const value = profile(); change(value); expect(() => validatePersonnel(value, entity())).toThrow();
  }
  for (const change of [e => e.system = [], e => e.repair.repair_team_count = 0, e => delete e.damage]) {
    const value = entity(); change(value); expect(() => validatePersonnel(profile(), value)).toThrow();
  }
});
it('accepts exact source/history and explicit inherited empty arrays, preserving unrelated fields', async () => {
  const draft = documentFor('includes=["base.toml"]\n# child stays\nname="Child"\n');
  const deps = { base_files: { 'assets/entities/base.toml': stringify({ ...entity(), personnel: profile() }) }, packs: [] };
  expect(inspectPersonnel(draft, deps, path).inherited).toBe(true);
  const operation = { path, value: { repair_team: [], officer: [] } };
  expect(() => preparePersonnelOperation(draft, deps, operation)).toThrow(/materialise/);
  const before = draft.read(path), runtime = { validate: vi.fn(async () => ({ accepted: true })) };
  await applyPersonnelOperation({ draft, dependencies: deps, runtime, operation: { ...operation, materialise: true } });
  expect(inspectPersonnel(draft, deps, path).personnel).toEqual(operation.value);
  expect(parse(draft.read(path)).name).toBe('Child'); expect(draft.read(path)).toContain('# child stays');
  draft.undo(); expect(draft.read(path)).toBe(before); draft.redo();
  const reloaded = new WorkshopDocument(draft.archive());
  expect(inspectPersonnel(reloaded, deps, path).personnel).toEqual(operation.value);
});
it('refuses backend, stale and misleading-header candidates without history writes', async () => {
  const source = stringify(entity()), draft = documentFor(source), operation = { path, value: profile() };
  const runtime = { validate: vi.fn(async () => ({ accepted: false, findings: [{ message: 'physical owner refused' }] })) };
  await expect(applyPersonnelOperation({ draft, dependencies, runtime, operation })).rejects.toThrow();
  expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0);
  runtime.validate.mockResolvedValue({ accepted: true });
  await expect(applyPersonnelOperation({ draft, dependencies, runtime, operation, current: () => false })).rejects.toThrow();
  const tricky = documentFor('description="""\n[personnel]\nsecret = 4\n"""\n' + source);
  expect(() => preparePersonnelOperation(tricky, dependencies, operation)).toThrow();
  expect(tricky.snapshot().history.undo).toHaveLength(0);
});
it('uses the resolved optional Cruiser roster and guards actual team/home/owner changes', () => {
  const base_files = {};
  const visit = directory => { for (const row of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = pathModule.join(directory, row.name); if (row.isDirectory()) visit(full);
    else if (row.name.endsWith('.toml')) base_files[full.replaceAll('\\', '/')] = fs.readFileSync(full, 'utf8');
  } }; visit('assets/entities');
  const draft = documentFor('includes=["personnel_cruiser.toml"]\n'), resolved = inspectPersonnel(draft, { base_files, packs: [] }, path).resolved.value;
  expect(() => validatePersonnel(resolved.personnel, resolved)).not.toThrow();
  expect(resolved.personnel.officer[0].repair_work_multiplier).toBe(1.25);
  for (const change of [e => e.repair.repair_team_count = 1, e => e.interior.team_homes.fill('bridge'), e => e.system = e.system.filter(s => s.id !== 'repair')]) {
    const after = structuredClone(resolved); change(after); expect(() => guardPersonnelReferences(resolved, after)).toThrow();
  }
});
it('renders structured real fields, accepts edits, preserves focus and invokes existing Preview/Test', async () => {
  document.body.innerHTML = '<main></main>';
  const source = stringify({ ...entity(), personnel: profile() }), draft = documentFor(source);
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true })) }, preview = vi.fn(), test = vi.fn();
  let busy = false;
  const panel = mountWorkshopPersonnel({ root: document.querySelector('main'), runtime, draft: () => draft,
    busy: () => busy, setBusy: value => { busy = value; }, preview, test }); await settle();
  const input = panel.node.querySelector('input[id*="repair_work_multiplier"]'); input.value = '1.5'; input.focus(); panel.refresh();
  expect(document.activeElement).toBe(input); expect(input.value).toBe('1.5'); expect(runtime.validate).not.toHaveBeenCalled();
  control('preview').click(); control('test').click(); expect(preview).toHaveBeenCalledWith(path); expect(test).toHaveBeenCalledWith(path);
  control('save').click(); await settle(); expect(parse(draft.read(path)).personnel.officer[0].repair_work_multiplier).toBe(1.5);
  draft.undo(); panel.refresh(); expect(draft.read(path)).toBe(source); panel.dispose();
});
it('blocks controls during validation and discards completion after disposal', async () => {
  document.body.innerHTML = '<main></main>'; const source = stringify(entity()), draft = documentFor(source);
  let answer, busy = false;
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(() => new Promise(resolve => { answer = resolve; })) };
  const panel = mountWorkshopPersonnel({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => busy, setBusy: value => { busy = value; } }); await settle();
  control('save').click(); await settle(); expect(control('save').disabled).toBe(true);
  panel.dispose(); answer({ accepted: true }); await settle(); expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0);
});
it('replaces same-revision pack controls before saving its different roster', async () => {
  document.body.innerHTML = '<main></main>';
  let draft = documentFor(stringify({ ...entity(), personnel: profile() })), busy = false;
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true })) };
  const panel = mountWorkshopPersonnel({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => busy, setBusy: value => { busy = value; } }); await settle();
  const replacement = profile(); replacement.officer[0].id = 'other'; replacement.officer[0].label = 'Other';
  draft = documentFor(stringify({ ...entity(), personnel: replacement })); panel.refresh();
  expect(panel.node.querySelector('input[id*="officer-id"]').value).toBe('other');
  control('save').click(); await settle(); expect(parse(draft.read(path)).personnel).toEqual(replacement); panel.dispose();
});

it('validates the real optional Medical hull, finite medication source and structured care history', async () => {
  const base_files = {};
  const visit = directory => { for (const row of fs.readdirSync(directory,{withFileTypes:true})) { const full=pathModule.join(directory,row.name); if(row.isDirectory()) visit(full); else if(row.name.endsWith('.toml')) base_files[full.replaceAll('\\','/')]=fs.readFileSync(full,'utf8'); } }; visit('assets/entities');
  const draft=documentFor('includes=["medical_cruiser.toml"]\n'); const deps={base_files,packs:[]};
  const resolved=inspectPersonnel(draft,deps,path).resolved.value;
  expect(()=>validatePersonnel(resolved.personnel,resolved)).not.toThrow();
  expect(resolved.personnel.officer[0].initial_condition).toBe('minor');
  expect(resolved.personnel.care.requirement[0].resource).toBe('medical_supplies');
  for (const change of [e=>e.personnel.care.minor_work_secs=Infinity,e=>e.personnel.care.requirement[0].amount=999,e=>e.personnel.repair_team[1].competencies=['mechanical_repair'],e=>delete e.resource_services.medical_supplies,e=>e.logistics.store=e.logistics.store.filter(store=>store.id!=='local-medication')]) {
    const bad=structuredClone(resolved);change(bad);expect(()=>validatePersonnel(bad.personnel,bad)).toThrow();
  }
  document.body.innerHTML='<main></main>';let busy=false;
  const runtime={dependencies:async()=>deps,validate:vi.fn(async()=>({accepted:true}))};
  const panel=mountWorkshopPersonnel({root:document.querySelector('main'),runtime,draft:()=>draft,busy:()=>busy,setBusy:value=>{busy=value;}});await settle();
  expect(control('care-enabled').checked).toBe(true); control('materialise').checked=true;
  control('care-minor').value='8';control('save').click();await settle();
  expect(inspectPersonnel(draft,deps,path).personnel.care.minor_work_secs).toBe(8);
  expect(inspectPersonnel(draft,deps,path).personnel.repair_team[1].competencies).toContain('medical_care');
  draft.undo();panel.refresh();expect(control('care-minor').value).toBe('6');
  control('materialise').checked=true;control('care-enabled').checked=false;control('save').click();await settle();
  expect(inspectPersonnel(draft,deps,path).personnel.care).toBe(false);panel.dispose();
});
