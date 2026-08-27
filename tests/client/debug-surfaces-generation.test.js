import { describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  DEBUG_SURFACE,
  DEBUG_SURFACE_GLOBAL,
  DEBUG_SURFACE_ORDER,
} from '../../gui/debug-surfaces.generated.js';
import {
  DEBUG_OUTPUTS,
  DEBUG_TOGGLES,
  HOST_DEBUG_SURFACE_ADAPTERS,
} from '../../gui/server-settings.js';
import { CLIENT_DEBUG_FLAGS } from '../../gui/settings-panel.js';
import { CONSOLE_LATENCY_FLAG } from '../../gui/console-latency.js';
import { projectDebugSurfaceAdapters } from '../../gui/debug-surface-adapters.js';
import {
  parseDebugSurfaceCatalogue,
  renderDebugSurfaceModule,
} from '../../scripts/generate-debug-surfaces.mjs';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const RUST_SOURCE = fs.readFileSync(path.join(ROOT, 'src/core/debug_surface.rs'), 'utf8');
const GENERATED_SOURCE = fs.readFileSync(
  path.join(ROOT, 'gui/debug-surfaces.generated.js'),
  'utf8',
);

describe('generated Debug Surface catalogue', () => {
  it('matches the authoritative Rust macro byte-for-byte', () => {
    const rows = parseDebugSurfaceCatalogue(RUST_SOURCE);
    expect(GENERATED_SOURCE).toBe(renderDebugSurfaceModule(rows));
    expect(Object.keys(DEBUG_SURFACE)).toEqual(rows.map((row) => row.variant));
    expect(DEBUG_SURFACE_ORDER).toEqual(rows.map((row) => row.wireName));
  });

  it('is the identity source used by both UI adapter sets', () => {
    const phone = CLIENT_DEBUG_FLAGS.map((entry) => entry.flag);
    const host = HOST_DEBUG_SURFACE_ADAPTERS.map((entry) => entry.flag);
    const hostPresentation = [...DEBUG_OUTPUTS, ...DEBUG_TOGGLES]
      .map((entry) => entry.flag).filter(Boolean);

    expect(phone).toEqual(DEBUG_SURFACE_ORDER);
    expect(host).toEqual(DEBUG_SURFACE_ORDER);
    expect(new Set(hostPresentation)).toEqual(new Set(DEBUG_SURFACE_ORDER));
    expect(hostPresentation).toHaveLength(DEBUG_SURFACE_ORDER.length);
    expect(CONSOLE_LATENCY_FLAG).toBe(DEBUG_SURFACE.ConsoleLatency);
    expect(DEBUG_SURFACE_GLOBAL.DebugSurface).toBe(DEBUG_SURFACE);
    expect(DEBUG_SURFACE_GLOBAL.order).toBe(DEBUG_SURFACE_ORDER);
    expect(globalThis.phDebugSurfaces).toBe(DEBUG_SURFACE_GLOBAL);
  });

  it('projects module-owned adapters in generated order and rejects bad coverage', () => {
    const reversed = DEBUG_SURFACE_ORDER.toReversed().map((surface) => [surface, { surface }]);
    expect(projectDebugSurfaceAdapters(reversed, 'test').map((adapter) => adapter.flag))
      .toEqual(DEBUG_SURFACE_ORDER);

    expect(() => projectDebugSurfaceAdapters(reversed.slice(1), 'test'))
      .toThrow(/missing Debug Surface adapters/);
    expect(() => projectDebugSurfaceAdapters([
      ...reversed,
      [DEBUG_SURFACE.Regions, { duplicate: true }],
    ], 'test')).toThrow(/duplicate Debug Surface adapter/);
    expect(() => projectDebugSurfaceAdapters([
      ...reversed,
      ['FutureSurface', {}],
    ], 'test')).toThrow(/unknown Debug Surface adapter/);
  });

  it('rejects an unrecognised row instead of silently omitting identity', () => {
    expect(() => parseDebugSurfaceCatalogue(`
define_debug_surface_catalogue! {
    Regions = "Regions",
}
`)).toThrow(/Unrecognised Debug Surface row/);
  });

  it('rejects duplicate variants and duplicate wire names', () => {
    expect(() => parseDebugSurfaceCatalogue(`
define_debug_surface_catalogue! {
    Regions => "Regions",
    Regions => "Other",
}
`)).toThrow(/variants must be unique/);

    expect(() => parseDebugSurfaceCatalogue(`
define_debug_surface_catalogue! {
    Regions => "Regions",
    Other => "Regions",
}
`)).toThrow(/wire names must be unique/);
  });
});
