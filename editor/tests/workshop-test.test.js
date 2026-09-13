import { describe, expect, it, vi } from 'vitest';
import { createWorkshopTest } from '../workshop-test.js';

function fixture() {
  const sources = { 'assets/worlds/test.toml': '# exact\r\n[global]\r\n',
    'assets/models/hull.glb': { asset: '0000000000000011-8', length: 8 } };
  let run = null;
  const provider = {
    start: vi.fn(async (_sources, options) => (run = { running: true, paused: false, tick: 0, ...options })),
    control: vi.fn(async command => {
      if (command.command === 'pause') run = { ...run, paused: true };
      return run;
    }),
    status: vi.fn(async () => run || { running: false }),
    stop: vi.fn(async () => { run = null; }),
  };
  const changes = vi.fn();
  const session = createWorkshopTest({ provider, snapshot: () => sources, onChange: changes });
  return { sources, provider, session, changes };
}
const selection = { world: 'assets/worlds/test.toml', ship: 'assets/entities/hull.toml', seed: 7 };

describe('disposable Workshop Test lifecycle', () => {
  it('cancels a queued start before it acquires any runtime or frame', async () => {
    const { session, provider } = fixture();
    const started = session.start(selection);
    const stopped = session.stop();
    await Promise.all([started, stopped]);
    expect(provider.start).not.toHaveBeenCalled();
    expect(provider.stop).toHaveBeenCalledOnce();
    expect(session.state()).toMatchObject({ mode: 'authoring', run: null, busy: false });
  });

  it('retains child failure details during a control and does not send visibility to a closed child', async () => {
    const { session, provider } = fixture();
    await session.start(selection);
    provider.control.mockResolvedValueOnce({ running: false, error: 'Test rendering failed' });
    await session.authoring();
    expect(provider.control).toHaveBeenCalledTimes(1);
    expect(session.state()).toMatchObject({ mode: 'authoring', run: null });
    expect(session.state().error.message).toBe('Test rendering failed');
  });

  it('captures unsaved source bytes, retains one stale run while authoring, and restarts from the new draft', async () => {
    const { session, provider, sources } = fixture();
    const started = session.start(selection);
    sources['assets/worlds/test.toml'] += '# later\n';
    session.changed();
    await started;
    expect(provider.start.mock.calls[0][0]['assets/worlds/test.toml']).not.toContain('later');
    expect(session.state()).toMatchObject({ mode: 'test', stale: true });
    await session.authoring();
    expect(provider.control.mock.calls.map(([command]) => command)).toEqual([
      { command: 'pause' }, { command: 'visibility', visible: false },
    ]);
    expect(session.state()).toMatchObject({ mode: 'authoring', stale: true, run: { paused: true } });
    await session.control({ command: 'resume' });
    expect(provider.control).toHaveBeenCalledTimes(2);
    await session.test();
    expect(provider.start).toHaveBeenCalledTimes(1);
    expect(session.state()).toMatchObject({ mode: 'test', stale: true });
    await session.start(selection);
    expect(provider.start.mock.calls[1][0]['assets/worlds/test.toml']).toContain('later');
    expect(session.state()).toMatchObject({ mode: 'test', stale: false });
  });
  it('preserves the draft and previous run after failed runtime validation', async () => {
    const { session, provider, sources } = fixture();
    await session.start(selection);
    sources['assets/worlds/test.toml'] += 'invalid'; session.changed();
    const before = structuredClone(sources);
    provider.start.mockRejectedValueOnce(new Error('Rhai compilation refused'));
    await expect(session.start(selection)).rejects.toThrow('Rhai compilation');
    expect(sources).toEqual(before);
    expect(session.state()).toMatchObject({ mode: 'test', stale: true, run: { running: true }, busy: false });
  });
  it('stops a late accepted start when the shared view is disposed', async () => {
    const { session, provider, changes } = fixture();
    let complete;
    provider.start.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
    const started = session.start(selection);
    await Promise.resolve();
    const stopped = session.dispose();
    const notifications = changes.mock.calls.length;
    expect(provider.stop).not.toHaveBeenCalled();
    complete({ running: true, tick: 0 });
    await Promise.all([started, stopped]);
    expect(provider.stop).toHaveBeenCalledOnce();
    expect(changes).toHaveBeenCalledTimes(notifications);
  });

  it('immediately cancels an isolated browser boot while keeping native source work serialized', async () => {
    const { session, provider } = fixture();
    let complete;
    provider.start.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
    provider.cancelStart = vi.fn();
    const started = session.start(selection);
    await Promise.resolve();
    const stopped = session.dispose();
    expect(provider.cancelStart).toHaveBeenCalledOnce();
    expect(provider.stop).not.toHaveBeenCalled();
    complete({ running: false });
    await Promise.all([started, stopped]);
    expect(provider.stop).toHaveBeenCalledOnce();
  });
  it('returns to Authoring when the disposable child closes and serializes Stop after controls', async () => {
    const { session, provider } = fixture();
    await session.start(selection);
    let complete;
    provider.control.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
    const command = session.control({ command: 'step' });
    await Promise.resolve();
    const stopped = session.stop();
    expect(provider.stop).not.toHaveBeenCalled();
    complete({ running: true, paused: true, tick: 1 });
    await Promise.all([command, stopped]);
    expect(session.state()).toMatchObject({ mode: 'authoring', run: null, stale: false });
    await session.start(selection);
    provider.status.mockResolvedValueOnce({ running: false });
    await session.poll();
    expect(session.state()).toMatchObject({ mode: 'authoring', run: null });
  });
});
