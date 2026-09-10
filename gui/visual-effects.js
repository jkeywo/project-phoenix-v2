/**
 * gui/visual-effects.js — the three visual effects an operator can turn down
 * separately, and the one table that says where each of them is real
 * (issue #1428, PRD #1418 stories 13-17).
 *
 * Until this module Phoenix had ONE motion lever: the `reducedMotion` tri-state
 * in `gui/accessibility-profile.js`, resolved to a boolean and stamped as
 * `data-reduced-motion`. On it hung everything at once — the viewscreen's
 * hull-damage camera shake, the red-alert flash on both the phone bezel and the
 * viewscreen vignette, and every decorative loop in the interface. An operator
 * who could not take the camera shake had to give up the loading spinner too,
 * and an operator with photosensitivity had to accept a moving interface to get
 * the flashing to stop. PRD #1418 story 13 asks for the three separately.
 *
 * ## The vocabulary: one number per effect
 *
 * Each effect's stored value is either [`FOLLOW_PREFERENCE`] — follow whatever the motion
 * preference resolved to, which is exactly the behaviour that shipped before
 * this issue — or a NUMBER in `0..=1`:
 *
 *   * `0` is **off**. The effect does not happen at all.
 *   * `1` is **full**. The effect is what it has always been.
 *   * anything between is a genuine intensity: the shake moves less far, the
 *     flash peaks dimmer and pulses slower, decorative loops settle.
 *
 * A number rather than an enum because two of the three consumers are already
 * continuous — `ViewscreenMotion::shake_intensity` multiplies a magnitude in
 * `src/server/viewscreen_border.rs`, and the flash scales a shader uniform — so
 * an enum here would be a lossy re-encoding of a scale that exists anyway. The
 * CONTROLS offer three named stops ([`EFFECT_FULL`], each effect's
 * [`EFFECT_REDUCED`], [`EFFECT_OFF`]) because three honest choices read better
 * than a slider whose middle is hard to judge by eye; the record can carry any
 * value in range, so a hand-edited profile or a later slider needs no new
 * contract.
 *
 * ## Bands, for the consumers that cannot take a number
 *
 * CSS cannot rescale an animation whose authored duration it does not know, so
 * the decorative-motion consumer reads a BAND — `full`, `reduced`, `off` —
 * stamped as a data attribute, and the numeric value is published alongside it
 * as a custom property for the rules that can use one (a pulse's period). See
 * `gui/tokens.css`, which owns what each band does.
 *
 * ## Where each effect is REAL
 *
 * [`EFFECT_INVENTORY`] is the inventory PRD #1418 asks for: per settings
 * surface, per effect, the consumer that actually renders it — or the reason
 * there is none here. A surface builds controls for the effects it can answer
 * and NAMES the ones it cannot, because a control with nothing behind it is a
 * worse answer than a sentence saying so.
 *
 * DOM-free and window-free at import time, so vitest can import it in Node.
 */

/**
 * The follow-the-preference sentinel, defined HERE and re-exported by
 * `gui/accessibility-profile.js` as its `FOLLOW_OS`.
 *
 * One string, one definition, and the dependency runs one way: this module
 * imports nothing, so the profile, the viewscreen record and both panels can
 * all import it without a cycle. The name differs because on an effect control
 * the thing being followed is the resolved MOTION preference rather than an OS
 * query directly — the operator's own explicit Motion choice is upstream of it.
 */
export const FOLLOW_PREFERENCE = 'default';

/** The three effects, in the order every surface displays them. */
export const EFFECT_IDS = Object.freeze(['shake', 'flash', 'decorativeMotion']);

/** The effect is off entirely. */
export const EFFECT_OFF = 0;

/** The effect is what it has always been. */
export const EFFECT_FULL = 1;

/**
 * The conservative middle stop each effect offers, and the value the
 * **Reduce effects** preset writes for it.
 *
 * Tuned per effect rather than shared, because "gentler" is a different number
 * for a camera that moves the whole room's picture than for a spinner:
 *
 *   * `shake` — a third of the travel. The hull-damage shake exists to make a
 *     hit felt; at 0.3 it still registers as an impact without pushing the
 *     horizon around.
 *   * `flash` — a third of the peak, and a correspondingly slower pulse. Flash
 *     is the effect with a photosensitivity cost, so its middle stop is the
 *     dimmest of the three.
 *   * `decorativeMotion` — 0.4, which lands in the `reduced` band: loops settle
 *     on their first frame and one-shot transitions still play. An interface
 *     that snaps between states with no transition at all reads as broken, and
 *     PRD #1418 is explicit that comfort must not hide information.
 *
 * Exported constants rather than literals in three panels: a designer retunes
 * "gentler" here, once.
 */
export const EFFECT_REDUCED = Object.freeze({
  shake: 0.3,
  flash: 0.3,
  decorativeMotion: 0.4,
});

/**
 * What each effect resolves to when it FOLLOWS the motion preference and that
 * preference asked for reduction.
 *
 * All three are `0`, deliberately: that is precisely what shipped before this
 * issue — `shake_magnitude` returned exactly `0.0` under reduced motion,
 * `REDUCED_MOTION_FLASH_CAP` was `0.0`, and `gui/tokens.css` collapsed every
 * animation. So an operator who never opens these controls sees no change at
 * all, which is the only acceptable behaviour for a preference they already set.
 */
export const EFFECT_UNDER_REDUCED_MOTION = Object.freeze({
  shake: EFFECT_OFF,
  flash: EFFECT_OFF,
  decorativeMotion: EFFECT_OFF,
});

/**
 * The stops the controls offer, in display order: follow the motion preference,
 * full, this effect's gentler value, off.
 *
 * `value` is [`FOLLOW_PREFERENCE`] or a number, so a panel can press it straight through
 * to the writer without a second vocabulary.
 */
export function effectChoices(effect) {
  return [
    { key: 'default', value: FOLLOW_PREFERENCE, labelId: 'settings.effects.follow_motion' },
    { key: 'full', value: EFFECT_FULL, labelId: 'settings.effects.level_full' },
    { key: 'reduced', value: EFFECT_REDUCED[effect], labelId: 'settings.effects.level_reduced' },
    { key: 'off', value: EFFECT_OFF, labelId: 'settings.effects.level_off' },
  ];
}

/** The stop a stored value sits on, or null when it sits between them — what a
 *  panel paints as the pressed button. A hand-edited record CAN sit between
 *  them, which is why this can answer null rather than rounding to the nearest. */
export function effectChoiceKey(effect, value) {
  const stored = normalizeEffectLevel(value);
  const match = effectChoices(effect).find((choice) => choice.value === stored);
  return match ? match.key : null;
}

/** The effect's id in kebab case, for a `data-control` attribute. Never for a
 *  String-Table id — those are written out in full in [`EFFECT_COPY`], so
 *  `scripts/check-strings.mjs` can see every one of them. */
export function effectSlug(effect) {
  return String(effect).replace(/[A-Z]/g, (c) => '-' + c.toLowerCase());
}

/**
 * The String-Table ids naming each effect, spelled out rather than composed.
 *
 * A composed id (`'settings.effects.' + effect`) is invisible to
 * `scripts/check-strings.mjs`, which would then never notice a missing row —
 * the failure mode issue #949 named. Written out, every id below is a literal
 * the gate can find and `tests/client/visual-effects.test.js` re-checks against
 * `assets/strings/strings.csv`.
 *
 * The hint differs per surface because the CONSUMER differs: on a console the
 * flash is the red-alert bezel on the operator's own phone, and on the shared
 * screen it is the vignette a whole room is looking at. One sentence for both
 * would have to describe neither.
 */
export const EFFECT_COPY = Object.freeze({
  shake: Object.freeze({
    labelId: 'settings.effects.shake',
    resetId: 'settings.effects.shake_reset',
    hintIds: Object.freeze({
      viewscreen: 'settings.effects.shake_hint_viewscreen',
    }),
  }),
  flash: Object.freeze({
    labelId: 'settings.effects.flash',
    resetId: 'settings.effects.flash_reset',
    hintIds: Object.freeze({
      console: 'settings.effects.flash_hint_console',
      viewscreen: 'settings.effects.flash_hint_viewscreen',
    }),
  }),
  decorativeMotion: Object.freeze({
    labelId: 'settings.effects.decorative_motion',
    resetId: 'settings.effects.decorative_motion_reset',
    hintIds: Object.freeze({
      console: 'settings.effects.decorative_motion_hint_console',
      viewscreen: 'settings.effects.decorative_motion_hint_viewscreen',
      gm: 'settings.effects.decorative_motion_hint_gm',
    }),
  }),
});

/** The heading id for one effect's control group. */
export function effectLabelId(effect) {
  return (EFFECT_COPY[effect] || {}).labelId || null;
}

/** The id of one effect's per-setting reset button. */
export function effectResetId(effect) {
  return (EFFECT_COPY[effect] || {}).resetId || null;
}

/** The hint id under one effect's control on `surface`, or null where that
 *  surface does not offer the effect at all. */
export function effectHintId(effect, surface) {
  const copy = EFFECT_COPY[effect];
  return (copy && copy.hintIds[surface]) || null;
}

/** Clamp an untrusted intensity into `0..=1`. A non-number is not an intensity. */
export function clampEffectIntensity(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return EFFECT_FULL;
  return Math.min(EFFECT_FULL, Math.max(EFFECT_OFF, n));
}

/**
 * Coerce a stored effect value into the vocabulary: [`FOLLOW_PREFERENCE`], or a clamped
 * number. Anything else — a legacy record written before this issue, a string,
 * `null` — reads as follow-the-preference, which is the value that behaves
 * exactly as the build before this one did.
 */
export function normalizeEffectLevel(value) {
  if (typeof value === 'number' && Number.isFinite(value)) return clampEffectIntensity(value);
  return FOLLOW_PREFERENCE;
}

/**
 * The concrete intensity for one effect: the explicit choice where there is
 * one, otherwise what following the motion preference means right now.
 *
 * @param {string} effect one of [`EFFECT_IDS`]
 * @param {number|string} value the stored value
 * @param {boolean} reducedMotion the RESOLVED motion preference
 */
export function resolveEffectIntensity(effect, value, reducedMotion) {
  const stored = normalizeEffectLevel(value);
  if (stored !== FOLLOW_PREFERENCE) return stored;
  return reducedMotion ? EFFECT_UNDER_REDUCED_MOTION[effect] : EFFECT_FULL;
}

/** Every effect's concrete intensity, from a `{shake, flash, decorativeMotion}`
 *  record and the resolved motion preference. */
export function resolveEffectIntensities(record, reducedMotion) {
  const source = record && typeof record === 'object' ? record : {};
  const out = {};
  for (const effect of EFFECT_IDS) {
    out[effect] = resolveEffectIntensity(effect, source[effect], reducedMotion);
  }
  return out;
}

/** The band a consumer that cannot take a number reads: `off`, `reduced`, `full`. */
export function effectBand(intensity) {
  const n = clampEffectIntensity(intensity);
  if (n <= 0) return 'off';
  return n >= EFFECT_FULL ? 'full' : 'reduced';
}

/** The data attribute each effect's band is stamped under, and the custom
 *  property carrying its raw intensity for the rules that can use one. */
export const EFFECT_ATTRIBUTES = Object.freeze({
  shake: 'data-shake',
  flash: 'data-flash',
  decorativeMotion: 'data-decorative-motion',
});

/** The CSS custom properties the numeric intensities are published as. */
export const EFFECT_VARS = Object.freeze({
  shake: '--a11y-shake-scale',
  flash: '--a11y-flash-scale',
  decorativeMotion: '--a11y-decorative-scale',
});

/**
 * Stamp the resolved intensities onto ONE document root: a band attribute and a
 * numeric custom property per effect.
 *
 * Both, rather than one or the other, because the two kinds of consumer are
 * both real. `gui/tokens.css` switches whole blocks on the band (a loop either
 * settles or it does not); `#hud-vignette`'s pulse period and the phone bezel's
 * divide by the number, so a gentler flash is visibly slower as well as dimmer.
 *
 * Swallows DOM errors so one detached root never stops the rest, matching
 * `applyEffectsToRoot` next door.
 *
 * @param {Element|null} root a `documentElement`
 * @param {{shake: number, flash: number, decorativeMotion: number}} intensities
 */
export function applyEffectIntensitiesToRoot(root, intensities) {
  if (!root || !intensities) return;
  try {
    for (const effect of EFFECT_IDS) {
      const value = clampEffectIntensity(intensities[effect]);
      if (typeof root.setAttribute === 'function') {
        root.setAttribute(EFFECT_ATTRIBUTES[effect], effectBand(value));
      }
      if (root.style && typeof root.style.setProperty === 'function') {
        root.style.setProperty(EFFECT_VARS[effect], String(value));
      }
    }
  } catch (_) {
    /* detached / cross-origin root — best-effort */
  }
}

// ── The surface-to-effect inventory ─────────────────────────────────────────

/**
 * Which settings surface is which.
 *
 *   * `console`   — the private per-operator surface: the phone client's
 *                   Accessibility tab and the native Station pane's, which are
 *                   the same panel over the same profile (`client.html`).
 *   * `viewscreen`— the shared display's own menu, on both runtimes: the
 *                   browser cog on `server.html` and the native host lobby's.
 *   * `gm`        — the same cog on `server.html` while it is a Game Master
 *                   session (`html.phoenix-gm-page`).
 */
export const EFFECT_SURFACES = Object.freeze(['console', 'viewscreen', 'gm']);

/**
 * The inventory PRD #1418 and issue #1428 require: for every settings surface
 * and every effect, the REAL consumer that renders it — or the reason this
 * surface has none.
 *
 * `consumer` is a source location, so the claim "this control does something"
 * is checkable by reading one file. `absent` is a String-Table id explaining
 * why the effect is not offered here; a surface renders that sentence instead
 * of a dead control, which is the difference between an honest omission and a
 * lie.
 *
 * The two `absent` entries are not oversights and not deferrals:
 *
 *   * A **console** has no camera and no page-level shake. The hull-damage
 *     shake is `viewscreen_border::apply_camera_shake`, and its whole-page
 *     translate arrives on the `shake` host channel, which only `server.html`
 *     listens to. `client.html` contains the word nowhere.
 *   * A **GM session** hides the render surface outright —
 *     `html.phoenix-gm-page #canvas` and `#hud-overlay` are `display: none`
 *     (server.html) — so neither the shake nor the red-alert vignette exists to
 *     turn down. The GM's decorative motion is real and is offered.
 */
export const EFFECT_INVENTORY = Object.freeze({
  console: Object.freeze({
    shake: Object.freeze({
      absent: 'settings.effects.absent.console_shake',
    }),
    flash: Object.freeze({
      consumer: 'client.html #phone-bezel.alert-on (red-alert bezel pulse)',
    }),
    decorativeMotion: Object.freeze({
      consumer: 'gui/tokens.css decorative bands; client.html spinners, '
        + 'indeterminate loading bar and ready glows; gui/console.css '
        + '.tutorial-highlight; gui/components/ph-battery-bar.js',
    }),
  }),
  viewscreen: Object.freeze({
    shake: Object.freeze({
      consumer: 'src/server/viewscreen_border.rs apply_camera_shake via '
        + 'ViewscreenMotion::shake_intensity (native camera jitter, WASM '
        + 'whole-page translate through the shake host channel)',
    }),
    flash: Object.freeze({
      consumer: 'src/server/viewscreen_border.rs drive_vignette_intensity '
        + '(shield-hit flash uniform); the #hud-vignette red-alert pulse on '
        + 'BOTH runtimes’ documents — server.html in a browser, and '
        + 'gui/viewscreen-hud.html on native, which is stamped over the HUD '
        + 'channel by panes::ultralight::cache_hud_state',
    }),
    decorativeMotion: Object.freeze({
      consumer: 'gui/tokens.css decorative bands; server.html spinner and '
        + 'Coordination chatter entrance; the native lobby chrome; the native '
        + 'HUD overlay document (gui/viewscreen-hud.html), which carries no '
        + 'decorative loop of its own — the band is stamped there because '
        + 'the sweep and the flash re-assertion both key off it',
    }),
  }),
  gm: Object.freeze({
    shake: Object.freeze({
      absent: 'settings.effects.absent.gm_shake',
    }),
    flash: Object.freeze({
      absent: 'settings.effects.absent.gm_flash',
    }),
    decorativeMotion: Object.freeze({
      consumer: 'gui/tokens.css decorative bands over the GM workspace chrome '
        + 'on server.html',
    }),
  }),
});

/** True when `surface` has a real consumer for `effect`. */
export function effectApplies(surface, effect) {
  const entry = EFFECT_INVENTORY[surface] && EFFECT_INVENTORY[surface][effect];
  return !!(entry && entry.consumer);
}

/** The effects `surface` offers controls for, in display order. */
export function applicableEffects(surface) {
  return EFFECT_IDS.filter((effect) => effectApplies(surface, effect));
}

/** The effects `surface` deliberately does not offer, each with the String-Table
 *  id of the sentence that says why. */
export function inapplicableEffects(surface) {
  return EFFECT_IDS
    .filter((effect) => !effectApplies(surface, effect))
    .map((effect) => ({
      effect,
      reasonId: (EFFECT_INVENTORY[surface] || {})[effect]?.absent
        || 'settings.effects.absent.generic',
    }));
}

/**
 * The **Reduce effects** preset (PRD #1418 story 14): one press applying
 * conservative values to every effect this surface can answer.
 *
 * It writes EXPLICIT values rather than returning the effects to follow-system,
 * which is what makes "subsequent individual adjustments remain possible" true:
 * afterwards each control shows a chosen value the operator can move on its own,
 * and a per-setting reset puts just that one back to following the preference.
 *
 * Scoped to the effects `surface` actually renders — a preset that silently
 * wrote a shake value on a surface with no shake would be the inert control this
 * inventory exists to prevent.
 *
 * @param {string} surface
 * @returns {Object<string, number>} effect id → the value to store
 */
export function reduceEffectsChoices(surface) {
  const out = {};
  for (const effect of applicableEffects(surface)) out[effect] = EFFECT_REDUCED[effect];
  return out;
}

/** True when every effect this surface offers already sits at its Reduce
 *  effects value — what the preset's pressed state reports. */
export function effectsAreReduced(surface, record) {
  const source = record && typeof record === 'object' ? record : {};
  const applicable = applicableEffects(surface);
  if (applicable.length === 0) return false;
  return applicable.every((effect) => normalizeEffectLevel(source[effect]) === EFFECT_REDUCED[effect]);
}
