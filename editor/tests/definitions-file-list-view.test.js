import { describe, it, expect } from 'vitest';
import { installDom, FakeElement, fireClick } from './slice-5-helpers.js';
import { renderDefinitionsFileListView } from '../definitions-file-list-view.js';
import { ModeShell } from '../mode-shell.js';

describe('renderDefinitionsFileListView', () => {
  it('renders one row per path with display labels stripped of asset prefix', () => {
    installDom();
    const host = new FakeElement('div');
    const modeShell = new ModeShell();
    renderDefinitionsFileListView(host, {
      paths: ['assets/factions/federation.toml', 'assets/factions/pirate.toml'],
      activePath: null,
      modeShell,
      onSelect: () => {},
    });
    const rows = host.querySelectorAll('.definitions-file-list-row');
    expect(rows.length).toBe(2);
    expect(rows[0].dataset.path).toBe('assets/factions/federation.toml');
    const label = rows[0].querySelector('.definitions-file-list-label');
    expect(label.textContent).toBe('federation.toml');
  });

  it('marks the active row with the active class', () => {
    installDom();
    const host = new FakeElement('div');
    const modeShell = new ModeShell();
    renderDefinitionsFileListView(host, {
      paths: ['assets/complexity/tactical.toml', 'assets/complexity/power.toml'],
      activePath: 'assets/complexity/power.toml',
      modeShell,
      onSelect: () => {},
    });
    const rows = host.querySelectorAll('.definitions-file-list-row');
    expect(rows[0].classList.contains('definitions-file-list-row-active')).toBe(false);
    expect(rows[1].classList.contains('definitions-file-list-row-active')).toBe(true);
  });

  it('renders a dirty-dot for dirty files and fires onSelect on click', () => {
    installDom();
    const host = new FakeElement('div');
    const modeShell = new ModeShell();
    modeShell.markDirty('Definitions', 'assets/factions/federation.toml', true);

    const selected = [];
    renderDefinitionsFileListView(host, {
      paths: ['assets/factions/federation.toml', 'assets/factions/pirate.toml'],
      activePath: null,
      modeShell,
      onSelect: (p) => selected.push(p),
    });
    const rows = host.querySelectorAll('.definitions-file-list-row');
    const dot0 = rows[0].querySelector('.dirty-dot');
    const dot1 = rows[1].querySelector('.dirty-dot');
    expect(dot0).toBeTruthy();
    expect(dot1).toBeFalsy();

    fireClick(rows[1]);
    expect(selected).toEqual(['assets/factions/pirate.toml']);
  });
});
