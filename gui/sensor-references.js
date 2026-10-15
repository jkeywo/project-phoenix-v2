/** Held references and measured contacts delivered to this actual Console only. */
export function buildReferenceControls({ radars = [], dossiers = [], systemIds = [], controlSources = {} } = {}) {
  const records = new Map(), ambiguous = new Set();
  for (const { data } of dossiers) for (const subject of data?.subjects || []) for (const entry of subject.evidence || []) {
    const reference = entry.reference;
    if (!reference || typeof reference.id !== 'string' || !reference.id || typeof reference.unusual !== 'boolean'
      || !Number.isSafeInteger(reference.reading_ticks) || reference.reading_ticks < 0 || reference.reading_ticks > 4294967295
      || typeof entry.text !== 'string' || !Number.isSafeInteger(entry.gathered_at_tick) || entry.gathered_at_tick < 0) continue;
    const row = { reference: { ...reference }, text: entry.text, provenance: entry.provenance,
      gathered_at_tick: entry.gathered_at_tick, source_name: subject.name || '' };
    if (records.has(reference.id) && JSON.stringify(records.get(reference.id)) !== JSON.stringify(row)) ambiguous.add(reference.id);
    else records.set(reference.id, row);
  }
  const held = [...records.values()].filter(row => !ambiguous.has(row.reference.id)).sort((a, b) => a.reference.id.localeCompare(b.reference.id));
  return radars.filter(row => systemIds.includes(row.systemId) && Array.isArray(row.data?.tracks))
    .map(row => ({ system_id: row.systemId, controllable: controlSources[row.systemId] === 'Human',
      contacts: row.data.tracks.flatMap(track => typeof track?.contact_id === 'string' && track.contact_id ? [track.contact_id] : []), records: held }));
}
