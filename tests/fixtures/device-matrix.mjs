/**
 * tests/fixtures/device-matrix.mjs — shared device/viewport/scale/content
 * fixtures for issue #1421 (PRD #1418 T3 presentation, comfort and GM
 * usability acceptance).
 *
 * Dependency-free ESM (explicit `.mjs` so it is unambiguous ESM regardless of
 * a `package.json` "type" field) so the SAME module imports unchanged from
 * `tests/client/**` (vitest, Node) and `tests/smoke/**` (Playwright, also
 * Node) — one list of viewports/scales/content instead of two copies that
 * drift.
 *
 * **No fabricated device minimums.** Every `DEVICE_MATRIX` entry's `source`
 * names the exact spec file (and, where the same file defines several, which
 * list) this module took the number from — so a reviewer can verify each row
 * without trusting this file's say-so. Rows this repository cannot yet
 * answer (a physical phone model, the room's actual landscape-tablet
 * viewport, browser build numbers, viewing distance) are NOT invented here;
 * see docs/acceptance/1421-device-matrix.md for the explicit operator-to-
 * record fields that stand in for them until a human measures the real
 * thing.
 */

// ── DEVICE_MATRIX ────────────────────────────────────────────────────────
//
// `kind` is one of 'phone' | 'tablet' | 'desktop' | 'split-pane'.
// `orientation` is 'portrait' | 'landscape' (the pane floor's own layout
// direction for the 'split-pane' row, per its `source`).
//
// Two rows can legitimately share one width/height: 1280x720 is BOTH the
// codebase's "desktop" scope-responsive/lobby-responsive stand-in AND (per
// PRD #1418's Testing Decisions) the *interim* landscape-tablet case until an
// operator records the real tablet's viewport — those are two different
// claims about the same pixels, so they are kept as two rows rather than
// merged into one that overstates either.

export const DEVICE_MATRIX = Object.freeze([
  // ── Phone — portrait/landscape pairs actually asserted in tests/smoke ──
  {
    id: 'phone-375x812-portrait',
    width: 375,
    height: 812,
    kind: 'phone',
    orientation: 'portrait',
    source: 'tests/smoke/scope-responsive.spec.js VIEWPORTS[0]; also tests/smoke/cruiser-tactical-responsive.spec.js',
  },
  {
    id: 'phone-812x375-landscape',
    width: 812,
    height: 375,
    kind: 'phone',
    orientation: 'landscape',
    source: 'tests/smoke/scope-responsive.spec.js VIEWPORTS[1]',
  },
  {
    id: 'phone-390x844-portrait',
    width: 390,
    height: 844,
    kind: 'phone',
    orientation: 'portrait',
    source: 'tests/smoke/hero-bar-responsive.spec.js (390x844 case); tests/smoke/console-redesign-accessibility.spec.js; tests/smoke/cruiser-tactical-responsive.spec.js. PRD #1418 Testing Decisions names 390x844 a starting regression case.',
  },
  {
    id: 'phone-844x390-landscape',
    width: 844,
    height: 390,
    kind: 'phone',
    orientation: 'landscape',
    source: 'tests/smoke/hero-bar-responsive.spec.js (844x390 case); tests/smoke/console-redesign-accessibility.spec.js',
  },
  // ── Tablet / mid-size — sizes the suite already catches regressions at ──
  {
    id: 'tablet-650x450',
    width: 650,
    height: 450,
    kind: 'tablet',
    orientation: 'landscape',
    source: "tests/smoke/scope-responsive.spec.js VIEWPORTS[2] — the file's own comment: \"650x450 is in the list because it is the size this codebase has caught layout regressions at before\". Not tied to a named physical device.",
  },
  {
    id: 'tablet-1280x720-interim-landscape',
    width: 1280,
    height: 720,
    kind: 'tablet',
    orientation: 'landscape',
    source: 'PRD #1418 Testing Decisions: "1280x720 GM fixtures are starting regression cases" and the issue brief: interim landscape-tablet case UNTIL the operator records the real landscape tablet\'s viewport. See docs/acceptance/1421-device-matrix.md for that recording.',
  },
  // ── Desktop / GM host — landscape-only per PRD #1418 GM surface policy ──
  {
    id: 'desktop-1280x720-gm',
    width: 1280,
    height: 720,
    kind: 'desktop',
    orientation: 'landscape',
    source: 'tests/smoke/scope-responsive.spec.js VIEWPORTS[3] ("1280x720 stands in for a desktop"); tests/smoke/gm-layout.spec.js ("GM desktop layout is usable at both host viewport sizes"); tests/smoke/lobby-responsive.spec.js',
  },
  {
    id: 'desktop-1440x900-gm',
    width: 1440,
    height: 900,
    kind: 'desktop',
    orientation: 'landscape',
    source: 'tests/smoke/hero-bar-responsive.spec.js (1440x900, "a wider rail with room for full Station names on a desktop"); tests/smoke/gm-layout.spec.js second host viewport; docs/acceptance/gm-screen-1440.png',
  },
  // ── Supplementary — real viewports asserted in hull-specific specs, kept
  //    for completeness; not part of the PRD's named starting set above. ──
  {
    id: 'phone-390x792-portrait',
    width: 390,
    height: 792,
    kind: 'phone',
    orientation: 'portrait',
    source: "tests/smoke/cruiser-engineering-responsive.spec.js / cruiser-helm-dock-responsive.spec.js / cruiser-tactical-responsive.spec.js viewports list, labelled 'phone-390x792'",
  },
  {
    id: 'phone-375x760-portrait',
    width: 375,
    height: 760,
    kind: 'phone',
    orientation: 'portrait',
    source: "tests/smoke/cruiser-engineering-responsive.spec.js / cruiser-helm-dock-responsive.spec.js / cruiser-tactical-responsive.spec.js viewports list, labelled 'phone-375x760'",
  },
  {
    id: 'phone-716x375-landscape',
    width: 716,
    height: 375,
    kind: 'phone',
    orientation: 'landscape',
    source: "tests/smoke/cruiser-engineering-responsive.spec.js / cruiser-helm-dock-responsive.spec.js / cruiser-tactical-responsive.spec.js viewports list, labelled 'landscape-716x375'",
  },
  {
    id: 'desktop-1308x900',
    width: 1308,
    height: 900,
    kind: 'desktop',
    orientation: 'landscape',
    source: "tests/smoke/cruiser-engineering-responsive.spec.js / cruiser-helm-dock-responsive.spec.js viewports list, labelled 'desktop-1308x900'",
  },
  {
    id: 'lobby-480x900-narrow',
    width: 480,
    height: 900,
    kind: 'phone',
    orientation: 'portrait',
    source: 'tests/smoke/lobby-responsive.spec.js setupLobby({ width: 480, height: 900 }) — below the 720px compact-mode breakpoint the file documents',
  },
  // ── Native split-pane floor — the smallest supported split-pane geometry
  //    (a logical-pixel floor per pane, not itself a device). ──
  {
    id: 'native-split-pane-floor',
    width: 320,
    height: 320,
    kind: 'split-pane',
    orientation: 'landscape',
    source: 'src/native_host/setup_accessibility.rs MIN_CONSOLE_LOGICAL_WIDTH_PX / MIN_CONSOLE_LOGICAL_HEIGHT_PX = 320 logical px, at text-scale 1.0x — a per-pane floor, both dimensions fixed at that scale; the WIDTH floor scales linearly with text scale up to SUPPORTED_TEXT_SCALE_MAX (1.5x today, so 480px at max) while the height floor stays fixed because consoles scroll vertically. MAX_PANES_PER_STATION = 2 (src/native_host/bridge_profile.rs) bounds how many such panes one Station may split into; default split direction is side-by-side (PaneSplit::SideBySide, src/native_host/bridge_layout.rs).',
  },
]);

// ── TEXT_SCALES ──────────────────────────────────────────────────────────
//
// PRD #1418 Testing Decisions: "exercise 100%, 150% and 200% with realistic
// long text and dense states." 1.5x (150%) is the client's CURRENT shipped
// ceiling (gui/accessibility-profile.js TEXT_SCALE_MAX) and the native
// mirror's CURRENT ceiling (src/native_host/setup_accessibility.rs
// SUPPORTED_TEXT_SCALE_MAX); 2.0x (200%) is PRD #1418's target that later T3
// issues raise the ceiling to — kept here as the regression case those
// issues and this acceptance kit exercise against, not a claim that 200% is
// selectable today.
export const TEXT_SCALES = Object.freeze([1, 1.5, 2]);

// ── BROWSER_ZOOMS ────────────────────────────────────────────────────────
//
// PRD #1418: "Verify browser zoom separately; do not assume a universally
// available browser query for the Windows text-size percentage." No repo
// fixture asserts specific browser-zoom levels today (a gap, not a
// contradiction), so these are Chromium/Firefox/Edge's own built-in zoom
// STEPS (their zoom menus stop at 25/33/50/67/75/80/90/100/110/125/150/175/
// 200/250/300/400/500%) — 100/125/150/200 chosen to bracket TEXT_SCALES
// above at levels every mainstream browser actually offers, not an invented
// number. Record the browser/version/actual zoom used in the acceptance doc.
export const BROWSER_ZOOMS = Object.freeze([1.0, 1.25, 1.5, 2.0]);

// ── DENSE_CONTENT ────────────────────────────────────────────────────────
//
// Representative long/dense String-Table-backed content for the three
// surfaces PRD #1418 names: player consoles, Comms and GM lists. Every
// `id` is a REAL row in assets/strings/strings.csv today (pinned by
// tests/client/device-matrix.test.js via gui/strings.js `has()`) — never
// placeholder/lorem text, per the repository's String Table rule
// (AGENTS.md: all UI copy goes through t()/data-i18n).
//
// `repeatHint` is not a string-count claim: the String Table intentionally
// does not carry dozens of near-duplicate rows for list density (that volume
// is scenario/session data — NPCs, contacts, mission events — not string
// count). It is the operator's cue for how many rows to produce (by
// repeating an id across different targets/timestamps, or loading a
// scenario with that many live entries) when a task needs a genuinely dense
// list rather than one long string.
export const DENSE_CONTENT = Object.freeze({
  consoles: Object.freeze([
    {
      id: 'help.navigation.3.body',
      source: 'assets/strings/strings.csv — "Help modal — navigation station, section 4 body text" (194 chars); rendered by the in-console Ship Manual/help overlay',
      repeatHint: 1,
    },
    {
      id: 'help.helm.3.body',
      source: 'assets/strings/strings.csv — "Help modal — helm station, section 4 body text" (152 chars)',
      repeatHint: 1,
    },
    {
      id: 'help.tactical.3.body',
      source: 'assets/strings/strings.csv — "Help modal — tactical station, section 4 body text" (113 chars)',
      repeatHint: 1,
    },
  ]),
  comms: Object.freeze([
    {
      id: 'world.falling_skyway.comms.window_opens_short',
      source: 'assets/strings/strings.csv — Falling Skyway dialogue body, "Control opening a transfer window..." (463 chars, the longest Comms body in the table); {available} param',
      repeatHint: 1,
    },
    {
      id: 'world.falling_skyway.comms.rigger_account',
      source: "assets/strings/strings.csv — Falling Skyway dialogue body, \"the rigger's account of the Ladder B inspections\" (322 chars)",
      repeatHint: 1,
    },
    {
      id: 'world.falling_skyway.comms.committee_rides_it_out_promised',
      source: 'assets/strings/strings.csv — Falling Skyway dialogue body, "the strike committee naming the broken safe-passage promise" (321 chars)',
      repeatHint: 1,
    },
  ]),
  gmLists: Object.freeze([
    {
      id: 'server.gm.knowledge.hint',
      source: "assets/strings/strings.csv — \"Explains the comparison's scope and Station-privacy boundary\" (189 chars), rendered in the GM knowledge-compare panel (gui/gm-knowledge-compare.js)",
      repeatHint: 12,
    },
    {
      id: 'server.gm.objective.preview_complete',
      source: 'assets/strings/strings.csv — GM Objective completion preview (117 chars, {objective}/{ships} params), rendered per-row in the GM Objective panel (gui/gm-objective-panel.js)',
      repeatHint: 12,
    },
    {
      id: 'server.gm.mission.skip_result_timed_out',
      source: 'assets/strings/strings.csv — timed-out local GM event-skip result ({name}/{event}/{correlation} params), rendered per-row in the GM mission log (gui/gm-mission-panel.js)',
      repeatHint: 12,
    },
    {
      id: 'server.gm.knowledge.summary',
      source: 'assets/strings/strings.csv — comparison category summary counts ({same}/{changed}/{truth_only}/{crew_only} params)',
      repeatHint: 12,
    },
  ]),
});
