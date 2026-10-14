import fs from 'node:fs';
import path from 'node:path';
import { parse, stringify } from 'smol-toml';
import { resolveTemplate } from '../entity-includes.js';

/** Actual accepted-source machinery, flattened only for a disposable test document. */
export function damageFixture() {
  const root = path.resolve(import.meta.dirname, '../..');
  const resolved = resolveTemplate('assets/entities/alliance_cruiser.toml', {
    read: file => fs.existsSync(path.join(root, file)) ? fs.readFileSync(path.join(root, file), 'utf8') : null,
  }, parse);
  if (!resolved.ok) throw new Error(JSON.stringify(resolved));
  const value = structuredClone(resolved.resolved.value);
  const publications = Object.fromEntries((value.console || []).flatMap(console => (console.feed || []).map(feed => [feed.source, feed.kind])));
  return { value, source: stringify(value), schema: {
    system_kinds: [...new Set([...value.system.map(system => system.kind), 'phaser_bank', 'blaster_bank', 'torpedo_tube'])],
    console_feed_kinds: [...new Set(Object.values(publications))], console_publication_kinds: publications,
    console_instance_publication_kinds: {}, directive_kinds: [...new Set((value.behaviour?.doctrine || []).map(row => row.directive_kind).filter(Boolean))],
  } };
}
