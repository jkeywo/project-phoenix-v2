/**
 * tests/client/ship-cards.test.js — the ship picker's art, resolved at build
 * time (PRD #1023 module 4, user story 3).
 *
 * What is worth asserting here is that the resolution chain is really entity →
 * model → sidecar → billboard, because guessing the filename from the entity
 * stem is wrong the moment two hulls share a model (`ship_civilian_hauler.toml`
 * already uses `dynasty_courier.glb`). What is NOT worth asserting is which
 * hulls happen to be playable today — that is world content and will change.
 *
 * The other half of the contract, the strip maths that turns one of these
 * entries into a visible tile, lives with the component in
 * tests/client/ph-ship-picker.test.js.
 */
import { describe, it, expect } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { billboardFor, playableHulls, shipCardIndex } from '../../scripts/ship-cards.mjs';

const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

describe('billboardFor — entity → model → sidecar → billboard', () => {
  it('resolves a playable hull to its captured atlas', () => {
    const found = billboardFor(ROOT, 'assets/entities/alliance_destroyer.toml');
    expect(found).not.toBeNull();
    expect(found.atlas).toMatch(/alliance_destroyer/);
    expect(found.views).toBeGreaterThan(1);
    expect(fs.existsSync(path.join(ROOT, found.atlas))).toBe(true);
  });

  it('follows the mesh model, not the entity filename', () => {
    // The entity is `ship_civilian_hauler`; the model is `dynasty_courier`.
    // A stem-guessing implementation gets this one wrong.
    const found = billboardFor(ROOT, 'assets/entities/ship_civilian_hauler.toml');
    expect(found).not.toBeNull();
    expect(found.atlas).toContain('dynasty_courier');
    expect(found.atlas).not.toContain('civilian_hauler');
  });

  it('is null for a template that does not exist', () => {
    expect(billboardFor(ROOT, 'assets/entities/no_such_hull.toml')).toBeNull();
  });

  it('is null for an entity with no mesh at all', () => {
    // Region bands are gameplay volumes, not models.
    expect(billboardFor(ROOT, 'assets/entities/region_storm_band.toml')).toBeNull();
  });
});

describe('playableHulls / shipCardIndex', () => {
  it('scopes the index to hulls a world offers as a playable choice', () => {
    const hulls = playableHulls(ROOT);
    expect(hulls.size).toBeGreaterThan(0);
    for (const templatePath of hulls) {
      expect(templatePath.startsWith('assets/entities/')).toBe(true);
    }
  });

  it('keys the index by template_path — the only hull identity on the wire', () => {
    const index = shipCardIndex(ROOT);
    for (const [templatePath, entry] of Object.entries(index)) {
      expect(templatePath).toMatch(/^assets\/entities\/.+\.toml$/);
      expect(entry.image).toMatch(/^assets\/ship-cards\/.+\.png$/);
      expect(entry.views).toBeGreaterThan(1);
      expect(entry.tile).toBeGreaterThanOrEqual(0);
      expect(entry.tile).toBeLessThan(entry.views);
    }
  });

  it('gives every playable hull with a captured billboard an entry', () => {
    const index = shipCardIndex(ROOT);
    for (const templatePath of playableHulls(ROOT)) {
      if (billboardFor(ROOT, templatePath)) {
        expect(index[templatePath]).toBeDefined();
      }
    }
  });
});
