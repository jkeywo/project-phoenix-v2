import { describe, it, expect } from 'vitest';
import {
  consoleSections,
  sectionForConsole,
  isBevyConsole,
  CONSOLE_SECTION,
  HTML_SECTION_IDS,
} from '../../gui/content-switcher.js';

describe('CONSOLE_SECTION map', () => {
  it('keys only the five consoles with HTML panels', () => {
    expect(Object.keys(CONSOLE_SECTION).sort())
      .toEqual(['CaptainChair', 'Helm', 'Power', 'Repair', 'Tactical']);
  });

  it('maps CaptainChair to captain-ui', () => {
    expect(CONSOLE_SECTION.CaptainChair).toBe('captain-ui');
  });

  it('maps Helm to helm-ui', () => {
    expect(CONSOLE_SECTION.Helm).toBe('helm-ui');
  });

  it('maps Tactical to weapons-ui', () => {
    expect(CONSOLE_SECTION.Tactical).toBe('weapons-ui');
  });

  it('maps Repair to repair-ui', () => {
    expect(CONSOLE_SECTION.Repair).toBe('repair-ui');
  });

  it('maps Power to power-ui', () => {
    expect(CONSOLE_SECTION.Power).toBe('power-ui');
  });

  it('does not key Sensors / Shields / Navigation / Comms (Bevy renders them)', () => {
    expect(CONSOLE_SECTION.Sensors).toBeUndefined();
    expect(CONSOLE_SECTION.Shields).toBeUndefined();
    expect(CONSOLE_SECTION.Navigation).toBeUndefined();
    expect(CONSOLE_SECTION.Comms).toBeUndefined();
  });

  it('CONSOLE_SECTION and HTML_SECTION_IDS are frozen', () => {
    expect(Object.isFrozen(CONSOLE_SECTION)).toBe(true);
    expect(Object.isFrozen(HTML_SECTION_IDS)).toBe(true);
  });

  it('HTML_SECTION_IDS lists all five section ids', () => {
    expect([...HTML_SECTION_IDS].sort()).toEqual(['captain-ui', 'helm-ui', 'power-ui', 'repair-ui', 'weapons-ui']);
  });
});

describe('sectionForConsole', () => {
  it('returns the right id for CaptainChair / Helm / Tactical / Repair / Power', () => {
    expect(sectionForConsole('CaptainChair')).toBe('captain-ui');
    expect(sectionForConsole('Helm')).toBe('helm-ui');
    expect(sectionForConsole('Tactical')).toBe('weapons-ui');
    expect(sectionForConsole('Repair')).toBe('repair-ui');
    expect(sectionForConsole('Power')).toBe('power-ui');
  });

  it('returns null for Bevy-rendered consoles', () => {
    expect(sectionForConsole('Sensors')).toBeNull();
    expect(sectionForConsole('Comms')).toBeNull();
  });

  it('returns null for empty / null / undefined', () => {
    expect(sectionForConsole('')).toBeNull();
    expect(sectionForConsole(null)).toBeNull();
    expect(sectionForConsole(undefined)).toBeNull();
  });

  it('returns null for unknown console strings', () => {
    expect(sectionForConsole('NotAConsole')).toBeNull();
  });
});

describe('consoleSections', () => {
  it('returns all-false when not in-game (lobby)', () => {
    const out = consoleSections('CaptainChair', false);
    expect(out).toEqual({ 'captain-ui': false, 'helm-ui': false, 'weapons-ui': false, 'repair-ui': false, 'power-ui': false });
  });

  it('returns all-false when active console is null', () => {
    const out = consoleSections(null, true);
    expect(out).toEqual({ 'captain-ui': false, 'helm-ui': false, 'weapons-ui': false, 'repair-ui': false, 'power-ui': false });
  });

  it('shows only captain-ui for CaptainChair', () => {
    const out = consoleSections('CaptainChair', true);
    expect(out).toEqual({ 'captain-ui': true, 'helm-ui': false, 'weapons-ui': false, 'repair-ui': false, 'power-ui': false });
  });

  it('shows only helm-ui for Helm', () => {
    const out = consoleSections('Helm', true);
    expect(out).toEqual({ 'captain-ui': false, 'helm-ui': true, 'weapons-ui': false, 'repair-ui': false, 'power-ui': false });
  });

  it('shows only weapons-ui for Tactical', () => {
    const out = consoleSections('Tactical', true);
    expect(out).toEqual({ 'captain-ui': false, 'helm-ui': false, 'weapons-ui': true, 'repair-ui': false, 'power-ui': false });
  });

  it('shows only repair-ui for Repair', () => {
    const out = consoleSections('Repair', true);
    expect(out).toEqual({ 'captain-ui': false, 'helm-ui': false, 'weapons-ui': false, 'repair-ui': true, 'power-ui': false });
  });

  it('shows only power-ui for Power', () => {
    const out = consoleSections('Power', true);
    expect(out).toEqual({ 'captain-ui': false, 'helm-ui': false, 'weapons-ui': false, 'repair-ui': false, 'power-ui': true });
  });

  it('returns all-false for Bevy-rendered consoles (canvas takes the content area)', () => {
    for (const c of ['Sensors', 'Shields', 'Navigation', 'Comms']) {
      const out = consoleSections(c, true);
      expect(out).toEqual({ 'captain-ui': false, 'helm-ui': false, 'weapons-ui': false, 'repair-ui': false, 'power-ui': false });
    }
  });
});

describe('isBevyConsole', () => {
  it('returns true for the four Bevy-rendered consoles', () => {
    for (const c of ['Sensors', 'Shields', 'Navigation', 'Comms']) {
      expect(isBevyConsole(c)).toBe(true);
    }
  });

  it('returns false for the five HTML-section consoles', () => {
    expect(isBevyConsole('CaptainChair')).toBe(false);
    expect(isBevyConsole('Helm')).toBe(false);
    expect(isBevyConsole('Tactical')).toBe(false);
    expect(isBevyConsole('Repair')).toBe(false);
    expect(isBevyConsole('Power')).toBe(false);
  });

  it('returns false for null / empty / undefined (no active console)', () => {
    expect(isBevyConsole(null)).toBe(false);
    expect(isBevyConsole('')).toBe(false);
    expect(isBevyConsole(undefined)).toBe(false);
  });
});
