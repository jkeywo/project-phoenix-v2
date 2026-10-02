// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import { createGamepadPresentation, updateGamepadPresentation } from '../../gui/settings-gamepad-presentation.js';

function mount(policy, state) {
  document.body.innerHTML = '<input id="binding">';
  const onSelect = vi.fn();
  const view = createGamepadPresentation(document, state, { policy, t, statusClass: 'existing-status', onSelect });
  document.body.append(view.label, view.status);
  return { ...view, onSelect, refresh: next => updateGamepadPresentation(view.selector, view.status, next, { policy, t }) };
}

it.each(['host', 'client'])('preserves %s option ordering, disabled devices and numeric selection', policy => {
  const view = mount(policy, { status: 'disconnected', selectedIndex: 4,
    devices: [{ index: 2, supported: false }, { index: 0, supported: true }] });
  expect([...view.selector.options].map(option => option.value)).toEqual(['', '2', '0', '4']);
  expect(view.selector.options[1].disabled).toBe(true);
  expect(view.selector.value).toBe('4');
  expect(view.status.getAttribute('role')).toBe('alert');
  expect(view.status.getAttribute('aria-live')).toBe('assertive');
  view.selector.value = '0'; view.selector.dispatchEvent(new Event('change'));
  expect(view.onSelect).toHaveBeenLastCalledWith(0);
  view.selector.value = ''; view.selector.dispatchEvent(new Event('change'));
  expect(view.onSelect).toHaveBeenLastCalledWith(null);
});

it('retains client assignment and capability-loss policy without changing host policy', () => {
  const state = { status: 'unavailable', retainedIndex: 7, selectedIndex: 4,
    devices: [{ index: 0, supported: true, available: false, assignedTo: 'Ada' }] };
  const client = mount('client', state);
  expect(client.selector.disabled).toBe(true);
  expect(client.selector.value).toBe('7');
  expect(client.selector.options[1].disabled).toBe(true);
  expect(client.selector.options[1].textContent).toContain('Ada');
  expect([...client.selector.options].map(option => option.value)).toEqual(['', '0', '7']);
  const host = mount('host', state);
  expect(host.selector.disabled).toBe(false);
  expect(host.selector.options[1].disabled).toBe(false);
  expect(host.selector.value).toBe('4');
  expect([...host.selector.options].map(option => option.value)).toEqual(['', '0', '4']);
});

it('keeps client unsupported-only status and refreshes around a focused binding input', () => {
  const state = { status: 'none', devices: [{ index: 1, supported: false }] };
  const view = mount('client', state), input = document.getElementById('binding');
  expect(view.status.textContent).toBe(t('settings.controls.gamepad.status_unsupported'));
  input.focus();
  view.refresh({ status: 'disconnected', selectedIndex: 3, devices: [] });
  expect(document.activeElement).toBe(input);
  expect(document.querySelector('[data-control="semantic-gamepad-select"]')).toBe(view.selector);
  expect(view.status.className).toBe('existing-status');
  expect(view.status.getAttribute('aria-live')).toBe('assertive');
  expect(mount('host', state).status.textContent).toBe(t('settings.controls.gamepad.status_none'));
});
