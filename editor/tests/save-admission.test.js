import { describe, it, expect, vi } from 'vitest';
import { ModeShell } from '../mode-shell.js';
import { SaveFlow } from '../save-flow.js';
import { InvalidationBus } from '../invalidation-bus.js';
import { hasBlockingErrors, partitionFindings } from '../validation.js';

// Issue #757 — editor error/warning save admission.
//
// Definite (error-severity) validation findings block save and export;
// warning-severity findings stay visible but never block. The gate returns
// BEFORE the write and BEFORE any cache/undo/invalidation fires, so a blocked
// save leaves the editor's local state untouched.

describe('admission primitive (validation.js)', () => {
  it('hasBlockingErrors is true iff a finding is error-severity', () => {
    expect(hasBlockingErrors([])).toBe(false);
    expect(hasBlockingErrors([{ severity: 'warning', message: 'w' }])).toBe(false);
    expect(hasBlockingErrors([{ severity: 'error', message: 'e' }])).toBe(true);
    expect(
      hasBlockingErrors([
        { severity: 'warning', message: 'w' },
        { severity: 'error', message: 'e' },
      ]),
    ).toBe(true);
    // Robust to junk input.
    expect(hasBlockingErrors(null)).toBe(false);
    expect(hasBlockingErrors(undefined)).toBe(false);
  });

  it('partitionFindings splits errors from warnings, preserving records', () => {
    const findings = [
      { severity: 'error', message: 'boom', path: 'global' },
      { severity: 'warning', message: 'meh', path: 'behaviour.initial_state' },
      { severity: 'error', message: 'bang', path: 'anchors' },
    ];
    const { errors, warnings } = partitionFindings(findings);
    expect(errors.map((r) => r.message)).toEqual(['boom', 'bang']);
    expect(warnings.map((r) => r.message)).toEqual(['meh']);
  });

  it('partitionFindings treats unknown severity as non-blocking', () => {
    const { errors, warnings } = partitionFindings([{ message: 'x' }, { severity: 'info', message: 'y' }]);
    expect(errors).toEqual([]);
    expect(warnings).toHaveLength(2);
  });
});

function worldShell(path, content) {
  const modeShell = new ModeShell();
  modeShell.setOpenFiles('World', [path]);
  modeShell.setActiveFile('World', path);
  modeShell.markDirty('World', path, true);
  return modeShell;
}

describe('SaveFlow admission gate', () => {
  it('BLOCKS on error findings: not written, still dirty, undo kept, no invalidation', async () => {
    const path = 'assets/worlds/bad.toml';
    const modeShell = worldShell(path);
    modeShell.pushUndoEntry('World', path, { v: 1 });

    const writeFile = vi.fn(async () => {});
    const bus = new InvalidationBus();
    const fired = [];
    bus.onWorldSaved((p) => fired.push(p));

    const saveFlow = new SaveFlow(
      modeShell,
      { world: () => 'X', entity: () => '' },
      writeFile,
      bus,
    );
    // Missing [global] + [anchors] => two error findings.
    saveFlow.setContent('World', path, { name: 'nope' });

    const result = await saveFlow.saveActive();

    expect(result.ok).toBe(false);
    expect(result.errors.length).toBeGreaterThan(0);
    // Nothing written.
    expect(writeFile).not.toHaveBeenCalled();
    // Dirty preserved.
    expect(modeShell.isDirty('World', path)).toBe(true);
    // Undo history preserved (clearUndoHistory never ran).
    expect(modeShell.getUndoHistory('World', path)).toHaveLength(1);
    // No invalidation fired.
    expect(fired).toEqual([]);
  });

  it('PROCEEDS on warning-only findings: written, warnings surfaced, caches invalidated', async () => {
    const path = 'assets/entities/warn.toml';
    const modeShell = new ModeShell();
    modeShell.switchMode('Entity');
    modeShell.setOpenFiles('Entity', [path]);
    modeShell.setActiveFile('Entity', path);
    modeShell.markDirty('Entity', path, true);

    const writeFile = vi.fn(async () => {});
    const bus = new InvalidationBus();
    const fired = [];
    bus.onEntitySaved((p) => fired.push(p));

    const saveFlow = new SaveFlow(
      modeShell,
      { world: () => '', entity: () => 'E' },
      writeFile,
      bus,
    );
    // Structurally valid, but `initial_state` names no declared state
    // (warning only).
    saveFlow.setContent('Entity', path, {
      tags: ['ship'],
      behaviour: { state: [{ name: 'idle' }], initial_state: 'phantom' },
    });

    const result = await saveFlow.saveActive();

    expect(result.ok).toBe(true);
    expect(result.errors).toEqual([]);
    expect(result.warnings.some((w) => w.includes('phantom'))).toBe(true);
    expect(writeFile).toHaveBeenCalledTimes(1);
    // Caches invalidate only on the ok-path.
    expect(fired).toEqual([path]);
    expect(modeShell.isDirty('Entity', path)).toBe(false);
  });

  it('blocked entity save does not fire EntitySaved invalidation', async () => {
    const path = 'assets/entities/broken.toml';
    const modeShell = new ModeShell();
    modeShell.switchMode('Entity');
    modeShell.setOpenFiles('Entity', [path]);
    modeShell.setActiveFile('Entity', path);
    modeShell.markDirty('Entity', path, true);

    const writeFile = vi.fn(async () => {});
    const bus = new InvalidationBus();
    const fired = [];
    bus.onEntitySaved((p) => fired.push(p));

    const saveFlow = new SaveFlow(
      modeShell,
      { world: () => '', entity: () => 'E' },
      writeFile,
      bus,
    );
    // Entity requires non-empty `tags` => error finding.
    saveFlow.setContent('Entity', path, { tags: [] });

    const result = await saveFlow.saveActive();
    expect(result.ok).toBe(false);
    expect(writeFile).not.toHaveBeenCalled();
    expect(fired).toEqual([]);
    expect(modeShell.isDirty('Entity', path)).toBe(true);
  });
});

describe('no-live-host-reload boundary (issue #757 AC4)', () => {
  // A successful save invalidates ONLY editor-local subscribers via the
  // InvalidationBus; it never crosses a wire codec to a running host. The
  // boundary is that SaveFlow writes to disk + notifies the in-process bus,
  // and nothing else. A new runtime load consumes the saved files.
  it('save notifies editor-local subscribers only — no host/runtime channel', async () => {
    const path = 'assets/worlds/ok.toml';
    const modeShell = worldShell(path);

    const bus = new InvalidationBus();
    const local = [];
    bus.onWorldSaved((p) => local.push(p));

    // The bus exposes ONLY editor-local editor-save notifications; there is
    // no host/runtime/wire emit surface to leak a save across the boundary.
    expect(typeof bus.fireWorldSaved).toBe('function');
    expect(bus.fireHostReload).toBeUndefined();
    expect(bus.sendToHost).toBeUndefined();
    expect(bus.fireRuntimeReload).toBeUndefined();

    const saveFlow = new SaveFlow(
      modeShell,
      { world: () => 'X', entity: () => '' },
      async () => {},
      bus,
    );
    saveFlow.setContent('World', path, { global: {}, anchors: {} });

    const result = await saveFlow.saveActive();
    expect(result.ok).toBe(true);
    // Exactly one editor-local notification; nothing else was reachable.
    expect(local).toEqual([path]);
  });
});
