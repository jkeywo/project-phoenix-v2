import { describe, expect, it } from 'vitest';
import { addressedModifierSnippet, objectiveInstanceSnippet, upsertObjectiveInstanceSnippet } from '../workshop-objective-snippet.js';

describe('Workshop Objective source template', () => {
  it('supports all recipient selectors and preserves addressed identity', () => {
    const snippet = objectiveInstanceSnippet({ id: 'escort', instanceId: 'lead', text: 'objective.escort',
      shipSlots: ['lead'], factions: ['alliance'], allPlayerShips: true, completeAfterSeconds: 30 });
    expect(snippet).toContain('text: "objective.escort"');
    expect(snippet).toContain('recipient_ship_slots: ["lead"]');
    expect(snippet).toContain('recipient_factions: ["alliance"]');
    expect(snippet).toContain('all_player_ships: true');
    expect(snippet).toContain('complete_objective("escort", "lead")');
    const next = objectiveInstanceSnippet({ id: 'escort', instanceId: 'lead', text: 'objective.escort',
      shipSlots: ['lead'] }, snippet);
    expect(next).toContain('fn workshop_objective_escort_lead_2(ctx)');
    expect(objectiveInstanceSnippet({ id: 'escort', instanceId: 'lead', text: 'objective.escort',
      shipSlots: ['lead scout'] })).toContain('recipient_ship_slots: ["lead scout"]');
  });

  it('refuses anonymous recipients and invalid identifiers before touching an editor', () => {
    expect(() => objectiveInstanceSnippet({ id: 'escort', instanceId: 'lead', text: 'x' })).toThrow('workshop.objective.error_recipient');
    expect(() => objectiveInstanceSnippet({ id: 'bad id', instanceId: 'lead', text: 'x', allPlayerShips: true }))
      .toThrow('workshop.objective.error_fields');
    expect(() => objectiveInstanceSnippet({ id: 'escort', instanceId: 'lead', text: 'x', shipSlots: ['lead', 'lead'] }))
      .toThrow('workshop.objective.error_selector');
  });

  it('updates an authored template in place while preserving surrounding script', () => {
    const original = 'fn other(ctx) {}\n';
    const values = { id: 'escort', instanceId: 'lead', text: 'objective.escort', shipSlots: ['lead'] };
    const first = upsertObjectiveInstanceSnippet(values, original);
    const updated = upsertObjectiveInstanceSnippet({ ...values, shipSlots: ['wing'] }, first);
    expect(updated).toContain('fn other(ctx) {}');
    expect(updated).toContain('recipient_ship_slots: ["wing"]');
    expect(updated).not.toContain('recipient_ship_slots: ["lead"]');
    expect(updated.match(/on_world_loaded\(/g)).toHaveLength(1);
  });

  it('inserts a modifier for live recipients without inventing an entity target', () => {
    const snippet = addressedModifierSnippet({ kind: 'apply_modifier', slot: 'MaxSpeed', tag: 'escort_boost',
      bonus: 1.5, shipSlots: ['lead'], factions: ['Alliance'], instanceMembers: true,
      objectiveId: 'escort', instanceId: 'pair' });
    expect(snippet).toContain('ctx.effects.addressed(#{');
    expect(snippet).toContain('type: "apply_modifier"');
    expect(snippet).toContain('bonus: flt("1.5")');
    expect(snippet).toContain('recipient_ship_slots: ["lead"]');
    expect(snippet).toContain('recipient_objective_instances: [#{ objective_id: "escort", instance_id: "pair" }]');
    expect(snippet).not.toMatch(/\bentity:/);
    expect(addressedModifierSnippet({ kind: 'remove_modifier', slot: 'MaxSpeed', tag: 'escort_boost' }))
      .toContain('recipient_ship_slots: []');
    expect(() => addressedModifierSnippet({ kind: 'apply_modifier', slot: 'MaxSpeed', tag: 'x', bonus: 0 }))
      .toThrow('workshop.objective.error_bonus');
  });
});
