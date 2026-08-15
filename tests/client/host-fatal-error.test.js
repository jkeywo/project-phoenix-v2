/**
 * tests/client/host-fatal-error.test.js — the host's fatal-error surface.
 *
 * When a ship template fails station validation, or the world preload throws,
 * the viewscreen replaces its spinner with the only panel the operator will
 * ever see for that failure. Every word of it was hard-coded English —
 * "Fatal configuration error", "Fix the TOML and reload the page.", "Failed to
 * load game configuration." — in a codebase whose whole client is otherwise
 * driven off assets/strings/strings.csv (PRD #1023's defect list).
 *
 * It is display text, and it now reads from the table like display text.
 *
 * The exception, deliberately: the validator's own message INSIDE the panel
 * stays English. That is a diagnostic for whoever is editing the TOML, not
 * copy for a player, and it arrives from Rust already composed.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { t, has } from '../../gui/strings.js';

const REPO_ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const SERVER_HTML = fs.readFileSync(path.join(REPO_ROOT, 'server.html'), 'utf8');

const IDS = [
  'server.fatal.title',
  'server.fatal.remedy',
  'server.fatal.station_config',
  'server.fatal.load_failed',
  'server.preload.failed',
];

describe('the host fatal-error panel is authored copy', () => {
  for (const id of IDS) {
    it(`${id} has a row in the string table`, () => {
      expect(has(id)).toBe(true);
      expect(t(id)).not.toBe(id);
    });
  }

  it('interpolates the validator detail rather than concatenating it', () => {
    expect(t('server.fatal.station_config', { detail: 'no [[station]] blocks' }))
      .toContain('no [[station]] blocks');
    expect(t('server.fatal.load_failed', { path: 'assets/ships/x.toml', detail: 'HTTP 404' }))
      .toContain('assets/ships/x.toml');
  });
});

describe('server.html writes no raw English into the failure panel', () => {
  it('composes the panel chrome from string ids', () => {
    const fn = SERVER_HTML.slice(SERVER_HTML.indexOf('function showFatalError'));
    const body = fn.slice(0, fn.indexOf('\n}'));
    expect(body).toContain("t('server.fatal.title')");
    expect(body).toContain("t('server.fatal.remedy')");
  });

  for (const phrase of [
    'Fatal configuration error',
    'Fix the TOML and reload the page.',
    'Failed to load game configuration',
    "'Station config error: '",
  ]) {
    it(`no longer hard-codes ${JSON.stringify(phrase)}`, () => {
      expect(SERVER_HTML).not.toContain(phrase);
    });
  }

  it('escapes what it injects, now that the text comes from data', () => {
    // The panel builds innerHTML. It used to escape only `<`, and only on the
    // caller's message; every piece now goes through the shared escaper,
    // including the translated chrome.
    const fn = SERVER_HTML.slice(SERVER_HTML.indexOf('function showFatalError'));
    const body = fn.slice(0, fn.indexOf('\n}'));
    const injections = body.match(/\+\s*(?:escapeHtml\()?[a-zA-Z]/g) || [];
    expect(injections.length).toBeGreaterThan(0);
    expect(body).not.toMatch(/\+\s*message\b/);
    expect(body).toContain('escapeHtml(message)');
  });
});
