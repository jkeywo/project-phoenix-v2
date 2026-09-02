/**
 * Shared semantic Controls remapper for the phone and host Settings shells.
 *
 * The parent registry owns bindings and conflict resolution.  This module owns
 * only the short-lived presentation involved in capturing a keyboard chord,
 * confirming a replacement, reporting a reserved chord, and resetting slots.
 * Shell-specific layout and optional gamepad selection are injected by callers.
 */

import { t } from './strings.js';
import {
  CONTINUOUS_DEADZONE_MAX,
  formatSemanticBinding,
  keyboardBindingFromEvent,
} from './semantic-action-registry.js';

const SEMANTIC_MODIFIER_CODES = new Set([
  'ControlLeft', 'ControlRight',
  'ShiftLeft', 'ShiftRight',
  'AltLeft', 'AltRight',
  'MetaLeft', 'MetaRight',
]);
const SEMANTIC_MODIFIER_KEY_CODES = Object.freeze({
  Control: 'ControlLeft',
  Shift: 'ShiftLeft',
  Alt: 'AltLeft',
  Meta: 'MetaLeft',
  OS: 'MetaLeft',
});

/** Resolve a modifier-only event by physical code, with a key-name fallback. */
export function semanticModifierCode(event) {
  if (!event) return null;
  if (SEMANTIC_MODIFIER_CODES.has(event.code)) return event.code;
  return SEMANTIC_MODIFIER_KEY_CODES[event.key] || null;
}

/** True when this event is one modifier key rather than a completed chord. */
export function isSemanticModifierEvent(event) {
  return semanticModifierCode(event) !== null;
}

function semanticModifierBindingFromEvent(event) {
  const code = semanticModifierCode(event);
  if (!code) return null;
  return keyboardBindingFromEvent({
    code,
    ctrlKey: event.ctrlKey,
    shiftKey: event.shiftKey,
    altKey: event.altKey,
    metaKey: event.metaKey,
  });
}

function isPlainEscapeEvent(event) {
  const escape = !!event && (
    event.code === 'Escape' || event.key === 'Escape' || event.key === 'Esc'
  );
  return escape
    && !event.ctrlKey && !event.shiftKey && !event.altKey && !event.metaKey;
}

/**
 * Create one transient remapping presenter over a parent-owned registry.
 *
 * @param {{
 *   doc: Document,
 *   root: Element,
 *   setBinding: function,
 *   resetAction: function,
 *   resetAll: function,
 *   setTuning?: function,
 *   onCapture?: function,
 *   rebuild?: function,
 * }} options
 */
export function createSemanticControlsRemapper(options) {
  const {
    doc,
    root,
    setBinding,
    resetAction,
    resetAll,
    setTuning = (actionId) => ({ status: 'unavailable', actionId }),
    onCapture = () => {},
    rebuild = () => {},
  } = options;
  let pendingConflict = null;
  let bindingFeedback = null;

  function focusBinding(actionId, slot) {
    if (!root || typeof root.querySelector !== 'function') return;
    const control = root.querySelector(
      `[data-control="semantic-binding-${actionId}-${slot}"]`,
    );
    if (control && typeof control.focus === 'function') control.focus();
  }

  function proposeBinding(actionId, slot, binding) {
    onCapture(actionId, slot, false);
    const result = setBinding(actionId, slot, binding);
    const reserved = result && result.status === 'reserved';
    if (reserved) {
      pendingConflict = null;
      bindingFeedback = {
        labelId: 'settings.controls.reserved',
        values: { binding: formatSemanticBinding(binding, t) },
      };
    } else if (result && result.status === 'conflict') {
      pendingConflict = result;
      bindingFeedback = null;
    } else {
      pendingConflict = null;
      bindingFeedback = null;
    }
    rebuild();
    if (reserved) focusBinding(actionId, slot);
    return result;
  }

  function cancelConflict() {
    if (!pendingConflict) return false;
    const { actionId, slot } = pendingConflict;
    pendingConflict = null;
    bindingFeedback = null;
    rebuild();
    focusBinding(actionId, slot);
    return true;
  }

  function resetTransient() {
    pendingConflict = null;
    bindingFeedback = null;
    onCapture(null, null, false);
  }

  const onRootKeydown = (event) => {
    if (!pendingConflict || !isPlainEscapeEvent(event)) return;
    if (typeof event.preventDefault === 'function') event.preventDefault();
    if (typeof event.stopPropagation === 'function') event.stopPropagation();
    cancelConflict();
  };
  if (root && typeof root.addEventListener === 'function') {
    root.addEventListener('keydown', onRootKeydown);
  }

  /**
   * Render the shared remapper into `body`.
   *
   * @param {Element} body
   * @param {{
   *   actions: Array<object>,
   *   section: function,
   *   hint: function,
   *   row: function,
   *   action: function,
   *   beforeActions?: function,
   *   capturing?: {actionId: string, slot: number}|null,
   *   hintId?: string,
   *   pressPromptId?: string,
   *   continuousPressPromptId?: string,
   * }} view
   */
  function render(body, view) {
    const semanticActions = Array.isArray(view.actions) ? view.actions : [];
    const intro = view.section('settings.controls.heading');
    intro.appendChild(view.hint(view.hintId || 'settings.controls.hint'));
    const resetAllControl = view.action(t('settings.controls.reset_all'), () => {
      const result = resetAll();
      if (!result || result.status === 'applied') {
        pendingConflict = null;
        bindingFeedback = null;
        rebuild();
      }
    });
    resetAllControl.setAttribute('data-control', 'semantic-binding-reset-all');
    intro.appendChild(resetAllControl);
    body.appendChild(intro);

    if (typeof view.beforeActions === 'function') view.beforeActions(body);

    if (bindingFeedback) {
      const feedback = doc.createElement('div');
      feedback.className = 'settings-binding-feedback';
      feedback.setAttribute('role', 'alert');
      feedback.setAttribute('aria-live', 'assertive');
      feedback.textContent = t(bindingFeedback.labelId, bindingFeedback.values || {});
      body.appendChild(feedback);
    }

    if (pendingConflict) {
      const proposal = pendingConflict;
      const prompt = doc.createElement('div');
      prompt.className = 'settings-binding-conflict';
      prompt.setAttribute('role', 'alert');
      prompt.setAttribute('aria-live', 'assertive');

      const heading = doc.createElement('div');
      heading.className = 'settings-binding-conflict-heading';
      heading.textContent = t('settings.controls.conflict_heading', {
        binding: formatSemanticBinding(proposal.binding, t),
      });
      prompt.appendChild(heading);

      for (const conflict of proposal.conflicts || []) {
        const conflictAction = semanticActions.find((entry) => entry.id === conflict.actionId);
        const item = doc.createElement('div');
        item.className = 'settings-binding-conflict-item';
        item.textContent = t('settings.controls.conflict_item', {
          action: t(conflict.labelId || (conflictAction && conflictAction.labelId) || ''),
          slot: String(conflict.slot + 1),
        });
        prompt.appendChild(item);
      }

      const conflictControls = view.row('settings-binding-conflict-actions');
      const replace = view.action(t('settings.controls.replace'), () => {
        const result = setBinding(
          proposal.actionId,
          proposal.slot,
          proposal.binding,
          { replace: true },
        );
        if (!result || result.status === 'applied') {
          const { actionId, slot } = proposal;
          pendingConflict = null;
          bindingFeedback = null;
          rebuild();
          focusBinding(actionId, slot);
        }
      });
      replace.setAttribute('data-control', 'semantic-binding-conflict-replace');
      const cancel = view.action(t('settings.controls.cancel'), cancelConflict);
      cancel.setAttribute('data-control', 'semantic-binding-conflict-cancel');
      conflictControls.appendChild(replace);
      conflictControls.appendChild(cancel);
      prompt.appendChild(conflictControls);
      body.appendChild(prompt);
      if (typeof cancel.focus === 'function') cancel.focus();
    }

    for (const semanticAction of semanticActions) {
      const actionSection = view.section(semanticAction.labelId);
      actionSection.appendChild(view.hint(semanticAction.accessibilityLabelId));
      const slots = Array.isArray(semanticAction.bindings) ? semanticAction.bindings : [];
      for (let slot = 0; slot < 2; slot += 1) {
        const binding = slots[slot] || null;
        const bindingRow = view.row('settings-binding-row');
        const label = doc.createElement('label');
        label.className = 'settings-binding-label';
        label.textContent = t('settings.controls.slot', { slot: String(slot + 1) });

        const capture = doc.createElement('input');
        capture.type = 'text';
        capture.readOnly = true;
        const gamepadCapturing = view.capturing
          && view.capturing.actionId === semanticAction.id
          && view.capturing.slot === slot;
        const pressPromptId = semanticAction.continuous
          ? (view.continuousPressPromptId || view.pressPromptId || 'settings.controls.press_key')
          : (view.pressPromptId || 'settings.controls.press_key');
        capture.value = gamepadCapturing
          ? t(pressPromptId)
          : formatSemanticBinding(binding, t);
        capture.className = 'settings-binding-capture';
        capture.setAttribute('data-control', `semantic-binding-${semanticAction.id}-${slot}`);
        capture.setAttribute('data-semantic-binding-capture', 'true');
        capture.setAttribute('aria-label', t('settings.controls.capture_label', {
          action: t(semanticAction.labelId),
          slot: String(slot + 1),
        }));
        label.appendChild(capture);

        let pendingModifier = null;
        capture.addEventListener('focus', () => {
          pendingModifier = null;
          capture.value = t(pressPromptId);
          onCapture(semanticAction.id, slot, true);
        });
        capture.addEventListener('blur', () => {
          pendingModifier = null;
          capture.value = formatSemanticBinding(binding, t);
          onCapture(semanticAction.id, slot, false);
        });
        capture.addEventListener('keydown', (event) => {
          const tab = event.code === 'Tab' || event.key === 'Tab';
          const navigationTab = tab && !event.ctrlKey && !event.altKey && !event.metaKey;
          if (navigationTab || isPlainEscapeEvent(event)) {
            pendingModifier = null;
            return;
          }
          // A held standalone Control/Shift key may auto-repeat before its
          // matching keyup. Ignore that repeat without discarding the original
          // candidate; Tab/Escape above are the deliberate cancellation paths.
          if (event.repeat) return;
          if (semanticAction.continuous) {
            // Continuous semantic actions accept an undirected standard axis.
            // Keyboard steering remains on the existing pointer/WASD path.
            if (typeof event.preventDefault === 'function') event.preventDefault();
            if (typeof event.stopPropagation === 'function') event.stopPropagation();
            return;
          }
          if (isSemanticModifierEvent(event)) {
            const candidate = semanticModifierBindingFromEvent(event);
            if (!candidate) return;
            if (typeof event.preventDefault === 'function') event.preventDefault();
            if (typeof event.stopPropagation === 'function') event.stopPropagation();
            pendingModifier = { code: semanticModifierCode(event), binding: candidate };
            return;
          }
          pendingModifier = null;
          const next = keyboardBindingFromEvent(event);
          if (!next) return;
          if (typeof event.preventDefault === 'function') event.preventDefault();
          if (typeof event.stopPropagation === 'function') event.stopPropagation();
          proposeBinding(semanticAction.id, slot, next);
        });
        capture.addEventListener('keyup', (event) => {
          if (!pendingModifier || semanticModifierCode(event) !== pendingModifier.code) return;
          if (typeof event.preventDefault === 'function') event.preventDefault();
          if (typeof event.stopPropagation === 'function') event.stopPropagation();
          const candidate = pendingModifier.binding;
          pendingModifier = null;
          proposeBinding(semanticAction.id, slot, candidate);
        });

        bindingRow.appendChild(label);
        actionSection.appendChild(bindingRow);
      }
      if (semanticAction.continuous) {
        const tuning = semanticAction.tuning || { deadzone: 0, inverted: false };
        const tuningRow = view.row('settings-binding-tuning-row');

        const deadzoneLabel = doc.createElement('label');
        deadzoneLabel.className = 'settings-binding-label';
        deadzoneLabel.textContent = t('settings.controls.gamepad.deadzone');
        const deadzone = doc.createElement('input');
        deadzone.type = 'range';
        deadzone.min = '0';
        deadzone.max = String(CONTINUOUS_DEADZONE_MAX);
        deadzone.step = '0.01';
        deadzone.value = String(tuning.deadzone);
        deadzone.setAttribute('data-control', `semantic-tuning-deadzone-${semanticAction.id}`);
        deadzone.setAttribute('aria-label', t('settings.controls.gamepad.deadzone'));
        const deadzoneValue = doc.createElement('output');
        deadzoneValue.setAttribute(
          'data-control',
          `semantic-tuning-deadzone-value-${semanticAction.id}`,
        );
        const updateDeadzoneValue = () => {
          deadzoneValue.textContent = t('settings.controls.gamepad.deadzone_value', {
            value: String(Math.round(Number(deadzone.value) * 100)),
          });
        };
        updateDeadzoneValue();
        deadzone.addEventListener('input', () => {
          updateDeadzoneValue();
          setTuning(semanticAction.id, { deadzone: Number(deadzone.value) });
        });
        deadzoneLabel.appendChild(deadzone);
        deadzoneLabel.appendChild(deadzoneValue);
        tuningRow.appendChild(deadzoneLabel);

        const inversionLabel = doc.createElement('label');
        inversionLabel.className = 'settings-binding-label';
        const inversion = doc.createElement('input');
        inversion.type = 'checkbox';
        inversion.checked = tuning.inverted === true;
        inversion.setAttribute('data-control', `semantic-tuning-inverted-${semanticAction.id}`);
        inversion.setAttribute('aria-label', t('settings.controls.gamepad.inverted'));
        inversion.addEventListener('change', () => {
          setTuning(semanticAction.id, { inverted: inversion.checked });
        });
        inversionLabel.appendChild(inversion);
        const inversionText = doc.createElement('span');
        inversionText.textContent = t('settings.controls.gamepad.inverted');
        inversionLabel.appendChild(inversionText);
        tuningRow.appendChild(inversionLabel);
        actionSection.appendChild(tuningRow);
      }
      const reset = view.action(t('settings.controls.reset_action'), () => {
        const result = resetAction(semanticAction.id);
        if (!result || result.status === 'applied') {
          pendingConflict = null;
          bindingFeedback = null;
          rebuild();
        }
      });
      reset.setAttribute('data-control', `semantic-binding-reset-${semanticAction.id}`);
      actionSection.appendChild(reset);
      body.appendChild(actionSection);
    }
  }

  function destroy() {
    if (root && typeof root.removeEventListener === 'function') {
      root.removeEventListener('keydown', onRootKeydown);
    }
    resetTransient();
  }

  return { render, proposeBinding, resetTransient, cancelConflict, destroy };
}
