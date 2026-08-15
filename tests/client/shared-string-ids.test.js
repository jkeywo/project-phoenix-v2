/**
 * Shared display-text ids for shield arcs and weapon mounts (issue #950).
 *
 * A shield arc's "Fore" and a torpedo tube's "Fore" mean the same thing on every
 * hull that has one, so they are authored once — `shield_arc.<arc>.label` and
 * `weapon.<mount>.display_name` — instead of once per hull under
 * `entity.<hull>.…`. check-strings.mjs enforces the half that fails loudly (an
 * id in TOML with no CSV row) but says nothing when a hull quietly re-adds its
 * own id for text that already exists. That regression is invisible in game
 * (both ids resolve to "Fore") and only shows up as a translator asking why they
 * are being asked to translate the same word nine times.
 *
 * Shared by default, NOT shared by force
 * --------------------------------------
 * A hull that genuinely means something different by a mount says so in its own
 * words, and that has to keep working: the courier already diverges in the
 * neighbouring namespace, calling its `helm-engine-port` "Port Engine" where the
 * other three hulls call the same system "Engine (Port)". So what is asserted
 * here is CONSISTENCY, not a fixed string:
 *
 *   1. arc labels and mount names stay inside the shared namespace, which is
 *      what keeps the pre-#950 `entity.<hull>.…` shape from creeping back one
 *      hull at a time;
 *   2. every such id resolves to text — a shared id with no row goes blank on
 *      every hull at once;
 *   3. hulls that render the SAME text reach it through ONE id;
 *   4. one id names one arc or one mount, so a pasted `shield_arc.fore.label`
 *      cannot quietly relabel the aft arc.
 *
 * Together (3) and (4) let `weapon.battleship_phaser_fore.display_name` exist
 * for a hull that wants its own wording, while still failing the day a hull
 * re-authors "Fore" under a second id.
 */
import { describe, it, expect } from 'vitest';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse as parseToml } from 'smol-toml';
import { buildTable } from '../../gui/strings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const entitiesDir = path.join(root, 'assets', 'entities');
const registryFile = path.join(root, 'src', 'ship', 'system_registry.rs');
const stringsFile = path.join(root, 'assets', 'strings', 'strings.csv');

/**
 * Registry system kinds that are NOT weapon mounts — a mount being a weapon
 * bolted to a facing, one `[[hull.system_hull]]` block per barrel.
 *
 * The mount set is the registry's whole `*_KIND` vocabulary MINUS this list.
 * That inversion is the point: a new weapon family (`railgun_bank`,
 * `missile_pod`) is covered the day its constant lands. The predecessor here was
 * a hand-written `/^(phaser|torpedo|blaster)/` allowlist, which silently skipped
 * every family nobody had remembered to teach it — a per-hull id on
 * `railgun-fore` would have sailed straight past — and over-matched
 * `torpedo-magazine`, a weapons-station system but not a mount, into the
 * bargain.
 *
 * The failure direction is deliberate. A new NON-weapon kind that is not listed
 * here turns up as a false mount and fails this file loudly, naming the kind: a
 * one-line fix. A new weapon kind slipping through unnoticed is not a one-line
 * fix, because by the time anyone notices, the hulls have been authored.
 */
const NON_MOUNT_KINDS = new Set([
  // Station-owned coarse systems.
  'captain', 'comms', 'navigation', 'power', 'red_alert', 'repair', 'sensors',
  'shields', 'viewscreen',
  // Fine helm systems.
  'helm_boost', 'helm_engine', 'helm_impulse', 'helm_joystick', 'helm_radar',
  'helm_steering', 'helm_thrust', 'lateral_thrust', 'vertical_thrust',
  // Fine tactical systems that are not mounts: ship-wide phaser settings, the
  // two radars, and the magazine the tubes claim rounds from.
  'phaser_control', 'sensor_radar', 'tactical_radar', 'torpedo_magazine',
  // Fine power and shield systems.
  'power_battery', 'power_reactor', 'shield_arc',
]);

/** Every `*_KIND` constant declared in src/ship/system_registry.rs. */
function registryKinds(src) {
  const found = [...src.matchAll(/pub const [A-Z0-9_]+_KIND\s*:\s*&str\s*=\s*"([^"]+)"/g)];
  return new Set(found.map((m) => m[1]));
}

/** Every `assets/entities/**\/*.toml`, including fragments and test-only hulls. */
async function entityFiles(dir = entitiesDir, acc = []) {
  for (const name of await readdir(dir, { withFileTypes: true })) {
    const full = path.join(dir, name.name);
    if (name.isDirectory()) await entityFiles(full, acc);
    else if (name.name.endsWith('.toml')) acc.push(full);
  }
  return acc;
}

/** Parse every entity template once; the assertions all read off the same set. */
async function entityDocs() {
  const docs = [];
  for (const file of (await entityFiles()).sort()) {
    docs.push({
      where: path.relative(root, file).replace(/\\/g, '/'),
      doc: parseToml(await readFile(file, 'utf8')),
    });
  }
  return docs;
}

/**
 * The `[[shield_arc]]` labels and weapon-mount display names authored across the
 * fleet, as `{ family, slot, id, where }`.
 *
 * A `[[hull.system_hull]]` block names a system; that system's `[[system]] kind`
 * is what decides whether it is a mount. Kinds are pooled across every template
 * because a hull may inherit its `[[system]]` declarations from a fragment, and
 * a hull-only component with no `[[system]]` block at all (`core`, `science`) is
 * not a system and so cannot be a weapon.
 */
function authoredSlots(docs, mountKinds) {
  const kindOf = new Map();
  for (const { doc } of docs) {
    for (const system of doc.system ?? []) {
      if (system.id && system.kind) kindOf.set(system.id, system.kind);
    }
  }

  const slots = [];
  for (const { where, doc } of docs) {
    for (const arc of doc.shield_arc ?? []) {
      if (arc.id && arc.label) {
        slots.push({ family: 'shield arc', prefix: 'shield_arc', slot: arc.id, id: arc.label, where });
      }
    }
    for (const block of doc.hull?.system_hull ?? []) {
      if (!mountKinds.has(kindOf.get(block.system_id))) continue;
      if (block.system_id && block.display_name) {
        slots.push({ family: 'weapon mount', prefix: 'weapon', slot: block.system_id, id: block.display_name, where });
      }
    }
  }
  return slots;
}

describe('shared display-text ids', () => {
  it('takes its weapon-mount kinds from the Rust registry, not a hand-written list', async () => {
    const kinds = registryKinds(await readFile(registryFile, 'utf8'));
    expect(kinds.size, 'no *_KIND constants parsed out of system_registry.rs').toBeGreaterThan(0);

    // A denylist entry that names no real kind is a rename nobody followed
    // through, and it silently promotes that kind to a mount.
    for (const kind of NON_MOUNT_KINDS) {
      expect([...kinds], `NON_MOUNT_KINDS names '${kind}', which no *_KIND constant declares`)
        .toContain(kind);
    }

    // A lower bound, deliberately not an equality: a new weapon family must be
    // picked up here without anyone editing this file.
    const mounts = [...kinds].filter((k) => !NON_MOUNT_KINDS.has(k)).sort();
    expect(mounts).toEqual(expect.arrayContaining(['blaster_bank', 'phaser_bank', 'torpedo_tube']));

    // …and the vocabulary has to be complete, or the subtraction above classifies
    // nothing: a kind the registry has never heard of would be neither a mount
    // nor a listed non-mount, and its hull blocks would be skipped in silence —
    // the exact hole the old hardcoded allowlist had. Rust rejects an unknown
    // kind at load; this says so at the point the classification depends on it.
    const unknown = new Set();
    for (const { doc } of await entityDocs()) {
      for (const system of doc.system ?? []) if (!kinds.has(system.kind)) unknown.add(system.kind);
    }
    expect([...unknown].sort(), 'ship TOML declares [[system]] kinds system_registry.rs does not')
      .toEqual([]);
  });

  it('gives hulls that word an arc or mount the same way the same id for it', async () => {
    const table = buildTable(await readFile(stringsFile, 'utf8'));
    const kinds = registryKinds(await readFile(registryFile, 'utf8'));
    const mountKinds = new Set([...kinds].filter((k) => !NON_MOUNT_KINDS.has(k)));
    const slots = authoredSlots(await entityDocs(), mountKinds);

    expect(slots.length, 'no arc labels or weapon mounts found — the walk is broken').toBeGreaterThan(0);

    /**
     * `${family}\u0000${text}` -> the first id that rendered it.
     *
     * The separator is written as an ESCAPE SEQUENCE, never as a raw byte. A
     * literal NUL in the source makes ripgrep and git classify the whole file
     * as binary, after which it answers no content search in the repo at all:
     * gui/components/ph-operation-panel.js hid from an entire audit that way.
     */
    const idForText = new Map();
    /** id → the arc or mount it names. */
    const slotForId = new Map();

    for (const { family, prefix, slot, id, where } of slots) {
      expect(
        id,
        `${where}: ${family} '${slot}' is authored as '${id}'. Arc labels and mount `
        + `names live in the shared '${prefix}.' namespace — the per-hull `
        + `'entity.<hull>.…' shape #950 removed must not come back. A hull that `
        + `wants its own wording authors its own id inside the namespace, e.g. `
        + `'${prefix}.battleship_${slot.replace(/-/g, '_')}.…'.`,
      ).toMatch(new RegExp(`^${prefix}\\.`));

      const text = table.get(id);
      expect(text, `${where}: ${family} '${slot}' → '${id}' has no strings.csv row`).toBeTruthy();

      const textKey = `${family}\u0000${text}`;
      const first = idForText.get(textKey);
      if (first) {
        expect(
          id,
          `${where}: ${family} '${slot}' renders "${text}" through a second id. `
          + `${first.where} already authors that text as '${first.id}' — reuse it, `
          + `or give this one wording of its own.`,
        ).toBe(first.id);
      } else {
        idForText.set(textKey, { id, where });
      }

      const holder = slotForId.get(id);
      if (holder) {
        expect(
          `${family} '${slot}'`,
          `${where}: '${id}' already names ${holder.family} '${holder.slot}' at ${holder.where}. `
          + `One shared id means one arc or one mount — nothing else notices when the `
          + `wrong one is pasted in, because it resolves either way.`,
        ).toBe(`${holder.family} '${holder.slot}'`);
      } else {
        slotForId.set(id, { family, slot, where });
      }
    }

    // Guard the guard: hulls really do share, so the assertions above ran on
    // more authored blocks than there are distinct ids.
    expect(slots.length).toBeGreaterThan(slotForId.size);
  });

  it('resolves every shared arc and weapon id to text', async () => {
    const table = buildTable(await readFile(stringsFile, 'utf8'));
    const shared = [...table.keys()].filter((id) => /^(shield_arc|weapon)\./.test(id));
    expect(shared.length).toBeGreaterThan(0);
    for (const id of shared) expect(table.get(id), id).not.toBe('');
  });
});
