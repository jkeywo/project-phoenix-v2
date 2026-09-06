/**
 * Truth / Crew Knowledge / Difference comparison for one selected ship
 * (issue #1318).
 *
 * This is deliberately a PRESENTATION-ONLY reuse of the projections the GM
 * page already receives every tick, plus the exact per-ship builders a
 * crew's own console iframe calls. It invents no second knowledge model:
 *
 *  - Truth is the omniscient `gm_entity` Host Channel payload (every world
 *    entity, unfiltered) — the same array `gm-local-projection.js` parses.
 *  - Crew Knowledge is built by feeding the selected ship's raw replica
 *    (from the `gm_station` Host Channel payload) through the SAME
 *    `buildGmStationConsoleInput` fold `gm-station-puppet.js` uses for
 *    Station puppeting, then reading it with the SAME public builders a
 *    crew member's own iframe calls: `buildSensorsConsoleState` for
 *    contacts, `buildCommsConsoleState` for Comms, and the folded
 *    `ClientSimState.objectives` for Objectives.
 *  - Difference is a pure identity-keyed diff over those two views.
 *
 * T2 has no fog-of-war/confidence model yet (issues #1063, #1065, #1070 are
 * the accepted future extension this comparison is built to host without a
 * hard dependency on them landing first). Concretely, today:
 *  - Objectives and Comms are broadcast to every ship identically (neither
 *    `GmPuppetShipProjection.objectives`/`.blackboards` nor the folded
 *    `ClientSimState` values are filtered per ship), so those two categories
 *    are expected to diff as "equal" until a future information-control
 *    feature makes crew knowledge diverge from Truth. That is expected, not
 *    a bug — this module does not invent per-ship filtering to manufacture a
 *    difference. Objectives is a tautological "equal": `ClientSimState.objectives`
 *    is stored verbatim from the SAME `ObjectiveSnapshot` array the ship
 *    projection carries (`gui/sim-state.js`'s `ObjectiveSummary` handler), so
 *    the two arms diff one array against itself by reference, not merely by
 *    value. Comms is "equal" for a related but distinct reason: the crew
 *    arm's `blackboardsOfKind` (`gui/console-state.js`) falls back to the
 *    lexically-first Comms system id, and the producer
 *    (`src/gm_projection.rs`) already sorts `blackboards` by system id before
 *    it reaches the wire, so both arms resolve the identical Comms system
 *    entry. Neither category is wired to genuinely differ today — both are
 *    documented here as the seam a future per-ship filter (#1063/#1065/#1070,
 *    or a Comms Station-preference read) lands in, not as live comparisons
 *    exercising two independent sources (issue #1318 review, findings 1 & 3).
 *  - Contacts CAN genuinely differ today: `buildSensorsConsoleState` reads
 *    the ship's own sensors radar range/tag filters, so a Truth entity
 *    outside that ship's scan range or hidden by its radar tag filters is
 *    absent from Crew Knowledge while still present in Truth.
 *
 * Two wire-boundary details this module must respect, both stemming from
 * `gui/host-channel.js`'s `localiseHostPayload`:
 *  - The `gm_entity` Host Channel payload (Truth's source) is deliberately
 *    EXEMPT from localisation — its string fields arrive exactly as Rust
 *    sent them, i.e. raw String Table ids more often than not. `gm_station`
 *    (Crew Knowledge's source), by contrast, crosses the localisation
 *    boundary and arrives with every resolvable id already substituted.
 *    Rendering a raw Truth id next to a resolved Crew Knowledge string would
 *    both leak a player-visible id (AGENTS.md rule 11) and manufacture a
 *    spurious `changed` row, so any Truth field that can carry a display
 *    name is resolved through `wireText` before it is compared or rendered.
 *  - Conversely, text that has ALREADY crossed that boundary (objective
 *    text, comms subjects) must never be re-resolved with a bare `t(...)`
 *    call — `t` on already-resolved English produces a `console.warn` and a
 *    visible `⟨...⟩` wrapper. Use `wireText(value, value)` there instead: a
 *    genuine id still resolves, and already-resolved prose passes through.
 *
 * Station-private detail (`console_hull`, non-Sensors/Comms blackboards) is
 * intentionally never read here — see the doc comment on
 * `GmPuppetShipProjection` in `src/gm_projection.rs` for why that payload
 * carries it unfiltered for the takeover feature. Station-private detail
 * stays absent from this comparison; it is reachable only by the GM opening
 * that ship's authentic Station interface (issue #1299).
 */

import { buildSensorsConsoleState, buildCommsConsoleState } from './console-state.js';
import { buildGmStationConsoleInput } from './gm-station-puppet.js';
import { wireText } from './strings.js';

// Synthetic overlay blips `buildSensorsConsoleState` adds on top of real
// contacts (the tactical-target duplicate and the navigation waypoint) are
// presentation markers, not world identities — excluded before diffing.
const SYNTHETIC_CONTACT_KINDS = new Set(['tactical-target']);
const SYNTHETIC_CONTACT_IDS = new Set(['navigation-waypoint']);

function sameValue(left, right) {
  if (Array.isArray(left) && Array.isArray(right)) {
    return left.length === right.length && left.every((value, index) => sameValue(value, right[index]));
  }
  return left === right;
}

/**
 * `hull_percent` is only a genuine knowledge difference when BOTH arms
 * actually carry a hull reading for this identity. Truth's `broad_status`
 * (src/gm_projection.rs) omits it (`None`) for infrastructure-only
 * structures with no `EntitySystemHull`; the crew arm omits it (`null`)
 * whenever the folded replica carries no `hull_fraction`. Either direction
 * can also be the ONLY side with a reading (e.g. an authored asteroid's
 * `hull_fraction` has no Truth `EntitySystemHull` counterpart at all), so a
 * `null` on either side means "not modelled here", never "modelled and
 * different" — treat it as not comparable rather than as a mismatch.
 */
function hullPercentSame(truth, crew) {
  if (truth === null || crew === null) return true;
  return truth === crew;
}

/**
 * Generic identity-keyed diff. Every row is exactly one of:
 *  - `truth_only`  — present in Truth, absent from Crew Knowledge (the crew
 *                    does not currently know about this identity).
 *  - `crew_only`   — present in Crew Knowledge, absent from Truth (a
 *                    removed-identity edge: Truth has already dropped it,
 *                    e.g. the entity despawned, but the folded crew replica
 *                    has not been refreshed since).
 *  - `changed`     — present on both sides but at least one compared field
 *                    differs.
 *  - `same`        — present on both sides with every compared field equal.
 *
 * A field entry may be a plain string (compared with strict/array equality)
 * or a `[name, compare]` tuple supplying a field-specific comparator (see
 * `hullPercentSame`) for a field whose two arms use `null` to mean different
 * things than "differs".
 *
 * @param {Array<object>} truthItems
 * @param {Array<object>} crewItems
 * @param {{ fields?: Array<string|[string, (truth: unknown, crew: unknown) => boolean]> }} [options]
 * @returns {{ rows: Array<{id:string,status:string,truth:object|null,crew:object|null}>,
 *             empty: boolean, equal: boolean, different: boolean }}
 */
export function diffByIdentity(truthItems, crewItems, { fields = [] } = {}) {
  const truthMap = new Map((truthItems || []).filter(Boolean).map((item) => [item.id, item]));
  const crewMap = new Map((crewItems || []).filter(Boolean).map((item) => [item.id, item]));
  const ids = new Set([...truthMap.keys(), ...crewMap.keys()]);
  const rows = [...ids].sort().map((id) => {
    const truth = truthMap.get(id) ?? null;
    const crew = crewMap.get(id) ?? null;
    let status;
    if (truth && !crew) status = 'truth_only';
    else if (!truth && crew) status = 'crew_only';
    else {
      status = fields.some((field) => {
        const [name, compare] = Array.isArray(field) ? field : [field, sameValue];
        return !compare(truth[name], crew[name]);
      }) ? 'changed' : 'same';
    }
    return { id, status, truth, crew };
  });
  return {
    rows,
    empty: rows.length === 0,
    equal: rows.length > 0 && rows.every((row) => row.status === 'same'),
    different: rows.some((row) => row.status !== 'same'),
  };
}

/** Truth-side contact rows: point entities only (geometry === null) — a
 * Region/hazard/asteroid-field renders on Sensors as an area, not a point
 * contact, and is out of scope for this identity-keyed comparison.
 *
 * `entity.name` is a raw String Table id exactly as Rust sent it (`gm_entity`
 * is exempt from `localiseHostPayload` — see the module doc comment), so it
 * is resolved through `displayText` (`wireText` by default) before use: the
 * same seam `gui/entity-inspector.js` uses for this exact DTO. `wireText`
 * only substitutes when the value really is a known id, so an already-plain
 * name passes through unchanged and the Crew arm's already-resolved name
 * never falsely reports `changed` against a raw id. */
export function truthContactRows(truthEntities, displayText = wireText) {
  return (truthEntities || [])
    .filter((entity) => entity && entity.geometry === null)
    .map((entity) => ({
      id: entity.entity_id,
      name: displayText(entity.name),
      hull_percent: entity.status.hull_percent,
      destroyed: entity.status.destroyed,
    }));
}

/** Tags Truth's `world_kind` (src/gm_projection.rs) can express for a point
 * contact: `structure`/`station` (an `InfrastructureCondition` component OR a
 * tagged structure/station — `world_kind`'s `infrastructure.is_some()` arm
 * fires on the live ECS component regardless of tags, which is why the raw
 * wire entity's `infrastructure` field is checked directly below rather than
 * re-derived from tags: it is minted by `InfrastructureSnapshot::from_state`
 * and is `Some` whenever the authored `[infrastructure]` table is present
 * with its default `publish = true`, e.g. `assets/entities/stranded_lighter.
 * toml` and `assets/entities/skyway_castaway_lifeboat.toml`, which carry
 * `tags = ["civilian", "infrastructure"]` with no `structure`/`station` tag
 * at all), `ship` (every PlayerShip/NpcShip), or `asteroid` when the entity
 * additionally carries an authored name (`AuthoredAsteroid`). A bare
 * `planet`/`star`/`moon` — or any other radar-shown tag — is outside Truth's
 * vocabulary entirely: `world_kind` returns `None` for it, so Truth never
 * models the identity at all, in either direction.
 *
 * Residual: an entity authored `publish = false` on its `[infrastructure]`
 * table is still `Structure` to Truth (the live component the backend reads
 * is untouched by `publish`) but arrives on the wire with `infrastructure:
 * None` (`InfrastructureSnapshot::from_state` returns `None` for it) and, if
 * it also carries no `structure`/`station` tag, is invisible to this
 * function — the same "Truth knows more than any wire payload can carry" gap
 * every other Truth-only field already has, not something a crew-side check
 * can close (issue #1318 review, finding 1). */
const TRUTH_MODELLED_TAGS = new Set(['structure', 'station', 'ship']);

function truthModelsRawEntity(raw) {
  if (!raw) return false;
  if (raw.infrastructure) return true;
  const tags = (raw.tags || []).map((tag) => String(tag).toLowerCase());
  if (tags.some((tag) => TRUTH_MODELLED_TAGS.has(tag))) return true;
  return tags.includes('asteroid') && !!raw.name;
}

/** Crew-side contact rows: exactly what `buildSensorsConsoleState` currently
 * shows this ship, enriched with the same `hull_fraction` its own consoles
 * already read off the folded entity replica (never a second hull model).
 *
 * Restricted to identities `truthModelsRawEntity` recognises, mirroring the
 * Region/geometry exclusion above: a ship's sensors can show a `planet` or
 * `star` blip that Truth's `world_kind` never classifies at all (`None`, not
 * "dropped"), and admitting it here would misreport `crew_only` — documented
 * to mean "Truth already dropped this identity" — for an identity Truth
 * never modelled in the first place. */
export function crewContactRows(sensorsBlips, rawEntities) {
  const rawById = new Map((rawEntities || []).filter((entity) => entity && entity.uuid)
    .map((entity) => [entity.uuid, entity]));
  return (sensorsBlips || [])
    .filter((blip) => blip && typeof blip.uuid === 'string'
      && !SYNTHETIC_CONTACT_KINDS.has(blip.kind) && !SYNTHETIC_CONTACT_IDS.has(blip.uuid))
    .filter((blip) => truthModelsRawEntity(rawById.get(blip.uuid)))
    .map((blip) => {
      const raw = rawById.get(blip.uuid);
      const hullFraction = raw && typeof raw.hull_fraction === 'number' ? raw.hull_fraction : null;
      return {
        id: blip.uuid,
        name: blip.name,
        // Mirror Rust's arithmetic exactly (src/gm_projection.rs `percent`):
        // `((current / maximum) * 100.0).clamp(...).round()` is computed
        // entirely in f32, while `hullFraction` here is an f32 value carried
        // in a JS f64. Multiplying it by 100 in plain f64 can round to a
        // different integer than the f32 multiply whenever the exact product
        // straddles an x.5 boundary. `Math.fround` re-rounds the (exact,
        // since both operands are f32-representable) f64 product to the
        // nearest f32 — reproducing Rust's f32 multiply bit-for-bit — before
        // `Math.round` agrees with f32 `.round()` for a non-negative value
        // (issue #1318 review, finding 4).
        hull_percent: hullFraction === null ? null : Math.round(Math.fround(hullFraction * 100)),
        // No hull model here means "not destroyed", the same semantics as
        // Truth's `hull.is_some_and(...)` (src/gm_projection.rs) — never
        // `null`, which would falsely diff against Truth's always-boolean
        // `destroyed` field for every hull-less identity (issue #1318 review).
        destroyed: hullFraction === null ? false : hullFraction <= 0,
      };
    });
}

/** Objective rows shared by Truth and Crew Knowledge (see the module doc
 * comment: both read the same unfiltered `ObjectiveSnapshot` list today). */
export function objectiveRows(objectives) {
  return (objectives || []).filter((objective) => objective && typeof objective.id === 'string').map((objective) => ({
    id: objective.id,
    text: objective.text,
    text_params: objective.text_params || {},
    mandatory: !!objective.mandatory,
    status: objective.status,
  }));
}

/** Comms message rows, keyed by the message's own stable id. */
export function commsMessageRows(messages) {
  return (messages || []).filter((message) => message && typeof message.id === 'string').map((message) => ({
    id: message.id,
    sender_name: message.sender_name,
    subject: message.subject,
    is_read: !!message.is_read,
  }));
}

/** Comms dossier/contact rows, keyed by world-entity uuid. */
export function commsContactRows(contacts) {
  return (contacts || []).filter((contact) => contact && typeof contact.uuid === 'string').map((contact) => ({
    id: contact.uuid,
    name: contact.name,
    in_range: contact.in_range !== false,
  }));
}

/**
 * Compose the full Truth / Crew Knowledge / Difference model for one
 * selected ship.
 *
 * @param {Array<object>} truthEntities parsed `parseGmEntityProjection` output
 * @param {{ activity: Array }} projection the parsed `gm_station` payload
 *   (only its `activity` array is read, via `buildGmStationConsoleInput`)
 * @param {object} ship one entry of `projection.ships` (a `GmPuppetShipProjection`)
 * @param {{ displayText?: (value: string, fallback?: string) => string }} [options]
 *   `displayText` resolves a Truth string that may still be a raw String
 *   Table id (see the module doc comment); defaults to `wireText` and exists
 *   as a parameter purely so tests can inject a table-aware stand-in.
 * @returns {{ contacts: object, objectives: object,
 *             comms: { messages: object, contacts: object } }}
 */
export function buildKnowledgeCompare(truthEntities, projection, ship, { displayText = wireText } = {}) {
  if (!ship) {
    const empty = diffByIdentity([], []);
    return { contacts: empty, objectives: empty, comms: { messages: empty, contacts: empty } };
  }
  const state = buildGmStationConsoleInput(projection || { activity: [] }, ship);
  const sensors = JSON.parse(buildSensorsConsoleState(state));
  const comms = JSON.parse(buildCommsConsoleState(state));

  const contacts = diffByIdentity(
    truthContactRows(truthEntities, displayText),
    crewContactRows(sensors.blips, state.asteroids),
    { fields: ['name', ['hull_percent', hullPercentSame], 'destroyed'] },
  );
  // Objectives: Truth reads the ship projection's own unconditional list;
  // Crew Knowledge reads the ordinary folded `ClientSimState.objectives`,
  // which `gui/sim-state.js` stores as the SAME array reference the
  // projection carried (`this.objectives = d.objectives || []`) — this
  // category is tautologically equal today (one array diffed against
  // itself), not merely equal-by-value, and stays that way until a future
  // per-ship objective-visibility feature makes the two arms diverge (issue
  // #1318 review, finding 3).
  const objectives = diffByIdentity(
    objectiveRows(ship.objectives),
    objectiveRows(state.objectives),
    { fields: ['text', 'mandatory', 'status'] },
  );
  // Comms: Truth reads the ship's own raw Comms blackboard entry (the
  // producer's `blackboards` array, sorted by system id — src/gm_projection.rs);
  // Crew Knowledge reads `buildCommsConsoleState`'s output, which falls back
  // to that SAME lexically-first Comms system id when no Station preference
  // is given (`blackboardsOfKind`, gui/console-state.js). With today's single
  // authored Comms system per ship, both arms therefore resolve the
  // identical entry — structurally "equal", not two independent sources
  // diffed against each other — until a per-ship Comms filter (#1063/#1065/
  // #1070) or a Comms Station-preference read lands (issue #1318 review,
  // finding 1).
  const commsEntry = (ship.blackboards || []).find(([, entry]) => entry && entry.kind === 'Comms');
  const commsTruth = commsEntry ? commsEntry[1].data || {} : {};
  const commsMessages = diffByIdentity(
    commsMessageRows(commsTruth.messages),
    commsMessageRows(comms.messages),
    { fields: ['subject', 'is_read'] },
  );
  const commsContacts = diffByIdentity(
    commsContactRows(commsTruth.contacts),
    commsContactRows(comms.contacts),
    { fields: ['name', 'in_range'] },
  );
  return { contacts, objectives, comms: { messages: commsMessages, contacts: commsContacts } };
}

// ── DOM shell ────────────────────────────────────────────────────────────

function describeContact(entry, t) {
  if (!entry) return t('server.gm.knowledge.none');
  if (entry.destroyed) return `${entry.name || entry.id} — ${t('server.gm.entity.destroyed')}`;
  if (entry.hull_percent !== null && entry.hull_percent !== undefined) {
    return `${entry.name || entry.id} — ${t('server.gm.entity.hull', { percent: entry.hull_percent })}`;
  }
  return entry.name || entry.id;
}

/**
 * `entry.text` has already crossed the `gm_station` localisation boundary by
 * the time it reaches `objectiveRows` (see the module doc comment): it is
 * plain resolved English, not a String Table id. Calling `t(entry.text, ...)`
 * on it would re-resolve already-resolved text — `t` on an unknown id warns
 * and renders `⟨...⟩` around every objective. `displayText(value, value)`
 * (the same seam `gui/gm-activity-feed.js` uses for wire text) only
 * substitutes when the value genuinely is a table id and otherwise passes
 * the resolved sentence through unchanged (issue #1318 review).
 */
function describeObjective(entry, t, displayText = wireText) {
  if (!entry) return t('server.gm.knowledge.none');
  const text = displayText(entry.text, entry.text);
  const status = t(`server.gm.activity.objective_status.${String(entry.status).toLowerCase()}`);
  return `${text} — ${status}`;
}

function describeMessage(entry, t) {
  if (!entry) return t('server.gm.knowledge.none');
  return `${entry.sender_name || ''} — ${entry.subject || ''}`;
}

function describeCommsContact(entry, t) {
  if (!entry) return t('server.gm.knowledge.none');
  return entry.name || entry.id;
}

function summaryCounts(rows) {
  const counts = { same: 0, changed: 0, truth_only: 0, crew_only: 0 };
  for (const row of rows) counts[row.status] += 1;
  return counts;
}

function renderCategory({ tbody, table, empty, summary }, diff, describe, doc, t) {
  if (!tbody) return;
  tbody.replaceChildren();
  if (table) table.hidden = diff.empty;
  if (empty) empty.hidden = !diff.empty;
  if (summary) {
    summary.textContent = diff.empty ? '' : t('server.gm.knowledge.summary', summaryCounts(diff.rows));
  }
  for (const row of diff.rows) {
    const tr = doc.createElement('tr');
    tr.dataset.status = row.status;
    tr.dataset.identity = row.id;
    const idCell = doc.createElement('td');
    idCell.textContent = row.id;
    const truthCell = doc.createElement('td');
    truthCell.textContent = describe(row.truth, t);
    const crewCell = doc.createElement('td');
    crewCell.textContent = describe(row.crew, t);
    const statusCell = doc.createElement('td');
    statusCell.textContent = t(`server.gm.knowledge.status.${row.status}`);
    tr.append(idCell, truthCell, crewCell, statusCell);
    tbody.appendChild(tr);
  }
}

/**
 * Stateful GM page controller. Reads already-parsed Truth entities (from
 * `createGmLocalProjection(...).state().entities`) and the already-parsed
 * `gm_station` projection (from `createGmStationPuppet(...).state().projection`)
 * — never re-parses the raw Host Channel payload itself — so this stays a
 * pure consumer of the other two controllers' public state, exactly as
 * `gm-activity-feed.js` reads `gmProjection.contains`/`.select` rather than
 * re-deriving entity identity.
 *
 * Selection here is independent of `gm-station-puppet.js`'s takeover
 * selection (issue #1318 AC #2): choosing a ship to compare never opens its
 * Station iframe or requires takeover.
 */
export function createGmKnowledgeCompare({ doc = document, t = (id) => id, displayText = wireText } = {}) {
  const describeObjectiveBound = (entry, tt) => describeObjective(entry, tt, displayText);
  const pending = doc.getElementById('gm-knowledge-pending');
  const panel = doc.getElementById('gm-knowledge-panel');
  const select = doc.getElementById('gm-knowledge-select');
  const categories = {
    contacts: {
      tbody: doc.getElementById('gm-knowledge-contacts-rows'),
      table: doc.getElementById('gm-knowledge-contacts-table'),
      empty: doc.getElementById('gm-knowledge-contacts-empty'),
      summary: doc.getElementById('gm-knowledge-contacts-summary'),
      describe: describeContact,
    },
    objectives: {
      tbody: doc.getElementById('gm-knowledge-objectives-rows'),
      table: doc.getElementById('gm-knowledge-objectives-table'),
      empty: doc.getElementById('gm-knowledge-objectives-empty'),
      summary: doc.getElementById('gm-knowledge-objectives-summary'),
      describe: describeObjectiveBound,
    },
    'comms-messages': {
      tbody: doc.getElementById('gm-knowledge-comms-messages-rows'),
      table: doc.getElementById('gm-knowledge-comms-messages-table'),
      empty: doc.getElementById('gm-knowledge-comms-messages-empty'),
      summary: doc.getElementById('gm-knowledge-comms-messages-summary'),
      describe: describeMessage,
    },
    'comms-contacts': {
      tbody: doc.getElementById('gm-knowledge-comms-contacts-rows'),
      table: doc.getElementById('gm-knowledge-comms-contacts-table'),
      empty: doc.getElementById('gm-knowledge-comms-contacts-empty'),
      summary: doc.getElementById('gm-knowledge-comms-contacts-summary'),
      describe: describeCommsContact,
    },
  };

  let truthEntities = [];
  let projection = { ships: [], activity: [] };
  let selectedShipId = null;

  function selectedShip() {
    return projection.ships.find((ship) => ship.ship_id === selectedShipId) || null;
  }

  function rebuildOptions() {
    if (!select) return;
    select.replaceChildren(...projection.ships.map((ship) => {
      const option = doc.createElement('option');
      option.value = ship.ship_id;
      option.textContent = ship.name;
      return option;
    }));
    select.value = selectedShipId || '';
  }

  function renderEmptyCategories() {
    // Clear every category tbody explicitly rather than relying on the
    // panel's `hidden` attribute to hide stale rows: a reconnect must never
    // paint even one frame of the previous ship's contacts (issue #1318
    // review) before the freshly rebuilt panel replaces them.
    const empty = diffByIdentity([], []);
    renderCategory(categories.contacts, empty, describeContact, doc, t);
    renderCategory(categories.objectives, empty, describeObjectiveBound, doc, t);
    renderCategory(categories['comms-messages'], empty, describeMessage, doc, t);
    renderCategory(categories['comms-contacts'], empty, describeCommsContact, doc, t);
  }

  function render() {
    const ships = projection.ships;
    if (ships.length === 0) {
      selectedShipId = null;
      if (pending) pending.hidden = false;
      if (panel) panel.hidden = true;
      if (select) select.replaceChildren();
      renderEmptyCategories();
      return;
    }
    if (!ships.some((ship) => ship.ship_id === selectedShipId)) {
      selectedShipId = ships[0].ship_id;
    }
    if (pending) pending.hidden = true;
    if (panel) panel.hidden = false;
    rebuildOptions();
    const compare = buildKnowledgeCompare(truthEntities, projection, selectedShip(), { displayText });
    renderCategory(categories.contacts, compare.contacts, describeContact, doc, t);
    renderCategory(categories.objectives, compare.objectives, describeObjectiveBound, doc, t);
    renderCategory(categories['comms-messages'], compare.comms.messages, describeMessage, doc, t);
    renderCategory(categories['comms-contacts'], compare.comms.contacts, describeCommsContact, doc, t);
  }

  function updateTruth(entities) {
    truthEntities = Array.isArray(entities) ? entities : [];
    render();
    return true;
  }

  function updateStations(nextProjection) {
    projection = (nextProjection && Array.isArray(nextProjection.ships))
      ? nextProjection : { ships: [], activity: [] };
    render();
    return true;
  }

  function selectShip(shipId) {
    if (!projection.ships.some((ship) => ship.ship_id === shipId)) return false;
    selectedShipId = shipId;
    render();
    return true;
  }

  function clear() {
    truthEntities = [];
    projection = { ships: [], activity: [] };
    selectedShipId = null;
    render();
    return true;
  }

  if (select) {
    select.addEventListener('change', () => { selectShip(select.value); });
  }

  return {
    updateTruth,
    updateStations,
    select: selectShip,
    clear,
    state: () => ({
      selectedShipId,
      shipIds: projection.ships.map((ship) => ship.ship_id),
    }),
  };
}
