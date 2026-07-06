import { describe, it, expect } from 'vitest';
import {
  consoleSections,
  sectionForConsole,
  isBevyConsole,
  CONSOLE_SECTION,
  HTML_SECTION_IDS,
} from '../../gui/content-switcher.js';

describe('CONSOLE_SECTION map', () => {
  it('keys all ten HTML-panel consoles by lowercase station id', () => {
    // Post issues #618/#619 the map is single-keyed on lowercase station ids
    // (matching the Rust `StationId` newtype); the PascalCase Console aliases
    // are gone along with the enum.
    const lowercase = ['captain', 'comms', 'engineering', 'helm', 'navigation', 'power', 'repair', 'science', 'sensors', 'shields', 'tactical'];
    for (const id of lowercase) {
      expect(Object.prototype.hasOwnProperty.call(CONSOLE_SECTION, id)).toBe(true);
    }
  });

  it('does NOT expose PascalCase Console-enum aliases (retired in #619)', () => {
    const pascal = ['CaptainChair', 'Helm', 'Tactical', 'Repair', 'Power', 'Sensors', 'Shields', 'Comms', 'Navigation', 'Science'];
    for (const name of pascal) {
      expect(Object.prototype.hasOwnProperty.call(CONSOLE_SECTION, name)).toBe(false);
    }
  });

  it('maps captain to captain-ui', () => {
    expect(CONSOLE_SECTION.captain).toBe('captain-ui');
  });

  it('maps helm to helm-ui', () => {
    expect(CONSOLE_SECTION.helm).toBe('helm-ui');
  });

  it('maps tactical to weapons-ui', () => {
    expect(CONSOLE_SECTION.tactical).toBe('weapons-ui');
  });

  it('maps repair to repair-ui', () => {
    expect(CONSOLE_SECTION.repair).toBe('repair-ui');
  });

  it('maps power to power-ui', () => {
    expect(CONSOLE_SECTION.power).toBe('power-ui');
  });

  it('maps sensors, shields, comms, navigation, and engineering', () => {
    expect(CONSOLE_SECTION.sensors).toBe('sensors-ui');
    expect(CONSOLE_SECTION.shields).toBe('shields-ui');
    expect(CONSOLE_SECTION.comms).toBe('comms-ui');
    expect(CONSOLE_SECTION.navigation).toBe('navigation-ui');
    expect(CONSOLE_SECTION.engineering).toBe('engineering-ui');
  });

  it('CONSOLE_SECTION and HTML_SECTION_IDS are frozen', () => {
    expect(Object.isFrozen(CONSOLE_SECTION)).toBe(true);
    expect(Object.isFrozen(HTML_SECTION_IDS)).toBe(true);
  });

  it('HTML_SECTION_IDS lists all eleven section ids', () => {
    expect([...HTML_SECTION_IDS].sort()).toEqual([
      'captain-ui', 'comms-ui', 'engineering-ui', 'helm-ui', 'navigation-ui',
      'power-ui', 'repair-ui', 'science-ui', 'sensors-ui', 'shields-ui', 'weapons-ui',
    ]);
  });
});

describe('sectionForConsole', () => {
  it('returns the right id for all HTML-section consoles', () => {
    expect(sectionForConsole('captain')).toBe('captain-ui');
    expect(sectionForConsole('helm')).toBe('helm-ui');
    expect(sectionForConsole('tactical')).toBe('weapons-ui');
    expect(sectionForConsole('repair')).toBe('repair-ui');
    expect(sectionForConsole('power')).toBe('power-ui');
    expect(sectionForConsole('sensors')).toBe('sensors-ui');
    expect(sectionForConsole('shields')).toBe('shields-ui');
    expect(sectionForConsole('science')).toBe('science-ui');
    expect(sectionForConsole('comms')).toBe('comms-ui');
    expect(sectionForConsole('navigation')).toBe('navigation-ui');
    expect(sectionForConsole('engineering')).toBe('engineering-ui');
  });

  it('returns null for empty / null / undefined', () => {
    expect(sectionForConsole('')).toBeNull();
    expect(sectionForConsole(null)).toBeNull();
    expect(sectionForConsole(undefined)).toBeNull();
  });

  it('returns null for unknown console strings', () => {
    expect(sectionForConsole('NotAConsole')).toBeNull();
  });

  it('returns null for retired PascalCase Console-enum names', () => {
    expect(sectionForConsole('CaptainChair')).toBeNull();
    expect(sectionForConsole('Helm')).toBeNull();
  });
});

describe('consoleSections', () => {
  function allFalse() {
    return {
      'captain-ui': false, 'helm-ui': false, 'weapons-ui': false,
      'repair-ui': false, 'power-ui': false, 'science-ui': false,
      'sensors-ui': false, 'shields-ui': false, 'comms-ui': false,
      'navigation-ui': false, 'engineering-ui': false,
    };
  }

  function withTrue(key) {
    return { ...allFalse(), [key]: true };
  }

  it('returns all-false when not in-game (lobby)', () => {
    const out = consoleSections('captain', false);
    expect(out).toEqual(allFalse());
  });

  it('returns all-false when active console is null', () => {
    const out = consoleSections(null, true);
    expect(out).toEqual(allFalse());
  });

  it('shows only captain-ui for captain', () => {
    const out = consoleSections('captain', true);
    expect(out).toEqual(withTrue('captain-ui'));
  });

  it('shows only helm-ui for helm', () => {
    const out = consoleSections('helm', true);
    expect(out).toEqual(withTrue('helm-ui'));
  });

  it('shows only weapons-ui for tactical', () => {
    const out = consoleSections('tactical', true);
    expect(out).toEqual(withTrue('weapons-ui'));
  });

  it('shows only repair-ui for repair', () => {
    const out = consoleSections('repair', true);
    expect(out).toEqual(withTrue('repair-ui'));
  });

  it('shows only power-ui for power', () => {
    const out = consoleSections('power', true);
    expect(out).toEqual(withTrue('power-ui'));
  });

  it('shows only sensors-ui for sensors', () => {
    const out = consoleSections('sensors', true);
    expect(out).toEqual(withTrue('sensors-ui'));
  });

  it('shows only shields-ui for shields', () => {
    const out = consoleSections('shields', true);
    expect(out).toEqual(withTrue('shields-ui'));
  });

  it('shows only comms-ui for comms', () => {
    const out = consoleSections('comms', true);
    expect(out).toEqual(withTrue('comms-ui'));
  });

  it('shows only navigation-ui for navigation', () => {
    const out = consoleSections('navigation', true);
    expect(out).toEqual(withTrue('navigation-ui'));
  });

  it('shows only engineering-ui for engineering', () => {
    const out = consoleSections('engineering', true);
    expect(out).toEqual(withTrue('engineering-ui'));
  });

  it('returns all-false for unknown console strings', () => {
    const out = consoleSections('Unknown', true);
    expect(out).toEqual(allFalse());
  });
});

describe('isBevyConsole', () => {
  it('returns false for all eleven HTML-section consoles', () => {
    for (const c of ['captain', 'helm', 'tactical', 'repair', 'power', 'sensors', 'shields', 'comms', 'navigation', 'science', 'engineering']) {
      expect(isBevyConsole(c)).toBe(false);
    }
  });

  it('returns false for null / empty / undefined (no active console)', () => {
    expect(isBevyConsole(null)).toBe(false);
    expect(isBevyConsole('')).toBe(false);
    expect(isBevyConsole(undefined)).toBe(false);
  });
});
