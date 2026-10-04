/** Shared device presentation; input ownership and panel refresh remain local. */
export function createGamepadPresentation(doc, state, { policy, t, statusClass, onSelect }) {
  const label = doc.createElement('label');
  label.className = 'settings-binding-label';
  label.textContent = t('settings.controls.gamepad.selector');
  const selector = doc.createElement('select');
  selector.setAttribute('data-control', 'semantic-gamepad-select');
  selector.setAttribute('aria-label', t('settings.controls.gamepad.selector'));
  selector.addEventListener('change', () => onSelect(selector.value === '' ? null : Number(selector.value)));
  label.appendChild(selector);
  const status = doc.createElement('div');
  status.className = statusClass;
  status.setAttribute('data-control', 'semantic-gamepad-status');
  updateGamepadPresentation(selector, status, state, { policy, t });
  return { label, selector, status };
}

export function updateGamepadSelector(selector, gamepad, { policy, t }) {
    if (!selector) return;
    selector.innerHTML = '';
    const client = policy === 'client';
    const unavailable = client && gamepad && gamepad.status === 'unavailable';
    selector.disabled = !!unavailable;
    const none = selector.ownerDocument.createElement('option');
    none.value = '';
    none.textContent = t('settings.controls.gamepad.none');
    selector.appendChild(none);
    const seen = new Set();
    for (const device of (gamepad && gamepad.devices) || []) {
      const option = selector.ownerDocument.createElement('option');
      option.value = String(device.index);
      option.textContent = t(device.supported
        ? 'settings.controls.gamepad.device'
        : 'settings.controls.gamepad.device_unsupported', {
        slot: String(Number(device.index) + 1),
      });
      option.disabled = !device.supported || (client && device.available === false);
      if (client && device.available === false && device.assignedTo) {
        option.textContent = t('settings.controls.gamepad.device_assigned', {
          slot: String(Number(device.index) + 1), owner: device.assignedTo,
        });
      }
      selector.appendChild(option);
      seen.add(Number(device.index));
    }
    if (!unavailable && gamepad && gamepad.selectedIndex != null
        && !seen.has(Number(gamepad.selectedIndex))) {
      const disconnected = selector.ownerDocument.createElement('option');
      disconnected.value = String(gamepad.selectedIndex);
      disconnected.textContent = t('settings.controls.gamepad.device_disconnected', {
        slot: String(Number(gamepad.selectedIndex) + 1),
      });
      selector.appendChild(disconnected);
    }
    if (unavailable && gamepad.retainedIndex != null) {
      const retained = selector.ownerDocument.createElement('option');
      retained.value = String(gamepad.retainedIndex);
      retained.textContent = t('settings.controls.gamepad.device_retained', {
        slot: String(Number(gamepad.retainedIndex) + 1),
      });
      selector.appendChild(retained);
      selector.value = retained.value;
    } else {
      selector.value = !gamepad || gamepad.selectedIndex == null
        ? '' : String(gamepad.selectedIndex);
    }
  }


export function updateGamepadPresentation(selector, status, state, { policy, t }) {
  updateGamepadSelector(selector, state, { policy, t });
  if (!status) return;
  const unsupportedOnly = policy === 'client' && state && state.status === 'none'
    && (state.devices || []).some(device => !device.supported)
    && !(state.devices || []).some(device => device.supported);
  const visibleStatus = unsupportedOnly ? 'unsupported' : ((state && state.status) || 'none');
  status.setAttribute('role', visibleStatus === 'disconnected' ? 'alert' : 'status');
  status.setAttribute('aria-live', visibleStatus === 'disconnected' ? 'assertive' : 'polite');
  status.textContent = t(`settings.controls.gamepad.status_${visibleStatus}`);
}
