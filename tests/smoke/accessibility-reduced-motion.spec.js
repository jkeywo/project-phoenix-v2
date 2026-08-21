// Issue #1172 — the reduced-motion setting stills console animation.
//
// The #1102 accessibility profile already RESOLVES the tri-state motion
// preference and STAMPS `data-reduced-motion="reduce"`/`"no-preference"` on
// every root; the resolver and the stamping are unit-tested (tests/client/
// accessibility-profile.test.js), and the global CSS layer the attribute
// selects is pinned as source in tests/client/control-floors.test.js. What no
// unit test can prove — jsdom computes no layout and resolves no cascade — is
// that stamping the attribute actually recomputes the rendered styles and a
// looping animation stops. That is this smoke's whole job: drive the real
// client-local write path the Accessibility tab's reduced-motion choice invokes,
// and assert a VISIBLE animation change in a real browser.
//
// The choice row calls `setAccessibility(effect, value)`, which client.html
// wires to `window.setAccessibilityPresentation` (update simState → persist
// privately → re-apply to the shell + console iframes). Calling that function
// IS the round-trip; the DOM click only reaches it. Both `window.simState` and
// the function load at module eval, independent of a host connection, so the
// bare client shell is enough — no WASM, no peer, no reconnect flake.

import { test, expect } from './fixtures';

/**
 * Read the RENDERED motion state off a probe with a REAL looping animation
 * (`spin` is a keyframe the lobby already defines), plus the shell's inherited
 * play-state token and the stamped attribute. The assertion is on computed
 * style — the cascade the global layer drives — not merely the attribute.
 */
async function readMotion(page) {
  return page.evaluate(() => {
    let probe = document.getElementById('__motion_probe');
    if (!probe) {
      probe = document.createElement('span');
      probe.id = '__motion_probe';
      // A genuinely looping animation off a keyframe the lobby defines.
      probe.style.animation = 'spin 1s linear infinite';
      document.body.appendChild(probe);
    }
    const root = document.documentElement;
    const probeCs = getComputedStyle(probe);
    return {
      dataReducedMotion: root.getAttribute('data-reduced-motion'),
      // The loop's iteration count: 'infinite' while it runs, '1' once the
      // global layer collapses it to a single settling frame.
      iterationCount: probeCs.animationIterationCount,
      animationName: probeCs.animationName,
      // The inherited token every shadow-DOM loop reads to pause itself.
      animPlay: getComputedStyle(root).getPropertyValue('--a11y-anim-play').trim(),
    };
  });
}

test('reduced-motion round-trip: data-reduced-motion="reduce" stops a looping animation', async ({ context }) => {
  const page = await context.newPage();
  // Bare client shell: gui/tokens.css is linked and the accessibility module
  // exposes the write path at load — no host hash needed for the motion seam.
  await page.goto('/client/');

  await page.waitForFunction(
    () => typeof window.setAccessibilityPresentation === 'function'
      && !!window.simState,
    { timeout: 30_000 },
  );

  // ── Full motion (no explicit choice; CI reports no prefers-reduced-motion) ──
  const running = await readMotion(page);
  expect(running.dataReducedMotion).not.toBe('reduce');
  expect(running.animationName).toBe('spin');   // the probe really is animating
  expect(running.iterationCount).toBe('infinite');
  expect(running.animPlay).toBe('running');

  // ── Force reduced motion "on" — exactly what the Accessibility tab's choice
  //    row does (update simState, persist privately, re-apply to the roots). ──
  await page.evaluate(() => window.setAccessibilityPresentation('reducedMotion', 'on'));

  const reduced = await readMotion(page);
  expect(reduced.dataReducedMotion).toBe('reduce');   // the attribute flipped
  expect(reduced.iterationCount).toBe('1');           // the loop no longer repeats
  expect(reduced.animPlay).toBe('paused');            // shadow-DOM loops pause too
  expect(reduced.iterationCount).not.toBe(running.iterationCount); // a VISIBLE change

  // The change is a genuine round-trip: it persisted to the private profile.
  const persisted = await page.evaluate(() => {
    try { return JSON.parse(localStorage.getItem('phoenix-accessibility-v1')); }
    catch (_) { return null; }
  });
  expect(persisted && persisted.presentation && persisted.presentation.reducedMotion).toBe('on');

  // ── Tri-state overrides both ways: explicit "off" restores motion, and the
  //    attribute the profile stamps for it — "no-preference" — is what lifts
  //    the OS-default path too, proving the explicit choice drives the render. ─
  await page.evaluate(() => window.setAccessibilityPresentation('reducedMotion', 'off'));

  const allowed = await readMotion(page);
  expect(allowed.dataReducedMotion).toBe('no-preference');
  expect(allowed.iterationCount).toBe('infinite');
  expect(allowed.animPlay).toBe('running');
});
