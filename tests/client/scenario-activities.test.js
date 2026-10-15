// @vitest-environment jsdom
import {describe, it, expect} from 'vitest';
import {renderScenarioStatePanel} from '../../gui/scenario-state-panel.js';

describe('actual bound activity author inspection', () => {
  it('keeps pending, achieved, settled and refused activation distinct without inferring success', () => {
    const container = document.createElement('div');
    const lifecycle = {definition:'restore', request:{kind:'repair',actor:'ship',owner:'repair',team:0},
      armed_tick:2, accepted:{actor:'actual-ship',owner:'repair',identity:{kind:'physical',job:7}},accepted_tick:6,
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

describe('retained temporal decisions',()=>{
  it('renders committed intervals, original participation and exact workload without changing live evidence',()=>{
    const container=document.createElement('div');
    const temporal={profile_index:1,deadline_tick:100,input:{owners:{repair:{source:'human',station:'engineering',rating:'Std'},logistics:{source:'ai'}}},outcome:{state:'committed',nodes:{'root/work':{start_tick:10,end_tick:80}}}};
    const planning={source:{task:'maintenance::maintain'},choice:{chosen_tick:10,temporal},observed_tick:101,workload:[{station:'engineering',minute:0,attention_nanos:'99999999999999999'}]};
    renderScenarioStatePanel(container,JSON.stringify({flags:[],triggers:[],planning,situations:[]}));
    expect(container.querySelector('.ss-plan-schedule').textContent).toContain('committed');
    expect(container.querySelector('.ss-plan-deadline').textContent).toContain('100');
    expect(container.querySelector('.ss-plan-interval').textContent).toContain('10 → 80');
    expect(container.querySelector('.ss-plan-operator').textContent).toContain('engineering');
    expect(container.querySelector('.ss-plan-workload').textContent).toContain('99999999999999999');
    expect(container.querySelector('.ss-plan-overdue').textContent).toContain('101');
    expect(container.querySelector('.ss-situation-job')).toBeNull();
    expect(container.querySelector('button,input,select')).toBeNull();
    temporal.outcome={state:'refused',reason:'<unavailable owner>'};
    planning.workload=[];
    renderScenarioStatePanel(container,JSON.stringify({flags:[],triggers:[],planning,situations:[]}));
    expect(container.querySelector('.ss-plan-schedule_refused').textContent).toContain('<unavailable owner>');
    expect(container.querySelector('unavailable')).toBeNull();
    expect(container.querySelector('.ss-plan-interval,.ss-plan-overdue')).toBeNull();
  });
});

it('shows the dated reconsideration while retaining the original refused decision',()=>{
  const container=document.createElement('div');
  const temporal={profile_index:0,deadline_tick:300,input:{owners:{repair:{source:'unavailable',reason:'unoccupied'}}},outcome:{state:'refused',reason:'unavailable'}};
  const decision={...temporal,input:{owners:{repair:{source:'ai'}}},outcome:{state:'committed',nodes:{repair:{start_tick:30,end_tick:90}}}};
  const planning={source:{task:'maintenance'},choice:{chosen_tick:10,temporal,temporal_reconsiderations:[{tick:30,cause:{kind:'crew_control_changed',owners:['repair']},decision}]}};
  renderScenarioStatePanel(container,JSON.stringify({flags:[],triggers:[],planning,situations:[]}));
  expect(container.querySelector('.ss-plan-original_schedule').textContent).toContain('refused');
  expect(container.querySelector('.ss-plan-reconsideration').textContent).toContain('30: crew_control_changed / repair / committed');
  expect(container.querySelector('.ss-plan-schedule').textContent).toContain('committed');
  expect(container.querySelector('.ss-plan-deadline').textContent).toContain('300');
  expect(container.querySelector('.ss-plan-interval').textContent).toContain('30 → 90');
  expect(container.querySelector('button,input,select')).toBeNull();
});
