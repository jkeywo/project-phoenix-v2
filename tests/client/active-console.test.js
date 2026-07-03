import { describe, it, expect } from 'vitest';
import { nextActiveConsole } from '../../gui/active-console.js';

describe('nextActiveConsole', () => {
  it('null name from a set console -> next null, changed', () => {
    expect(nextActiveConsole('captain', null))
      .toEqual({ changed: true, next: null });
  });

  it('undefined name behaves like null', () => {
    expect(nextActiveConsole('tactical', undefined))
      .toEqual({ changed: true, next: null });
  });

  it('empty-string name normalises to null', () => {
    expect(nextActiveConsole('helm', ''))
      .toEqual({ changed: true, next: null });
  });

  it('null -> null is idempotent', () => {
    expect(nextActiveConsole(null, null).changed).toBe(false);
  });

  it('same name -> same name is idempotent', () => {
    expect(nextActiveConsole('helm', 'helm').changed).toBe(false);
  });

  it('one name to another -> next is the new name', () => {
    expect(nextActiveConsole('helm', 'tactical'))
      .toEqual({ changed: true, next: 'tactical' });
  });

  it('null -> a name -> next is the new name', () => {
    expect(nextActiveConsole(null, 'captain'))
      .toEqual({ changed: true, next: 'captain' });
  });

  it('undefined current is treated like null', () => {
    expect(nextActiveConsole(undefined, 'helm'))
      .toEqual({ changed: true, next: 'helm' });
  });

  it('empty-string current is treated like null (no-op when next is also null)', () => {
    expect(nextActiveConsole('', null).changed).toBe(false);
  });
});
