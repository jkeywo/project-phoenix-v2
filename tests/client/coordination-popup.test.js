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

  it('PowerBrownout carries the allocation level', () => {
    expect(normalizeCoordinationPayload(
      { type: 'PowerBrownout', data: { label: 'Phasers', allocated_level: 1 } }, 'x'))
      .toMatchObject({ title: 'Phasers Power Brownout', body: 'Allocation: 1' });
  });

  it('unknown variants fall back to the type name with an empty body', () => {
    expect(normalizeCoordinationPayload({ type: 'FutureThing' }, 'x'))
      .toMatchObject({ title: 'FutureThing', body: '' });
    expect(normalizeCoordinationPayload({}, 'x'))
      .toMatchObject({ title: 'Advisory', body: '' });
  });
});
