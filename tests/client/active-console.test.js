import { describe, it, expect } from 'vitest';
import { nextActiveConsole } from '../../gui/active-console.js';

describe('nextActiveConsole', () => {
  it('null name from a set console -> wasmArg "", next null, changed', () => {
    expect(nextActiveConsole('CaptainChair', null))
      .toEqual({ changed: true, next: null, wasmArg: '' });
  });

  it('undefined name behaves like null (auto sentinel)', () => {
    expect(nextActiveConsole('Tactical', undefined))
      .toEqual({ changed: true, next: null, wasmArg: '' });
  });

  it('empty-string name maps to null + wasmArg "" (Bevy bridge.rs:160 contract)', () => {
    expect(nextActiveConsole('Helm', ''))
      .toEqual({ changed: true, next: null, wasmArg: '' });
  });

  it('null -> null is idempotent (no WASM round-trip)', () => {
    expect(nextActiveConsole(null, null).changed).toBe(false);
  });

  it('same name -> same name is idempotent', () => {
    expect(nextActiveConsole('Helm', 'Helm').changed).toBe(false);
  });

  it('one name to another -> wasmArg is the new name', () => {
    expect(nextActiveConsole('Helm', 'Tactical'))
      .toEqual({ changed: true, next: 'Tactical', wasmArg: 'Tactical' });
  });

  it('null -> a name -> wasmArg is the new name', () => {
    expect(nextActiveConsole(null, 'CaptainChair'))
      .toEqual({ changed: true, next: 'CaptainChair', wasmArg: 'CaptainChair' });
  });

  it('undefined current is treated like null', () => {
    expect(nextActiveConsole(undefined, 'Helm'))
      .toEqual({ changed: true, next: 'Helm', wasmArg: 'Helm' });
  });

  it('empty-string current is treated like null (no-op when next is also null)', () => {
    expect(nextActiveConsole('', null).changed).toBe(false);
  });
});
