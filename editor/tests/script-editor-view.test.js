// @vitest-environment jsdom
/**
 * script-editor-view.test.js — DOM view for the Rhai script editor (#983).
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { mountScriptEditor, renderScriptList } from '../script-editor-view.js';

const HOST_FNS = [
  { name: 'on_destroyed', receiver: '', category: 'trigger', signature: 'on_destroyed(entity, handler)', summary: 'Fire when destroyed.' },
  { name: 'on_timer', receiver: '', category: 'trigger', signature: 'on_timer(after_secs, handler)', summary: 'Fire on a timer.' },
  { name: 'complete_objective', receiver: 'effects', category: 'effect', signature: 'effects.complete_objective(id)', summary: 'Complete it.' },
];

describe('renderScriptList', () => {
  it('renders a placeholder when there are no units', () => {
    const host = document.createElement('div');
    renderScriptList(host, [], {});
    expect(host.querySelector('.placeholder')).toBeTruthy();
  });

  it('renders a clickable row per unit and dispatches onSelect', () => {
    const host = document.createElement('div');
    const picked = [];
    renderScriptList(host, [
      { id: 'inline:setup', label: '[script.setup]', kind: 'inline' },
      { id: 'sibling:a.rhai', label: 'a.rhai', kind: 'sibling' },
    ], { selectedId: 'inline:setup', onSelect: (u) => picked.push(u.id) });

    const rows = host.querySelectorAll('.script-list-row');
    expect(rows.length).toBe(2);
    expect(rows[0].classList.contains('active')).toBe(true);
    rows[1].dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    expect(picked).toEqual(['sibling:a.rhai']);
  });
});

describe('mountScriptEditor', () => {
  let host;
  beforeEach(() => {
    host = document.createElement('div');
    document.body.appendChild(host);
  });

  it('renders a textarea seeded with the source and a highlight layer', () => {
    const ctrl = mountScriptEditor({
      host,
      source: 'fn on_x(ctx) { }',
      hostFns: HOST_FNS,
      diagnosticsDelayMs: 0,
    });
    const ta = host.querySelector('.script-editor-input');
    expect(ta.value).toBe('fn on_x(ctx) { }');
    // Highlight layer coloured the `fn` keyword.
    expect(host.querySelector('.script-highlight .tok-keyword').textContent).toBe('fn');
    ctrl.destroy();
  });

  it('offers host-fn autocomplete for a top-level prefix', () => {
    const ctrl = mountScriptEditor({ host, hostFns: HOST_FNS, diagnosticsDelayMs: 0 });
    const ta = host.querySelector('.script-editor-input');
    ta.value = 'on_';
    ta.selectionStart = 3;
    ta.dispatchEvent(new window.Event('input'));
    const items = host.querySelectorAll('.script-autocomplete-item');
    expect(items.length).toBe(2); // on_destroyed, on_timer
    expect(ctrl.completions.map((c) => c.name)).toEqual(['on_destroyed', 'on_timer']);
    ctrl.destroy();
  });

  it('offers effects members after ctx.effects.', () => {
    const ctrl = mountScriptEditor({ host, hostFns: HOST_FNS, diagnosticsDelayMs: 0 });
    const ta = host.querySelector('.script-editor-input');
    ta.value = 'ctx.effects.';
    ta.selectionStart = ta.value.length;
    ta.dispatchEvent(new window.Event('input'));
    expect(ctrl.completions.map((c) => c.name)).toEqual(['complete_objective']);
    ctrl.destroy();
  });

  it('inserts a completion, replacing the prefix and opening a call', () => {
    const changes = [];
    const ctrl = mountScriptEditor({
      host, hostFns: HOST_FNS, diagnosticsDelayMs: 0,
      onChange: (s) => changes.push(s),
    });
    const ta = host.querySelector('.script-editor-input');
    ta.value = 'on_d';
    ta.selectionStart = 4;
    ta.dispatchEvent(new window.Event('input'));
    ctrl.applyCompletion({ name: 'on_destroyed', category: 'trigger' });
    expect(ta.value).toBe('on_destroyed(');
    expect(changes.at(-1)).toBe('on_destroyed(');
    ctrl.destroy();
  });

  it('runs diagnostics through the injected pass with the configured line offset', async () => {
    const calls = [];
    const getDiagnostics = (src, offset) => {
      calls.push({ src, offset });
      return [{ message: 'parse error', line: 2 + offset, column: 5, severity: 'error' }];
    };
    const ctrl = mountScriptEditor({
      host, hostFns: HOST_FNS, getDiagnostics, lineOffset: 10, diagnosticsDelayMs: 0,
    });
    await ctrl.runDiagnostics();
    expect(calls.at(-1).offset).toBe(10);
    const diag = host.querySelector('.script-diagnostic');
    expect(diag.textContent).toContain('Line 12');
    expect(diag.textContent).toContain('parse error');
    ctrl.destroy();
  });

  it('shows "No problems" when diagnostics are clean', async () => {
    const ctrl = mountScriptEditor({
      host, hostFns: HOST_FNS, getDiagnostics: () => [], diagnosticsDelayMs: 0,
    });
    await ctrl.runDiagnostics();
    expect(host.querySelector('.script-diagnostics-ok')).toBeTruthy();
    ctrl.destroy();
  });

  it('shows the #995 unavailable hint (not "No problems") when the WASM seam is dead', async () => {
    // Empty diagnostics but the live pass is not wired: an honest editor must
    // say the check did not run, not claim the script is clean.
    const ctrl = mountScriptEditor({
      host,
      hostFns: [],
      getDiagnostics: () => [], // degraded-to-empty, exactly like a dead wasm load
      isDiagnosticsAvailable: () => false,
      diagnosticsDelayMs: 0,
    });
    await ctrl.runDiagnostics();
    expect(host.querySelector('.script-diagnostics-ok')).toBeNull();
    const hint = host.querySelector('.script-diagnostics-unavailable');
    expect(hint).toBeTruthy();
    expect(hint.textContent).toContain('#995');
    expect(hint.textContent).not.toContain('No problems');
    ctrl.destroy();
  });

  it('recovers "No problems" once diagnostics become available again', async () => {
    // The same view should stop showing the hint when the seam reports available
    // and the compile is genuinely clean — the hint is not sticky.
    let available = false;
    const ctrl = mountScriptEditor({
      host,
      hostFns: HOST_FNS,
      getDiagnostics: () => [],
      isDiagnosticsAvailable: () => available,
      diagnosticsDelayMs: 0,
    });
    await ctrl.runDiagnostics();
    expect(host.querySelector('.script-diagnostics-unavailable')).toBeTruthy();
    available = true;
    await ctrl.runDiagnostics();
    expect(host.querySelector('.script-diagnostics-unavailable')).toBeNull();
    expect(host.querySelector('.script-diagnostics-ok')).toBeTruthy();
    ctrl.destroy();
  });

  it('fires onSave from the Save button', () => {
    const saved = [];
    const ctrl = mountScriptEditor({
      host, source: 'fn a(ctx){}', hostFns: HOST_FNS, diagnosticsDelayMs: 0,
      onSave: (s) => saved.push(s),
    });
    host.querySelector('.script-editor-save').dispatchEvent(new window.MouseEvent('click', { bubbles: true }));
    expect(saved).toEqual(['fn a(ctx){}']);
    ctrl.destroy();
  });
});
