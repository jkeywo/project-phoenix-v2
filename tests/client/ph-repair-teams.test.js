// @vitest-environment jsdom
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
    expect(el.shadowRoot.textContent).toContain('NO REPAIR TEAMS');
  });

  it('renders empty state when teams is null', () => {
    const { el } = setup();
    el.state = { teams: null };
    expect(el.shadowRoot.textContent).toContain('NO REPAIR TEAMS');
  });

  it('renders idle team with standing by text', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'Team 1', status: 'idle', target: null, progress_pct: 0, eta_secs: 0 }],
    };
    expect(el.shadowRoot.textContent).toContain('Standing by');
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

  it('dispatch button is disabled for idle team', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'idle' }],
    };
    const btn = el.shadowRoot.querySelector('.dispatch-btn');
    expect(btn.disabled).toBe(true);
  });

  it('dispatch button is enabled for non-idle team', () => {
    const { el } = setup();
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'repairing', target: 'Helm', progress_pct: 0.3 }],
    };
    const btn = el.shadowRoot.querySelector('.dispatch-btn');
    expect(btn.disabled).toBe(false);
  });

  it('dispatch button dispatches repair_repair action with team_id', () => {
    const sendAction = vi.fn();
    const { el } = setup({ sendAction });
    el.state = {
      teams: [{ id: 0, label: 'T1', status: 'travelling', target: 'Helm', progress_pct: 0.5 }],
    };
    const btn = el.shadowRoot.querySelector('.dispatch-btn');
    btn.click();
    expect(sendAction).toHaveBeenCalledWith('dispatch_repair', { team_id: 0, target_system_id: 'Helm' });
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
