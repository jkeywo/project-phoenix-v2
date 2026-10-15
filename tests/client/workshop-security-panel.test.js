/** @vitest-environment jsdom */
import { expect, it, vi } from 'vitest';
import fs from 'node:fs';
import pathModule from 'node:path';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { inspectSecurity, validateSecurity, prepareSecurityOperation, applySecurityOperation, guardSecurityReferences } from '../../editor/workshop-security.js';
import { mountWorkshopSecurity } from '../../gui/workshop-security-panel.js';

const path = 'assets/entities/test.toml';
const profile = () => ({ team_count: 2, deploy_duration_secs: 8, withdraw_duration_secs: 6, range: 400,
  local: { team_homes: ['bridge', 'bridge'], target: [{ compartment: 'engineering', action: [{ action: 'secure_contain', duration_secs: 6, risk: 0, priority: 'life_safety', outcome_flag: 'contained' }] }] } });
const entity = () => ({ name: 'Hull', system: [{ id: 'security', kind: 'security' }], security: profile(),
  interior: { compartment: [{ id: 'bridge', label: 'interior.room.bridge.label' }, { id: 'engineering', label: 'interior.room.engineering.label' }] },
  energy: { consumer: [{ system: 'security', continuous_cost_per_sec: 1 }] } });
const documentFor = source => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="test"\nname="Test"\nversion="1"\n' }, { path, text: source }]));
const dependencies = { base_files: {}, packs: [] };
const settle = async () => { for (let index = 0; index < 20; index++) await new Promise(resolve => setTimeout(resolve, 1)); };
const control = id => document.getElementById('workshop-security-' + id);

it('validates finite actual local homes/actions/energy and preserves explicit external-only mode', () => {
  expect(() => validateSecurity(profile(), entity())).not.toThrow();
  expect(() => validateSecurity({ ...profile(), local: false }, entity())).not.toThrow();
  for (const change of [value => value.local.team_homes.pop(), value => value.local.team_homes[0] = 'elsewhere',
    value => value.local.target[0].action[0].action = 'board', value => value.local.target[0].action[0].risk = Infinity,
    value => value.local.target[0].action[0].duration_secs = 0, value => value.local.target.push(structuredClone(value.local.target[0])),
    value => value.local.target[0].action.push(structuredClone(value.local.target[0].action[0])), value => value.local.other = 1]) {
    const value = profile(); change(value); expect(() => validateSecurity(value, entity())).toThrow();
  }
  for (const change of [value => value.system = [], value => value.energy.consumer[0].continuous_cost_per_sec = 0, value => value.interior.compartment.pop()]) {
    const value = entity(); change(value); expect(() => guardSecurityReferences(entity(), value)).toThrow();
  }
});

it('materialises inherited work through accepted source/history, preserving unrelated values and explicit disabling', async () => {
  const draft = documentFor('includes=["base.toml"]\n# child stays\nname="Child"\n');
  const deps = { base_files: { 'assets/entities/base.toml': stringify(entity()) }, packs: [] };
  expect(inspectSecurity(draft, deps, path).inherited).toBe(true);
  const value = { ...profile(), local: false }, operation = { path, value };
  expect(() => prepareSecurityOperation(draft, deps, operation)).toThrow(/materialise/);
  const before = draft.read(path), runtime = { validate: vi.fn(async () => ({ accepted: true })) };
  await applySecurityOperation({ draft, dependencies: deps, runtime, operation: { ...operation, materialise: true } });
  expect(inspectSecurity(draft, deps, path).security).toEqual(value);
  expect(draft.read(path)).toContain('# child stays'); expect(parse(draft.read(path)).name).toBe('Child');
  draft.undo(); expect(draft.read(path)).toBe(before); draft.redo();
  const reopened = new WorkshopDocument(draft.archive()); expect(inspectSecurity(reopened, deps, path).security).toEqual(value);
});

it('refuses backend, stale and misleading headers without accepted history writes', async () => {
  const source = stringify(entity()), draft = documentFor(source), operation = { path, value: profile() };
  const runtime = { validate: vi.fn(async () => ({ accepted: false, findings: [{ message: 'route refused' }] })) };
  await expect(applySecurityOperation({ draft, dependencies, runtime, operation })).rejects.toThrow();
  expect(draft.read(path)).toBe(source); expect(draft.snapshot().history.undo).toHaveLength(0);
  runtime.validate.mockResolvedValue({ accepted: true });
  await expect(applySecurityOperation({ draft, dependencies, runtime, operation, current: () => false })).rejects.toThrow();
  const tricky = documentFor('description="""\n[security]\nsecret=1\n"""\n' + source);
  expect(() => prepareSecurityOperation(tricky, dependencies, operation)).toThrow();
});

it('resolves the optional real Cruiser without altering inherited equipment and reference terms', () => {
  const base_files = {};
  const visit = directory => { for (const row of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = pathModule.join(directory, row.name); if (row.isDirectory()) visit(full);
    else if (row.name.endsWith('.toml')) base_files[full.replaceAll('\\', '/')] = fs.readFileSync(full, 'utf8');
  } }; visit('assets/entities');
  const draft = documentFor('includes=["security_cruiser.toml"]\n'), resolved = inspectSecurity(draft, { base_files, packs: [] }, path).resolved.value;
  expect(() => validateSecurity(resolved.security, resolved)).not.toThrow();
  expect(resolved.security.team_count).toBe(2); expect(resolved.security.deploy_duration_secs).toBe(8);
  expect(resolved.security.local.team_homes).toEqual(['bridge', 'bridge']);
  expect(resolved.security.local.target).toHaveLength(3);
  const after = structuredClone(resolved); after.interior.compartment = after.interior.compartment.filter(room => room.id !== 'bridge');
  expect(() => guardSecurityReferences(resolved, after)).toThrow();
});

it('edits structured actual fields, retains dirty focus and invokes existing Preview/Test', async () => {
  document.body.innerHTML = '<main></main>'; const draft = documentFor(stringify(entity())), preview = vi.fn(), test = vi.fn();
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true })) }; let busy = false;
  const panel = mountWorkshopSecurity({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => busy, setBusy: value => { busy = value; }, preview, test }); await settle();
  const duration = panel.node.querySelector('[id*="duration_secs"]'); duration.value = '7'; duration.focus(); panel.refresh();
  expect(document.activeElement).toBe(duration); expect(duration.value).toBe('7'); expect(runtime.validate).not.toHaveBeenCalled();
  control('preview').click(); control('test').click(); expect(preview).toHaveBeenCalledWith(path); expect(test).toHaveBeenCalledWith(path);
  control('save').click(); await settle(); expect(parse(draft.read(path)).security.local.target[0].action[0].duration_secs).toBe(7);
  draft.undo(); panel.refresh(); expect(panel.node.querySelector('[id*="duration_secs"]').value).toBe('6'); panel.dispose();
});

it('discards answers after document replacement and disposal', async () => {
  document.body.innerHTML = '<main></main>'; const source = stringify(entity()); let draft = documentFor(source), answer, busy = false;
  const original = draft, runtime = { dependencies: async () => dependencies, validate: vi.fn(() => new Promise(resolve => { answer = resolve; })) };
  const panel = mountWorkshopSecurity({ root: document.querySelector('main'), runtime, draft: () => draft, busy: () => busy, setBusy: value => { busy = value; } }); await settle();
  control('save').click(); await settle(); expect(control('save').disabled).toBe(true);
  draft = documentFor(source); panel.refresh(); answer({ accepted: true }); await settle();
  expect(original.snapshot().history.undo).toHaveLength(0); expect(draft.snapshot().history.undo).toHaveLength(0); panel.dispose();
});
