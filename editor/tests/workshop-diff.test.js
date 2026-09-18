import { describe, expect, it } from 'vitest';
import { workspaceDiff, workspaceDiffIsEmpty } from '../workshop-diff.js';
import { WorkshopDocument } from '../workshop-document.js';
import { workshopPack, WORKSHOP_WORLD, WORKSHOP_WORLD_TEXT } from '../../tests/fixtures/workshop-pack.js';

const bytes = values => new Uint8Array(values);

describe('workspaceDiff', () => {
  it('reports added, removed and modified members', () => {
    const diff = workspaceDiff(
      { 'a.toml': 'one', 'b.toml': 'two', 'c.toml': 'three' },
      { 'a.toml': 'one', 'b.toml': 'CHANGED', 'd.toml': 'four' });
    expect(diff.added).toEqual(['d.toml']);
    expect(diff.removed).toEqual(['c.toml']);
    expect(diff.modified).toEqual(['b.toml']);
    expect(diff.unchanged).toEqual(['a.toml']);
    expect(workspaceDiffIsEmpty(diff)).toBe(false);
  });

  it('never normalises unchanged source before comparing', () => {
    // Byte-identical is the whole test: a member whose CRLF, BOM or trailing
    // newline a serializer would have rewritten must still read unchanged,
    // because nobody edited it.
    const source = '﻿[pack]\r\nname = "x" # keep\r\n\r\n';
    const diff = workspaceDiff({ 'scenarios.toml': source }, { 'scenarios.toml': source });
    expect(diff.unchanged).toEqual(['scenarios.toml']);
    expect(workspaceDiffIsEmpty(diff)).toBe(true);
    // And a single byte of difference is a modification, not a rounding error.
    const nudged = workspaceDiff({ 'scenarios.toml': source },
      { 'scenarios.toml': source.replace('\r\n\r\n', '\n\n') });
    expect(nudged.modified).toEqual(['scenarios.toml']);
  });

  it('recognises a rename rather than reporting a removal and an addition', () => {
    const diff = workspaceDiff(
      { 'assets/worlds/old.toml': '[global]\n', 'assets/worlds/keep.toml': 'keep' },
      { 'assets/worlds/new.toml': '[global]\n', 'assets/worlds/keep.toml': 'keep' });
    expect(diff.renamed).toEqual([{ from: 'assets/worlds/old.toml', to: 'assets/worlds/new.toml' }]);
    expect(diff.added).toEqual([]);
    expect(diff.removed).toEqual([]);
  });

  it('pairs binary renames by content and leaves an edited move as both halves', () => {
    const glb = bytes([1, 2, 3, 4]);
    const renamed = workspaceDiff({ 'assets/models/a.glb': glb }, { 'assets/models/b.glb': bytes([1, 2, 3, 4]) });
    expect(renamed.renamed).toEqual([{ from: 'assets/models/a.glb', to: 'assets/models/b.glb' }]);
    // Moved AND changed is not a rename this surface can claim: the bytes that
    // arrived are not the bytes that left, so it reports what it can prove.
    const edited = workspaceDiff({ 'assets/models/a.glb': glb }, { 'assets/models/b.glb': bytes([9, 9]) });
    expect(edited.renamed).toEqual([]);
    expect(edited.removed).toEqual(['assets/models/a.glb']);
    expect(edited.added).toEqual(['assets/models/b.glb']);
  });

  it('matches native asset references by the asset they name, not by bytes', () => {
    // A native draft never holds the bytes, so identity is the stored asset.
    const reference = { asset: '0000000000000001-10', length: 10 };
    const diff = workspaceDiff({ 'assets/models/a.glb': reference },
      { 'assets/models/b.glb': { ...reference } });
    expect(diff.renamed).toEqual([{ from: 'assets/models/a.glb', to: 'assets/models/b.glb' }]);
  });

  it('describes a real draft against what it was imported as', () => {
    const draft = new WorkshopDocument(workshopPack());
    expect(workspaceDiffIsEmpty(workspaceDiff(draft.importedMembers(), draft.members()))).toBe(true);
    draft.edit(WORKSHOP_WORLD, `${WORKSHOP_WORLD_TEXT}# added\n`);
    draft.put('assets/worlds/extra.toml', '[global]\n');
    expect(workspaceDiff(draft.importedMembers(), draft.members())).toMatchObject({
      modified: [WORKSHOP_WORLD], added: ['assets/worlds/extra.toml'], removed: [], renamed: [],
    });
    draft.rename('assets/worlds/extra.toml', 'assets/worlds/moved.toml');
    expect(workspaceDiff(draft.importedMembers(), draft.members()).renamed).toEqual([]);
    // The moved member was never in the imported source, so against THAT
    // baseline it is simply an addition under its current name.
    expect(workspaceDiff(draft.importedMembers(), draft.members()).added)
      .toEqual(['assets/worlds/moved.toml']);
  });
});
