// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { t } from '../../gui/strings.js';
import { renderJobWorkList } from '../../gui/job-work-list.js';

describe('permitted job work list', () => {
  it('reports workflow interruption without adding commands or exposing private memory', () => {
    const container = document.createElement('div');
    const job = { id: 42, team_idx: 1, phase: 'working', workflow_error: true,
      runtime: { secret: 'concealed diagnosis' }, target_entity: '<button>inject</button>' };
    renderJobWorkList(container, [job], [{ id: 1, label: 'Team 2' }]);
    expect(container.textContent).toContain('42');
    expect(container.textContent).toContain('Team 2');
    expect(container.textContent).toContain(t('job.workflow_error'));
    expect(container.textContent).not.toContain('concealed');
    expect(container.querySelector('button')).toBeNull();
    const held = container.firstElementChild;
    renderJobWorkList(container, [job], [{ id: 1, label: 'Team 2' }]);
    expect(container.firstElementChild).toBe(held);
    renderJobWorkList(container, [{ ...job, phase: 'finished', outcome: 'cancelled', workflow_error: false }], []);
    expect(container.textContent).toContain(t('job.outcome.cancelled'));
    expect(container.querySelectorAll('[data-job-id]')).toHaveLength(1);
    renderJobWorkList(container, [], []);
    expect(container.children).toHaveLength(0);
  });
});
