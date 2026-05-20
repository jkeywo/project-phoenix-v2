/**
 * stations-editor.js
 *
 * Pure data-model editor for the [[stations]] block in entity TOML.
 *
 * The [stations] block defines per-player-count station presets for
 * assigning consoles to crew members.  This editor reads/writes the
 * raw TOML representation (console names as strings).
 *
 * No DOM manipulation; fully testable in Node.
 */

import { validateStations } from './stations-validate.js';

function cloneStation(s) {
  const out = {
    name: s.name,
    description: s.description,
    consoles: [...s.consoles],
    rank: s.rank,
    short_code: s.short_code,
  };
  if (s.next !== undefined) out.next = s.next;
  if (s.previous !== undefined) out.previous = s.previous;
  return out;
}

export class StationsEditor {
  constructor() {
    this._minPlayers = 1;
    this._maxPlayers = 1;
    this._configs = {};
  }

  load(config) {
    if (!config || typeof config !== 'object') {
      this._minPlayers = 1;
      this._maxPlayers = 1;
      this._configs = {};
      return;
    }

    this._minPlayers = Number.isFinite(Number(config.min_players))
      ? Number(config.min_players)
      : 1;
    this._maxPlayers = Number.isFinite(Number(config.max_players))
      ? Number(config.max_players)
      : 1;
    this._configs = {};

    for (const key of Object.keys(config)) {
      if (key === 'min_players' || key === 'max_players' || key === 'complexity_presets') continue;
      const count = Number(key);
      if (!Number.isInteger(count) || isNaN(count)) continue;
      if (!Array.isArray(config[key])) continue;

      this._configs[count] = config[key]
        .filter((d) => d && typeof d === 'object')
        .map((d) => ({
          name: String(d.name || ''),
          description: String(d.description || ''),
          consoles: Array.isArray(d.consoles) ? [...d.consoles] : [],
          rank: String(d.rank || ''),
          short_code: String(d.short_code || ''),
          ...(d.next !== undefined ? { next: String(d.next) } : {}),
          ...(d.previous !== undefined ? { previous: String(d.previous) } : {}),
        }));
    }
  }

  getMinPlayers() {
    return this._minPlayers;
  }

  getMaxPlayers() {
    return this._maxPlayers;
  }

  getCounts() {
    const counts = [];
    for (let c = this._minPlayers; c <= this._maxPlayers; c++) {
      counts.push(c);
    }
    return counts;
  }

  getStations(count) {
    const defs = this._configs[count];
    if (!defs) return [];
    return defs.map(cloneStation);
  }

  addStation(count, name, consoles, rank, short_code, description) {
    if (!this._configs[count]) {
      this._configs[count] = [];
    }
    this._configs[count].push({
      name: String(name || ''),
      description: String(description || ''),
      consoles: Array.isArray(consoles) ? [...consoles] : [],
      rank: String(rank || ''),
      short_code: String(short_code || ''),
    });
  }

  removeStation(count, name) {
    const defs = this._configs[count];
    if (!defs) return;
    this._configs[count] = defs.filter((d) => d.name !== name);
  }

  updateStation(count, name, changes) {
    const defs = this._configs[count];
    if (!defs) return;
    const station = defs.find((d) => d.name === name);
    if (!station) return;

    if (changes.description !== undefined) station.description = String(changes.description);
    if (changes.consoles !== undefined) station.consoles = [...changes.consoles];
    if (changes.rank !== undefined) station.rank = String(changes.rank);
    if (changes.short_code !== undefined) station.short_code = String(changes.short_code);
    if (changes.next !== undefined) {
      if (changes.next === null || changes.next === '') {
        delete station.next;
      } else {
        station.next = String(changes.next);
      }
    }
    if (changes.previous !== undefined) {
      if (changes.previous === null || changes.previous === '') {
        delete station.previous;
      } else {
        station.previous = String(changes.previous);
      }
    }
  }

  getNextOptions(count) {
    const nextCount = count + 1;
    const defs = this._configs[nextCount];
    if (!defs) return [];
    return defs.map((d) => d.name);
  }

  getPreviousOptions(count) {
    const prevCount = count - 1;
    const defs = this._configs[prevCount];
    if (!defs) return [];
    return defs.map((d) => d.name);
  }

  toStations() {
    const out = {
      min_players: this._minPlayers,
      max_players: this._maxPlayers,
    };

    const counts = Object.keys(this._configs)
      .map(Number)
      .sort((a, b) => a - b);

    for (const count of counts) {
      const defs = this._configs[count];
      if (!defs || defs.length === 0) continue;
      out[count] = defs.map(cloneStation);
    }

    return out;
  }

  validate() {
    return validateStations(this.toStations());
  }

  getCountInfo() {
    return {
      min: this._minPlayers,
      max: this._maxPlayers,
      counts: this.getCounts(),
    };
  }
}
