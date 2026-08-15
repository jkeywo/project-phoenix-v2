/**
 * tests/client/link-state.test.js — PRD #1023 module 4, user stories 14 and
 * 15: a console that has lost the host must look lost, and one that has not
 * received its first state must look like it is loading rather than broken.
 *
 * The distinction between the two is the whole point, so it is what these
 * assert: `connecting` dims but stays live to the touch, `dead` dims and goes
 * inert. Collapsing them into one "not live" state would put a strike through
 * controls that are about to work.
 */
import { describe, it, expect } from 'vitest';
import { linkView } from '../../gui/link-state.js';

describe('linkView', () => {
  it('is live once the link is up and data has arrived', () => {
    expect(linkView('ready', true)).toEqual({
      mode: 'live', bannerId: null, dim: false, disable: false,
    });
  });

  it('is connecting while the link is still being established', () => {
    const vm = linkView('connecting', false);
    expect(vm.mode).toBe('connecting');
    expect(vm.bannerId).toBe('client.link_connecting');
  });

  // User story 15: an open DataChannel with nothing on it yet renders every
  // panel's empty shape, which is exactly what a broken console looks like.
  it('is connecting when the link is up but no state has landed', () => {
    expect(linkView('ready', false).mode).toBe('connecting');
  });

  it('dims a connecting console but leaves its controls usable', () => {
    const vm = linkView('ready', false);
    expect(vm.dim).toBe(true);
    expect(vm.disable).toBe(false);
  });

  it('is dead on a disconnect, and says it is reconnecting', () => {
    expect(linkView('disconnected', true)).toEqual({
      mode: 'dead', bannerId: 'client.reconnecting', dim: true, disable: true,
    });
  });

  it('is dead on a connection error, with the error banner', () => {
    expect(linkView('error', true)).toEqual({
      mode: 'dead', bannerId: 'client.conn_error', dim: true, disable: true,
    });
  });

  // User story 14: "so that I never keep issuing commands into the void."
  it('disables the console on every dead status, whatever data it had', () => {
    for (const status of ['disconnected', 'error']) {
      for (const hadData of [true, false]) {
        expect(linkView(status, hadData).disable).toBe(true);
      }
    }
  });

  it('treats an unknown status as not-yet-live rather than live', () => {
    expect(linkView(undefined, true).mode).toBe('connecting');
    expect(linkView('reconnecting', true).mode).toBe('connecting');
  });
});
