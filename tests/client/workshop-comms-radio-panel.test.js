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
it('authors explicit finite listening and contextual alert choices and retires disposed buttons', async () => {
  const endpoint=entity();endpoint.console[0].feed=[{source:'captain',kind:'Captain'}];endpoint.system.push(
    {id:'captain',kind:'captain',console:'comms-room'},{id:'alert',kind:'red_alert',console:'comms-room'});
  endpoint.comms.radio={system:'comms',keys:[],channel:[{id:'open',range:60,work_units:1},{id:'fleet',range:60,work_units:1}]};
  const f=fixture(stringify(endpoint));await settle();
  control('listening-enabled').checked=true;control('coverage').value='1';control('priority').value='fleet\nopen';
  control('alert-enabled').checked=true;control('alert-source').value='captain';control('alert-channel').value='open';control('alert-spacing').value='90';
  control('routine-text').value='We selected routine alert.';control('red-text').value='We selected red alert.';
  control('save').click();await settle();
  const traffic=parse(f.draft().read(path)).comms.radio.traffic;expect(traffic.listening).toEqual({simultaneous_channels:1,priority:['fleet','open']});expect(traffic.alert.source).toBe('captain');
  f.draft().undo();f.panel.refresh();expect(control('listening-enabled').checked).toBe(false);f.draft().redo();f.panel.refresh();expect(control('priority').value).toBe('fleet\nopen');
  const before=f.draft().read(path),save=control('save'),preview=control('preview'),test=control('test');f.runtime.validate.mockClear();f.panel.dispose();
  save.dispatchEvent(new MouseEvent('click'));preview.dispatchEvent(new MouseEvent('click'));test.dispatchEvent(new MouseEvent('click'));
  expect(f.draft().read(path)).toBe(before);expect(f.runtime.validate).not.toHaveBeenCalled();expect(f.preview).not.toHaveBeenCalled();expect(f.test).not.toHaveBeenCalled();
});
it('authors optional capture and decoder equipment with normal accepted history', async () => {
  const f = fixture(); await settle(); control('add-channel').click();
  field('channel', 'id').value = 'monitor'; field('channel', 'range').value = '60'; field('channel', 'work_units').value = '1';
  control('capture-enabled').checked = true; control('capture-level').value = '1'; control('capture-ticks').value = '45'; control('capture-key-ticks').value = '2';
  field('channel', 'protection_level').value = '3'; control('save').click(); await settle();
  const radio = parse(f.draft().read(path)).comms.radio;
  expect(radio.capture).toEqual({ max_level: 1, ticks_per_level: 45, held_key_ticks: 2 }); expect(radio.channel[0].protection_level).toBe(3);
  f.draft().undo(); f.panel.refresh(); expect(control('capture-enabled').checked).toBe(false);
  f.draft().redo(); f.panel.refresh(); expect(control('capture-enabled').checked).toBe(true); expect(control('capture-ticks').value).toBe('45');
  control('capture-enabled').checked = false; control('save').click(); await settle();
  expect(parse(f.draft().read(path)).comms.radio.capture).toBeUndefined(); f.panel.dispose();
});
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
