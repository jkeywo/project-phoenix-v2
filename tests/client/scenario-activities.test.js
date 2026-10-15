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


describe('privileged imported plan inspection',()=>{
  it('keeps original chosen/refused/released metadata separate from live job evidence and escapes source text',()=>{
    const container=document.createElement('div');
    const planning={source:{task:'maintenance::care'},choice:{chosen_tick:4,actor_uuid:'actual',methods:{root:{task:{library_path:'assets/worlds/lib.toml',task:'care'},method:'<chosen>'}},refused_methods:['<unavailable third team>']},
      leaves:[{node:'root/repair',instance:'htn/care/root/repair',released_tick:5},{node:'root/supply',instance:'htn/care/root/supply',released_tick:null}],
      dependencies:[{from:'root/repair',to:'root/supply',when:{kind:'achieved'}}],derivation_error:null};
    renderScenarioStatePanel(container,JSON.stringify({flags:[],triggers:[],planning,situations:[]}));
    expect(container.querySelector('.ss-empty')).toBeNull();
    expect(container.querySelector('.ss-plan-chosen').textContent).toContain('4');
    expect(container.querySelector('.ss-plan-method').textContent).toContain('<chosen>');
    expect(container.querySelector('chosen')).toBeNull();
    expect(container.querySelector('.ss-plan-refused').textContent).toContain('<unavailable third team>');
    expect(container.querySelector('unavailable')).toBeNull();
    expect(container.querySelectorAll('.ss-plan-released')[0].textContent).toContain('5');
    expect(container.querySelectorAll('.ss-plan-released')[1].textContent).not.toContain('5');
    expect(container.querySelector('.ss-plan-dependency').textContent).toContain('achieved');
    expect(container.querySelector('button,input,select')).toBeNull();
    planning.choice=null;planning.leaves=[];planning.dependencies=[];
    renderScenarioStatePanel(container,JSON.stringify({flags:[],triggers:[],planning}));
    expect(container.querySelector('.ss-plan-root').textContent).toContain('maintenance::care');
    expect(container.querySelector('.ss-plan-method')).toBeNull();
  });
});
