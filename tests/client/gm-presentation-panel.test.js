// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createGmPresentationPanel } from '../../gui/gm-presentation-panel.js';
import { createPresentationCard } from '../../gui/presentation-card.js';

beforeEach(() => { document.body.innerHTML = '<section id="gm-mission-panel"></section>'; });
const t = id => id.split('.').at(-1);
function setup() {
  const submit = vi.fn(() => true), scheduled = [];
  const panel = createGmPresentationPanel({ t, getOperator: () => ({ id: 'Ada' }), submit,
    correlation: () => 'cue-1', schedule: fn => { scheduled.push(fn); return 1; }, cancelSchedule: vi.fn() });
  panel.update({ entities: [{ kind: 'player_ship', entity_id: 'ship-a', name: 'Horizon' }], presentation_cameras: { 'ship-a': ['camera_fore'] } });
  document.getElementById('gm-presentation-duration').value = '120';
  const click = label => [...document.querySelectorAll('button')].find(b => b.textContent === label).click();
  return { panel, submit, click, scheduled };
}
describe('shared presentation GM controls', () => {
  it('submits typed absolute cues once and settles only the matching canonical result', () => {
    const { panel, submit, click } = setup();
    document.getElementById('gm-presentation-view').value = 'sensors_radar';
    click('force'); click('force');
    expect(submit).toHaveBeenCalledTimes(1);
    expect(submit).toHaveBeenCalledWith({ operator_id: 'Ada', correlation: 'cue-1', ship: 'ship-a',
      cue: { force_view: { view: 'sensors_radar', duration_ticks: 120 } } });
    panel.update({ entities: [{ kind: 'player_ship', entity_id: 'ship-a', name: 'Horizon' }],
      presentation_results: [{ operator_id: 'someone-else', correlation: 'cue-1', outcome: 'applied' }] });
    expect(panel.state().pending).not.toBeNull();
    panel.update({ entities: [{ kind: 'player_ship', entity_id: 'ship-a', name: 'Horizon' }],
      presentation_results: [{ operator_id: 'Ada', correlation: 'cue-1', outcome: 'applied' }] });
    expect(panel.state().pending).toBeNull();
    expect(document.querySelector('[role=status]').textContent).toBe('applied');
  });
  it('refuses invalid durations before dispatch and reports missed results without replay', () => {
    const { panel, submit, click, scheduled } = setup();
    document.getElementById('gm-presentation-duration').value = '-1'; click('force');
    expect(submit).not.toHaveBeenCalled();
    document.getElementById('gm-presentation-duration').value = '120'; click('force'); scheduled[0]();
    expect(panel.state().pending).toBeNull(); expect(submit).toHaveBeenCalledTimes(1);
    expect(document.querySelector('[role=status]').textContent).toBe('timed_out');
  });
  it('retains focused dropdown options across unchanged live snapshots', () => {
    const { panel } = setup(); const option = document.querySelector('#gm-presentation-ship option');
    panel.update({ entities: [{ kind: 'player_ship', entity_id: 'ship-a', name: 'Horizon' }] });
    expect(document.querySelector('#gm-presentation-ship option')).toBe(option);
    panel.reset(); expect([...document.querySelectorAll('button')].every(b => b.disabled)).toBe(true);
  });
  it('keeps a transport refusal distinct from invalid input and uses only eligible messages', () => {
    const panel = createGmPresentationPanel({ t, getOperator: () => ({ id: 'gm' }), submit: () => false });
    panel.update({ entities: [{ kind: 'player_ship', entity_id: 'a', name: 'Alpha' }],
      presentation_messages: [{ message: 'private-b', sender: 'Secret', ship: 'b' }, { message: 'allowed', sender: 'Lyra', ship: 'a' }],
      presentation_cameras: { a: ['camera_aft'] } });
    expect([...document.querySelectorAll('#gm-presentation-message option')].map(o => o.value)).toEqual(['allowed']);
    expect([...document.querySelectorAll('#gm-presentation-camera option')].map(o => o.value)).toEqual(['camera_aft']);
    document.getElementById('gm-presentation-duration').value = '10';
    [...document.querySelectorAll('button')].find(b => b.textContent === 'force').click();
    expect(document.querySelector('[role=status]').textContent).toBe('refused');
    expect(panel.state().pending).toBeNull();
  });
  it('never retargets a removed ship, camera or incoming message during live refresh', () => {
    const { panel, submit, click } = setup();
    const entities = [{ kind: 'player_ship', entity_id: 'ship-a', name: 'Horizon' }];
    panel.update({ entities, presentation_cameras: { 'ship-a': ['camera_fore'] },
      presentation_messages: [{ message: 'one', sender: 'Lyra', ship: 'ship-a' }] });
    panel.update({ entities, presentation_cameras: { 'ship-a': ['camera_aft'] },
      presentation_messages: [{ message: 'two', sender: 'Lyra', ship: 'ship-a' }] });
    expect(document.getElementById('gm-presentation-camera').value).toBe('camera_fore');
    expect(document.getElementById('gm-presentation-message').value).toBe('one');
    click('force'); click('incoming'); expect(submit).not.toHaveBeenCalled();
    panel.update({ entities: [{ kind: 'player_ship', entity_id: 'ship-b', name: 'New ship' }] });
    expect(document.getElementById('gm-presentation-ship').value).toBe('ship-a');
    click('release'); expect(submit).not.toHaveBeenCalled();
    panel.reset();
    panel.update({ entities, presentation_cameras: { 'ship-a': ['camera_aft'] } });
    expect(document.getElementById('gm-presentation-ship').value).toBe('ship-a');
    expect(document.getElementById('gm-presentation-camera').value).toBe('camera_aft');
    click('force'); expect(submit).toHaveBeenCalledOnce();
  });
});
describe('shared live Viewscreen card', () => {
  it('renders untrusted text without markup and clears it on the next absolute empty state', () => {
    const view = createPresentationCard();
    view.update({ kind: 'title', title: '<img src=x onerror=alert(1)>', body: 'Scene two', literal_title: true, literal_body: true });
    const card = document.querySelector('.vs-presentation-card');
    expect(card.hidden).toBe(false); expect(card.querySelector('img')).toBeNull();
    expect(card.querySelector('h1').textContent).toContain('<img');
    view.update(null); expect(card.hidden).toBe(true); expect(card.textContent).toBe('');
    view.destroy(); expect(document.querySelector('.vs-presentation-card')).toBeNull();
  });
  it('has a named keyboard-readable region and no replay/history affordance', () => {
    const view = createPresentationCard();
    view.update({ kind: 'incoming', title: 'Lyra', body: 'Proceed to dock' });
    expect(document.querySelector('[role=region]').getAttribute('aria-label')).toBe('Lyra');
    expect(document.querySelector('[role=region]').querySelector('button')).toBeNull();
    view.update({ kind: 'unknown', title: 'bad', body: 'bad' });
    expect(document.querySelector('[role=region]').hidden).toBe(true);
  });
});
