import { describe, expect, it } from 'vitest';
import shouldRunCi from '../../scripts/ci-schedule.cjs';

const context = { eventName: 'schedule', runId: 100, sha: 'current', repo: { owner: 'owner', repo: 'repo' } };
const run = (overrides = {}) => ({ id: 99, head_sha: 'current', event: 'schedule', conclusion: 'success', ...overrides });
function check(pages, overrides = {}) {
  return shouldRunCi({
    context: { ...context, ...overrides },
    core: { info() {} },
    github: {
      rest: { actions: {
        getWorkflowRun: async () => ({ data: { workflow_id: 7 } }),
        listWorkflowRuns() {},
      } },
      paginate: { async *iterator() {
        for (const data of pages) yield { data };
      } },
    },
  });
}

describe('scheduled full CI coverage', () => {
  it('runs a new revision even when an older revision has full coverage', async () => {
    expect(await check([[run({ head_sha: 'older' })]])).toBe(true);
  });
  it.each(['success', 'failure', 'cancelled', 'skipped'])('does not retest an unchanged %s revision', async conclusion => {
    expect(await check([[run({ conclusion })]])).toBe(false);
  });
  it('counts manual full coverage and an earlier run still in progress', async () => {
    expect(await check([[run({ event: 'workflow_dispatch' })]])).toBe(false);
    expect(await check([[run({ conclusion: null })]])).toBe(false);
  });
  it('ignores core-only, current and newer runs', async () => {
    expect(await check([[
      run({ event: 'push' }), run({ event: 'pull_request' }),
      run({ id: 100 }), run({ id: 101 }),
    ]])).toBe(true);
  });
  it('finds earlier coverage beyond the first page', async () => {
    expect(await check([[run({ event: 'push' })], [run()]])).toBe(false);
  });
  it.each(['push', 'pull_request', 'workflow_dispatch'])('always permits %s', async eventName => {
    expect(await shouldRunCi({ context: { eventName } })).toBe(true);
  });
});
