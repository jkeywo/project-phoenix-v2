import { describe, it, expect } from 'vitest';
import {
  consoleSections,
  sectionForConsole,
  isBevyConsole,
  CONSOLE_SECTION,
  HTML_SECTION_IDS,
} from '../../gui/content-switcher.js';

describe('CONSOLE_SECTION map', () => {
  it('keys all nine HTML-panel consoles by lowercase station id', () => {
    // Post issue #618: `CONSOLE_SECTION` is dual-keyed — lowercase station
    // ids (the primary keys, sourced from `REGISTRY`) and PascalCase Console
    // enum aliases (retained so callers passing wire-field values from
    // `player.consoles` still resolve). The lowercase side must expose all
    // nine consoles.
    const lowercase = ['captain', 'comms', 'helm', 'navigation', 'power', 'repair', 'sensors', 'shields', 'tactical'];
    for (const id of lowercase) {
      expect(Object.prototype.hasOwnProperty.call(CONSOLE_SECTION, id)).toBe(true);
    }
  });

  it('exposes PascalCase Console-enum aliases for wire-field compatibility', () => {
    const pascal = ['CaptainChair', 'Comms', 'Helm', 'Navigation', 'Power', 'Repair', 'Sensors', 'Shields', 'Tactical'];
    for (const name of pascal) {
      expect(Object.prototype.hasOwnProperty.call(CONSOLE_SECTION, name)).toBe(true);
    }
  });

  it('maps captain to captain-ui (both cases)', () => {
    expect(CONSOLE_SECTION.captain).toBe('captain-ui');
    expect(CONSOLE_SECTION.CaptainChair).toBe('captain-ui');
  });

  it('maps helm to helm-ui (both cases)', () => {
    expect(CONSOLE_SECTION.helm).toBe('helm-ui');
    expect(CONSOLE_SECTION.Helm).toBe('helm-ui');
  });

  it('maps tactical to weapons-ui (both cases)', () => {
    expect(CONSOLE_SECTION.tactical).toBe('weapons-ui');
    expect(CONSOLE_SECTION.Tactical).toBe('weapons-ui');
  });

  it('maps repair to repair-ui (both cases)', () => {
    expect(CONSOLE_SECTION.repair).toBe('repair-ui');
    expect(CONSOLE_SECTION.Repair).toBe('repair-ui');
  });

  it('maps power to power-ui (both cases)', () => {
    expect(CONSOLE_SECTION.power).toBe('power-ui');
    expect(CONSOLE_SECTION.Power).toBe('power-ui');
  });

  it('maps sensors, shields, comms, and navigation (both cases)', () => {
    expect(CONSOLE_SECTION.sensors).toBe('sensors-ui');
    expect(CONSOLE_SECTION.Sensors).toBe('sensors-ui');
    expect(CONSOLE_SECTION.shields).toBe('shields-ui');
    expect(CONSOLE_SECTION.Shields).toBe('shields-ui');
    expect(CONSOLE_SECTION.comms).toBe('comms-ui');
    expect(CONSOLE_SECTION.Comms).toBe('comms-ui');
    expect(CONSOLE_SECTION.navigation).toBe('navigation-ui');
    expect(CONSOLE_SECTION.Navigation).toBe('navigation-ui');
  });

  it('CONSOLE_SECTION and HTML_SECTION_IDS are frozen', () => {
    expect(Object.isFrozen(CONSOLE_SECTION)).toBe(true);
    expect(Object.isFrozen(HTML_SECTION_IDS)).toBe(true);
  });

  it('HTML_SECTION_IDS lists all nine section ids', () => {
    expect([...HTML_SECTION_IDS].sort()).toEqual([
      'captain-ui', 'comms-ui', 'helm-ui', 'navigation-ui',
      'power-ui', 'repair-ui', 'sensors-ui', 'shields-ui', 'weapons-ui',
    ]);
  });
});

describe('sectionForConsole', () => {
  it('returns the right id for all HTML-section consoles', () => {
    expect(sectionForConsole('CaptainChair')).toBe('captain-ui');
    expect(sectionForConsole('Helm')).toBe('helm-ui');
    expect(sectionForConsole('Tactical')).toBe('weapons-ui');
    expect(sectionForConsole('Repair')).toBe('repair-ui');
    expect(sectionForConsole('Power')).toBe('power-ui');
    expect(sectionForConsole('Sensors')).toBe('sensors-ui');
    expect(sectionForConsole('Shields')).toBe('shields-ui');
    expect(sectionForConsole('Comms')).toBe('comms-ui');
    expect(sectionForConsole('Navigation')).toBe('navigation-ui');
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
  function allFalse() {
    return {
      'captain-ui': false, 'helm-ui': false, 'weapons-ui': false,
      'repair-ui': false, 'power-ui': false, 'sensors-ui': false,
      'shields-ui': false, 'comms-ui': false, 'navigation-ui': false,
    };
  }

  function withTrue(key) {
    return { ...allFalse(), [key]: true };
  }

  it('returns all-false when not in-game (lobby)', () => {
    const out = consoleSections('CaptainChair', false);
    expect(out).toEqual(allFalse());
  });

  it('returns all-false when active console is null', () => {
    const out = consoleSections(null, true);
    expect(out).toEqual(allFalse());
  });

  it('shows only captain-ui for CaptainChair', () => {
    const out = consoleSections('CaptainChair', true);
    expect(out).toEqual(withTrue('captain-ui'));
  });

  it('shows only helm-ui for Helm', () => {
    const out = consoleSections('Helm', true);
    expect(out).toEqual(withTrue('helm-ui'));
  });

  it('shows only weapons-ui for Tactical', () => {
    const out = consoleSections('Tactical', true);
    expect(out).toEqual(withTrue('weapons-ui'));
  });

  it('shows only repair-ui for Repair', () => {
    const out = consoleSections('Repair', true);
    expect(out).toEqual(withTrue('repair-ui'));
  });

  it('shows only power-ui for Power', () => {
    const out = consoleSections('Power', true);
    expect(out).toEqual(withTrue('power-ui'));
  });

  it('shows only sensors-ui for Sensors', () => {
    const out = consoleSections('Sensors', true);
    expect(out).toEqual(withTrue('sensors-ui'));
  });

  it('shows only shields-ui for Shields', () => {
    const out = consoleSections('Shields', true);
    expect(out).toEqual(withTrue('shields-ui'));
  });

  it('shows only comms-ui for Comms', () => {
    const out = consoleSections('Comms', true);
    expect(out).toEqual(withTrue('comms-ui'));
  });

  it('shows only navigation-ui for Navigation', () => {
    const out = consoleSections('Navigation', true);
    expect(out).toEqual(withTrue('navigation-ui'));
  });

  it('returns all-false for unknown console strings', () => {
    const out = consoleSections('Unknown', true);
    expect(out).toEqual(allFalse());
  });
});

describe('isBevyConsole', () => {
  it('returns false for all nine HTML-section consoles', () => {
    for (const c of ['CaptainChair', 'Helm', 'Tactical', 'Repair', 'Power', 'Sensors', 'Shields', 'Comms', 'Navigation']) {
      expect(isBevyConsole(c)).toBe(false);
    }
  });

  it('returns false for null / empty / undefined (no active console)', () => {
    expect(isBevyConsole(null)).toBe(false);
    expect(isBevyConsole('')).toBe(false);
    expect(isBevyConsole(undefined)).toBe(false);
  });
});
