// @vitest-environment jsdom
/**
 * tests/client/ph-element.test.js — gui/components/ph-element.js (issue
 * #1231, T4.C0 of the console-seam programme).
 *
 * `PhElement` is purely additive right now — no shipped component extends
 * it yet (that is a later phase, one component per PR). This suite proves
 * the base-class CONTRACT directly, against small probe subclasses defined
 * here, since there is no real consumer yet to prove it through:
 *   - the constructor ordering (template built and appended, THEN
 *     onTemplate() runs, so a subclass caching refs there sees real nodes)
 *   - synchronous `set state` by default
 *   - `static coalesce = true` batches renders via requestAnimationFrame,
 *     collapsing several assignments in one frame to the latest value
 *   - `sendAction` wiring in connectedCallback, `??=` so it never clobbers
 *     an explicitly assigned one
 *   - the `$(id)` shadow-lookup helper
 *   - `phDefine` registers under jsdom (where customElements exists) and is
 *     idempotent
 *
 * Node-safe import (no HTMLElement/document/customElements) is proved
 * separately, in plain Node, by ph-element-node-safe.test.js — this file
 * needs jsdom for the rest of the contract, which cannot be proved without
 * a real shadow DOM.
 */
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { PhElement, phDefine } from '../../gui/components/ph-element.js';

// ── Probe subclasses ─────────────────────────────────────────────────────
// Registered once, at module load, the same way a real ph-* component
// registers itself — through phDefine, never a bare customElements.define.

class PhElementProbe extends PhElement {
  template() {
    return '<div id="label"></div><div id="other"></div>';
  }

  // The one place a subclass may cache shadow refs (see the ordering note
  // on the base class): by the time this runs, the template above has
  // already been built and appended. Cached as a PLAIN property
  // (`this.labelRef = …`) — deliberately NOT also declared as a class field
  // anywhere in this body. Declaring it (public or private) is the exact
  // hazard the base class documents: this hook runs strictly BEFORE this
  // class's own field-init phase, so a declared private field crashes
  // construction (`Cannot write private member … to an object whose class
  // did not declare it` — the precise error an earlier draft of this suite
  // hit) and a declared public field would silently overwrite the ref back
  // to its declared initial value moments later.
  onTemplate() {
    this.labelRef = this.$('label');
  }

  render(state) {
    this.renderCount = (this.renderCount || 0) + 1;
    this.lastRendered = state;
    if (this.labelRef) this.labelRef.textContent = state.text || '';
  }
}
phDefine('ph-element-probe', PhElementProbe);

class PhElementCoalesceProbe extends PhElement {
  static coalesce = true;

  template() { return '<div id="out"></div>'; }

  render(state) {
    this.renderCount = (this.renderCount || 0) + 1;
    this.lastRendered = state;
  }
}
phDefine('ph-element-coalesce-probe', PhElementCoalesceProbe);

// No template()/onTemplate()/render() overrides at all — proves the base
// defaults (empty template, no-op render) are safe to leave un-overridden.
class PhElementBareProbe extends PhElement {}
phDefine('ph-element-bare-probe', PhElementBareProbe);

describe('PhElement', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });
  afterEach(() => {
    document.body.innerHTML = '';
    delete window.sendAction;
  });

  it('phDefine registers the custom element', () => {
    expect(customElements.get('ph-element-probe')).toBe(PhElementProbe);
  });

  it('phDefine is idempotent — a second call for an already-defined tag does not throw', () => {
    expect(() => phDefine('ph-element-probe', PhElementProbe)).not.toThrow();
    expect(customElements.get('ph-element-probe')).toBe(PhElementProbe);
  });

  it('attaches an open shadow root and adopts the shared console styles', () => {
    const el = document.createElement('ph-element-probe');
    expect(el.shadowRoot).toBeTruthy();
    // phAdoptConsoleStyles adopts via adoptedStyleSheets where supported, or
    // falls back to a <style data-ph-shared> element — either counts.
    const adopted = (el.shadowRoot.adoptedStyleSheets || []).length > 0;
    const fallbackStyle = !!el.shadowRoot.querySelector('style[data-ph-shared]');
    expect(adopted || fallbackStyle).toBe(true);
  });

  it('builds the shadow template from template() before onTemplate() runs, so cached refs are real nodes', () => {
    const el = document.createElement('ph-element-probe');
    expect(el.shadowRoot.getElementById('label')).toBeTruthy();
    expect(el.shadowRoot.getElementById('other')).toBeTruthy();
    // onTemplate cached a real reference, not null — proves it ran AFTER
    // the template was appended, not before.
    expect(el.labelRef).toBe(el.shadowRoot.getElementById('label'));
  });

  it('the default template() is empty and the default render() is a harmless no-op', () => {
    const el = document.createElement('ph-element-bare-probe');
    // phAdoptConsoleStyles may itself append a <style data-ph-shared> element
    // as its fallback (where adoptedStyleSheets is unsupported) — that is not
    // markup from template(), so exclude it before asserting template()
    // contributed nothing.
    const nonStyleChildren = Array.from(el.shadowRoot.childNodes).filter(
      (n) => !(n.tagName === 'STYLE' && n.hasAttribute && n.hasAttribute('data-ph-shared')),
    );
    expect(nonStyleChildren.length).toBe(0);
    expect(() => { el.state = { anything: true }; }).not.toThrow();
  });

  it('$(id) is the shadowRoot.getElementById(id) helper', () => {
    const el = document.createElement('ph-element-probe');
    expect(el.$('other')).toBe(el.shadowRoot.getElementById('other'));
    expect(el.$('does-not-exist')).toBeNull();
  });

  // ── set state: synchronous by default ─────────────────────────────────

  it('set state renders synchronously — render() has already run by the very next line', () => {
    const el = document.createElement('ph-element-probe');
    el.state = { text: 'hello' };
    expect(el.renderCount).toBe(1);
    expect(el.lastRendered).toEqual({ text: 'hello' });
    expect(el.shadowRoot.getElementById('label').textContent).toBe('hello');
  });

  it('set state defaults a null/undefined value to {} before calling render', () => {
    const el = document.createElement('ph-element-probe');
    el.state = null;
    expect(el.lastRendered).toEqual({});
    el.state = undefined;
    expect(el.lastRendered).toEqual({});
  });

  it('get state reflects the latest assignment', () => {
    const el = document.createElement('ph-element-probe');
    el.state = { text: 'a' };
    expect(el.state).toEqual({ text: 'a' });
    el.state = { text: 'b' };
    expect(el.state).toEqual({ text: 'b' });
  });

  it('renders once per assignment (no batching) by default', () => {
    const el = document.createElement('ph-element-probe');
    el.state = { text: 'a' };
    el.state = { text: 'b' };
    el.state = { text: 'c' };
    expect(el.renderCount).toBe(3);
  });

  // ── sendAction wiring ───────────────────────────────────────────────────

  it('wires sendAction from window.sendAction on connect', () => {
    const winSend = vi.fn();
    window.sendAction = winSend;
    const el = document.createElement('ph-element-probe');
    expect(el.sendAction).toBeUndefined();
    document.body.appendChild(el); // fires connectedCallback
    expect(el.sendAction).toBe(winSend);
  });

  it('does not clobber an explicitly assigned sendAction on a later connect (??=, not =)', () => {
    window.sendAction = vi.fn();
    const el = document.createElement('ph-element-probe');
    document.body.appendChild(el);
    const explicit = vi.fn();
    el.sendAction = explicit;
    // Re-parent: connectedCallback runs again.
    document.body.removeChild(el);
    document.body.appendChild(el);
    expect(el.sendAction).toBe(explicit);
  });

  it('leaves sendAction undefined when connected with no window.sendAction wired', () => {
    const el = document.createElement('ph-element-probe');
    document.body.appendChild(el);
    expect(el.sendAction).toBeUndefined();
  });

  // ── static coalesce opt-in ───────────────────────────────────────────────

  describe('static coalesce = true', () => {
    let rafCallback;
    let originalRAF;

    beforeEach(() => {
      originalRAF = window.requestAnimationFrame;
      rafCallback = undefined;
      window.requestAnimationFrame = vi.fn((cb) => { rafCallback = cb; return 1; });
    });
    afterEach(() => {
      window.requestAnimationFrame = originalRAF;
    });

    it('does not render synchronously — it schedules a frame instead', () => {
      const el = document.createElement('ph-element-coalesce-probe');
      el.state = { x: 1 };
      expect(el.renderCount).toBeUndefined();
      expect(window.requestAnimationFrame).toHaveBeenCalledTimes(1);
    });

    it('renders once the scheduled frame fires, with the assigned value', () => {
      const el = document.createElement('ph-element-coalesce-probe');
      el.state = { x: 1 };
      rafCallback();
      expect(el.renderCount).toBe(1);
      expect(el.lastRendered).toEqual({ x: 1 });
    });

    it('collapses several assignments within one frame into a single render of the LATEST value', () => {
      const el = document.createElement('ph-element-coalesce-probe');
      el.state = { x: 1 };
      el.state = { x: 2 };
      el.state = { x: 3 };
      // Only one frame was scheduled for all three assignments.
      expect(window.requestAnimationFrame).toHaveBeenCalledTimes(1);
      rafCallback();
      expect(el.renderCount).toBe(1);
      expect(el.lastRendered).toEqual({ x: 3 });
    });

    it('state getter reflects the latest assignment immediately, even before the deferred render fires', () => {
      const el = document.createElement('ph-element-coalesce-probe');
      el.state = { x: 1 };
      expect(el.state).toEqual({ x: 1 });
      expect(el.renderCount).toBeUndefined(); // render itself is still pending
    });

    it('schedules a fresh frame for assignments after a previous frame already fired', () => {
      const el = document.createElement('ph-element-coalesce-probe');
      el.state = { x: 1 };
      rafCallback();
      el.state = { x: 2 };
      expect(window.requestAnimationFrame).toHaveBeenCalledTimes(2);
      rafCallback();
      expect(el.renderCount).toBe(2);
      expect(el.lastRendered).toEqual({ x: 2 });
    });
  });
});
