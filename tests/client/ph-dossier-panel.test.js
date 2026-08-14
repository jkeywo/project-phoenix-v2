// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { formatValue, PROVENANCE_LABELS } from '../../gui/components/ph-dossier-panel.js';
import '../../gui/components/ph-dossier-panel.js';

function setup() {
  document.body.innerHTML = '<ph-dossier-panel id="test-panel"></ph-dossier-panel>';
  return document.getElementById('test-panel');
}

function subjects(host) {
  return [...host.shadowRoot.querySelectorAll('.subject')].map((row) => ({
    uuid: row.dataset.uuid,
    name: row.querySelector('.name').textContent,
    count: row.querySelector('.count').textContent,
  }));
}

function facts(host) {
  return [...host.shadowRoot.querySelectorAll('.fact')].map((row) => [
    row.querySelector('.label').textContent,
    row.querySelector('.value').textContent,
  ]);
}

const DEPOT = {
  uuid: 'skyway_depot-1',
  name: 'world.probe_dossier.entity.skyway_depot.name',
  summary: 'entity.depot_transfer.target.description',
  facts: [
    { label: 'dossier.fact.condition', value: { kind: 'fraction', value: 0.42 } },
    { label: 'dossier.fact.commitment_open', value: { kind: 'text', value: 'world.probe_dossier.commitment.safe_passage.terms' } },
  ],
  evidence: [],
};

const CLAIMANT = {
  uuid: 'claimant-1',
  name: 'world.probe_dossier.entity.strike_committee.name',
  summary: '',
  facts: [],
  evidence: [],
};

describe('PhDossierPanel', () => {
  beforeEach(() => { document.body.innerHTML = ''; });
  afterEach(() => { document.body.innerHTML = ''; });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-dossier-panel')).toBeDefined();
  });

  it('renders the empty placeholder for no files, a null state, and a null list', () => {
    const el = setup();
    for (const state of [{ dossiers: [] }, null, { dossiers: null }]) {
      el.state = state;
      expect(el.shadowRoot.querySelector('.empty').textContent)
        .toBe(t('component.dossier.empty'));
      expect(subjects(el)).toEqual([]);
    }
  });

  it('lists every subject by resolved name, in the order the host sent them', () => {
    const el = setup();
    el.state = { dossiers: [CLAIMANT, DEPOT] };
    expect(subjects(el)).toEqual([
      { uuid: 'claimant-1', name: t(CLAIMANT.name), count: t('component.dossier.nothing_on_file') },
      { uuid: 'skyway_depot-1', name: t(DEPOT.name), count: '2' },
    ]);
  });

  // "Empty, not missing" is the whole distinction the projection makes, and the
  // list is where a crew sees it: a subject with nothing on file is still a row,
  // and the row says so rather than showing a bare name.
  it('shows a subject with no known facts as present and explicitly empty', () => {
    const el = setup();
    el.state = { dossiers: [CLAIMANT] };
    expect(subjects(el)).toHaveLength(1);
    el.select('claimant-1');
    expect(el.shadowRoot.querySelector('.sheet .empty').textContent)
      .toBe(t('component.dossier.nothing_on_file'));
    expect(facts(el)).toEqual([]);
  });

  it('opens the fact sheet on tap and renders each typed value as itself', () => {
    const el = setup();
    el.state = { dossiers: [DEPOT] };
    el.shadowRoot.querySelector('.subject').click();
    expect(el.shadowRoot.querySelector('.sheet .name').textContent).toBe(t(DEPOT.name));
    expect(el.shadowRoot.querySelector('.sheet .summary').textContent).toBe(t(DEPOT.summary));
    expect(facts(el)).toEqual([
      [t('dossier.fact.condition'), '42%'],
      [t('dossier.fact.commitment_open'), t('world.probe_dossier.commitment.safe_passage.terms')],
    ]);
  });

  it('goes back to the list from the sheet without closing the overlay', () => {
    const el = setup();
    el.state = { dossiers: [DEPOT] };
    el.shadowRoot.querySelector('.subject').click();
    expect(el.open.uuid).toBe('skyway_depot-1');
    el.shadowRoot.getElementById('dossier-back').click();
    expect(el.open).toBeNull();
    expect(subjects(el)).toHaveLength(1);
  });

  // The open sheet is resolved by uuid on every render, so a live world reaches
  // it: the facts move under the operator rather than freezing at the tap.
  it('re-renders the open sheet from the newest payload', () => {
    const el = setup();
    el.state = { dossiers: [DEPOT] };
    el.select('skyway_depot-1');
    el.state = {
      dossiers: [{
        ...DEPOT,
        facts: [{ label: 'dossier.fact.condition', value: { kind: 'fraction', value: 0.1 } }],
      }],
    };
    expect(facts(el)).toEqual([[t('dossier.fact.condition'), '10%']]);
  });

  it('drops back to the list when the open subject leaves the world', () => {
    const el = setup();
    el.state = { dossiers: [DEPOT, CLAIMANT] };
    el.select('skyway_depot-1');
    el.state = { dossiers: [CLAIMANT] };
    expect(el.open).toBeNull();
    expect(subjects(el)).toEqual([
      { uuid: 'claimant-1', name: t(CLAIMANT.name), count: t('component.dossier.nothing_on_file') },
    ]);
  });

  // Issue #1031's seam, exercised from this slice: an appended entry renders in
  // its own block with its provenance, and the block is ABSENT — not empty —
  // until there is something in it.
  it('separates gathered evidence from the baseline facts, and hides the block when empty', () => {
    const el = setup();
    el.state = { dossiers: [DEPOT] };
    el.select('skyway_depot-1');
    expect(el.shadowRoot.querySelector('.gathered')).toBeNull();

    el.state = {
      dossiers: [{
        ...DEPOT,
        evidence: [{
          text: 'world.probe_dossier.commitment.safe_passage.resolves',
          provenance: 'scan',
          gathered_at_tick: 900,
        }],
      }],
    };
    const entry = el.shadowRoot.querySelector('.gathered .entry');
    expect(el.shadowRoot.querySelector('.gathered .heading').textContent)
      .toBe(t('component.dossier.gathered'));
    expect(entry.querySelector('.text').textContent)
      .toBe(t('world.probe_dossier.commitment.safe_passage.resolves'));
    expect(entry.querySelector('.provenance').textContent)
      .toBe(t('component.dossier.provenance.scan'));
  });

  it('falls back to the uuid when a subject carries no name id', () => {
    const el = setup();
    el.state = { dossiers: [{ ...CLAIMANT, name: '' }] };
    expect(subjects(el)[0].name).toBe('claimant-1');
  });

  describe('formatValue', () => {
    it('renders each tagged kind as itself, and an unknown kind as nothing', () => {
      expect(formatValue({ kind: 'text', value: 'component.dossier.empty' }))
        .toBe(t('component.dossier.empty'));
      expect(formatValue({ kind: 'fraction', value: 0.425 })).toBe('43%');
      expect(formatValue({ kind: 'count', value: 4 })).toBe('4');
      expect(formatValue({ kind: 'flag', value: true })).toBe(t('component.dossier.yes'));
      expect(formatValue({ kind: 'flag', value: false })).toBe(t('component.dossier.no'));
      // A kind this client does not know renders as nothing rather than as
      // "[object Object]" — the failure an untagged pre-formatted value hides.
      expect(formatValue({ kind: 'cloaking_status', value: 1 })).toBe('');
      expect(formatValue(null)).toBe('');
    });

    it('clamps a fraction the host could not have sent but a bug could', () => {
      expect(formatValue({ kind: 'fraction', value: 1.4 })).toBe('100%');
      expect(formatValue({ kind: 'fraction', value: -0.2 })).toBe('0%');
    });
  });

  describe('PROVENANCE_LABELS', () => {
    it('maps every provenance issue #1031 names to a string id that resolves', () => {
      for (const code of ['scan', 'dialogue', 'records', 'briefing']) {
        expect(t(PROVENANCE_LABELS[code])).not.toContain('⟨');
      }
    });
  });
});
