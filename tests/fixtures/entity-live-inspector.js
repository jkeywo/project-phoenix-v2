/** Shared fixture for the entities/AI Live Inspector domain (issue #1489).
 *
 * Mirrors the wire shape `src/gm_entity_inspector.rs` publishes: a descriptor
 * table for the whole domain plus one reading per live entity.
 */
export function entityDescriptor(id, {
  kind = 'string', live_mutability = 'recreate-required', label = `inspector.entity.${id}`,
  group = 'identity', validation = [],
} = {}) {
  return {
    id,
    label,
    group,
    kind,
    default_source: null,
    live_mutability,
    origin: { schema_path: id, document: null, line: null, layer: null },
    validation,
  };
}

export const ENTITY_FIELDS = [
  entityDescriptor('name', { group: 'identity' }),
  entityDescriptor('transform.translation', { kind: 'vec3', group: 'placement' }),
  entityDescriptor('tags', { kind: 'list', group: 'tags' }),
  entityDescriptor('behaviour.doctrine', {
    kind: 'list', group: 'behaviour', live_mutability: 'named-action',
    validation: ['inspector.validation.authored_choice'],
  }),
  entityDescriptor('ai_profile.aggression', { kind: 'float', group: 'ai' }),
  entityDescriptor('scored_objectives.chosen', { group: 'derived', live_mutability: 'derived' }),
  entityDescriptor('current_target', { group: 'derived', live_mutability: 'derived' }),
];

/** A fresh payload every call.
 *
 * The descriptor table is deep-copied deliberately: a test that corrupts one
 * descriptor to prove the parser refuses it would otherwise corrupt the shared
 * fixture for every test after it, and the failures would land far from the
 * cause. */
export function entityInspectorPayload(readings, { npc_doctrines = { raider: {} } } = {}) {
  return {
    entity_inspector: { fields: structuredClone(ENTITY_FIELDS), readings },
    // The same projection the NPC doctrine panel reads, so the Inspector can
    // tell whether the named action can actually act on this selection.
    npc_doctrines,
  };
}

export const RAIDER_READING = {
  references: { current_target: 'courier' },
  values: {
    name: 'Raider',
    'transform.translation': '1.000, 2.000, 3.000',
    tags: 'hostile, raider',
    'behaviour.doctrine': 'destroy-hostiles',
    'ai_profile.aggression': '0.800',
    'scored_objectives.chosen': 'Destroy the courier',
    current_target: 'Courier',
  },
};

export const COURIER_READING = {
  values: { name: 'Courier', 'transform.translation': '9.000, 0.000, 0.000' },
};
