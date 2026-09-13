// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { createPrivateAlerts, createStationPrivateAlerts, attachPrivateAlertLifecycle } from '../../gui/private-alerts.js';
import { ClientSimState } from '../../gui/sim-state.js';
import { createPrivateAudio } from '../../gui/private-audio.js';
import { mountGmWorkspace } from '../../gui/gm-workspace.js';
import { FakeAudioContext, audioFetch, settleAudio } from './audio-fixtures.js';
import { CHANGE_DOMAINS } from '../../gui/reducer-result.js';
import '../../gui/components/ph-comms-hail-list.js';
import '../../gui/components/ph-comms-current-message.js';
import { t } from '../../gui/strings.js';

const manifest = JSON.parse(readFileSync(resolve('assets/audio/private-feedback.json'), 'utf8'));
const message = (id, extra = {}) => ({ id, thread_id: id, sender_name: 'Axiom', body: 'Current transmission',
  priority: 'Urgent', is_read: false, selected_response: null, responses: [], ...extra });
const occurrence = (id, extra = {}) => ({ id, category: 'pending_comms', band: 'urgent', first_seen_tick: 0, age_ms: 0,
  reason: { id: 'server.gm.attention.reason.pending_comms', params: { sender: 'Axiom', ship: 'Alpha' } },
  target: { route: 'r', conversation: id }, ...extra });
const health = (id, kind = 'station_disconnected', extra = {}) => ({ id, kind, severity: 'disconnected',
  reason: { id: 'server.gm.health.reason.recovery_failed', params: { tick: '30' } }, ...extra });
function owner() { const actionable = vi.fn(); return { alerts: createPrivateAlerts({ audio: { actionable } }), actionable }; }
function station(audio) {
  const state = new ClientSimState();
  state.stationSystems = { captain: ['command'], tactical: ['weapons'], auxiliary: ['actual-comms'] };
  state.systemKinds = { 'actual-comms': 'comms' };
  state.controlSources = { 'actual-comms': 'Human' };
  const uiState = { phase: 'InProgress', players: [{ token: 'me', station: 'captain', connected: true }] };
  const alerts = createStationPrivateAlerts({ audio });
  const sample = (messages, { generation = 1, host = 'captain', changes } = {}) => {
    const folded = state.apply({ type: 'BlackboardUpdate', data: { presentation_generation: generation,
      updates: [['actual-comms', { kind: 'Comms', data: { messages, host_station: host } }]] } });
    return alerts.update({ state, uiState, token: 'me', connected: true, changes: changes || folded });
  };
  return { alerts, state, uiState, sample };
}

afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); delete window.PhoenixPrivateAudioProvider; });

describe('finite private alerts', () => {
  it('uses actual human-hosted Comms, baselines acquisition, and never grants Intel readers sound', () => {
    const actionable = vi.fn(), app = station({ actionable });
    app.sample([message('old')]); expect(actionable).not.toHaveBeenCalled();
    app.sample([message('old'), message('fresh')]); expect(actionable).toHaveBeenCalledTimes(1);
    app.uiState.players[0].station = 'tactical';
    app.sample([message('wrong')]); expect(actionable).toHaveBeenCalledTimes(1);
    app.sample([message('wrong')], { host: 'tactical' }); expect(actionable).toHaveBeenCalledTimes(1);
    app.sample([message('wrong'), message('visiting')], { host: 'tactical' }); expect(actionable).toHaveBeenCalledTimes(2);
    for (const source of ['Ai', 'Offline']) {
      app.state.controlSources['actual-comms'] = source;
      app.sample([message(source)], { host: 'tactical' });
    }
    expect(actionable).toHaveBeenCalledTimes(2);
    app.state.controlSources['actual-comms'] = 'Human';
    app.sample([message('reacquired')], { host: 'tactical' }); expect(actionable).toHaveBeenCalledTimes(2);
    app.uiState.players[0].afk = true;
    app.sample([message('afk')], { host: 'tactical' }); expect(actionable).toHaveBeenCalledTimes(2);
  });
  it('keeps read Critical actionable, reads only latest live messages and consumes repeats/range/unread changes', () => {
    const { alerts, actionable } = owner();
    const sample = messages => alerts.comms({ key: 'station', generation: 1, messages });
    sample([]);
    const critical = message('critical', { priority: 'Critical', is_read: true, sender_in_range: false });
    sample([critical]); expect(actionable).toHaveBeenCalledTimes(1);
    sample([{ ...critical, sender_in_range: true }]); expect(actionable).toHaveBeenCalledTimes(1);
    sample([critical, message('urgent')]); expect(actionable).toHaveBeenCalledTimes(2);
    sample([critical, message('urgent', { is_read: true })]);
    sample([critical, message('urgent')]); expect(actionable).toHaveBeenCalledTimes(2);
    sample([critical, message('urgent', { priority: 'Critical', is_read: true })]); expect(actionable).toHaveBeenCalledTimes(3);
    sample([message('orphan', { is_orphaned: true }), message('answered', { selected_response: 0 }), message('read', { is_read: true })]);
    sample([message('superseded', { thread_id: 'thread' }), message('new-routine', { thread_id: 'thread', priority: 'Routine' })]);
    sample([message('superseded', { thread_id: 'thread' })]); expect(actionable).toHaveBeenCalledTimes(3);
    sample([]); sample([message('future')]); expect(actionable).toHaveBeenCalledTimes(4);
  });
  it('silently baselines missing generation, reconnect, connected restore and rejects an older continuation', () => {
    const actionable = vi.fn(), app = station({ actionable });
    app.sample([message('legacy')], { generation: null });
    app.sample([message('base')], { generation: 2 });
    app.sample([message('base'), message('new')], { generation: 2 }); expect(actionable).toHaveBeenCalledTimes(1);
    app.sample([message('restored')], { generation: 3 }); expect(actionable).toHaveBeenCalledTimes(1);
    app.sample([message('stale')], { generation: 2 }); expect(app.state.blackboards['actual-comms'].messages[0].id).toBe('restored');
    app.sample([message('restored'), message('after')], { generation: 3 }); expect(actionable).toHaveBeenCalledTimes(2);
    app.sample([message('reconnected')], { generation: 3, changes: { changedDomains: new Set([CHANGE_DOMAINS.WELCOME]) } });
    expect(actionable).toHaveBeenCalledTimes(2);
    // An unrelated board in a newer continuation cannot stamp old Comms data.
    app.state.apply({ type: 'BlackboardUpdate', data: { presentation_generation: 4, updates: [['helm', { kind: 'Helm', data: {} }]] } });
    expect(app.state.blackboardPresentation['actual-comms']).toBe(3);
    app.sample([message('next-restore')], { generation: 4 }); expect(actionable).toHaveBeenCalledTimes(2);
  });
  it('baselines BFCache/tab return and bounds comparison without evicting rows into fresh cues', () => {
    const { alerts, actionable } = owner();
    const dispose = attachPrivateAlertLifecycle(alerts, window);
    const sample = messages => alerts.comms({ key: 's', generation: 1, messages });
    sample([]); sample([message('a')]); expect(actionable).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event('pageshow')); sample([message('missed')]); expect(actionable).toHaveBeenCalledTimes(1);
    sample(Array.from({ length: 4097 }, (_, i) => message(String(i))));
    sample([message('still-current')]); expect(actionable).toHaveBeenCalledTimes(1);
    sample([message('fresh')]); expect(actionable).toHaveBeenCalledTimes(2); dispose();
  });
  it.each(['pending_comms', 'eligible_beat', 'idle_npc'])('cues only fresh/escalated urgent %s and consumes suppressed edges', category => {
    const { alerts, actionable } = owner();
    const sample = (occurrences, extra = {}) => alerts.attention({ key: 'gm', generation: 1, occurrences, ...extra });
    sample([occurrence('base', { category })]);
    sample([occurrence('fresh', { category })]); expect(actionable).toHaveBeenCalledTimes(1);
    sample([occurrence('fresh', { category })]); expect(actionable).toHaveBeenCalledTimes(1);
    sample([occurrence('low', { category, band: 'attention' })]);
    sample([occurrence('low', { category })]); expect(actionable).toHaveBeenCalledTimes(2);
    sample([occurrence('held', { category })], { held: true }); sample([occurrence('held', { category })]);
    sample([occurrence('filtered', { category })], { visible: () => false }); sample([occurrence('filtered', { category })]);
    sample([occurrence('excluded', { category: 'station_health' })]);
    sample([occurrence('wrong', { category })], { key: null });
    sample([occurrence('acquired', { category })]); expect(actionable).toHaveBeenCalledTimes(2);
  });
  it.each(['station_disconnected', 'ship_peer_lost', 'operator_disconnected', 'recovery_failed'])('deduplicates technical %s by generation id without attention rules', kind => {
    const { alerts, actionable } = owner();
    const sample = (rows, extra = {}) => alerts.health({ key: 'gm', generation: 1, alerts: rows, ...extra });
    sample([]); sample([health('first#1', kind)]); expect(actionable).toHaveBeenCalledTimes(1);
    sample([health('first#1', kind)]); sample([]); sample([health('first#2', kind)]); expect(actionable).toHaveBeenCalledTimes(2);
    for (const severity of ['live', 'paused', 'stale', 'recovering']) sample([health(severity, kind, { severity })]);
    for (const excluded of ['recovery_in_progress', 'live_restore_in_progress', 'live_restore_settled']) sample([health(excluded, excluded)]);
    sample([health('restored#1', kind)], { generation: 2 }); sample([health('stale#1', kind)], { generation: 1 });
    expect(actionable).toHaveBeenCalledTimes(2);
  });
  it('runs Station edges through the actual private gain graph and never catches up after mute', async () => {
    const context = new FakeAudioContext({ state: 'running' }); let clock = 1000;
    const audio = createPrivateAudio({ manifest, contextFactory: () => context, fetchAudio: audioFetch(), now: () => clock });
    await audio.ready; await settleAudio(); const app = station(audio);
    app.sample([]); audio.setBus('alerts', { muted: true }); app.sample([message('missed')]);
    expect(context.sample()).toBe(0);
    clock += 101; audio.setBus('alerts', { muted: false }); app.sample([message('missed')]); expect(context.sample()).toBe(0);
    app.sample([message('missed'), message('current')]); expect(context.sample()).toBeGreaterThan(0);
    audio.setBus('master', { muted: true }); expect(context.sample()).toBe(0); audio.dispose();
  });
  it('draws Urgent in the shared hail and current message accessible text without enabling replies', () => {
    const row = message('urgent', { responses: [{ text: 'Reply', available: false }] });
    const hails = document.createElement('ph-comms-hail-list'), current = document.createElement('ph-comms-current-message');
    document.body.append(hails, current); hails.state = { messages: [row] }; current.state = { thread: row, messages: [row] };
    expect(hails.shadowRoot.querySelector('.priority-text').textContent).toBe(t('component.comms.priority.urgent'));
    expect(current.shadowRoot.querySelector('#priority-cue').textContent).toContain(t('component.comms.priority.urgent'));
    expect(current.shadowRoot.querySelector('.resp-btn').disabled).toBe(true);
    current.state = { thread: row, messages: [row, message('superseding', { thread_id: 'urgent', priority: 'Routine' })] };
    expect(current.shadowRoot.querySelector('#priority-cue').hidden).toBe(true);
  });
});

describe('actual private GM workspace alert adapters', () => {
  it('uses parsed current rows, real hold/filter/snooze rules, unfilterable health, and session baselines', async () => {
    document.body.innerHTML = `<section id="gm-attention-panel"><h2 id="gm-attention-heading"></h2><div id="gm-attention-banners"></div>
      <select id="gm-attention-filter-band"></select><select id="gm-attention-filter-category"></select><select id="gm-attention-filter-ship"></select>
      <p id="gm-attention-status" tabindex="-1"></p><button id="gm-attention-live"></button><div id="gm-attention-list"></div><p id="gm-attention-empty"></p></section>`;
    let session = 'one', connected = true;
    window.__phoenixGmPage = true;
    window.__hostLocalGm = () => ({ id: 'gm', connected }); window.__hostGmSessionId = () => session;
    const cue = vi.fn(() => true);
    window.PhoenixPrivateAudioProvider = () => ({ register() {}, setMix() {}, cue, snapshot: () => ({}), stopAll() {}, dispose() {}, enable: async () => true });
    vi.spyOn(window, 'fetch').mockResolvedValue({ ok: true, json: async () => manifest });
    vi.spyOn(Date, 'now').mockImplementation((() => { let clock = 1000; return () => clock += 150; })());
    const workspace = mountGmWorkspace({ win: window }); await window.__privateAudio.ready;
    const attention = (rows, generation = 1) => workspace.handlers.gm_attention({ occurrences: rows, presentation_generation: generation });
    const technical = rows => workspace.handlers.gm_health({ peers: [], alerts: rows, presentation_generation: 1 });
    attention([]); technical([]); attention([occurrence('fresh')]); expect(cue).toHaveBeenCalledTimes(1);
    const open = document.querySelector('[data-action="open"]'); open.focus();
    attention([occurrence('fresh'), occurrence('held')]); expect(cue).toHaveBeenCalledTimes(1);
    technical([health('failure#1')]); expect(cue).toHaveBeenCalledTimes(2);
    document.getElementById('gm-attention-live').click(); attention([occurrence('fresh'), occurrence('held')]); expect(cue).toHaveBeenCalledTimes(2);
    const filter = document.getElementById('gm-attention-filter-category'); filter.value = 'eligible_beat'; filter.dispatchEvent(new Event('change'));
    attention([occurrence('filtered')]); filter.value = 'all'; filter.dispatchEvent(new Event('change')); attention([occurrence('filtered')]);
    expect(cue).toHaveBeenCalledTimes(2);
    attention([occurrence('snoozed', { band: 'attention' })]);
    document.querySelector('[data-action="snooze"]').click();
    attention([occurrence('snoozed')]); expect(cue).toHaveBeenCalledTimes(3);
    document.querySelector('[data-action="snooze"]').click(); attention([occurrence('snoozed')]); expect(cue).toHaveBeenCalledTimes(3);
    attention([occurrence('restored')], 2); attention([occurrence('old')], 1); expect(cue).toHaveBeenCalledTimes(3);
    session = 'two'; attention([occurrence('new-session')], 2); expect(cue).toHaveBeenCalledTimes(3);
    connected = false; workspace.refreshAdmission(); technical([health('unentitled#1')]); expect(cue).toHaveBeenCalledTimes(3);
    connected = true; workspace.refreshAdmission(); attention([], 2); technical([]);
    for (const category of ['pending_comms', 'eligible_beat', 'idle_npc']) {
      attention([occurrence(`live-${category}`, { category })], 2);
    }
    for (const kind of ['station_disconnected', 'ship_peer_lost', 'operator_disconnected', 'recovery_failed']) {
      technical([health(`live-${kind}#1`, kind)]);
    }
    expect(cue).toHaveBeenCalledTimes(10);
    workspace.dispose(); delete window.__phoenixGmPage;
  });
});
