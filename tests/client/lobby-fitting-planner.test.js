/** @vitest-environment jsdom */
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { LobbyState } from '../../gui/lobby-state.js';
import { lobbyViewModel } from '../../gui/lobby-view.js';
import { renderClientLobby } from '../../gui/client-lobby-render.js';
import { mountFittingPlans } from '../../gui/components/fitting-plan-controls.js';
import { defaultFittingPlan, fittingPlanStorageKey } from '../../gui/fitting-plans.js';
const context = () => ({ hull_path: 'assets/entities/alliance_cruiser.toml', content_id: 'accepted-resolved-hull', slots: [
  { slot: 'beam-slot', system: 'phaser-fore', home_station: 'tactical', default_equipment: 'beam', compatible_equipment: ['beam', 'efficient'] },
  { slot: 'reactor-slot', system: 'power-reactor', home_station: 'engineering', default_equipment: 'reactor', compatible_equipment: ['reactor', 'peak'] },
] });
function fixture() {
  const parsed = new DOMParser().parseFromString(readFileSync(path.join(path.dirname(fileURLToPath(import.meta.url)), '../../client.html'), 'utf8'), 'text/html');
  document.body.append(document.importNode(parsed.getElementById('lobby-ui'), true));
  const state = new LobbyState(); state.apply({ type: 'Welcome', data: { state: { phase: 'Lobby', players: [{ token: 'me', station: 'tactical', connected: true, ready: false }] }, ship_config: { fitting_planner: context() } } });
  const s = { phase: state.phase, players: state.players, stations: [{ id: 'tactical', name: 'Tactical', holder_token: 'me', holder_name: 'Me' }], maxPlayers: 6 };
  const paint = () => { const vm = lobbyViewModel(s, 'me', 'visiting-other-console', { fittingPlanner: state.shipConfig.fitting_planner }); renderClientLobby(document, vm, id => id); return vm; };
  return { s, state, paint, root: document.getElementById('lobby-fitting-planner') };
}
afterEach(() => { document.body.replaceChildren(); sessionStorage.clear(); localStorage.clear(); delete window.activateSemanticAction; });
describe('prelaunch fitting plan journey', () => {
  it('uses Welcome accepted static catalog and actual home claimant, then carries only choices into ordinary in-play requests', () => {
    const { s, paint, root } = fixture(); window.activateSemanticAction = vi.fn(() => true);
    expect(paint().fittingPlanner.station).toBe('tactical'); expect(root.hidden).toBe(false);
    const select = [...root.querySelectorAll('select')].find(select => [...select.options].some(option => option.value === 'efficient'));
    select.value = 'efficient'; select.dispatchEvent(new Event('change')); select.focus();
    s.players[0].ready = true; paint(); expect(document.activeElement).toBe(select);
    expect(root.querySelector('[data-plan-action=queue]').hidden).toBe(true); expect(window.activateSemanticAction).not.toHaveBeenCalled();
    const live = mountFittingPlans(document); document.body.append(live.node);
    const rows = [{ system_id: 'phaser-fore', fitting_planner: context(), fitting: [{ slot: 'beam-slot', system: 'phaser-fore', installed: 'beam', can_request: true, choices: [{ equipment: 'efficient', available: 1 }] }] }];
    s.phase = 'InProgress'; paint(); expect(root.hidden).toBe(true);
    live.update(rows, 'tactical'); expect(window.activateSemanticAction).not.toHaveBeenCalled();
    live.node.querySelector('[data-plan-action=export]').click(); const plan = JSON.parse(live.node.querySelector('textarea').value);
    expect(plan.sections.tactical).toEqual({ explicit: true, choices: { 'beam-slot': 'efficient' } });
    expect(plan.sections.engineering.choices['reactor-slot']).toBe('reactor');
    live.node.querySelector('[data-plan-action=queue]').click();
    expect(window.activateSemanticAction).toHaveBeenCalledExactlyOnceWith('fitting.request', expect.objectContaining({ detail: { owner: 'phaser-fore', slot: 'beam-slot', equipment: 'efficient' } }));
    expect(rows[0].fitting[0].installed).toBe('beam'); live.dispose();
  });
  it('refreshes an already-mounted console draft, preserves explicit home composition and never borrows visiting authority', () => {
    const live = mountFittingPlans(document); document.body.append(live.node); live.update([{ fitting_planner: context() }], 'engineering');
    const { paint, root, s } = fixture(); paint();
    const select = [...root.querySelectorAll('select')].find(select => [...select.options].some(option => option.value === 'efficient'));
    select.value = 'efficient'; select.dispatchEvent(new Event('change'));
    const imported = defaultFittingPlan(context()); imported.sections.engineering.choices['reactor-slot'] = 'peak';
    root.querySelector('textarea').value = JSON.stringify(imported); root.querySelector('[data-plan-action=import]').click();
    live.update([{ fitting_planner: context() }], 'engineering'); live.node.querySelector('[data-plan-action=export]').click();
    const plan = JSON.parse(live.node.querySelector('textarea').value); expect(plan.sections.tactical.choices['beam-slot']).toBe('efficient');
    expect(plan.sections.engineering.choices['reactor-slot']).toBe('peak'); expect(live.node.querySelector('[data-plan-action=queue]').disabled).toBe(true);
    s.stations[0].holder_token = null; s.players[0].station = null; expect(paint().fittingPlanner.station).toBeNull();
    expect([...root.querySelector('select').options].map(option => option.value)).toEqual(['*']); live.dispose();
  });
  it('hides missing or invalid catalog and refuses obsolete drafts without issuing commands', () => {
    const { state, paint, root } = fixture(); state.shipConfig.fitting_planner = null; paint(); expect(root.hidden).toBe(true);
    state.shipConfig.fitting_planner = { ...context(), hull_path: '../bad.toml' }; paint(); expect(root.hidden).toBe(true);
    state.shipConfig.fitting_planner = context();
    const stale = defaultFittingPlan(context()); stale.content_id = 'obsolete-hull';
    sessionStorage.setItem(fittingPlanStorageKey(context()) + '/draft', JSON.stringify(stale));
    window.activateSemanticAction = vi.fn(); paint(); expect(window.activateSemanticAction).not.toHaveBeenCalled(); root.querySelector('[data-plan-action=export]').click();
    expect(JSON.parse(root.querySelector('textarea').value)).toEqual(defaultFittingPlan(context()));
  });
});
