const integer = value => Number.isSafeInteger(value) && value >= 0;
const text = value => typeof value === 'string' && value.trim().length > 0;
export function capturedPayload(raw, system, controllable = false) {
  if (!raw || !text(system) || !Array.isArray(raw.rows) || raw.rows.length > 256) return null;
  const rows = [];
  for (const row of raw.rows) {
    if (!row || !integer(row.capture) || row.capture === 0 || !/^[a-z0-9-]{1,128}$/.test(row.channel)
      || !integer(row.level) || row.level > 3 || !integer(row.captured_tick)
      || typeof row.feasible !== 'boolean' || typeof row.selected !== 'boolean'
      || !integer(row.progress_per_mille) || row.progress_per_mille > 1000
      || (row.decoded_tick != null && (!integer(row.decoded_tick) || row.decoded_tick < row.captured_tick))) return null;
    const evidence = row.evidence;
    if (evidence != null && (row.decoded_tick == null || !text(evidence.sender_name) || !text(evidence.body)
      || typeof evidence.literal_body !== 'boolean' || !['text', 'request', 'outcome'].includes(evidence.kind)
      || !integer(evidence.emitted_tick) || evidence.emitted_tick > row.captured_tick)) return null;
    if ((row.decoded_tick != null) !== (evidence != null)) return null;
    rows.push({ capture: row.capture, channel: row.channel, level: row.level, captured_tick: row.captured_tick,
      feasible: row.feasible, selected: row.selected, progress_per_mille: row.progress_per_mille,
      decoded_tick: row.decoded_tick ?? null, ...(evidence == null ? {} : { evidence: {
        sender_name: evidence.sender_name, body: evidence.body, literal_body: evidence.literal_body,
        kind: evidence.kind, emitted_tick: evidence.emitted_tick } }) });
  }
  if (new Set(rows.map(row => row.capture)).size !== rows.length || rows.filter(row => row.selected).length > 1) return null;
  return { system, controllable: controllable === true, rows };
}
