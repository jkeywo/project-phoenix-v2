// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createGmCommsPanel } from '../../gui/gm-comms-panel.js';
import { localiseTree, setTable } from '../../gui/strings.js';
import { createHostChannel } from '../../gui/host-channel.js';
import { createGmConfirmationController, createGmConfirmationProfile } from '../../gui/gm-confirmation.js';

const payload = () => ({ routes: [{ id: 'private', label: 'Private', visibility: 'selected_ships',
  senders: [{ id: 'speaker', name: 'Axiom' }], hails: [{ id: 'offer', name: 'Offer' }] },
{ id: 'fleet', label: 'Fleet', visibility: 'fleet', senders: [{ id: 'speaker', name: 'Axiom' }], hails: [] }],
recipients: [{ id: 'ship-a', name: 'Alpha' }, { id: 'ship-b', name: 'Beta' }], results: [], max_text_bytes: 64 });
function mount(extra = {}) {
  let id = 0;
  document.body.innerHTML = `<select id="gm-comms-route"></select><select id="gm-comms-sender"></select>
    <select id="gm-comms-recipients" multiple></select><textarea id="gm-comms-text"></textarea><select id="gm-comms-hail"></select>
    <button id="gm-comms-send"></button><button id="gm-comms-start-hail"></button><p id="gm-comms-feedback"></p>
    <p id="gm-comms-count"></p><p id="gm-comms-empty"></p><ol id="gm-comms-log"></ol>`;
  const submitTransmission = vi.fn(() => true), correlation = vi.fn(() => `comms-${++id}`);
  const panel = createGmCommsPanel({ doc: document, submitTransmission, correlation,
    getOperator: () => ({ id: 'gm-a' }), schedule: vi.fn(), cancelSchedule: vi.fn(),
    t: (id, params = {}) => `${id} ${Object.values(params).join(' ')}`, ...extra });
  panel.update(payload());
  for (const option of document.getElementById('gm-comms-recipients').options) option.selected = true;
  document.getElementById('gm-comms-text').value = '  <b>🌒</b> {name}\n';
  panel.refreshAdmission();
  return { panel, submitTransmission, correlation };
}

describe('GM Comms ordinary intent', () => {
  it('shared confirmation sends the captured stale recipient and settles canonical Refused', () => {
    let controller;
    const { panel, submitTransmission, correlation } = mount({ confirmAction: request => controller.request(request) });
    const profile = createGmConfirmationProfile({ storage: { getItem: () => null, setItem: () => {} } });
    profile.setMode('comms.send', 'confirm-preview');
    controller = createGmConfirmationController({ doc: document, profile });
    document.getElementById('gm-comms-send').click();
    expect(controller.isOpen()).toBe(true);
    expect(correlation).not.toHaveBeenCalled();
    expect(panel.state().pending).toEqual([]);
    const preview = document.querySelector('[data-confirmation-preview]').textContent;
    const next = payload(); next.recipients.pop(); panel.update(next);
    document.getElementById('gm-comms-text').value = 'A different draft';
    controller.refresh();
    expect(document.querySelector('[data-confirmation-preview]').textContent).toBe(preview);
    document.querySelector('[data-confirmation-accept]').click();
    document.querySelector('[data-confirmation-accept]').click();
    expect(submitTransmission).toHaveBeenCalledTimes(1);
    const request = submitTransmission.mock.calls[0][0];
    expect(request).toEqual({ operator_id: 'gm-a', correlation: 'comms-1', transmission: {
      sender: 'speaker', route: 'private', recipients: ['ship-a', 'ship-b'],
      content: { literal: { text: '  <b>🌒</b> {name}\n' } },
    } });
    panel.update({ ...next, results: [{ ...request, outcome: 'refused', reason: 'unavailable-comms-recipient' }] });
    expect(panel.state().pending).toEqual([]);
    expect(document.getElementById('gm-comms-feedback').dataset.state).toBe('Refused');
    controller.destroy();
  });
  it('shared confirmation Cancel leaves no correlation, pending request or transmission', () => {
    let controller;
    const { panel, submitTransmission, correlation } = mount({ confirmAction: request => controller.request(request) });
    const profile = createGmConfirmationProfile({ storage: { getItem: () => null, setItem: () => {} } });
    profile.setMode('comms.send', 'confirm');
    controller = createGmConfirmationController({ doc: document, profile });
    document.getElementById('gm-comms-send').click();
    expect(controller.isOpen()).toBe(true);
    document.querySelector('[data-confirmation-cancel]').click();
    document.querySelector('[data-confirmation-accept]').click();
    expect(controller.isOpen()).toBe(false);
    expect(correlation).not.toHaveBeenCalled();
    expect(submitTransmission).not.toHaveBeenCalled();
    expect(panel.state().pending).toEqual([]);
    controller.destroy();
  });
  it('mints nothing before confirmation and submits the captured exact text once', () => {
    let confirmation;
    const { panel, submitTransmission, correlation } = mount({ confirmAction: request => { confirmation = request; return true; } });
    expect(panel.requestSend()).toBe(true);
    expect(correlation).not.toHaveBeenCalled(); expect(panel.state().pending).toEqual([]);
    expect(confirmation.category).toBe('comms.send');
    const preview = confirmation.preview();
    document.getElementById('gm-comms-text').value = 'edited after opening';
    document.getElementById('gm-comms-route').value = 'fleet';
    expect(confirmation.preview()).toBe(preview);
    expect(confirmation.accept()).toBe(true); expect(confirmation.accept()).toBe(false);
    expect(submitTransmission).toHaveBeenCalledTimes(1);
    expect(submitTransmission.mock.calls[0][0]).toEqual({ operator_id: 'gm-a', correlation: 'comms-1',
      transmission: { sender: 'speaker', route: 'private', recipients: ['ship-a', 'ship-b'], content: { literal: { text: '  <b>🌒</b> {name}\n' } } } });
  });
  it.each(['sender', 'recipient', 'route', 'hail'])('submits a captured stale %s for attributed canonical refusal', kind => {
    let confirmation;
    const { panel, submitTransmission, correlation } = mount({ confirmAction: request => { confirmation = request; return true; } });
    expect(panel.requestSend(kind === 'hail')).toBe(true);
    const preview = confirmation.preview();
    const next = payload();
    if (kind === 'sender') next.routes[0].senders = [{ id: 'replacement-speaker', name: 'Another speaker' }];
    if (kind === 'recipient') next.recipients.pop();
    if (kind === 'route') next.routes.shift();
    if (kind === 'hail') next.routes[0].hails = [{ id: 'replacement-hail', name: 'Another hail' }];
    panel.update(next);
    expect(confirmation.preview()).toBe(preview);
    expect(correlation).not.toHaveBeenCalled(); expect(panel.state().pending).toEqual([]);
    expect(confirmation.accept()).toBe(true); expect(confirmation.accept()).toBe(false);
    expect(submitTransmission).toHaveBeenCalledTimes(1);
    const request = submitTransmission.mock.calls[0][0];
    expect(request).toEqual({ operator_id: 'gm-a', correlation: 'comms-1', transmission: {
      sender: 'speaker', route: 'private', recipients: ['ship-a', 'ship-b'],
      content: kind === 'hail' ? { scripted_hail: { hail: 'offer' } } : { literal: { text: '  <b>🌒</b> {name}\n' } },
    } });
    expect(panel.state().pending).toHaveLength(1);
    next.results = [{ ...request, outcome: 'refused', reason: {
      sender: 'unavailable-comms-identity', recipient: 'unavailable-comms-recipient',
      route: 'unknown-comms-route', hail: 'unavailable-comms-hail',
    }[kind] }];
    panel.update(next);
    expect(panel.state().pending).toEqual([]);
    expect(document.getElementById('gm-comms-feedback').dataset.state).toBe('Refused');
    const row = document.querySelector('#gm-comms-log [data-correlation="comms-1"]');
    expect(row.dataset.operatorId).toBe('gm-a'); expect(row.dataset.outcome).toBe('refused');
  });
  it('refuses a changed operator before accepting confirmation', () => {
    let confirmation, operator = { id: 'gm-a' };
    const { panel, correlation } = mount({ getOperator: () => operator, confirmAction: request => { confirmation = request; return true; } });
    panel.requestSend(); operator = { id: 'gm-b' };
    expect(confirmation.accept()).toBe(false); expect(correlation).not.toHaveBeenCalled();
  });
  it('creates no action when the private confirmation seam declines the request', () => {
    const { panel, submitTransmission, correlation } = mount({ confirmAction: () => false });
    expect(panel.requestSend()).toBe(false);
    expect(correlation).not.toHaveBeenCalled(); expect(submitTransmission).not.toHaveBeenCalled();
    expect(panel.state().pending).toEqual([]);
  });
  it('counts UTF-8 bytes and never widens unknown choices', () => {
    const { panel, submitTransmission } = mount();
    document.getElementById('gm-comms-text').value = '🌒'.repeat(17);
    expect(panel.requestSend()).toBe(false);
    document.getElementById('gm-comms-text').value = '🌒'.repeat(16);
    expect(panel.requestSend()).toBe(true);
    expect(submitTransmission.mock.calls[0][0].transmission.content.literal.text).toBe('🌒'.repeat(16));
    document.getElementById('gm-comms-sender').value = 'invented';
    expect(panel.requestSend()).toBe(false);
  });
  it.each(['sender', 'route', 'hail'])('retains an unavailable %s until an explicit replacement is selected', kind => {
    const { panel, submitTransmission } = mount();
    const before = payload();
    before.routes[0].senders.push({ id: 'speaker-b', name: 'Another speaker' });
    before.routes[0].hails.push({ id: 'other-hail', name: 'Another hail' });
    panel.update(before);
    const input = document.getElementById(`gm-comms-${kind}`);
    const original = input.value;
    const after = structuredClone(before);
    if (kind === 'sender') after.routes[0].senders.shift();
    if (kind === 'route') after.routes.shift();
    if (kind === 'hail') after.routes[0].hails.shift();
    panel.update(after);
    expect(input.value).toBe(original);
    expect(input.selectedOptions[0].disabled).toBe(true);
    expect(panel.requestSend(kind === 'hail')).toBe(false);
    expect(submitTransmission).not.toHaveBeenCalled();
    input.value = { sender: 'speaker-b', route: 'fleet', hail: 'other-hail' }[kind];
    input.dispatchEvent(new Event('change'));
    expect(panel.requestSend(kind === 'hail')).toBe(true);
    const intent = submitTransmission.mock.calls[0][0].transmission;
    expect(kind === 'hail' ? intent.content.scripted_hail.hail : intent[kind]).toBe(input.value);
  });
  it('does not silently narrow the draft when one selected recipient disappears', () => {
    const { panel, submitTransmission } = mount();
    const next = payload(); next.recipients.shift(); panel.update(next);
    expect(panel.requestSend()).toBe(false);
    const input = document.getElementById('gm-comms-recipients');
    input.dispatchEvent(new Event('change'));
    expect(panel.requestSend()).toBe(true);
    expect(submitTransmission.mock.calls[0][0].transmission.recipients).toEqual(['ship-b']);
  });
  it('sends authored hail choices through the same confirmation and submission', () => {
    const { panel, submitTransmission } = mount();
    expect(panel.requestSend(true)).toBe(true);
    expect(submitTransmission.mock.calls[0][0].transmission.content).toEqual({ scripted_hail: { hail: 'offer' } });
    document.getElementById('gm-comms-hail').value = 'arbitrary_fn';
    expect(panel.requestSend(true)).toBe(false);
  });
  it('settles only the matching GM and reconnects from authoritative exact history', () => {
    const { panel, submitTransmission } = mount(); panel.requestSend();
    const intent = submitTransmission.mock.calls[0][0].transmission;
    const next = payload(); next.results = [{ operator_id: 'gm-b', correlation: 'comms-1', outcome: 'applied', transmission: intent }];
    panel.update(next); expect(panel.state().pending).toHaveLength(1);
    next.results.push({ operator_id: 'gm-a', correlation: 'comms-1', outcome: 'applied', transmission: intent });
    panel.update(next); expect(panel.state().pending).toEqual([]);
    expect(document.getElementById('gm-comms-feedback').dataset.state).toBe('Applied');
    panel.reset(); panel.update(next);
    expect([...document.querySelectorAll('#gm-comms-log pre')].map(el => el.textContent)).toEqual([intent.content.literal.text, intent.content.literal.text]);
    expect(document.querySelector('#gm-comms-log b')).toBeNull();
  });
  it('reset invalidates a private confirmation without creating a Pending action', () => {
    let request; const { panel, correlation } = mount({ confirmAction: value => { request = value; return true; } });
    panel.requestSend(); panel.reset(); panel.update(payload());
    expect(request.accept()).toBe(false); expect(correlation).not.toHaveBeenCalled();
  });
});

describe('literal Comms wire localisation', () => {
  beforeEach(() => setTable(new Map([['known.id', 'Translated {who}'], ['speaker.id', 'Speaker']])));
  it('preserves exact literal bodies and subjects while ordinary messages still translate', () => {
    const result = localiseTree({ messages: [
      { body: 'known.id', subject: 'known.id', body_params: { who: 'crew' }, literal_body: true, sender_name: 'speaker.id' },
      { body: 'known.id', body_params: { who: 'crew' }, literal_body: false },
    ] });
    expect(result.messages[0].body).toBe('known.id'); expect(result.messages[0].subject).toBe('known.id');
    expect(result.messages[0].sender_name).toBe('Speaker'); expect(result.messages[1].body).toBe('Translated crew');
  });
  it('accepts the actual JSON Host Channel while preserving exact intent and opaque ids', () => {
    const { panel } = mount();
    const next = payload();
    next.routes[0].id = 'known.id';
    next.recipients[0].fleet_slot = 1;
    next.results = [{ operator_id: 'gm-a', correlation: 'known.id', outcome: 'applied', transmission: {
      sender: 'speaker', route: 'known.id', recipients: ['ship-a'], content: { literal: { text: 'known.id' } },
    } }];
    const localise = vi.fn(localiseTree);
    createHostChannel({ handlers: { gm_comms: value => panel.update(value) }, strings: { localiseTree: localise } })('gm_comms', JSON.stringify(next));
    expect(localise).not.toHaveBeenCalled();
    expect(panel.state().results[0].transmission.content.literal.text).toBe('known.id');
    expect(document.querySelector('#gm-comms-log pre').textContent).toBe('known.id');
    expect(document.getElementById('gm-comms-recipients').options[0].textContent).toContain('(1)');
    expect(panel.update('invalid JSON')).toBe(false);
  });
});
