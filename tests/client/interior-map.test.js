// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import { renderInteriorMap } from '../../gui/interior-map.js';
const translate=(key,values={}) => `${key} ${Object.values(values).join(' ')}`;
function state() { return {
  interior:{deck:[{id:'main',label:'Main'},{id:'lower',label:'Lower'}],compartment:[
    {id:'bridge',deck:'main',label:'Bridge',x:0,z:0,width:5,depth:5},
    {id:'engine',deck:'lower',label:'Engine',x:0,z:0,width:5,depth:5},
  ],placements:{radar:'bridge',engine:'engine'}},
  navigation:[{location:{kind:'compartment',compartment:'bridge'},blocked:false}],
  selectedTeam:0,damageable_systems:['radar','engine'],auto:false,
};}
describe('permitted interior map',()=>{
  it('restores the shared deck and preserves other working-view fields',()=>{
    const host=document.createElement('div');const data=state();const changed=vi.fn();
    data.console_view={interior_deck:'lower',energy_depth:'minimum'};
    renderInteriorMap(host,data,translate,vi.fn(),vi.fn(),changed);
    expect(host.querySelector('select').value).toBe('lower');
    const select=host.querySelector('select');select.value='main';select.dispatchEvent(new Event('change'));
    expect(changed).toHaveBeenCalledWith({interior_deck:'main',energy_depth:'minimum'});
  });
  it('only offers access controls for the actual human-owned boundary',()=>{
    const host=document.createElement('div');const data=state();const seal=vi.fn();
    data.boundaries=[{system:'hatch',phase:'sealing',seal_requested:true,controllable:false}];
    renderInteriorMap(host,data,translate,vi.fn(),seal);
    expect(host.textContent).toContain('interior.boundary.sealing');
    expect([...host.querySelectorAll('button')].some(button=>button.textContent.includes('interior.boundary.release'))).toBe(false);
    data.boundaries[0].controllable=true;
    renderInteriorMap(host,data,translate,vi.fn(),seal);
    [...host.querySelectorAll('button')].find(button=>button.textContent.includes('interior.boundary.release')).click();
    expect(seal).toHaveBeenCalledWith('hatch',false);
  });
  it('uses typed System dispatch and disables delegated operation',()=>{
    const host=document.createElement('div'); const dispatch=vi.fn(); const data=state();
    renderInteriorMap(host,data,translate,dispatch);
    host.querySelector('button').click(); expect(dispatch).toHaveBeenCalledWith('radar');
    dispatch.mockClear(); renderInteriorMap(host,{...data,auto:true},translate,dispatch);
    expect(host.querySelector('button').disabled).toBe(true);
    host.querySelector('button').click(); expect(dispatch).not.toHaveBeenCalled();
  });
  it('retains the selected deck and shows actual blocked crossings',()=>{
    const host=document.createElement('div');const data=state();
    renderInteriorMap(host,data,translate,vi.fn());const select=host.querySelector('select');
    select.value='lower';select.dispatchEvent(new Event('change'));
    data.navigation=[{location:{kind:'crossing',from:'bridge',to:'engine'},blocked:true}];
    renderInteriorMap(host,data,translate,vi.fn());
    expect(host.querySelector('select')).toBe(select);
    expect(select.value).toBe('lower'); expect(host.querySelector('button').textContent).toBe('engine');
    expect(host.textContent).toContain('interior.map.blocked');
  });
  it('removes a withheld projection and never interprets labels as markup',()=>{
    const host=document.createElement('div');const data=state();data.interior.compartment[0].label='<img onerror="attack()">';
    renderInteriorMap(host,data,key=>key,vi.fn());expect(host.querySelector('img')).toBeNull();
    renderInteriorMap(host,{},translate,vi.fn());expect(host.hidden).toBe(true);expect(host.childNodes.length).toBe(0);
  });
});
