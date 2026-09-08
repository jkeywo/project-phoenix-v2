// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { hostLobbyViewModel, hostLobbyGmRow, hostLobbyStationRows } from '../../gui/host-lobby-view.js';
import { renderHostLobby, GM_MONITOR_BUTTON_ATTR } from '../../gui/host-lobby-render.js';
import { t } from '../../gui/strings.js';

function layout(overrides = {}) {
  return {
    monitors: [
      { identity: 'view', name: 'View', width: 1920, height: 1080, viewscreen: true, stations: [] },
      { identity: 'gm', name: 'GM', width: 1920, height: 1080, viewscreen: false, stations: [] },
    ],
    notices: [],
    gm: {
      assigned_to: 'gm', role_mutable: true,
      monitors: [{ identity: 'view', choice: 'excluded', excluded: 'occupied' },
        { identity: 'gm', choice: 'selected', excluded: null }],
      ...overrides,
    },
  };
}

describe('GM monitor row', () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="station-grid"></div><div id="monitor-row"><div id="monitor-row-buttons"></div></div>';
  });
  function render(value) {
    const vm = hostLobbyViewModel({ phase: 'Lobby', stations: [], spectators: [], gms: [] }, '', value);
    renderHostLobby(document, vm, t);
  }

  it('shows selected, occupied and Off states, reserving the GM monitor from the viewscreen', () => {
    render(layout());
    const gm = document.getElementById('gm-monitor-row');
    expect(gm.textContent).toContain(t('server.gm_monitor_row.label'));
    expect(gm.querySelector(`[${GM_MONITOR_BUTTON_ATTR}="gm"]`).getAttribute('aria-pressed')).toBe('true');
    expect(gm.querySelector(`[${GM_MONITOR_BUTTON_ATTR}="view"]`).disabled).toBe(true);
    expect(gm.querySelector(`[${GM_MONITOR_BUTTON_ATTR}=""]`).disabled).toBe(false);
    expect(document.querySelector('[data-monitor="gm"]').disabled).toBe(true);
    render(layout({ role_mutable: false }));
    expect(document.querySelectorAll('#gm-monitor-row')).toHaveLength(1);
    expect(gm.querySelector(`[${GM_MONITOR_BUTTON_ATTR}=""]`).disabled).toBe(true);
  });

  it('keeps Off available for a disconnected assigned screen in the lobby', () => {
    const value = layout({ assigned_to: 'missing', monitors: [] });
    value.monitors = [];
    expect(hostLobbyGmRow(value).off).toEqual({ selected: false, disabled: false });
  });

  it('offers no way to enable a new GM after launch, and explains GM exclusion on station rows', () => {
    expect(hostLobbyGmRow(layout({ assigned_to: null, role_mutable: false })).buttons.every(b => b.disabled)).toBe(true);
    const value = layout();
    value.stations = [{ station: 'helm', assigned_to: null,
      monitors: [{ identity: 'gm', choice: 'excluded', excluded: 'game-master' }] }];
    expect(hostLobbyStationRows(value).helm.buttons[0]).toMatchObject({
      disabled: true, reason: { id: 'server.gm_monitor_row.label' },
    });
  });

  it('adds no GM row to the ordinary browser host', () => {
    render(null);
    expect(document.getElementById('gm-monitor-row')).toBeNull();
  });
});
