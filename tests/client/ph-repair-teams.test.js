// @vitest-environment jsdom
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

function queryText(host, sel) {
  const el = host.shadowRoot.querySelector(sel);
  return el ? el.textContent.trim() : null;
}

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

  it('renders an idle team with no targets as "No repair targets"', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'idle', target: null, progress_pct: 0, eta_secs: 0 }],
      targets: [],
    };
    expect(el.shadowRoot.textContent).toContain('No repair targets');
  });

  it('offers one dispatch button per damageable station for an idle team', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'idle' }],
      targets: [
        { id: 'helm', label: 'Helm', damage_pct: 0.4 },
        { id: 'tactical', label: 'Tactical', damage_pct: 0 },
      ],
    };
    const drow = el.shadowRoot.querySelector('.dispatch-row');
    expect(drow.style.display).not.toBe('none');
    const btns = drow.querySelectorAll('.btn');
    expect(btns.length).toBe(2);
    expect(btns[0].dataset.target).toBe('helm');
    expect(btns[1].dataset.target).toBe('tactical');
  });

  it('renders repairing team with target name', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'repairing', target: 'Helm', progress_pct: 0.45, eta_secs: 12 }],
    };
    expect(el.shadowRoot.textContent).toContain('REPAIRING');
    expect(el.shadowRoot.textContent).toContain('Helm');
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
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      auto: true,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).not.toBe('none');
  });

  it('hides AUTO badge when auto=false', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      auto: false,
    };
    const badge = el.shadowRoot.getElementById('auto-badge');
    expect(badge.style.display).toBe('none');
  });

  it('no dispatch buttons render for an idle team with no damageable stations', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      targets: [],
    };
    const btns = el.shadowRoot.querySelectorAll('.btn');
    expect(btns.length).toBe(0);
  });

  it('dispatch buttons are disabled when auto is on', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      targets: [{ id: 'helm', label: 'Helm', damage_pct: 0.4 }],
      auto: true,
    };
    const btn = el.shadowRoot.querySelector('.btn');
    expect(btn.disabled).toBe(true);
  });

  it('hides the dispatch row for a busy team', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 0.3 }],
    };
    const drow = el.shadowRoot.querySelector('.dispatch-row');
    expect(drow.style.display).toBe('none');
  });

  it('dispatches dispatch_repair_team with the clicked target for an idle team', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
      targets: [
        { id: 'helm', label: 'Helm', damage_pct: 0.4 },
        { id: 'core', label: 'Core', damage_pct: 0.2 },
      ],
    };
    const btn = el.shadowRoot.querySelector('.btn[data-target="core"]');
    btn.click();
    expect(sendAction).toHaveBeenCalledWith('dispatch_repair_team', { team_idx: 0, target: 'core' });
  });

  // ── Damaged-systems list (issue #1015) ─────────────────────────────────────
  //
  // Replaces the per-team 1/2/3 ordinal buttons. Every assertion below is about
  // what the component RENDERS and what it SENDS; which team and system a tap
  // pins is resolved host-side and is not this component's business (see
  // gui/repair-dispatch.js for why the console cannot compute it — the
  // ordinal itself is untouched by the pin).

  const damagedRows = () => [
    { system_id: 'hull-plating', display_name: 'Hull Plating', tier: 'Destroyed', damage_pct: 1, prioritised: false, in_progress: false },
    { system_id: 'helm-engine-port', display_name: 'Port Engine', tier: 'Disabled', damage_pct: 0.8, prioritised: false, in_progress: true },
    { system_id: 'core', display_name: 'Core', tier: 'Damaged', damage_pct: 0.3, prioritised: false, in_progress: false },
  ];

  it('has no per-team priority buttons any more', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 1 }],
    };
    expect(el.shadowRoot.querySelector('.priority-row')).toBeNull();
    expect(el.shadowRoot.querySelectorAll('.priority-btn').length).toBe(0);
  });

  it('hides the damaged-systems section when nothing is damaged', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }], damaged: [] };
    expect(el.shadowRoot.getElementById('damaged').hidden).toBe(true);
  });

  it('renders one tappable row per damaged system, in the order given', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }], damaged: damagedRows() };
    expect(el.shadowRoot.getElementById('damaged').hidden).toBe(false);
    const rows = el.shadowRoot.querySelectorAll('.dmg-row');
    expect(rows.length).toBe(3);
    expect(Array.from(rows).map(r => r.dataset.systemId))
      .toEqual(['hull-plating', 'helm-engine-port', 'core']);
    expect(rows[0].querySelector('.name').textContent).toBe('Hull Plating');
    expect(rows[0].querySelector('.tier-chip').textContent)
      .toBe(t('component.repair_teams.tier.destroyed'));
    expect(rows[2].querySelector('.pct').textContent).toBe('30%');
  });

  it('sends set_repair_target_priority with the tapped system id', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm' }], damaged: damagedRows() };
    el.shadowRoot.querySelector('.dmg-row[data-system-id="hull-plating"]').click();
    expect(sendAction).toHaveBeenCalledWith('set_repair_target_priority', { system_id: 'hull-plating' });
  });

  it('sends no ordinal — the host resolves the rank', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: damagedRows() };
    el.shadowRoot.querySelector('.dmg-row[data-system-id="core"]').click();
    const [, payload] = sendAction.mock.calls[0];
    expect(Object.keys(payload)).toEqual(['system_id']);
  });

  it('highlights only the row the host reports as prioritised', () => {
    const { el } = setup();
    const rows = damagedRows();
    rows[2].prioritised = true;
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: rows };
    const highlighted = el.shadowRoot.querySelectorAll('.dmg-row.prioritised');
    expect(highlighted.length).toBe(1);
    expect(highlighted[0].dataset.systemId).toBe('core');
    expect(highlighted[0].querySelector('.flag').textContent)
      .toBe(t('component.repair_teams.next'));
  });

  it('moves the highlight when the host echoes a different pin', () => {
    const { el } = setup();
    const first = damagedRows();
    first[0].prioritised = true;
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: first };
    expect(el.shadowRoot.querySelector('.dmg-row.prioritised').dataset.systemId).toBe('hull-plating');

    const second = damagedRows();
    second[2].prioritised = true;
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: second };
    const highlighted = el.shadowRoot.querySelectorAll('.dmg-row.prioritised');
    expect(highlighted.length).toBe(1);
    expect(highlighted[0].dataset.systemId).toBe('core');
  });

  it('flags the row a team is already working on', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: damagedRows() };
    const row = el.shadowRoot.querySelector('.dmg-row[data-system-id="helm-engine-port"]');
    expect(row.querySelector('.flag').textContent).toBe(t('component.repair_teams.working'));
  });

  // The host's sweep never offers the system a team is standing on as a
  // candidate, so a tap on an in-progress row resolves to nothing. It must not
  // present as a control the player can spend a tap on.
  it('does not offer the row a team is already on as tappable', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: damagedRows() };
    const row = el.shadowRoot.querySelector('.dmg-row[data-system-id="helm-engine-port"]');
    expect(row.disabled).toBe(true);
    expect(row.title).toBe(t('component.repair_teams.working_title', { name: 'Port Engine' }));
    row.click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('keeps the other rows tappable while one is in progress', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: damagedRows() };
    const row = el.shadowRoot.querySelector('.dmg-row[data-system-id="core"]');
    expect(row.disabled).toBe(false);
    expect(row.title).toBe(t('component.repair_teams.prioritise_title', { name: 'Core' }));
    row.click();
    expect(sendAction).toHaveBeenCalledWith('set_repair_target_priority', { system_id: 'core' });
  });

  // A pin is a choice the host actually made, so the row keeps its live state
  // even if the same payload also reports a team on it — the highlight is not
  // something to grey out.
  it('leaves a prioritised row live even when it also reads in_progress', () => {
    const { el } = setup();
    const rows = damagedRows();
    rows[1].prioritised = true;
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: rows };
    const row = el.shadowRoot.querySelector('.dmg-row[data-system-id="helm-engine-port"]');
    expect(row.disabled).toBe(false);
    expect(row.title)
      .toBe(t('component.repair_teams.prioritised_title', { name: 'Port Engine' }));
  });

  it('disables the damaged rows and sends nothing while repair is on AUTO', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = { teams: [{ id: 0, label: 'T1', status: 'repairing' }], damaged: damagedRows(), auto: true };
    const row = el.shadowRoot.querySelector('.dmg-row[data-system-id="core"]');
    expect(row.disabled).toBe(true);
    row.click();
    expect(sendAction).not.toHaveBeenCalled();
  });

  it('drops rows that are no longer damaged', () => {
    const { el } = setup();
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }], damaged: damagedRows() };
    el.state = { teams: [{ id: 0, label: 'T1', status: 'idle' }], damaged: [damagedRows()[2]] };
    const rows = el.shadowRoot.querySelectorAll('.dmg-row');
    expect(rows.length).toBe(1);
    expect(rows[0].dataset.systemId).toBe('core');
  });

  it('shows the damaged list even when the ship has no repair teams', () => {
    const { el } = setup();
    el.state = { teams: [], damaged: damagedRows() };
    expect(el.shadowRoot.getElementById('damaged').hidden).toBe(false);
    expect(el.shadowRoot.querySelectorAll('.dmg-row').length).toBe(3);
  });

  it('updates when state changes', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
    };
    expect(el.shadowRoot.textContent).toContain('IDLE');
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 0.5 }],
    };
    expect(el.shadowRoot.textContent).toContain('REPAIRING');
  });
});
