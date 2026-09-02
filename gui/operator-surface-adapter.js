/**
 * Browser/native surface adapter for the private operator profile (issue #1280).
 *
 * The JSON schema and persistence remain owned by operator-profile.js.  This
 * module only decides which retained preferences can be active on the current
 * surface.  In particular, an unavailable gamepad or vibration API never
 * rewrites the portable profile: moving the same JSON to a capable browser
 * restores those choices.
 */

import { applyOperatorProfile } from './operator-profile.js';

export const OPERATOR_SURFACE_KIND = Object.freeze({
  BROWSER: 'browser',
  NATIVE_PANE: 'native-pane',
});

const DEFAULT_FEEDBACK = Object.freeze({
  vibration: true,
  semanticCues: true,
});

function declaredCapabilities(root) {
  const value = root && root.PhoenixOperatorCapabilities;
  return value && typeof value === 'object' ? value : null;
}

/**
 * Detect only APIs the operator profile actually consumes.
 *
 * A native pane declares its capabilities in pane_boot.js before any client
 * module runs.  That declaration is authoritative: Ultralight may expose
 * browser-shaped stubs, but #1124 is the pane's only input route and currently
 * carries keyboard/pointer/touch, not Gamepad API samples or vibration.
 */
export function detectOperatorCapabilities(root = globalThis) {
  const declared = declaredCapabilities(root);
  if (declared) {
    return Object.freeze({
      surface: declared.surface === OPERATOR_SURFACE_KIND.NATIVE_PANE
        ? OPERATOR_SURFACE_KIND.NATIVE_PANE : OPERATOR_SURFACE_KIND.BROWSER,
      keyboard: declared.keyboard !== false,
      gamepad: declared.gamepad === true,
      vibration: declared.vibration === true,
      semanticCues: declared.semanticCues !== false,
      accessibility: declared.accessibility !== false,
    });
  }

  const navigatorLike = root && root.navigator;
  return Object.freeze({
    surface: OPERATOR_SURFACE_KIND.BROWSER,
    keyboard: true,
    gamepad: !!(navigatorLike && typeof navigatorLike.getGamepads === 'function'),
    vibration: !!(navigatorLike && typeof navigatorLike.vibrate === 'function'),
    semanticCues: true,
    accessibility: true,
  });
}

function feedbackPreferences(profile) {
  const source = profile && profile.feedback;
  return {
    vibration: source && typeof source.vibration === 'boolean'
      ? source.vibration : DEFAULT_FEEDBACK.vibration,
    semanticCues: source && typeof source.semanticCues === 'boolean'
      ? source.semanticCues : DEFAULT_FEEDBACK.semanticCues,
  };
}

/**
 * Apply the shared binding/tuning profile and project retained preferences onto
 * one surface.  `profile` is returned unchanged as `portableProfile`; callers
 * must persist/export that value, never the capability-filtered active view.
 */
export function applyOperatorProfileToSurface(
  profile,
  registry,
  { root = globalThis, capabilities = null } = {},
) {
  const applied = applyOperatorProfile(profile, registry);
  if (!applied || applied.status !== 'applied') return applied;

  const surfaceCapabilities = capabilities || detectOperatorCapabilities(root);
  const preferredSlot = profile && profile.gamepad
    ? profile.gamepad.preferredSlot : null;
  const retainedFeedback = feedbackPreferences(profile);
  const unavailable = [];
  if (!surfaceCapabilities.gamepad) unavailable.push('gamepad');
  if (!surfaceCapabilities.vibration) unavailable.push('vibration');
  if (!surfaceCapabilities.semanticCues) unavailable.push('semantic-cues');
  if (!surfaceCapabilities.accessibility) unavailable.push('accessibility');

  return Object.freeze({
    status: 'applied',
    portableProfile: profile,
    capabilities: surfaceCapabilities,
    active: Object.freeze({
      accessibility: surfaceCapabilities.accessibility
        ? profile.accessibility : null,
      preferredGamepadSlot: surfaceCapabilities.gamepad ? preferredSlot : null,
      feedback: Object.freeze({
        vibration: surfaceCapabilities.vibration && retainedFeedback.vibration,
        semanticCues: surfaceCapabilities.semanticCues && retainedFeedback.semanticCues,
      }),
    }),
    unavailable: Object.freeze(unavailable),
  });
}

if (typeof window !== 'undefined') {
  window.OperatorSurfaceAdapter = Object.freeze({
    applyOperatorProfileToSurface,
    detectOperatorCapabilities,
  });
}
