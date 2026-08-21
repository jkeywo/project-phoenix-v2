// Issue #1171 — the contrast setting maps to a real high-contrast palette.
//
// The #1102 accessibility profile already RESOLVES the tri-state contrast
// preference and STAMPS `data-contrast="more"`/`"standard"` on every root; the
// resolver and the stamping are unit-tested (tests/client/accessibility-
// profile.test.js), and the palette VALUES the attribute selects are pinned in
// tests/client/design-tokens.test.js. What no unit test can prove — jsdom
// computes no layout and resolves no cascade — is that stamping the attribute
// actually recomputes the rendered styles. That is this smoke's whole job:
// drive the real client-local write path the Accessibility tab's contrast
// choice invokes, and assert a VISIBLE rendering change in a real browser.
//
// The choice row calls `setAccessibility(effect, value)`, which client.html
// wires to `window.setAccessibilityPresentation` (update simState → persist
// privately → re-apply to the shell + console iframes). Calling that function
// IS the round-trip; the DOM click only reaches it. Both `window.simState` and
// the function load at module eval, independent of a host connection, so the
// bare client shell is enough — no WASM, no peer, no reconnect flake.

import { test, expect } from './fixtures';

/** Read the RENDERED palette off the shell root, via a probe whose colour and
 *  border resolve from the tokens — so the assertion is on painted colour, not
 *  merely the stamped attribute. */
async function readPalette(page) {
  return page.evaluate(() => {
    const root = document.documentElement;
    let probe = document.getElementById('__contrast_probe');
    if (!probe) {
      probe = document.createElement('span');
      probe.id = '__contrast_probe';
      probe.style.color = 'var(--ink)';
      probe.style.borderTopStyle = 'solid';
      probe.style.borderTopColor = 'var(--edge-faint)';
      document.body.appendChild(probe);
    }
    const probeCs = getComputedStyle(probe);
    return {
      dataContrast: root.getAttribute('data-contrast'),
      ink: probeCs.color,                       // rendered text colour
      edgeFaint: probeCs.borderTopColor,        // rendered divider colour
      surfaceBase: getComputedStyle(root).getPropertyValue('--surface-base').trim(),
    };
  });
}

test('contrast setting round-trip: data-contrast="more" swaps in a visibly different palette', async ({ context }) => {
  const page = await context.newPage();
  // Bare client shell: gui/tokens.css is linked and the accessibility module
  // exposes the write path at load — no host hash needed for the palette seam.
  await page.goto('/client/');

  await page.waitForFunction(
    () => typeof window.setAccessibilityPresentation === 'function'
      && !!window.simState,
    { timeout: 30_000 },
  );

  // ── Standard palette (no explicit choice; CI reports no prefers-contrast) ──
  const standard = await readPalette(page);
  expect(standard.dataContrast).not.toBe('more');
  expect(standard.surfaceBase).toBe('#0a1028'); // the authored navy reference

  // ── Force contrast "more" — exactly what the Accessibility tab's choice row
  //    does (update simState, persist privately, re-apply to the roots). ──────
  await page.evaluate(() => window.setAccessibilityPresentation('contrast', 'on'));

  const more = await readPalette(page);
  expect(more.dataContrast).toBe('more');               // the attribute flipped
  expect(more.surfaceBase).toBe('#000000');             // reference bg went black
  expect(more.ink).not.toBe(standard.ink);              // a VISIBLE rendered change
  expect(more.edgeFaint).not.toBe(standard.edgeFaint);  // the divider is now drawn

  // The change is a genuine round-trip: it persisted to the private profile.
  const persisted = await page.evaluate(() => {
    try { return JSON.parse(localStorage.getItem('phoenix-accessibility-v1')); }
    catch (_) { return null; }
  });
  expect(persisted && persisted.presentation && persisted.presentation.contrast).toBe('on');

  // ── Tri-state overrides both ways: explicit "off" forces the standard
  //    palette back, even though it is ALSO the OS default here — proving the
  //    explicit path drives the render, not just the OS. ──────────────────────
  await page.evaluate(() => window.setAccessibilityPresentation('contrast', 'off'));

  const off = await readPalette(page);
  expect(off.dataContrast).toBe('standard');
  expect(off.surfaceBase).toBe('#0a1028');
  expect(off.ink).toBe(standard.ink);
  expect(off.edgeFaint).toBe(standard.edgeFaint);
});
