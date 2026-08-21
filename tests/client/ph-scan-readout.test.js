// @vitest-environment jsdom
import { t } from '../../gui/strings.js';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { formatCondition, formatTolerance } from '../../gui/components/ph-scan-readout.js';
import '../../gui/components/ph-scan-readout.js';

function setup() {
  document.body.innerHTML = '<ph-scan-readout id="test-panel"></ph-scan-readout>';
  return document.getElementById('test-panel');
}

const DETAILED = {
  subject_uuid: '00000000-0000-8000-8000-000000000042',
  subject_name: 'world.entity.skyhook_depot.name',
  band: 'detailed',
  band_label: 'entity.alliance_destroyer.scan.band.detailed.label',
  taken_at_tick: 900,
  condition_fraction: 0.31,
  condition_step: 0.01,
  mass: 250000,
  flags: [['world.skyhook.transfer.label', false]],
  capacities: [['world.skyhook.berths.label', 4]],
};

const COARSE = {
  ...DETAILED,
  band: 'coarse',
  band_label: 'entity.alliance_destroyer.scan.band.coarse.label',
  condition_fraction: 0.25,
  condition_step: 0.25,
  capacities: [],
};

function panelState(overrides = {}) {
  return {
    scan: { capable: true, reading: DETAILED, refusal: null },
    target_uuid: '00000000-0000-8000-8000-000000000042',
    ...overrides,
  };
}

function rows(el) {
  return [...el.shadowRoot.querySelectorAll('.row')].map((row) => [
    row.querySelector('.label').textContent,
    row.querySelector('.value').textContent,
  ]);
}

function button(el) {
  return el.shadowRoot.getElementById('action');
}

describe('PhScanReadout', () => {
  beforeEach(() => { document.body.innerHTML = ''; });
  afterEach(() => { document.body.innerHTML = ''; });

  it('is defined and registered as a custom element', () => {
    expect(customElements.get('ph-scan-readout')).toBeDefined();
  });

  it('tells a hull with no survey suite that it has none', () => {
    // Distinct from "fitted and nothing read yet": an empty box would leave the
    // crew wondering whether the panel was broken.
    const el = setup();
    el.state = { scan: { capable: false, reading: null, refusal: null } };
    expect(el.shadowRoot.querySelector('.empty').textContent)
      .toBe(t('component.scan.none'));
    expect(button(el).hidden).toBe(true);
  });

  it('renders a fitted suite that has read nothing yet, with a live button', () => {
    const el = setup();
    el.state = panelState({ scan: { capable: true, reading: null, refusal: null } });
    expect(el.shadowRoot.querySelector('.empty').textContent)
      .toBe(t('component.scan.idle'));
    expect(button(el).hidden).toBe(false);
    expect(button(el).disabled).toBe(false);
  });

  it('disables the button when nothing is selected to read', () => {
    const el = setup();
    el.state = panelState({ target_uuid: null });
    expect(button(el).disabled).toBe(true);
    expect(el.action).toBe(null);
  });

  // The whole slice, from the panel's side: every line on screen is a quantity
  // the server derived plus a label an author wrote against that quantity. There
  // is no scan-result string anywhere in the payload and none in this file.
  it('renders the reading as labelled quantities and nothing else', () => {
    const el = setup();
    el.state = panelState();
    expect(el.shadowRoot.querySelector('.subject .name').textContent)
      .toBe(t('world.entity.skyhook_depot.name'));
    expect(el.shadowRoot.querySelector('.subject .band').textContent)
      .toBe(t('entity.alliance_destroyer.scan.band.detailed.label'));
    expect(rows(el)).toEqual([
      [t('component.scan.condition'), '31%'],
      [t('component.scan.mass'), '250000'],
      [t('world.skyhook.transfer.label'), t('component.scan.flag.down')],
      [t('world.skyhook.berths.label'), '4'],
    ]);
  });

  // A component with no per-structure branch: a capacity nobody has ever heard
  // of renders exactly like one that ships today.
  it('renders a structure it has never heard of with no change to this file', () => {
    const el = setup();
    el.state = panelState({
      scan: {
        capable: true,
        reading: {
          ...DETAILED,
          flags: [['world.invented.pylon_intact.label', true]],
          capacities: [['world.invented.souls.label', 120]],
        },
        refusal: null,
      },
    });
    expect(rows(el)).toEqual([
      [t('component.scan.condition'), '31%'],
      [t('component.scan.mass'), '250000'],
      [t('world.invented.pylon_intact.label'), t('component.scan.flag.held')],
      [t('world.invented.souls.label'), '120'],
    ]);
  });

  // The fidelity model, on screen. A coarse band says how precise it is and
  // stops claiming to count what it did not resolve.
  it('states the tolerance of a coarse reading and drops the rows it did not resolve', () => {
    const el = setup();
    el.state = panelState({ scan: { capable: true, reading: COARSE, refusal: null } });
    const condition = rows(el)[0];
    expect(condition[1]).toBe('25%±13%');
    expect(rows(el)).toHaveLength(3, 'condition, mass and the flag; no capacity row');
  });

  // Mass (issue #1154) is content identity, not a live measurement: a coarse
  // band still reports the exact same number a detailed one would, with no
  // rounding and no tolerance beside it.
  it('reports mass unrounded and without a tolerance even at a coarse band', () => {
    const el = setup();
    el.state = panelState({ scan: { capable: true, reading: COARSE, refusal: null } });
    const mass = rows(el)[1];
    expect(mass).toEqual([t('component.scan.mass'), '250000']);
  });

  it('shows the refusal reason and no stale reading', () => {
    const el = setup();
    el.state = panelState({
      scan: { capable: true, reading: null, refusal: 'scan.refusal.out_of_range' },
    });
    const reason = el.shadowRoot.getElementById('reason');
    expect(reason.hidden).toBe(false);
    expect(reason.textContent).toBe(t('scan.refusal.out_of_range'));
    expect(el.shadowRoot.querySelector('.subject')).toBe(null);
  });

  it('sends scan_target for the selected contact when the button is pressed', () => {
    const el = setup();
    const sent = [];
    el.sendAction = (name, payload) => sent.push([name, payload]);
    el.state = panelState();
    button(el).click();
    expect(sent).toEqual([
      ['scan_target', { uuid: '00000000-0000-8000-8000-000000000042' }],
    ]);
  });

  it('sends nothing when there is no contact selected', () => {
    const el = setup();
    const sent = [];
    el.sendAction = (name, payload) => sent.push([name, payload]);
    el.state = panelState({ target_uuid: null });
    button(el).click();
    expect(sent).toEqual([]);
  });
});

describe('scan readout formatting', () => {
  it('renders whole percent at whole-percent fidelity', () => {
    expect(formatCondition(0.315, 0.01)).toBe('32%');
    expect(formatCondition(0.25, 0.25)).toBe('25%');
    expect(formatCondition(1, 0.01)).toBe('100%');
  });

  it('clamps a value outside the track rather than rendering nonsense', () => {
    expect(formatCondition(-0.5, 0.01)).toBe('0%');
    expect(formatCondition(4, 0.01)).toBe('100%');
    expect(formatCondition(undefined, 0.01)).toBe('0%');
  });

  it('states a tolerance only where the band is coarse enough for it to matter', () => {
    expect(formatTolerance(0.25)).toBe('±13%');
    expect(formatTolerance(0.05)).toBe('±3%');
    expect(formatTolerance(0.01)).toBe(null);
  });
});
