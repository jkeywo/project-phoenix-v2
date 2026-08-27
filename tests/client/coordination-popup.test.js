// @vitest-environment jsdom

import { describe, it, expect, beforeEach } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  normalizeCoordinationPresentation,
  renderCoordinationPopup,
  buildCoordinationChatterBubble,
} from '../../gui/coordination-popup.js';
import { t } from '../../gui/strings.js';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SOURCE = fs.readFileSync(path.resolve(HERE, '../../gui/coordination-popup.js'), 'utf8');

describe('normalizeCoordinationPresentation', () => {
  it('resolves a known title/body id and keeps numeric parameters typed', () => {
    const norm = normalizeCoordinationPresentation({
      title: 'coordination.frequency_hint.title',
      body: 'coordination.frequency_hint.body',
      body_params: { frequency: 121.5 },
    }, 'chatter.sender.sensors', 'station.tactical.name');

    expect(norm).toEqual({
      sender: `${t('chatter.sender.sensors')} → ${t('station.tactical.name')}`,
      from: t('chatter.sender.sensors'),
      to: t('station.tactical.name'),
      title: t('coordination.frequency_hint.title'),
      body: t('coordination.frequency_hint.body', { frequency: 121.5 }),
    });
  });

  it('passes literal authored text and interpolates literal parameters', () => {
    expect(normalizeCoordinationPresentation({
      title: 'Lark has the helm',
      title_params: {},
      body: 'Course {course}',
      body_params: { course: 'steady' },
    }, 'Lark', 'chatter.addressee.ship')).toMatchObject({
      title: 'Lark has the helm',
      body: 'Course steady',
      to: t('chatter.addressee.ship'),
    });
  });

  it('localises a String Table id used as a parameter value', () => {
    const norm = normalizeCoordinationPresentation({
      title: 'coordination.repair.title',
      title_params: { label: 'station.helm.name' },
      body: '',
    }, null, 'station.repair.name');
    expect(norm.title).toBe(t('coordination.repair.title', { label: t('station.helm.name') }));
    expect(norm.from).toBe(t('chatter.sender.ai'));
  });

  it('contains no payload-family renderer', () => {
    expect(SOURCE).not.toMatch(/payload\s*\.\s*type|CoordinationPayload|IntentKind|WeaponFamily/);
    expect(SOURCE).not.toMatch(/case\s+['"](?:FrequencyHint|RepairRequest|IntentAdvisory)['"]/);
  });
});

describe('Coordination DOM layouts', () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <div id="coordination-popup">
        <div class="popup-sender"><span id="popup-from"></span><span id="popup-arrow"></span><span id="popup-to"></span><span id="popup-colon"></span></div>
        <div class="popup-title" id="popup-title"></div>
        <div class="popup-body" id="popup-body"></div>
      </div>`;
  });

  it('keeps the phone route plus separate title/body lines', () => {
    const presentation = {
      title: 'coordination.power_brownout.title',
      title_params: { label: 'power.group.weapons' },
      body: 'coordination.power_brownout.body',
      body_params: { level: 2 },
    };
    const norm = renderCoordinationPopup(
      document, presentation, 'chatter.sender.power', 'station.tactical.name',
    );

    expect(norm).not.toBeNull();
    expect(document.querySelector('.popup-sender').textContent)
      .toBe(`[${t('chatter.sender.power')}] → [${t('station.tactical.name')}]:`);
    expect(document.querySelector('.popup-title').textContent)
      .toBe(t('coordination.power_brownout.title', { label: t('power.group.weapons') }));
    expect(document.querySelector('.popup-body').textContent)
      .toBe(t('coordination.power_brownout.body', { level: 2 }));
  });

  it('decorates the bare Ship destination exactly once on the phone', () => {
    renderCoordinationPopup(document, {
      title: 'Ship-wide advisory',
      body: 'All stations acknowledge',
    }, 'chatter.sender.ai', 'chatter.addressee.ship');

    expect(t('chatter.addressee.ship')).toBe('Ship');
    expect(document.querySelector('#popup-to').textContent).toBe('[Ship]');
    expect(document.querySelector('.popup-sender').textContent)
      .toBe(`[${t('chatter.sender.ai')}] → [Ship]:`);
  });

  it('keeps the Viewscreen route/title/body on one chatter line', () => {
    const bubble = buildCoordinationChatterBubble(document, {
      title: 'coordination.frequency_hint.title',
      body: 'coordination.frequency_hint.body',
      body_params: { frequency: 0.83 },
    }, 'chatter.sender.sensors', 'station.tactical.name');

    expect(bubble.children).toHaveLength(4);
    expect(bubble.querySelector('.chatter-from').textContent)
      .toBe(`[${t('chatter.sender.sensors')}]`);
    expect(bubble.querySelector('.chatter-to').textContent)
      .toBe(`[${t('station.tactical.name')}]`);
    expect(bubble.querySelector('.chatter-text').textContent).toBe(
      `: ${t('coordination.frequency_hint.title')} — ${t('coordination.frequency_hint.body', { frequency: 0.83 })}`,
    );
  });

  it('decorates the bare Ship destination exactly once on the Viewscreen', () => {
    const bubble = buildCoordinationChatterBubble(document, {
      title: 'Ship-wide advisory',
      body: 'All stations acknowledge',
    }, 'chatter.sender.ai', 'chatter.addressee.ship');

    expect(t('chatter.addressee.ship')).toBe('Ship');
    expect(bubble.querySelector('.chatter-to').textContent).toBe('[Ship]');
    expect(bubble.textContent).toBe(
      `[${t('chatter.sender.ai')}] → [Ship]: Ship-wide advisory — All stations acknowledge`,
    );
  });
});
