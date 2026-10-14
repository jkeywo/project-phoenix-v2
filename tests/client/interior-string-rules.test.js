import {describe,it,expect} from 'vitest';
import {isLocalisable} from '../../scripts/strings-rules.mjs';

describe('interior authoring string references',()=>{
  it('preserves passage endpoint identities while localising labels and message senders',()=>{
    expect(isLocalisable('from','interior.connection','entity')).toBe(false);
    expect(isLocalisable('from','entity.ship_overrides.interior.connection','world')).toBe(false);
    expect(isLocalisable('label','interior.compartment','entity')).toBe(true);
    expect(isLocalisable('from','comms.message','world')).toBe(true);
  });
});
