// @vitest-environment jsdom
import {describe, it, expect} from 'vitest';
import {renderScenarioStatePanel} from '../../gui/scenario-state-panel.js';

describe('actual bound activity author inspection', () => {
  it('keeps pending, achieved, settled and refused activation distinct without inferring success', () => {
    const container = document.createElement('div');
    const lifecycle = {definition:'restore', request:{kind:'repair',actor:'ship',owner:'repair',team:0},
      armed_tick:2, accepted:{actor:'actual-ship',owner:'repair',job:7},accepted_tick:6,
      achieved:{tick:14},settled:null,state:'return',activations:{work:'applied'}};
    const payload = {flags:[],triggers:[],situations:[{id:'<bound-job>',lifecycle,pending:[{tick:16,outcome:{kind:'repair_settled'}}]}]};
    renderScenarioStatePanel(container,JSON.stringify(payload));
    expect(container.querySelector('.ss-empty')).toBeNull();
    expect(container.querySelector('.ss-situation-id').textContent).toBe('<bound-job>');
    expect(container.querySelector('bound-job')).toBeNull();
    expect(container.querySelector('.ss-situation-job').textContent).toContain('7');
    expect(container.querySelector('.ss-situation-achieved').textContent).toContain('14');
    expect(container.querySelector('.ss-situation-settled').textContent).not.toContain('16');
    expect(container.querySelector('.ss-situation-pending').textContent).toContain('16');
    lifecycle.settled={tick:16,outcome:{kind:'repair_settled',outcome:'Cancelled'}};
    lifecycle.activations.return='refused';lifecycle.state=null;payload.situations[0].pending=[];
    renderScenarioStatePanel(container,JSON.stringify(payload));
    expect(container.querySelector('.ss-situation-settled').textContent).toContain('16');
    expect(container.querySelector('.ss-situation-outcome').textContent).toContain('Cancelled');
    expect([...container.querySelectorAll('.ss-situation-activation')].map(row=>row.textContent)).toEqual(['work: applied','return: refused']);
    expect(container.querySelector('.ss-situation-pending')).toBeNull();
    expect(container.querySelector('button')).toBeNull();
  });
});
