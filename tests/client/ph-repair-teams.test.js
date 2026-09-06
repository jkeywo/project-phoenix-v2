// @vitest-environment jsdom
//
// tests/client/ph-repair-teams.test.js — the repair roster as selectable cards
// (issue #1384, PRD #1371).
//
// The roster is a SUMMARY until a team is selected: nothing offers a verb until
// the player says which team they mean. Every assertion below therefore either
// checks what a collapsed card refuses to show, or selects a card first and
// then checks what that card offers. The host-trust assertions issue #1015
// wrote about the damaged-systems list are unchanged in substance — they simply
// live inside the on-site team's own card now instead of a standalone block.
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import '../../gui/components/ph-repair-teams.js';

function setup(opts) {
  if (opts && opts.sendAction) {
    window.sendAction = opts.sendAction;
  }
  document.body.innerHTML = '<ph-repair-teams id="test-el"></ph-repair-teams>';
  const el = document.getElementById('test-el');
  return { el };
}

/** Open (or close) one team's card the way a player does — by tapping it. */
function select(el, teamId) {
  const top = el.shadowRoot.querySelector(`.card[data-team-id="${teamId}"] .card-top`);
  top.click();
  return top;
}

const card = (el, teamId) => el.shadowRoot.querySelector(`.card[data-team-id="${teamId}"]`);

describe('PhRepairTeams', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-repair-teams')).toBeDefined();
  });

  it('creates a shadow root', () => {
    const { el } = setup();
    expect(el.shadowRoot).toBeDefined();
  });

  it('renders empty state with no repair teams', () => {
    const { el } = setup();
    el.state = {};
    expect(el.shadowRoot.textContent).toContain(t('component.repair_teams.empty'));
  });

  it('renders empty state when teams is null', () => {
    const { el } = setup();
    el.state = { teams: null };
    expect(el.shadowRoot.textContent).toContain(t('component.repair_teams.empty'));
  });

  it('drops the cards of a roster that empties', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }] };
    el.state = { teams: [] };
    expect(el.shadowRoot.querySelectorAll('.card').length).toBe(0);
    expect(el.shadowRoot.textContent).toContain(t('component.repair_teams.empty'));
  });

  // ── The roster is a summary until a team is selected ───────────────────────

  it('renders every team as a collapsed card that offers no controls', () => {
    const { el } = setup();
    el.state = {
      teams: [
        { id: 0, label: 'Team 1', status: 'idle' },
        { id: 1, label: 'Team 2', status: 'idle' },
      ],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    expect(el.shadowRoot.querySelectorAll('.card').length).toBe(2);
    expect(el.shadowRoot.querySelectorAll('.btn').length).toBe(0);
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(true);
    expect(card(el, 0).querySelector('.card-top').getAttribute('aria-expanded')).toBe('false');
  });

  it('opens the card the player selects and marks it expanded', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'idle' }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    const top = select(el, 0);
    expect(card(el, 0).classList.contains('selected')).toBe(true);
    expect(top.getAttribute('aria-expanded')).toBe('true');
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(false);
  });

  it('opens one card at a time', () => {
    const { el } = setup();
    el.state = {
      teams: [
        { id: 0, label: 'Team 1', status: 'idle' },
        { id: 1, label: 'Team 2', status: 'idle' },
      ],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    select(el, 0);
    select(el, 1);
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(true);
    expect(card(el, 0).querySelectorAll('.btn').length).toBe(0);
    expect(card(el, 1).querySelector('.card-body').hidden).toBe(false);
  });

  it('closes the open card when it is selected again', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'idle' }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    select(el, 0);
    select(el, 0);
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(true);
    expect(card(el, 0).classList.contains('selected')).toBe(false);
  });

  it('keeps the open card open across a state push', () => {
    const { el } = setup();
    const state = () => ({
      teams: [{ id: 0, label: 'Team 1', status: 'idle' }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    });
    el.state = state();
    select(el, 0);
    el.state = state();
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(false);
  });

  it('drops the selection when the selected team leaves the roster', () => {
    const { el } = setup();
    el.state = {
      teams: [
        { id: 0, label: 'Team 1', status: 'idle' },
        { id: 1, label: 'Team 2', status: 'idle' },
      ],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    select(el, 1);
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'idle' }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    expect(el.selectedTeamId).toBeNull();
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(true);
  });

  // ── An open idle card: DISPATCH TO ────────────────────────────────────────

  it('offers one destination per damageable station inside the open idle card', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'idle' }],
      targets: [
        { id: 'helm', label: 'Helm', damage_pct: 0.4 },
        { id: 'tactical', label: 'Tactical', damage_pct: 0 },
      ],
    };
    select(el, 0);
    const drow = card(el, 0).querySelector('.dispatch-row');
    expect(drow.style.display).not.toBe('none');
    expect(card(el, 0).querySelector('.body-head').textContent)
      .toBe(t('component.repair_teams.dispatch_to'));
    const btns = drow.querySelectorAll('.btn');
    expect(btns.length).toBe(2);
    expect(btns[0].dataset.target).toBe('helm');
    expect(btns[1].dataset.target).toBe('tactical');
  });

  it('says so inside the open card when there is nowhere to send a team', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'idle', target: null, progress_pct: 0 }],
      targets: [],
    };
    select(el, 0);
    expect(card(el, 0).textContent).toContain(t('component.repair_teams.no_targets'));
    expect(card(el, 0).querySelectorAll('.btn').length).toBe(0);
  });

  it('dispatches dispatch_repair_team with the open card team and the tapped target', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      teams: [
        { id: 0, label: 'T1', status: 'idle' },
        { id: 1, label: 'T2', status: 'idle' },
      ],
      targets: [
        { id: 'helm', label: 'Helm', damage_pct: 0.4 },
        { id: 'core', label: 'Core', damage_pct: 0.2 },
      ],
    };
    select(el, 1);
    card(el, 1).querySelector('.btn[data-target="core"]').click();
    expect(sendAction).toHaveBeenCalledWith('dispatch_repair_team', { team_idx: 1, target: 'core' });
  });

  it('disables the destinations while repair is on AUTO', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
      auto: true,
    };
    select(el, 0);
    expect(card(el, 0).querySelector('.btn').disabled).toBe(true);
  });

  // ── The field target (issue #1161's command, offered per team) ────────────

  it('offers no field row on a hull that authored no external dispatch', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
      external_dispatch: null,
    };
    select(el, 0);
    expect(card(el, 0).querySelector('.field-btn')).toBeNull();
  });

  it('adds the field target after the ship destinations on a hull that has one', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
      external_dispatch: { range: 800, target: null, target_name: null },
    };
    select(el, 0);
    const btns = card(el, 0).querySelectorAll('.btn');
    expect(btns.length).toBe(2);
    expect(btns[1].classList.contains('field-btn')).toBe(true);
    expect(btns[1].querySelector('.label').textContent)
      .toBe(t('component.repair_teams.field_target'));
  });

  it('names the field target from the host once a team is working it', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      targets: [],
      external_dispatch: { range: 800, target: 'uuid-1', target_name: 'console.repair.dispatch' },
    };
    select(el, 0);
    expect(card(el, 0).querySelector('.field-btn .label').textContent)
      .toBe(t('console.repair.dispatch'));
  });

  // Issue #1386: the field row is one destination among the stations and sends
  // the SAME verb, naming this card's own team. Before it sent the fieldless
  // command and the host picked whoever was free.
  it('sends a named dispatch to the field target from the field row', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      teams: [
        { id: 0, label: 'T1', status: 'idle' },
        { id: 1, label: 'T2', status: 'idle' },
      ],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
      external_dispatch: { range: 800, target: null, target_name: null, team_idx: null },
    };
    select(el, 1);
    card(el, 1).querySelector('.field-btn').click();
    expect(sendAction).toHaveBeenCalledWith(
      'dispatch_repair_team',
      expect.objectContaining({ team_idx: 1, target: 'external' }),
    );
  });

  // The EXTERNAL recall must not be reachable from a destination list: while
  // ANOTHER team already holds the ship's one claim the field row is a readout,
  // and it carries that refusal's own text so the crew are told what to do
  // instead. The way back is the abroad card's own RECALL row.
  it('makes the field row a readout with the refusal text while another team is abroad', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      teams: [
        { id: 0, label: 'T1', status: 'idle' },
        { id: 1, label: 'T2', status: 'idle' },
      ],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
      external_dispatch: {
        range: 800, target: 'uuid-1', target_name: 'console.repair.dispatch', team_idx: 1,
      },
    };
    select(el, 0);
    const field = card(el, 0).querySelector('.field-btn');
    expect(field.disabled).toBe(true);
    expect(field.title).toBe(t('repair.dispatch.refused.already_abroad'));
    field.click();
    expect(sendAction).not.toHaveBeenCalled();
    expect(el.shadowRoot.textContent).not.toContain(t('console.repair.dispatch.recall'));
    expect(card(el, 0).querySelector('.recall-btn')).toBeNull();
  });

  // ── The team abroad (issue #1386) ────────────────────────────────────────

  const ABROAD_STATE = {
    teams: [
      { id: 0, label: 'T1', status: 'idle' },
      { id: 1, label: 'T2', status: 'idle' },
    ],
    targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    external_dispatch: {
      range: 800,
      target: 'uuid-1',
      target_name: 'console.repair.dispatch',
      team_idx: 1,
      target_condition: 0.6,
    },
  };

  it('paints the team named by the claim ABROAD, with the target it is working', () => {
    const { el } = setup();
    el.state = ABROAD_STATE;

    const abroad = card(el, 1);
    expect(abroad.textContent).toContain(t('component.repair_teams.status.abroad'));
    expect(abroad.textContent).toContain(
      t('component.repair_teams.target', { target: t('console.repair.dispatch') }),
    );
    // …and the OTHER idle team is untouched: the claim names one slot, so no
    // second card may read as if it had gone anywhere.
    expect(card(el, 0).textContent).toContain(t('component.repair_teams.status.idle'));
  });

  it('fills the abroad card\u2019s bar from the target\u2019s own condition track', () => {
    const { el } = setup();
    el.state = ABROAD_STATE;

    const fill = card(el, 1).querySelector('.progress-fill');
    expect(fill.classList.contains('abroad')).toBe(true);
    expect(fill.style.width).toBe('60%');

    // A target with no condition track banks nothing, so the bar draws nothing
    // rather than inventing a number.
    el.state = {
      ...ABROAD_STATE,
      external_dispatch: { ...ABROAD_STATE.external_dispatch, target_condition: null },
    };
    expect(card(el, 1).querySelector('.progress-fill').style.width).toBe('0%');
  });

  it('offers the abroad team RECALL \u2014 the one recall verb, naming its team', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = ABROAD_STATE;

    const top = select(el, 1);
    expect(top.disabled).toBe(false);
    expect(top.getAttribute('aria-expanded')).toBe('true');
    const recall = card(el, 1).querySelector('.recall-btn');
    expect(recall).not.toBeNull();
    recall.click();
    expect(sendAction).toHaveBeenCalledWith(
      'recall_repair_team',
      expect.objectContaining({ team_idx: 1 }),
    );
    // No destinations on an abroad card: it is already somewhere.
    expect(card(el, 1).querySelector('.field-btn')).toBeNull();
    expect(card(el, 1).querySelectorAll('.btn').length).toBe(1);
  });

  // The claim names an index, and the SLOT still has to agree. A stale claim
  // naming a team the host has since sent across this hull must not paint that
  // team ABROAD while it walks.
  it('ignores a claim naming a team that is out on an internal job', () => {
    const { el } = setup();
    el.state = {
      ...ABROAD_STATE,
      teams: [
        { id: 0, label: 'T1', status: 'idle' },
        { id: 1, label: 'T2', status: 'travelling', target: 'Helm', progress_pct: 0.3 },
      ],
    };
    expect(card(el, 1).textContent).toContain(t('component.repair_teams.status.travelling'));
    expect(card(el, 1).textContent).not.toContain(t('component.repair_teams.status.abroad'));
  });

  it('disables the field row while repair is on AUTO', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      targets: [],
      external_dispatch: { range: 800, target: null, target_name: null },
      auto: true,
    };
    select(el, 0);
    expect(card(el, 0).querySelector('.field-btn').disabled).toBe(true);
  });

  // A hull with no field-repair capability at all: every idle team is an
  // ordinary idle team, and none of them is painted ABROAD off a claim that
  // does not exist.
  it('offers every idle team its destinations when the hull cannot dispatch abroad', () => {
    const { el } = setup();
    el.state = {
      teams: [
        { id: 0, label: 'T1', status: 'idle' },
        { id: 1, label: 'T2', status: 'idle' },
      ],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    expect(el.shadowRoot.textContent).not.toContain(t('component.repair_teams.status.abroad'));
    select(el, 1);
    expect(card(el, 1).querySelectorAll('.btn').length).toBe(1);
    select(el, 0);
    expect(card(el, 0).querySelectorAll('.btn').length).toBe(1);
  });

  // ── A team out on a job: RECALL, and on site the rows too ─────────────

  it('renders a busy team with its target name and no dispatch row', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'repairing', target: 'Helm', progress_pct: 0.45, eta_secs: 12 }],
    };
    expect(el.shadowRoot.textContent).toContain('REPAIRING');
    expect(el.shadowRoot.textContent).toContain('Helm');
    expect(card(el, 0).querySelectorAll('.btn').length).toBe(0);
  });

  // Issue #1385: a team sent to the wrong station is the one order a seat wants
  // to undo, and undoing it early is cheap — the walk back is only as long as
  // the walk out so far. The card offers RECALL and nothing else: the on-site
  // rows belong to a team that has actually arrived.
  it('offers a travelling team RECALL and no other control', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'travelling', target: 'Helm', progress_pct: 0.3 }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
      damaged: [{ system_id: 'core', display_name: 'Core', tier: 'Damaged', damage_pct: 0.3, prioritisable: true }],
    };
    select(el, 0);
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(false);
    const btns = card(el, 0).querySelectorAll('.btn');
    expect(btns.length).toBe(1);
    expect(btns[0].classList.contains('recall-btn')).toBe(true);
    expect(btns[0].querySelector('.label').textContent)
      .toBe(t('component.repair_teams.recall'));
    expect(card(el, 0).querySelectorAll('.dmg-row').length).toBe(0);
    expect(card(el, 0).querySelector('.dispatch-row').style.display).toBe('none');
  });

  it('sends recall_repair_team naming the card team', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      teams: [
        { id: 0, label: 'T1', status: 'idle' },
        { id: 1, label: 'T2', status: 'travelling', target: 'Helm', progress_pct: 0.3 },
      ],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    select(el, 1);
    card(el, 1).querySelector('.recall-btn').click();
    expect(sendAction).toHaveBeenCalledWith('recall_repair_team', { team_idx: 1 });
  });

  it('offers an on-site team RECALL alongside its damaged rows', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 0.5 }],
      damaged: [{ system_id: 'core', display_name: 'Core', tier: 'Damaged', damage_pct: 0.3, prioritisable: true }],
    };
    select(el, 0);
    expect(card(el, 0).querySelectorAll('.dmg-row').length).toBe(1);
    expect(card(el, 0).querySelector('.recall-btn')).not.toBeNull();
  });

  // AUTO is the seat saying it is not the one giving orders, so RECALL greys
  // with every other verb on the panel rather than staying live.
  it('disables RECALL while repair is on AUTO', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'travelling', target: 'Helm', progress_pct: 0.3 }],
      auto: true,
    };
    select(el, 0);
    const btn = card(el, 0).querySelector('.recall-btn');
    expect(btn.disabled).toBe(true);
    btn.click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('leaves no recall control behind on a card the player closed', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'travelling', target: 'Helm', progress_pct: 0.3 }],
    };
    select(el, 0);
    expect(card(el, 0).querySelector('.recall-btn')).not.toBeNull();
    select(el, 0);
    expect(card(el, 0).querySelector('.recall-btn')).toBeNull();
  });

  // The card must not SAY it opened when it did not: a rotated caret, the cyan
  // selected border and `aria-expanded="true"` over a hidden region are three
  // ways of promising a body that is not there. A returning team is already on
  // its way home, so the host refuses a recall and the card offers no toggle.
  it('does not offer a returning team as a toggle at all', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'returning', progress_pct: 0.9 }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
    };
    const top = card(el, 0).querySelector('.card-top');
    expect(top.disabled).toBe(true);
    select(el, 0);
    expect(top.getAttribute('aria-expanded')).toBe('false');
    expect(card(el, 0).classList.contains('selected')).toBe(false);
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(true);
  });

  it('makes a card a toggle again once its team has something to open onto', () => {
    const { el } = setup();
    const rows = [{ system_id: 'core', display_name: 'Core', tier: 'Damaged', damage_pct: 0.3, prioritisable: true }];
    el.state = { teams: [{ id: 0, label: 'T1', status: 'returning', progress_pct: 0.9 }], damaged: rows };
    expect(card(el, 0).querySelector('.card-top').disabled).toBe(true);
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 0.2 }], damaged: rows };
    const top = card(el, 0).querySelector('.card-top');
    expect(top.disabled).toBe(false);
    expect(card(el, 0).querySelector('.caret').style.visibility).toBe('');
    select(el, 0);
    expect(top.getAttribute('aria-expanded')).toBe('true');
    expect(card(el, 0).classList.contains('selected')).toBe(true);
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(false);
  });

  it('renders status badges for all statuses', () => {
    const { el } = setup();
    el.state = {
      teams: [
        { id: 0, label: 'T1', status: 'idle' },
        { id: 1, label: 'T2', status: 'travelling', target: 'Helm', progress_pct: 0.3 },
        { id: 2, label: 'T3', status: 'repairing', target: 'Engines', progress_pct: 0.6 },
        { id: 3, label: 'T4', status: 'returning', progress_pct: 0.9 },
      ],
    };
    expect(el.shadowRoot.textContent).toContain('IDLE');
    expect(el.shadowRoot.textContent).toContain('TRAVELLING');
    expect(el.shadowRoot.textContent).toContain('REPAIRING');
    expect(el.shadowRoot.textContent).toContain('RETURNING');
  });

  it('shows progress bar for active team', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 0.45 }],
    };
    const fill = el.shadowRoot.querySelector('.progress-fill');
    expect(fill).toBeDefined();
    expect(parseFloat(fill.style.width)).toBeGreaterThan(0);
  });

  it('shows AUTO badge when auto=true', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }], auto: true };
    expect(el.shadowRoot.getElementById('auto-badge').style.display).not.toBe('none');
  });

  it('hides AUTO badge when auto=false', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }], auto: false };
    expect(el.shadowRoot.getElementById('auto-badge').style.display).toBe('none');
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }] };
    expect(el.shadowRoot.textContent).toContain('IDLE');
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 0.5 }] };
    expect(el.shadowRoot.textContent).toContain('REPAIRING');
  });

  // ── On-site systems, inside the on-site team's own card ───────────────────
  //
  // Issue #1015's tap-to-prioritise list, re-homed by #1384. Every assertion is
  // about what the card RENDERS and what it SENDS; which team and system a tap
  // pins is resolved host-side and is not this component's business.

  const damagedRows = () => [
    { system_id: 'hull-plating', display_name: 'Hull Plating', tier: 'Destroyed', damage_pct: 1, prioritised: false, in_progress: false, prioritisable: true },
    { system_id: 'helm-engine-port', display_name: 'Port Engine', tier: 'Disabled', damage_pct: 0.8, prioritised: false, in_progress: true, prioritisable: false },
    { system_id: 'core', display_name: 'Core', tier: 'Damaged', damage_pct: 0.3, prioritised: false, in_progress: false, prioritisable: true },
  ];

  /** An on-site team plus a damaged ship, with that team's card already open. */
  function onSite(overrides = {}) {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 1 }],
      damaged: damagedRows(),
      ...overrides,
    };
    select(el, 0);
    return { el, sendAction };
  }

  it('has no standalone damaged-systems section any more', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }], damaged: damagedRows() };
    expect(el.shadowRoot.getElementById('damaged')).toBeNull();
    expect(el.shadowRoot.querySelectorAll('.dmg-row').length).toBe(0);
  });

  it('shows no damaged systems at all when the ship has no repair teams', () => {
    const { el } = setup();
    el.state = { teams: [], damaged: damagedRows() };
    expect(el.shadowRoot.querySelectorAll('.dmg-row').length).toBe(0);
  });

  it('has no per-team priority ordinal buttons', () => {
    const { el } = onSite();
    expect(el.shadowRoot.querySelector('.priority-row')).toBeNull();
    expect(el.shadowRoot.querySelectorAll('.priority-btn').length).toBe(0);
  });

  it('lists the on-site systems inside the selected team card, in the order given', () => {
    const { el } = onSite();
    const rows = card(el, 0).querySelectorAll('.dmg-row');
    expect(rows.length).toBe(3);
    expect(Array.from(rows).map(r => r.dataset.systemId))
      .toEqual(['hull-plating', 'helm-engine-port', 'core']);
    expect(card(el, 0).querySelector('.body-head').textContent)
      .toBe(t('component.repair_teams.damaged_title'));
    expect(rows[0].querySelector('.name').textContent).toBe('Hull Plating');
    expect(rows[0].querySelector('.tier-chip').textContent)
      .toBe(t('component.repair_teams.tier.destroyed'));
    expect(rows[2].querySelector('.pct').textContent).toBe('30%');
  });

  // Before issue #1385 an on-site team with no visible rows had no body at all.
  // It still has no rows — the host is showing this seat none — but it does now
  // have a verb, so the card opens onto RECALL alone rather than not opening.
  it('opens an on-site card onto RECALL alone while the visible systems are intact', () => {
    const { el } = onSite({ damaged: [] });
    const top = card(el, 0).querySelector('.card-top');
    expect(top.disabled).toBe(false);
    expect(top.getAttribute('aria-expanded')).toBe('true');
    expect(card(el, 0).querySelector('.card-body').hidden).toBe(false);
    expect(el.shadowRoot.querySelectorAll('.dmg-row').length).toBe(0);
    expect(card(el, 0).querySelector('.recall-btn')).not.toBeNull();
    // Zero rows is not the same claim as "the section went away": an empty
    // [DAMAGED SYSTEMS] heading over nothing would pass a count-only check.
    expect(card(el, 0).querySelector('.body-head').textContent).toBe('');
  });

  it('sends set_repair_target_priority with the tapped system id', () => {
    const { el, sendAction } = onSite();
    card(el, 0).querySelector('.dmg-row[data-system-id="hull-plating"]').click();
    expect(sendAction).toHaveBeenCalledWith('set_repair_target_priority', { system_id: 'hull-plating' });
  });

  it('sends no ordinal — the host resolves the rank', () => {
    const { el, sendAction } = onSite();
    card(el, 0).querySelector('.dmg-row[data-system-id="core"]').click();
    const [, payload] = sendAction.mock.calls[0];
    expect(Object.keys(payload)).toEqual(['system_id']);
  });

  it('highlights only the row the host reports as prioritised', () => {
    const rows = damagedRows();
    rows[2].prioritised = true;
    const { el } = onSite({ damaged: rows });
    const highlighted = el.shadowRoot.querySelectorAll('.dmg-row.prioritised');
    expect(highlighted.length).toBe(1);
    expect(highlighted[0].dataset.systemId).toBe('core');
    expect(highlighted[0].querySelector('.flag').textContent)
      .toBe(t('component.repair_teams.next'));
  });

  it('moves the highlight when the host echoes a different pin', () => {
    const first = damagedRows();
    first[0].prioritised = true;
    const { el } = onSite({ damaged: first });
    expect(el.shadowRoot.querySelector('.dmg-row.prioritised').dataset.systemId).toBe('hull-plating');

    const second = damagedRows();
    second[2].prioritised = true;
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 1 }],
      damaged: second,
    };
    const highlighted = el.shadowRoot.querySelectorAll('.dmg-row.prioritised');
    expect(highlighted.length).toBe(1);
    expect(highlighted[0].dataset.systemId).toBe('core');
  });

  it('flags the row a team is already working on', () => {
    const { el } = onSite();
    const row = card(el, 0).querySelector('.dmg-row[data-system-id="helm-engine-port"]');
    expect(row.querySelector('.flag').textContent).toBe(t('component.repair_teams.working'));
  });

  // The host's sweep never offers the system a team is standing on as a
  // candidate, so a tap on an in-progress row resolves to nothing. It must not
  // present as a control the player can spend a tap on.
  it('does not offer the row a team is already on as tappable', () => {
    const { el, sendAction } = onSite();
    const row = card(el, 0).querySelector('.dmg-row[data-system-id="helm-engine-port"]');
    expect(row.disabled).toBe(true);
    expect(row.title).toBe(t('component.repair_teams.working_title', { name: 'Port Engine' }));
    row.click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('keeps the other rows tappable while one is in progress', () => {
    const { el, sendAction } = onSite();
    const row = card(el, 0).querySelector('.dmg-row[data-system-id="core"]');
    expect(row.disabled).toBe(false);
    expect(row.title).toBe(t('component.repair_teams.prioritise_title', { name: 'Core' }));
    row.click();
    expect(sendAction).toHaveBeenCalledWith('set_repair_target_priority', { system_id: 'core' });
  });

  // A stale pin echo remains visible, but owner eligibility is the enablement
  // source; a highlight cannot turn an unreachable row into a live control.
  it('does not let a stale prioritised echo override owner eligibility', () => {
    const rows = damagedRows();
    rows[1].prioritised = true;
    const { el } = onSite({ damaged: rows });
    const row = card(el, 0).querySelector('.dmg-row[data-system-id="helm-engine-port"]');
    expect(row.disabled).toBe(true);
    expect(row.title)
      .toBe(t('component.repair_teams.prioritised_title', { name: 'Port Engine' }));
  });

  it('renders visible damage outside every on-site sweep as a disabled readout', () => {
    const rows = damagedRows();
    rows[2].prioritisable = false;
    const { el, sendAction } = onSite({ damaged: rows });
    const row = card(el, 0).querySelector('.dmg-row[data-system-id="core"]');
    expect(row.disabled).toBe(true);
    expect(row.title)
      .toBe(t('component.repair_teams.unavailable_title', { name: 'Core' }));
    row.click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('disables the on-site rows and sends nothing while repair is on AUTO', () => {
    const { el, sendAction } = onSite({ auto: true });
    const row = card(el, 0).querySelector('.dmg-row[data-system-id="core"]');
    expect(row.disabled).toBe(true);
    row.click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('drops rows that are no longer damaged', () => {
    const { el } = onSite();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 1 }],
      damaged: [damagedRows()[2]],
    };
    const rows = card(el, 0).querySelectorAll('.dmg-row');
    expect(rows.length).toBe(1);
    expect(rows[0].dataset.systemId).toBe('core');
  });
});
