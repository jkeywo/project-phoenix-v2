// @vitest-environment jsdom
import { beforeEach, expect, it, vi } from 'vitest';
import { applyToDom, t } from '../../gui/strings.js';
import {
  mountDockControl, mountTowLoadControl, mountTractorControl, mountUmbilicalControl,
  renderDockPanel, renderTowLoadPanel, renderTractorPanel, renderUmbilicalPanel,
} from '../../gui/stations/contextual-controls.js';

beforeEach(() => {
  document.body.innerHTML = '<div id="dock-panel" hidden></div><div id="tow-load-panel" hidden></div><div id="tractor-panel" hidden></div><div id="umbilical-panel" hidden></div>';
});
const el = id => document.getElementById(id);

it('mounts once, replaces its activation callback, disposes and remounts without duplicate input', () => {
  const first = vi.fn(), second = vi.fn();
  const owner = mountDockControl(el('dock-panel'), { activate: first });
  const button = el('dock-btn');
  expect(el('dock-status').textContent).toBe('\u2014');
  expect(mountDockControl(el('dock-panel'), { activate: second })).toBe(owner);
  expect(document.querySelectorAll('#dock-btn')).toHaveLength(1);
  button.click();
  expect(first).not.toHaveBeenCalled();
  expect(second).toHaveBeenCalledOnce();
  owner.dispose(); owner.dispose(); button.click();
  expect(second).toHaveBeenCalledOnce();
  mountDockControl(el('dock-panel'), { activate: first });
  expect(el('dock-btn')).toBe(button);
  owner.dispose(); button.click();
  expect(first).toHaveBeenCalledOnce();
});

it('paints contextual Dock and passive tow synchronously, retaining nodes on locale refresh', () => {
  mountDockControl(el('dock-panel')); mountTowLoadControl(el('tow-load-panel'));
  const button = el('dock-btn');
  renderDockPanel({ dock: { system_id: 'berth', available: true, refusal: 'console.dock.dock' } }, document, t);
  expect(el('dock-panel').hidden).toBe(false);
  expect(button.dataset.systemId).toBe('berth');
  expect(el('dock-refusal').hidden).toBe(false);
  renderDockPanel({ dock: { system_id: 'berth', docked: true, docked_to_name: 'Target' } }, document, t);
  expect(button.classList.contains('docked')).toBe(true);
  expect(el('dock-refusal').textContent).toBe('');
  expect(el('dock-refusal').hidden).toBe(true);
  applyToDom(document); renderDockPanel({ dock: { docked: true } }, document, t);
  expect(el('dock-btn')).toBe(button);
  expect(button.textContent).toBe(t('console.dock.undock'));
  renderTowLoadPanel({ tow_load: { active: true, target_name: 'Target' } }, document, t);
  expect(el('tow-load-target').textContent).toContain('Target');
  renderTowLoadPanel({}, document, t); renderDockPanel({}, document, t);
  expect(el('tow-load-panel').hidden).toBe(true);
  expect(el('dock-panel').hidden).toBe(true);
  expect(button.dataset.systemId).toBe('');
});

it.each([
  ['tractor', mountTractorControl, renderTractorPanel, 'engage_tractor', 'release_tractor'],
  ['umbilical', mountUmbilicalControl, renderUmbilicalPanel, 'start_transfer', 'stop_transfer'],
])('keeps %s legacy fallback verbs tied to authoritative painting', (kind, mount, render, start, stop) => {
  const activate = vi.fn();
  mount(el(`${kind}-panel`), { activate, headingMargin: true });
  const button = el(`${kind}-btn`);
  const payload = active => ({ system_ids: [kind], system_families: { [kind]: kind },
    systems: { [kind]: { system_id: kind, engaged: active, running: active } } });
  render(payload(false), document, t); button.click();
  expect(activate).toHaveBeenLastCalledWith(start);
  render(payload(true), document, t); button.click();
  expect(activate).toHaveBeenLastCalledWith(stop);
  expect(el(`${kind}-panel`).querySelector('h2').style.marginTop).toBe('10px');
  render({}, document, t);
  expect(el(`${kind}-panel`).hidden).toBe(true);
});

it('preserves Dynasty classes and button type without Alliance typography', () => {
  for (const [kind, mount] of [['dock', mountDockControl], ['tractor', mountTractorControl], ['umbilical', mountUmbilicalControl]]) {
    mount(el(`${kind}-panel`), { profile: 'dynasty' });
    expect(el(`${kind}-btn`).type).toBe('button');
    expect(el(`${kind}-btn`).className).toBe('');
    expect(el(`${kind}-panel`).querySelector('.dynasty-aux-row')).not.toBeNull();
  }
});

it('shows collection direction and parts labels, retaining exact-owner selectors', () => {
  const send = vi.fn(); window.sendAction = send;
  const state = running => ({ system_ids: ['umbilical'], system_families: { umbilical: 'umbilical' }, systems: {
    umbilical: { system_id: 'umbilical', direction: 'collect', running, allowed_resources: ['structural_parts', 'machinery_parts'], selected_resource: 'machinery_parts' }
  } });
  mountUmbilicalControl(el('umbilical-panel'));
  renderUmbilicalPanel(state(false), document, key => key);
  const direction = document.querySelector('[data-transfer-direction]');
  expect(direction.value).toBe('collect'); expect(direction.disabled).toBe(false);
  const resource = document.querySelector('[data-transfer-resource]');
  expect([...resource.options].map(option => option.textContent)).toEqual(['damage.resource.structural_parts', 'damage.resource.machinery_parts']);
  direction.value = 'deliver'; direction.dispatchEvent(new Event('change'));
  expect(send).toHaveBeenLastCalledWith('set_transfer_direction', { direction: 'deliver', control_system_id: 'umbilical' });
  renderUmbilicalPanel(state(true), document, key => key);
  expect(document.querySelector('[data-transfer-direction]')).toBe(direction); expect(direction.disabled).toBe(true);
  expect(el('umbilical-status').textContent).toContain('←');
  delete window.sendAction;
});

it('keeps accepted own separation visible while paused and withholds another dock activation', () => {
  document.body.innerHTML = '<section id="dock-panel"><button id="dock-btn"></button><span id="dock-status"></span><span id="dock-refusal"></span></section>';
  renderDockPanel({ dock: { system_id: 'dock', separating: true, separation_remaining: 12.5, refusal: 'dock.refused.unpowered' } }, document, t);
  expect(document.getElementById('dock-panel').hidden).toBe(false);
  expect(document.getElementById('dock-btn').disabled).toBe(true);
  expect(document.getElementById('dock-status').textContent).toBe(t('console.dock.separating', { distance: 13 }));
  renderDockPanel({ dock: { system_id: 'dock', available: true } }, document, t);
  expect(document.getElementById('dock-btn').disabled).toBe(false);
});
