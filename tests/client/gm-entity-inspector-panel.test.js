// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  createGmEntityInspectorPanel, parseGmEntityInspectorPayload, GM_INSPECTOR_HISTORY_LIMIT,
} from '../../gui/gm-entity-inspector-panel.js';
import {
  COURIER_READING, ENTITY_FIELDS, RAIDER_READING, entityInspectorPayload,
} from '../fixtures/entity-live-inspector.js';
import { readFileSync } from 'node:fs';

// The REAL panel markup, not a hand-written stand-in. A fixture that drifts
// from server.html lets the shipped surface lose an aria-live or a control
// while every test stays green.
const SOURCE = readFileSync('server.html', 'utf8');
const PANEL_MARKUP = SOURCE.slice(
  SOURCE.indexOf('<section id="gm-entity-fields-panel"'),
  SOURCE.indexOf('</section>', SOURCE.indexOf('<article id="gm-entity-fields-card"')) + '</section>'.length,
);

const PAYLOAD = () => entityInspectorPayload({ raider: RAIDER_READING, courier: COURIER_READING });

// The stub renders its parameters alongside the id, so a test can tell whether
// the panel put the entity's name INTO the line it claims announces it — an
// id-only stub would pass whether the name reached the line or not.
const t = (id, params) => (params ? `${id} ${Object.values(params).join(' ')}` : id);

function mount(options = {}) {
  document.body.innerHTML = PANEL_MARKUP;
  return createGmEntityInspectorPanel({ doc: document, t, ...options });
}

const row = id => document.querySelector(`[data-field="${id}"]`);
const valueOf = id => row(id)?.querySelector('input')?.value;

describe('parseGmEntityInspectorPayload', () => {
  it('accepts the published domain and keys readings by entity id', () => {
    const parsed = parseGmEntityInspectorPayload(PAYLOAD());
    expect(parsed.fields).toHaveLength(ENTITY_FIELDS.length);
    expect([...parsed.readings.keys()]).toEqual(['raider', 'courier']);
    expect(parsed.readings.get('raider').values.get('name')).toBe('Raider');
    // A reference carries the referred entity's ID, so a hop follows identity
    // rather than the displayed name.
    expect(parsed.readings.get('raider').references.get('current_target')).toBe('courier');
  });

  it('rejects a malformed domain whole rather than in part', () => {
    expect(parseGmEntityInspectorPayload({ entity_inspector: { fields: [], readings: null } })).toBeNull();
    // A descriptor whose id disagrees with its own schema path would make the
    // reading key and the metadata line describe different fields.
    const mismatched = entityInspectorPayload({});
    mismatched.entity_inspector.fields[0].origin.schema_path = 'elsewhere';
    expect(parseGmEntityInspectorPayload(mismatched)).toBeNull();
    // A reading naming a field with no descriptor has no mutability, so it
    // cannot be rendered honestly.
    const unknown = entityInspectorPayload({ raider: { values: { mystery: 'x' } } });
    expect(parseGmEntityInspectorPayload(unknown)).toBeNull();
    // An unclassified mutability is refused by the shared descriptor check.
    const bad = entityInspectorPayload({});
    bad.entity_inspector.fields[0].live_mutability = 'editable';
    expect(parseGmEntityInspectorPayload(bad)).toBeNull();
  });
});

describe('createGmEntityInspectorPanel', () => {
  beforeEach(() => { document.body.innerHTML = ''; });

  it('renders every reading the selected entity carries, and nothing it does not', () => {
    const panel = mount();
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    expect(valueOf('name')).toBe('Raider');
    expect(valueOf('ai_profile.aggression')).toBe('0.800');
    // The courier authored no AI profile. An absent field is absent, not blank:
    // "not authored" and "authored empty" are different facts.
    panel.select({ entity_id: 'courier' });
    expect(valueOf('name')).toBe('Courier');
    expect(row('ai_profile.aggression').hidden).toBe(true);
    expect(row('tags').hidden).toBe(true);
  });

  it('classifies every field and makes none of them editable', () => {
    const panel = mount();
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    for (const field of ENTITY_FIELDS) {
      const node = row(field.id);
      if (node.hidden) continue;
      const slot = node.querySelector('[data-inspector-value]');
      expect(slot.dataset.mutability, field.id).toBe(field.live_mutability);
      const input = slot.querySelector('input');
      expect(input.disabled, field.id).toBe(true);
      expect(input.readOnly, field.id).toBe(true);
    }
    // Derived context explains the authored field beside it and never becomes
    // an edit: there is no control here that submits anything.
    expect(document.querySelectorAll('#gm-entity-fields-list input:not([disabled])')).toHaveLength(0);
    expect(document.querySelectorAll('#gm-entity-fields-list select')).toHaveLength(0);
  });

  it('links the one named action instead of repeating its form', () => {
    const focusDoctrine = vi.fn();
    const panel = mount({ focusDoctrine });
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    const link = row('behaviour.doctrine').querySelector('[data-inspector-action]');
    expect(link).not.toBeNull();
    link.click();
    expect(focusDoctrine).toHaveBeenCalledWith('raider');
    // Reaching the owning panel is the whole of it: no second doctrine chooser
    // and no submission of any kind lives here.
    expect(row('behaviour.doctrine').querySelector('select')).toBeNull();
  });

  it('freezes a despawned entity as gone and disables its action', () => {
    const focusDoctrine = vi.fn();
    const panel = mount({ focusDoctrine });
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    panel.update(entityInspectorPayload({ courier: COURIER_READING }));
    expect(panel.state().gone).toBe(true);
    // The final reading is still on screen — it is what was true when the hull
    // left — and every action is unavailable.
    expect(valueOf('name')).toBe('Raider');
    expect(document.getElementById('gm-entity-fields-status').dataset.gone).toBe('true');
    expect(row('behaviour.doctrine').querySelector('[data-inspector-action]').disabled).toBe(true);
  });

  it('never follows a replacement that reuses the authored name', () => {
    const panel = mount();
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    // A new hull spawns with the same authored name and a different id. The
    // selection must stay gone rather than silently re-point at a stranger.
    panel.update(entityInspectorPayload({
      'raider-2': { values: { name: 'Raider', 'ai_profile.aggression': '0.100' } },
    }));
    expect(panel.state().gone).toBe(true);
    expect(panel.state().selected).toBe('raider');
    expect(valueOf('ai_profile.aggression')).toBe('0.800');
  });

  it('retraces reference hops through a bounded history', () => {
    const panel = mount();
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    panel.select({ entity_id: 'courier' });
    expect(panel.state().selected).toBe('courier');
    const back = document.getElementById('gm-entity-fields-back');
    const forward = document.getElementById('gm-entity-fields-forward');
    expect(back.disabled).toBe(false);
    back.click();
    expect(panel.state().selected).toBe('raider');
    expect(valueOf('name')).toBe('Raider');
    expect(forward.disabled).toBe(false);
    forward.click();
    expect(panel.state().selected).toBe('courier');
    expect(forward.disabled).toBe(true);

    // Bounded: a long walk keeps the most recent hops and drops the oldest
    // rather than growing without limit.
    for (let index = 0; index < GM_INSPECTOR_HISTORY_LIMIT + 5; index += 1) {
      panel.select({ entity_id: index % 2 ? 'raider' : 'courier' });
    }
    expect(panel.state().history.length).toBeLessThanOrEqual(GM_INSPECTOR_HISTORY_LIMIT);
  });

  it('follows a reference by id, through the desk selection owner', () => {
    const selectEntity = vi.fn();
    const panel = mount({ selectEntity });
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    const hop = row('current_target').querySelector('[data-inspector-reference]');
    expect(hop).not.toBeNull();
    expect(hop.disabled).toBe(false);
    hop.click();
    // The published id, never the rendered name: a replacement sharing the
    // name must not be reachable by following this control.
    expect(selectEntity).toHaveBeenCalledWith('courier');
  });

  it('offers no reference hop to an entity that is no longer live', () => {
    const selectEntity = vi.fn();
    const panel = mount({ selectEntity });
    // The raider still names the courier as its target, but the courier has
    // left. A control that leads nowhere is worse than no control.
    panel.update(entityInspectorPayload({ raider: RAIDER_READING }));
    panel.select({ entity_id: 'raider' });
    const hop = row('current_target').querySelector('[data-inspector-reference]');
    expect(hop === null || hop.disabled).toBe(true);
  });

  it('announces which entity is being read, and restores focus after a hop', () => {
    const panel = mount();
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    const status = document.getElementById('gm-entity-fields-status');
    // The subject line is the live region: without it a Back/Forward hop
    // rewrites fifty rows and announces nothing.
    expect(status.getAttribute('aria-live')).toBe('polite');
    expect(status.textContent).toContain('Raider');
    panel.select({ entity_id: 'courier' });
    expect(status.textContent).toContain('Courier');
    document.getElementById('gm-entity-fields-back').click();
    expect(status.textContent).toContain('Raider');
    // The reader is put back at the subject rather than left on a control
    // whose meaning just changed underneath them.
    expect(document.activeElement).toBe(status);
  });

  it('offers the doctrine link only where that action can act', () => {
    const focusDoctrine = vi.fn();
    const panel = mount({ focusDoctrine });
    // The courier carries a doctrine reading but is not a target the checked
    // doctrine transaction accepts.
    panel.update(entityInspectorPayload(
      { courier: { values: { name: 'Courier', 'behaviour.doctrine': 'hold' } } },
      { npc_doctrines: {} }));
    panel.select({ entity_id: 'courier' });
    expect(row('behaviour.doctrine').querySelector('[data-inspector-action]').disabled).toBe(true);
  });

  it('is wired into the desk: folded, selected, reset and linked', () => {
    // The panel is only a reading surface if something actually feeds it. This
    // reads the wiring the way its sibling panels' tests do.
    const workspace = readFileSync('gui/gm-workspace.js', 'utf8');
    expect(workspace).toContain('gmEntityFields.update(p)');
    expect(workspace).toContain('gmEntityFields.select(entity)');
    expect(workspace).toContain('gmEntityFields.reset()');
    // The doctrine link must AIM the owning panel, not merely reveal it: a
    // callback that drops the entity id shows a control pointed at whatever
    // was selected before.
    expect(workspace).toContain('focusDoctrine: entityId =>');
    expect(workspace).toContain('gmNpc.select(entity)');
    // And a reference hop goes through the desk's selection owner rather than
    // a private idea of what is selected.
    expect(workspace).toContain('selectEntity: entityId => gmProjection.select(entityId)');
  });

  it('keeps the previous domain when an update is malformed', () => {
    const panel = mount();
    panel.update(PAYLOAD());
    panel.select({ entity_id: 'raider' });
    expect(panel.update({ entity_inspector: { fields: 'broken', readings: {} } })).toBe(false);
    expect(valueOf('name')).toBe('Raider');
    expect(panel.state().fields).toBe(ENTITY_FIELDS.length);
  });
});
