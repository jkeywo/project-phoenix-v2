/**
 * tests/client/setup-strings.js — vitest setup: load the real string table.
 *
 * gui/strings-boot.js is a browser-only no-op (Node's fetch cannot read
 * file:// URLs), so tests load assets/strings/strings.csv from disk instead.
 * Component tests then assert via t('some.id') and stay correct through both
 * the [bracket] placeholder phase and any future copy edit — the id, not the
 * English, is the contract.
 */

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildTable, setTable } from '../../gui/strings.js';

const csvPath = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  '../../assets/strings/strings.csv',
);

setTable(buildTable(readFileSync(csvPath, 'utf8')));
