import { describe, expect, it, vi } from 'vitest';
import {
  iframeForSemanticContext,
  semanticActionsForIframe,
  semanticContextForIframe,
} from '../../gui/composite-action-routing.js';

describe('composite semantic context routing', () => {
  it('exposes the Courier overlay family while keeping the Captain iframe owner', () => {
    let active = 'comms';
    const captainIframe = {
      contentWindow: {
        __semanticActionContext: () => active,
        __supportsSemanticActionContext: (context) => (
          ['captain', 'comms', 'power', 'repair'].includes(context)
        ),
      },
    };
    const lookup = vi.fn(() => null);
    expect(semanticContextForIframe(captainIframe, 'captain')).toBe('comms');
    expect(iframeForSemanticContext('comms', captainIframe, lookup)).toBe(captainIframe);
    expect(lookup).not.toHaveBeenCalled();

    // A delayed neutral/result keeps its original family context after the
    // overlay closes and still returns to the owning Captain iframe.
    active = 'captain';
    expect(iframeForSemanticContext('comms', captainIframe, lookup)).toBe(captainIframe);
  });

  it('falls back to the ordinary Station iframe for a non-composite context', () => {
    const captainIframe = {
      contentWindow: { __supportsSemanticActionContext: () => false },
    };
    const helmIframe = {};
    const lookup = vi.fn((context) => context === 'helm' ? helmIframe : null);
    expect(iframeForSemanticContext('helm', captainIframe, lookup)).toBe(helmIframe);
    expect(lookup).toHaveBeenCalledWith('helm');
  });

  it('filters the global Captain catalogue to adapters installed in this iframe', () => {
    const actions = [
      { id: 'captain.red-alert' },
      { id: 'power.increase-allocation' },
      { id: 'repair.dispatch-team' },
    ];
    const ordinaryCaptain = {
      contentWindow: {
        __supportsSemanticAction: (id, context) => (
          context === 'captain' && id === 'captain.red-alert'
        ),
      },
    };
    expect(semanticActionsForIframe(actions, ordinaryCaptain, 'captain'))
      .toEqual([{ id: 'captain.red-alert' }]);

    const courierCaptain = {
      contentWindow: { __supportsSemanticAction: () => true },
    };
    expect(semanticActionsForIframe(actions, courierCaptain, 'captain')).toEqual(actions);
  });

  it('fails closed while an iframe capability seam is unavailable', () => {
    expect(semanticActionsForIframe([{ id: 'power.increase-allocation' }], {}, 'captain'))
      .toBeNull();
  });
});
