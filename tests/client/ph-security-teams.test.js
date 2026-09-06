// @vitest-environment jsdom
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { t } from '../../gui/strings.js';
import '../../gui/components/ph-security-teams.js';

const TARGET = '00000000-0000-8000-8000-000000000042';

const BLACKBOARD = {
  range: 400,
  teams: [
    {
      state: 'working',
      target: TARGET,
      target_name: 'world.falling_skyway.entity.skyhook.name',
      action: 'assist_evacuation',
      progress: 0.5,
      risk: 0.65,
    },
    { state: 'available' },
  ],
  targets: [
    {
      uuid: TARGET,
      name: 'world.falling_skyway.entity.skyhook.name',
      separation: 180,
      in_range: true,
      actions: [
        {
          action: 'assist_evacuation',
          duration_secs: 45,
          risk: 0.65,
          priority: 'life_safety',
          warning: 'world.falling_skyway.security.warning.head_evacuation',
        },
      ],
    },
    {
      uuid: 'far-away',
      separation: 9000,
      in_range: false,
      actions: [
        { action: 'secure_contain', duration_secs: 60, risk: 0.4, priority: 'threat_containment' },
      ],
    },
  ],
  refusal: null,
};

function setup(sendAction) {
  if (sendAction) window.sendAction = sendAction;
  document.body.innerHTML = '<ph-security-teams id="test-el"></ph-security-teams>';
  return document.getElementById('test-el');
}

describe('PhSecurityTeams', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element with a shadow root', () => {
    expect(customElements.get('ph-security-teams')).toBeDefined();
    expect(setup().shadowRoot).toBeDefined();
  });

  it('renders its own empty state for a hull that musters no teams', () => {
    const el = setup();
    el.state = { teams: [], targets: [] };
    const empties = el.shadowRoot.querySelectorAll('.empty');
    expect(empties).toHaveLength(2);
    expect(empties[0].textContent).toBe(t('component.security_teams.no_teams'));
    expect(empties[1].textContent).toBe(t('component.security_teams.no_jobs'));
  });

  // AC2 (#1382): teams and jobs render as two ordered blocks inside a shared
  // `.blocks` wrapper — the DOM shape both the default side-by-side row
  // layout and the `@media (orientation: portrait)` column-flip rule depend
  // on. Pinning it here means a future edit that merges the two blocks back
  // into one, drops the `.blocks` wrapper, or deletes the media query fails
  // a unit test instead of only being caught by eyeballing a screenshot.
  it('renders teams and jobs as two ordered blocks inside the shared wrapper', () => {
    const el = setup();
    el.state = BLACKBOARD;
    const blocks = el.shadowRoot.getElementById('blocks');
    expect(blocks).not.toBeNull();
    expect(blocks.children).toHaveLength(2);

    const [teamsBlock, jobsBlock] = blocks.children;
    expect(teamsBlock.classList.contains('block')).toBe(true);
    expect(teamsBlock.classList.contains('teams-block')).toBe(true);
    expect(teamsBlock.querySelector('#teams')).not.toBeNull();

    expect(jobsBlock.classList.contains('block')).toBe(true);
    expect(jobsBlock.classList.contains('jobs-block')).toBe(true);
    expect(jobsBlock.querySelector('#jobs')).not.toBeNull();
  });

  // AC2: the team list with state, assignment and progress.
  it('lists every team with its state badge, assignment and progress bar', () => {
    const el = setup();
    el.state = BLACKBOARD;
    const cards = el.shadowRoot.querySelectorAll('.card');
    expect(cards).toHaveLength(2);

    expect(cards[0].querySelector('.status-badge').textContent).toBe(
      t('component.security_teams.state.working'),
    );
    expect(cards[0].querySelector('.assignment').textContent).toContain(
      t('component.security_teams.action.assist_evacuation'),
    );
    expect(cards[0].querySelector('.progress-fill').style.width).toBe('50%');

    expect(cards[1].querySelector('.status-badge').textContent).toBe(
      t('component.security_teams.state.available'),
    );
    expect(cards[1].querySelector('.assignment').textContent).toBe('');
  });

  // AC2: the eligible targets and the actions each offers, with the authored
  // priority, risk and warning beside them.
  it('lists every target/action with its priority chip, risk and authored warning', () => {
    const el = setup();
    el.state = BLACKBOARD;
    const jobs = el.shadowRoot.querySelectorAll('#jobs .job');
    expect(jobs).toHaveLength(2);
    expect(jobs[0].querySelector('.chip').textContent).toBe(
      t('component.security_teams.priority.life_safety'),
    );
    expect(jobs[0].querySelector('.risk').textContent).toBe(
      t('component.security_teams.risk', { pct: '65', secs: '45' }),
    );
    expect(el.shadowRoot.querySelector('#jobs .warning').textContent).toBe(
      t('world.falling_skyway.security.warning.head_evacuation'),
    );
  });

  // The console never re-derives the reach: the server's `in_range` is what
  // greys the control, so the panel can never offer a dispatch the server
  // refuses for a reason the panel computed differently.
  it('greys an out-of-reach job on the server’s answer, not on the separation', () => {
    const el = setup();
    el.state = BLACKBOARD;
    const jobs = el.shadowRoot.querySelectorAll('#jobs .job');
    expect(jobs[0].disabled).toBe(false);
    expect(jobs[1].disabled).toBe(true);
  });

  it('offers nothing while every team is out, however reachable the work is', () => {
    const el = setup();
    el.state = { ...BLACKBOARD, teams: [{ state: 'working' }, { state: 'deploying' }] };
    const jobs = el.shadowRoot.querySelectorAll('#jobs .job');
    expect([...jobs].every((job) => job.disabled)).toBe(true);
  });

  // AC2/AC5: the dispatch and the recall are the same admitted commands the AI
  // emits, routed through the action map.
  it('dispatches the first free team to the tapped target and action', () => {
    const sendAction = vi.fn();
    const el = setup(sendAction);
    el.state = BLACKBOARD;
    el.shadowRoot.querySelector('#jobs .job').click();
    expect(sendAction).toHaveBeenCalledWith('dispatch_security_team', {
      team_idx: 1,
      target: TARGET,
      action: 'assist_evacuation',
    });
  });

  it('offers a recall only for a team that is actually out, and names it', () => {
    const sendAction = vi.fn();
    const el = setup(sendAction);
    el.state = BLACKBOARD;
    const recalls = el.shadowRoot.querySelectorAll('[data-recall]');
    expect(recalls).toHaveLength(1);
    expect(recalls[0].dataset.recall).toBe('0');
    recalls[0].click();
    expect(sendAction).toHaveBeenCalledWith('recall_security_team', { team_idx: 0 });
  });

  it('shows the last refusal as a resolved strings id, and hides it when there is none', () => {
    const el = setup();
    el.state = { ...BLACKBOARD, refusal: 'security.dispatch.refused.team_busy' };
    const refusal = el.shadowRoot.getElementById('refusal');
    expect(refusal.hidden).toBe(false);
    expect(refusal.textContent).toBe(t('security.dispatch.refused.team_busy'));

    el.state = BLACKBOARD;
    expect(el.shadowRoot.getElementById('refusal').hidden).toBe(true);
  });
});
