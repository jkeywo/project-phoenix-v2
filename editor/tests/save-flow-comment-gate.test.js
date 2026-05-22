import { describe, it, expect, beforeEach } from 'vitest';
import { ModeShell } from '../mode-shell.js';
import { SaveFlow } from '../save-flow.js';

/**
 * Comment-confirm gate tests. The gate fires when the ON-DISK file
 * (read via the injected readFile) contains a line whose first
 * non-whitespace character is `#` — i.e. an actual TOML comment.
 *
 * It deliberately does NOT trigger on `#` characters appearing inside
 * string values of the serialised output (e.g. hex colours like
 * `"#ff0000"`), because smol-toml never emits comments — so any `#`
 * in stringified output is always inside a string.
 */

describe('SaveFlow comment-confirm gate', () => {
  let modeShell;
  let writeCalls;
  let writer;
  const stringifyFns = { world: () => 'WORLD', entity: () => 'colour="#ff0000"\nkey=1' };

  beforeEach(() => {
    modeShell = new ModeShell();
    modeShell.switchMode('Entity');
    modeShell.setOpenFiles('Entity', ['a.toml']);
    modeShell.setActiveFile('Entity', 'a.toml');
    modeShell.markDirty('Entity', 'a.toml', true);
    writeCalls = [];
    writer = async (p, c) => { writeCalls.push({ p, c }); };
  });

  it('passes the on-disk file text to commentConfirm when it contains a real # comment line', async () => {
    let received = null;
    const gate = (text) => { received = text; return true; };
    const onDisk = '# top comment\nkey=1\n';
    const reader = async () => onDisk;
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate, reader);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(received).toBe(onDisk);
    expect(writeCalls.length).toBe(1);
  });

  it('aborts the save and skips writeFile when the gate returns false', async () => {
    const gate = () => false;
    const reader = async () => '# comment\nkey=1';
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate, reader);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(false);
    expect(r.errors[0]).toMatch(/aborted/i);
    expect(writeCalls.length).toBe(0);
  });

  it('detects indented comment lines (whitespace then #)', async () => {
    let called = false;
    const gate = () => { called = true; return true; };
    const reader = async () => 'key=1\n   # indented comment\nfoo=2';
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate, reader);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    await sf.saveActive();
    expect(called).toBe(true);
  });

  it('does not fire when on-disk text has # only inside string values (e.g. hex colour)', async () => {
    let called = false;
    const gate = () => { called = true; return true; };
    const reader = async () => 'colour="#ff0000"\nname="tag#1"\n';
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate, reader);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(called).toBe(false);
    expect(writeCalls.length).toBe(1);
  });

  it('does not fire when on-disk file is empty / has no # at all', async () => {
    let called = false;
    const gate = () => { called = true; return true; };
    const reader = async () => 'key=1\nfoo="bar"';
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate, reader);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(called).toBe(false);
    expect(writeCalls.length).toBe(1);
  });

  it('does not fire when readFile throws (treat as new file → no comments to lose)', async () => {
    let called = false;
    const gate = () => { called = true; return true; };
    const reader = async () => { throw new Error('not found'); };
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate, reader);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(called).toBe(false);
    expect(writeCalls.length).toBe(1);
  });

  it('back-compat: omitting commentConfirm/readFile behaves as before (no gate)', async () => {
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(writeCalls.length).toBe(1);
  });

  it('with commentConfirm but no readFile: gate is skipped (no on-disk content to check)', async () => {
    let called = false;
    const gate = () => { called = true; return true; };
    const sf = new SaveFlow(modeShell, stringifyFns, writer, null, gate);
    sf.setContent('Entity', 'a.toml', { tags: ['x'] });
    const r = await sf.saveActive();
    expect(r.ok).toBe(true);
    expect(called).toBe(false);
    expect(writeCalls.length).toBe(1);
  });
});
