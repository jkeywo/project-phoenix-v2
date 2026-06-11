import { describe, it, expect } from 'vitest';
import { nextActiveConsole } from '../../gui/active-console.js';

describe('nextActiveConsole', () => {
  it('null name from a set console -> next null, changed', () => {
    expect(nextActiveConsole('CaptainChair', null))
      .toEqual({ changed: true, next: null });
  });

  it('undefined name behaves like null', () => {
    expect(nextActiveConsole('Tactical', undefined))
      .toEqual({ changed: true, next: null });
  });

  it('empty-string name normalises to null', () => {
    expect(nextActiveConsole('Helm', ''))
      .toEqual({ changed: true, next: null });
  });

  it('null -> null is idempotent', () => {
    expect(nextActiveConsole(null, null).changed).toBe(false);
  });

  it('same name -> same name is idempotent', () => {
    expect(nextActiveConsole('Helm', 'Helm').changed).toBe(false);
  });

  it('one name to another -> next is the new name', () => {
    expect(nextActiveConsole('Helm', 'Tactical'))
      .toEqual({ changed: true, next: 'Tactical' });
  });

  it('null -> a name -> next is the new name', () => {
    expect(nextActiveConsole(null, 'CaptainChair'))
      .toEqual({ changed: true, next: 'CaptainChair' });
  });

  it('undefined current is treated like null', () => {
    expect(nextActiveConsole(undefined, 'Helm'))
      .toEqual({ changed: true, next: 'Helm' });
  });

  it('empty-string current is treated like null (no-op when next is also null)', () => {
    expect(nextActiveConsole('', null).changed).toBe(false);
  });
});
