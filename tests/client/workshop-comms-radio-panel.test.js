/** @vitest-environment jsdom */
import { expect, it, vi } from 'vitest';
import { parse, stringify } from 'smol-toml';
import { WorkshopDocument } from '../../editor/workshop-document.js';
import { createStoreZip } from '../../editor/mod-pack-export.js';
import { mountWorkshopCommsRadio } from '../../gui/workshop-comms-radio-panel.js';
const path = 'assets/entities/test.toml';
const entity = () => ({ name: 'Radio endpoint', comms: { range: 100, hailable: true, display_name: 'Keep label' },
  console: [{ id: 'comms-room' }], system: [{ id: 'comms', kind: 'comms', console: 'comms-room' }],
  energy: { consumer: [{ system: 'comms', continuous_cost_per_sec: .05, discrete_cost: 1 }] } });
const documentFor = source => new WorkshopDocument(createStoreZip([{ path: 'scenarios.toml', text: '[pack]\nformat=1\nid="radio"\nname="Radio"\nversion="1"\n' }, { path, text: source }]));
const control = id => document.getElementById('workshop-comms-radio-' + id);
const settle = async () => { for (let i = 0; i < 15; i++) await new Promise(resolve => setTimeout(resolve, 1)); };
function fixture(source = stringify(entity()), dependencies = { base_files: {}, packs: [] }) {
  document.body.innerHTML = '<main></main>'; let draft = documentFor(source), busy = false;
  const runtime = { dependencies: async () => dependencies, validate: vi.fn(async () => ({ accepted: true })) }, preview = vi.fn(() => true), test = vi.fn(() => true);
  const panel = mountWorkshopCommsRadio({ root: document.querySelector('main'), draft: () => draft, runtime,
    busy: () => busy, setBusy: value => { busy = value; }, preview, test });
  return { panel, runtime, preview, test, draft: () => draft, replace: value => { draft = documentFor(value); } };
}
const field = (kind, name) => document.querySelector(`[id*="workshop-comms-radio-${kind}-${name}-"]`);
it('adds structured channels and held keys through actual accepted draft history and Preview/Test', async () => {
  const f = fixture(); await settle(); control('add-channel').click(); control('add-key').click();
  field('channel', 'id').value = 'operations'; field('channel', 'range').value = '70'; field('channel', 'work_units').value = '2';
  field('key', 'identity').value = 'crew-key'; field('channel', 'key').value = 'crew-key';
  for (const input of f.panel.node.querySelectorAll('input,select')) expect(f.panel.node.querySelector(`label[for="${input.id}"]`)).not.toBeNull();
  const input = field('channel', 'range'); input.focus(); f.panel.refresh(); expect(field('channel', 'range')).toBe(input); expect(document.activeElement).toBe(input); expect(input.value).toBe('70');
  control('save').click(); await settle(); expect(f.runtime.validate).toHaveBeenCalledOnce();
  expect(parse(f.draft().read(path)).comms).toEqual({ ...entity().comms, radio: { system: 'comms', keys: ['crew-key'], channel: [{ id: 'operations', range: 70, work_units: 2, key: 'crew-key' }] } });
  f.draft().undo(); f.panel.refresh(); expect(control('channels').children.length).toBe(0); f.draft().redo(); f.panel.refresh(); expect(field('channel', 'id').value).toBe('operations');
  control('preview').click(); control('test').click(); expect(f.preview).toHaveBeenCalledWith(path); expect(f.test).toHaveBeenCalledWith(path); f.panel.dispose();
});
it('requires inherited consent and never silently opens a channel when its held key is removed', async () => {
  const parent = entity(); parent.comms.radio = { system: 'comms', keys: ['crew-key'], channel: [{ id: 'secure', range: 80, work_units: 1, key: 'crew-key' }] };
  const child = 'includes=["base.toml"]\n# preserve child\n', f = fixture(child, { base_files: { 'assets/entities/base.toml': stringify(parent) }, packs: [] }); await settle();
  field('channel', 'range').value = '60'; control('save').click(); await settle(); expect(f.runtime.validate).not.toHaveBeenCalled(); expect(f.draft().read(path)).toBe(child);
  control('materialise').checked = true; control('save').click(); await settle(); expect(f.runtime.validate).toHaveBeenCalledOnce();
  const before = f.draft().read(path); control('keys').querySelector('button').click(); control('save').click(); await settle();
  expect(f.draft().read(path)).toBe(before); expect(parse(before).comms.radio.channel[0].key).toBe('crew-key');
  expect(f.runtime.validate).toHaveBeenCalledOnce(); f.panel.dispose();
});
it('refreshes a same-revision replacement document and refuses invalid equipment and late acceptance', async () => {
  const f = fixture(); await settle(); const replacement = entity(); replacement.comms.radio = { system: 'comms', keys: [], channel: [{ id: 'replacement', range: 40, work_units: 3 }] };
  f.replace(stringify(replacement)); f.panel.refresh(); expect(field('channel', 'id').value).toBe('replacement');
  let answer; f.runtime.validate.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; })); control('save').click(); await settle();
  const before = f.draft().read(path); expect(control('save').disabled).toBe(true); f.panel.dispose(); answer({ accepted: true }); await settle();
  expect(f.draft().read(path)).toBe(before); expect(f.draft().snapshot().history.undo).toHaveLength(0);
  const invalid = entity(); invalid.energy.consumer[0].discrete_cost = 0; const bad = fixture(stringify(invalid)); await settle(); expect(control('save').disabled).toBe(true); expect(control('add-channel').disabled).toBe(true); bad.panel.dispose();
});

it('round trips actual equipment media capability through the existing Workshop transaction and history', async () => {
  const authored = entity(); authored.comms.radio = {system:'comms', media:'audio_video', keys:[],
    channel:[{id:'open',range:80,work_units:1}]};
  const f = fixture(stringify(authored)); await settle();
  expect(control('media').value).toBe('audio_video');
  control('media').value = 'audio'; control('save').click(); await settle();
  expect(parse(f.draft().read(path)).comms.radio.media).toBe('audio');
  f.draft().undo(); f.panel.refresh(); expect(control('media').value).toBe('audio_video');
  f.draft().redo(); f.panel.refresh(); expect(control('media').value).toBe('audio');
  control('media').value = ''; control('save').click(); await settle();
  expect(parse(f.draft().read(path)).comms.radio.media).toBeUndefined();
  f.panel.dispose();
});
it('edits authored request reading ticks and refuses a same-revision document swap during validation', async () => {
  const authored = entity(); authored.comms.radio = { system: 'comms', request_reading_ticks: 45, keys: [], channel: [{id:'open',range:80,work_units:1}] };
  const f=fixture(stringify(authored)); await settle(); expect(control('request-reading').value).toBe('45');
  control('request-reading').value='92'; control('save').click(); await settle();
  expect(parse(f.draft().read(path)).comms.radio.request_reading_ticks).toBe(92);
  let answer; f.runtime.validate.mockImplementationOnce(() => new Promise(resolve => {answer=resolve;}));
  control('request-reading').value='17'; control('save').click(); await settle();
  const beforeOld=f.draft().read(path); const old=f.draft();
  const replacement=entity(); replacement.comms.radio={system:'comms',request_reading_ticks:12,keys:[],channel:[{id:'other',range:40,work_units:2}]};
  f.replace(stringify(replacement)); const beforeNew=f.draft().read(path); f.panel.refresh();
  answer({accepted:true}); await settle();
  expect(old.read(path)).toBe(beforeOld); expect(f.draft().read(path)).toBe(beforeNew);
  expect(control('request-reading').value).toBe('12'); f.panel.dispose();
});

it('edits relay forwarding as an explicit channel capability with undo and reload', async () => {
  const f=fixture(); await settle(); control('add-channel').click();
  field('channel','id').value='open';field('channel','range').value='80';field('channel','work_units').value='1';
  expect(field('channel','relay').checked).toBe(false);field('channel','relay').checked=true;
  control('save').click();await settle();
  expect(parse(f.draft().read(path)).comms.radio.channel[0].relay).toBe(true);
  f.draft().undo();f.panel.refresh();expect(control('channels').children.length).toBe(0);
  f.draft().redo();f.panel.refresh();expect(field('channel','relay').checked).toBe(true);
  field('channel','relay').checked=false;control('save').click();await settle();
  expect(parse(f.draft().read(path)).comms.radio.channel[0].relay).toBeUndefined();f.panel.dispose();
});
