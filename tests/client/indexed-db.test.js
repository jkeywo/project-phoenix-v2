import { it, expect, vi } from 'vitest';
import { createDatabaseOpener } from '../../editor/indexed-db.js';

it('shares an open, retries a failure, and reopens after version change', async () => {
  const requests = [];
  const indexedDB = { open: vi.fn(() => {
    const request = { result: { createObjectStore: vi.fn(), close: vi.fn() } };
    requests.push(request);
    return request;
  }) };
  const open = createDatabaseOpener({ indexedDB, name: 'draft', store: 'data',
    unavailable: 'unavailable', failed: error => error || Error('failed') });
  const first = open(), shared = open();
  requests[0].onerror();
  await expect(first).rejects.toThrow('failed');
  await expect(shared).rejects.toThrow('failed');
  const next = open();
  requests[1].onupgradeneeded();
  requests[1].onsuccess();
  const db = await next;
  expect(await open()).toBe(db);
  expect(indexedDB.open).toHaveBeenCalledTimes(2);
  expect(db.createObjectStore).toHaveBeenCalledWith('data');
  db.onversionchange();
  expect(db.close).toHaveBeenCalledOnce();
  const reopened = open();
  requests[2].onsuccess();
  expect(await reopened).not.toBe(db);
});

it('keeps a blocked-storage refusal distinct from a request error', async () => {
  const request = { error: Error('request error') };
  const open = createDatabaseOpener({ indexedDB: { open: () => request }, name: 'draft', store: 'data',
    unavailable: 'unavailable', failed: error => error,
    blocked: () => Error('blocked by another window') });
  const pending = open();
  request.onblocked();
  await expect(pending).rejects.toThrow('blocked by another window');
});
