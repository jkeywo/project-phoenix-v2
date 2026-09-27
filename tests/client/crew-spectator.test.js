// @vitest-environment jsdom
import { it, expect, vi } from 'vitest';
import { renderCrewSpectator } from '../../gui/crew-spectator-view.js';
import { ClientSimState } from '../../gui/sim-state.js';

const state = { active: true, can_select: true, target: 'ally', ships: [
  { uuid: 'ally', name: 'Ally' }, { uuid: 'enemy', name: '<Opponent>' },
] };
const t = id => id;

it('selects a camera target without changing authority or target private data', () => {
  const sim = new ClientSimState();
  sim.objectives = ['own objective'];
  sim.blackboards = { own: { secret: true } };
  sim.apply({ type: 'CrewSpectatorState', data: state });
  expect(sim.objectives).toEqual(['own objective']);
  expect(sim.blackboards).toEqual({ own: { secret: true } });
  const root = document.createElement('section'); document.body.append(root);
  const send = vi.fn();
  renderCrewSpectator(root, sim.crewSpectator, { t, send });
  const enemy = root.querySelector('[data-ship-uuid="enemy"]');
  expect(enemy.textContent).toBe('<Opponent>');
  enemy.focus(); enemy.click();
  expect(send).toHaveBeenCalledWith('SelectCrewSpectatorTarget', { uuid: 'enemy' });
  expect(root.querySelector('[aria-pressed="true"]').dataset.shipUuid).toBe('ally');
  renderCrewSpectator(root, { ...state, target: 'enemy' }, { t, send });
  expect(document.activeElement.dataset.shipUuid).toBe('enemy');
  expect(root.querySelector('[aria-pressed="true"]').dataset.shipUuid).toBe('enemy');
  sim.reset({ preserveAuthorityProjection: true });
  expect(sim.crewSpectator.target).toBe('ally');
  sim.reset(); expect(sim.crewSpectator).toBeNull();
});

it('keeps a stable control under repeated snapshots and handles no remaining target', () => {
  const root = document.createElement('section');
  const send = vi.fn();
  renderCrewSpectator(root, state, { t, send });
  const button = root.querySelector('button');
  renderCrewSpectator(root, structuredClone(state), { t, send });
  expect(root.querySelector('button')).toBe(button);
  button.click(); expect(send).toHaveBeenCalledTimes(1);
  renderCrewSpectator(root, { ...state, can_select: false }, { t, send });
  expect([...root.querySelectorAll('button')].every(button => button.disabled)).toBe(true);
  renderCrewSpectator(root, { ...state, ships: [], target: null }, { t, send });
  expect(root.querySelectorAll('button')).toHaveLength(0);
  expect(root.querySelector('[role="status"]').textContent).toBe('client.crew_spectator.empty');
});

it('keeps keyboard focus inside the picker when a followed ship is lost', () => {
  const root = document.createElement('section'); document.body.append(root);
  const send = vi.fn();
  renderCrewSpectator(root, state, { t, send });
  root.querySelector('[data-ship-uuid="ally"]').focus();
  renderCrewSpectator(root, { ...state, target: 'enemy', ships: [state.ships[1]] }, { t, send });
  expect(document.activeElement.dataset.shipUuid).toBe('enemy');
  renderCrewSpectator(root, { ...state, target: null, ships: [] }, { t, send });
  expect(document.activeElement).toBe(root);
});
