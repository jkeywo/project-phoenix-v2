import { describe, it, expect } from 'vitest';
import { normalizeCoordinationPayload } from '../../gui/coordination-popup.js';

describe('normalizeCoordinationPayload — sender', () => {
  it('uses the sender label, appending the target when present', () => {
    expect(normalizeCoordinationPayload({ type: 'Alert' }, 'Helm AI').sender).toBe('Helm AI');
    expect(normalizeCoordinationPayload({ type: 'Alert', target: 'tactical' }, 'Helm AI').sender)
      .toBe('Helm AI ? tactical');
  });

  it('falls back to AI when no label is given', () => {
    expect(normalizeCoordinationPayload({ type: 'Alert' }, null).sender).toBe('AI');
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
      .toMatchObject({ title: 'Alert', body: '' });
  });

  it('FrequencyHint carries the frequency', () => {
    expect(normalizeCoordinationPayload(
      { type: 'FrequencyHint', data: { frequency: 121.5 } }, 'x'))
      .toMatchObject({ title: 'Frequency Hint', body: 'Tune to: 121.5' });
  });

  it('ShieldFacingDown / Restored use the facing label', () => {
    expect(normalizeCoordinationPayload(
      { type: 'ShieldFacingDown', data: { label: 'Fore' } }, 'x').title).toBe('Fore Offline');
    expect(normalizeCoordinationPayload(
      { type: 'ShieldFacingRestored', label: 'Aft' }, 'x').title).toBe('Aft Restored');
    expect(normalizeCoordinationPayload(
      { type: 'ShieldFacingDown' }, 'x').title).toBe('Shield Offline');
  });

  it('TargetDesignation / ArcBearingRequest', () => {
    expect(normalizeCoordinationPayload(
      { type: 'TargetDesignation', data: { label: 'Raider-2' } }, 'x').title)
      .toBe('Sensors designates: Raider-2');
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingRequest', data: { label: 'Raider-2' } }, 'x'))
      .toMatchObject({ title: 'Tactical: come about, bring phasers to bear', body: 'Raider-2' });
  });

  it('ArcBearingRequest is weapon-family-aware (issue #767)', () => {
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingRequest', data: { label: 'Raider-2', family: 'Blasters' } }, 'x').title)
      .toBe('Tactical: come about, bring blasters to bear');
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingRequest', data: { label: 'Raider-2', family: 'Torpedoes' } }, 'x').title)
      .toBe('Tactical: come about, bring torpedoes to bear');
    // Phasers by default when the family field is absent (pre-#767 payloads).
    expect(normalizeCoordinationPayload(
      { type: 'ArcBearingRequest', data: { label: 'Raider-2' } }, 'x').title)
      .toBe('Tactical: come about, bring phasers to bear');
  });

  it('PowerBrownout carries the allocation level', () => {
    expect(normalizeCoordinationPayload(
      { type: 'PowerBrownout', data: { label: 'Phasers', allocated_level: 1 } }, 'x'))
      .toMatchObject({ title: 'Phasers Power Brownout', body: 'Allocation: 1' });
  });

  it('IntentAdvisory names the decision and its subject (issue #879)', () => {
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'TargetSwitched', subject: 'Raider-2', generation: 3 } }, 'Tactical'))
      .toMatchObject({ title: 'Switching target', body: 'Raider-2' });
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', kind: 'ShieldArcFocused', subject: 'FORE' }, 'Shields'))
      .toMatchObject({ title: 'Focusing shields', body: 'FORE' });
  });

  it('IntentAdvisory kinds that name nothing carry an empty body', () => {
    // The host omits `subject` entirely for these — and never sends the hull
    // figure the break-off decision was made from (the #737 boundary).
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'BreakingOff', generation: 9 } }, 'Helm'))
      .toMatchObject({ title: 'Breaking off', body: '' });
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'CombatPostureEntered', generation: 1 } }, 'Helm'))
      .toMatchObject({ title: 'Combat posture', body: '' });
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'CombatPostureLeft', generation: 2 } }, 'Helm'))
      .toMatchObject({ title: 'Standing down', body: '' });
  });

  it('an IntentAdvisory kind the client does not know still renders', () => {
    expect(normalizeCoordinationPayload(
      { type: 'IntentAdvisory', data: { kind: 'SomethingNew', generation: 1 } }, 'Helm'))
      .toMatchObject({ title: 'SomethingNew', body: '' });
  });

  it('unknown variants fall back to the type name with an empty body', () => {
    expect(normalizeCoordinationPayload({ type: 'FutureThing' }, 'x'))
      .toMatchObject({ title: 'FutureThing', body: '' });
    expect(normalizeCoordinationPayload({}, 'x'))
      .toMatchObject({ title: 'Advisory', body: '' });
  });
});
