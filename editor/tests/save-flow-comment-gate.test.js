import { describe, it, expect, beforeEach } from 'vitest';
import { ModeShell } from '../mode-shell.js';
import { SaveFlow } from '../save-flow.js';

/**
 * Slice 7 wiring tests for SaveFlow's optional 5th-arg commentConfirm
 * gate. Covers: gate is called with file content, abort path returns
 * { ok: false, ... } without writing, accept path writes normally,
 * uncommented content skips the gate entirely.
 */

describe('SaveFlow comment-confirm gate', () => {
  let modeShell;
  let writeCalls;
  let writer;
  const stringifyFns = { world: () => 'WORLD', entity: () => '# entity\nkey=1' };

  beforeEach(() => {
    modeShell = new ModeShell();
    modeShell.switchMode('Entity');
    modeShell.setOpenFiles('Entity', ['a.toml']);
    modeShell.setActiveFile('Entity', 'a.toml');
    modeShell.markDirty('Entity', 'a.toml', true);
    writeCalls = [];
    writer = async (p, c) => { writeCalls.push({ p, c }); };
  });

  it('passes the stringified content to commentConfirm when it contains #', async () => {
    let received = null;
    const gate = (content) => { received = content; return true; };
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(received).toContain('#');
    expect(writeCalls.length).toBe(1);
  });

  it('aborts the save and skips writeFile when the gate returns false', async () => {
    const gate = () => false;
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(false);
    expect(r.errors[0]).toMatch(/aborted/i);
    expect(writeCalls.length).toBe(0);
  });

  it('does not call the gate when the content has no #', async () => {
    let called = false;
    const gate = () => { called = true; return true; };
    const sf = new SaveFlow(
      modeShell,
      { world: () => 'WORLD', entity: () => 'key=1\nfoo="bar"' },
      writer,
      null,
      gate,
    );
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(called).toBe(false);
    expect(writeCalls.length).toBe(1);
  });

  it('back-compat: omitting the 5th arg behaves as before (no gate)', async () => {
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(writeCalls.length).toBe(1);
  });
});
