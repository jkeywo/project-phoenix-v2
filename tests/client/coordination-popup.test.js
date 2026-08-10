import { describe, it, expect } from 'vitest';
import { normalizeCoordinationPayload } from '../../gui/coordination-popup.js';
import { t } from '../../gui/strings.js';

// The normaliser resolves every sentence through the string table (issue #975),
// so these assert against `t(id, params)` rather than English literals — the
// text survives a copy edit, and the same ids are what the host viewscreen
// chatter resolves, which is what makes the two surfaces render identically.

describe('normalizeCoordinationPayload — sender', () => {
  it('uses the sender label, appending the target when present', () => {
    expect(normalizeCoordinationPayload({ type: 'Alert' }, 'Helm AI').sender).toBe('Helm AI');
    expect(normalizeCoordinationPayload({ type: 'Alert', target: 'tactical' }, 'Helm AI').sender)
      .toBe('Helm AI ? tactical');
  });

  it('falls back to the AI id when no label is given', () => {
    expect(normalizeCoordinationPayload({ type: 'Alert' }, null).sender).toBe(t('chatter.sender.ai'));
  });
});

describe('normalizeCoordinationPayload — variants', () => {
  it('Advisory: label as title, message as body (data-wrapped or flat)', () => {
    expect(normalizeCoordinationPayload(
      { type: 'Advisory', data: { message: 'Adjust course' } }, 'Nav AI'))
      .toMatchObject({ title: 'Nav AI', body: 'Adjust course' });
    expect(normalizeCoordinationPayload(
      { type: 'Advisory', message: 'Flat form' }, 'Nav AI').body).toBe('Flat form');
  });

  it('Alert: title/body from data or flat fields with defaults', () => {
    expect(normalizeCoordinationPayload(
      { type: 'Alert', data: { title: 'Hull breach', body: 'Deck 3' } }, 'x'))
      .toMatchObject({ title: 'Hull breach', body: 'Deck 3' });
    expect(normalizeCoordinationPayload({ type: 'Alert' }, 'x'))
      .toMatchObject({ title: t('coordination.alert.fallback_title'), body: '' });
  });

  it('FrequencyHint carries the frequency', () => {
    expect(normalizeCoordinationPayload(
      { type: 'FrequencyHint', data: { frequency: 121.5 } }, 'x'))
      .toMatchObject({
        title: t('coordination.frequency_hint.title'),
        body: t('coordination.frequency_hint.body', { frequency: 121.5 }),
      });
  });

  it('ShieldFacingDown / Restored use the facing label', () => {
    expect(normalizeCoordinationPayload(
      { type: 'ShieldFacingDown', data: { label: 'Fore' } }, 'x').title)
      .toBe(t('coordination.shield_offline.title', { label: 'Fore' }));
    expect(normalizeCoordinationPayload(
      { type: 'ShieldFacingRestored', label: 'Aft' }, 'x').title)
      .toBe(t('coordination.shield_restored.title', { label: 'Aft' }));
    expect(normalizeCoordinationPayload(
      { type: 'ShieldFacingDown' }, 'x').title)
      .toBe(t('coordination.shield_offline.title', { label: t('coordination.shield.fallback_label') }));
  });

  it('TargetDesignation / ArcBearingRequest', () => {
    expect(normalizeCoordinationPayload(
      { type: 'TargetDesignation', data: { label: 'Raider-2' } }, 'x').title)
      .toBe(t('coordination.target_designation.title', { label: 'Raider-2' }));
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingRequest', data: { label: 'Raider-2' } }, 'x'))
      .toMatchObject({
        title: t('coordination.arc_bearing.title', { weapon: t('coordination.weapon_family.phasers') }),
        body: 'Raider-2',
      });
  });

  it('ArcBearingRequest is weapon-family-aware (issue #767)', () => {
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingRequest', data: { label: 'Raider-2', family: 'Blasters' } }, 'x').title)
      .toBe(t('coordination.arc_bearing.title', { weapon: t('coordination.weapon_family.blasters') }));
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingRequest', data: { label: 'Raider-2', family: 'Torpedoes' } }, 'x').title)
      .toBe(t('coordination.arc_bearing.title', { weapon: t('coordination.weapon_family.torpedoes') }));
    // Phasers by default when the family field is absent (pre-#767 payloads).
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingRequest', data: { label: 'Raider-2' } }, 'x').title)
      .toBe(t('coordination.arc_bearing.title', { weapon: t('coordination.weapon_family.phasers') }));
  });

  it('ArcBearingWithdraw is weapon-family-aware (issue #932)', () => {
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingWithdraw', data: { family: 'Torpedoes' } }, 'x'))
      .toMatchObject({
        title: t('coordination.arc_withdraw.title', { weapon: t('coordination.weapon_family.torpedoes') }),
        body: '',
      });
  });

  it('PowerBrownout carries the allocation level', () => {
    expect(normalizeCoordinationPayload(
      { type: 'PowerBrownout', data: { label: 'Phasers', allocated_level: 1 } }, 'x'))
      .toMatchObject({
        title: t('coordination.power_brownout.title', { label: 'Phasers' }),
        body: t('coordination.power_brownout.body', { level: 1 }),
      });
  });

  it('NavigateTo / RepairRequest / ThreatBearing (now first-class, issue #975)', () => {
    // Coords carried on the payload now (issue #977); the popup rounds them and
    // the template composes "waypoint (x, z)". Rust no longer sends a label.
    expect(normalizeCoordinationPayload(
      { type: 'NavigateTo', data: { x: 300.4, z: -100.6 } }, 'x').title)
      .toBe(t('coordination.navigate.title', { x: 300, z: -101 }));
    expect(normalizeCoordinationPayload(
      { type: 'RepairRequest', data: { station_label: 'Tactical' } }, 'x').title)
      .toBe(t('coordination.repair.title', { label: 'Tactical' }));
    // bearing_rad = π/2 → 90°.
    expect(normalizeCoordinationPayload(
      { type: 'ThreatBearing', data: { bearing_rad: Math.PI / 2, label: 'Raider-2' } }, 'x').title)
      .toBe(t('coordination.threat_bearing.title', { deg: 90, label: 'Raider-2' }));
  });

  it('IntentAdvisory names the decision and its subject (issue #879)', () => {
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'TargetSwitched', subject: 'Raider-2', generation: 3 } }, 'Tactical'))
      .toMatchObject({ title: t('coordination.intent.target_switched'), body: 'Raider-2' });
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', kind: 'ShieldArcFocused', subject: 'FORE' }, 'Shields'))
      .toMatchObject({ title: t('coordination.intent.shield_arc_focused'), body: 'FORE' });
  });

  it('IntentAdvisory kinds that name nothing carry an empty body', () => {
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'BreakingOff', generation: 9 } }, 'Helm'))
      .toMatchObject({ title: t('coordination.intent.breaking_off'), body: '' });
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'CombatPostureEntered', generation: 1 } }, 'Helm'))
      .toMatchObject({ title: t('coordination.intent.combat_posture_entered'), body: '' });
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'CombatPostureLeft', generation: 2 } }, 'Helm'))
      .toMatchObject({ title: t('coordination.intent.combat_posture_left'), body: '' });
  });

  it('an IntentAdvisory kind the client does not know still renders its token', () => {
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'SomethingNew', generation: 1 } }, 'Helm'))
      .toMatchObject({ title: 'SomethingNew', body: '' });
  });

  it('unknown variants fall back to the type name with an empty body', () => {
    expect(normalizeCoordinationPayload({ type: 'FutureThing' }, 'x'))
      .toMatchObject({ title: 'FutureThing', body: '' });
    expect(normalizeCoordinationPayload({}, 'x'))
      .toMatchObject({ title: t('coordination.advisory.fallback_title'), body: '' });
  });
});
