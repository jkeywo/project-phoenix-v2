// @vitest-environment jsdom
import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import {
  createSemanticSubcontextController,
  iframeForSemanticContext,
  semanticActionsForIframe,
  semanticContextForIframe,
} from '../../gui/composite-action-routing.js';

describe('composite semantic context routing', () => {
  it('wires the Courier Tactical document to distinct accessible Tactical and Helm regions', () => {
    const html = readFileSync('gui/courier/tactical.html', 'utf8');
    expect(html).toContain("actionFamilies: ['helm']");
    expect(html).toContain('getActionContext: actionSubcontext.getContext');
    expect(html).toContain('supportsSemanticAction: (actionId, context)');
    expect(html).toMatch(/role="region" tabindex="0"[^>]+data-semantic-action-context="tactical"/);
    expect(html).toMatch(/role="region" tabindex="0"[^>]+data-semantic-action-context="helm"/);
  });

  it('selects an explicit accessible subcontext by focus or pointer interaction', () => {
    document.body.innerHTML = [
      '<section tabindex="0" data-semantic-action-context="tactical">',
      '<button id="fire">Fire</button></section>',
      '<section tabindex="0" data-semantic-action-context="helm">',
      '<button id="boost">Boost</button></section>',
    ].join('');
    const tactical = document.querySelector('[data-semantic-action-context="tactical"]');
    const helm = document.querySelector('[data-semantic-action-context="helm"]');
    const controller = createSemanticSubcontextController(document, {
      defaultContext: 'tactical',
    });

    expect(controller.getContext()).toBe('tactical');
    expect(tactical.hasAttribute('data-semantic-action-active')).toBe(true);
    document.getElementById('boost').dispatchEvent(new Event('pointerdown', { bubbles: true }));
    expect(controller.getContext()).toBe('helm');
    expect(helm.hasAttribute('data-semantic-action-active')).toBe(true);
    document.getElementById('fire').focus();
    expect(controller.getContext()).toBe('tactical');
    expect(controller.select('unknown')).toBe(false);

    controller.dispose();
    document.getElementById('boost').dispatchEvent(new Event('pointerdown', { bubbles: true }));
    expect(controller.getContext()).toBe('tactical');
  });

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

  it('keeps a delayed Helm neutral in the Courier Tactical owner after subcontext switch', () => {
    let active = 'helm';
    const tacticalIframe = {
      contentWindow: {
        __semanticActionContext: () => active,
        __supportsSemanticActionContext: (context) => (
          context === 'tactical' || context === 'helm'
        ),
      },
    };
    const standaloneHelmIframe = {};
    const lookup = vi.fn((context) => context === 'helm' ? standaloneHelmIframe : null);

    expect(semanticContextForIframe(tacticalIframe, 'tactical')).toBe('helm');
    expect(iframeForSemanticContext('helm', tacticalIframe, lookup)).toBe(tacticalIframe);
    active = 'tactical';
    expect(semanticContextForIframe(tacticalIframe, 'tactical')).toBe('tactical');
    expect(iframeForSemanticContext('helm', tacticalIframe, lookup)).toBe(tacticalIframe);
    expect(lookup).not.toHaveBeenCalled();
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
