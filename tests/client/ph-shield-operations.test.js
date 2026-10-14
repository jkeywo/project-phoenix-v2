// @vitest-environment jsdom
import { describe, it, expect, afterEach, vi } from 'vitest';
import '../../gui/components/ph-shield-facings.js';
import { SCIENCE_SHIELD_RAISED_ACTION_ID, SCIENCE_SHIELD_RECOVERY_ACTION_ID } from '../../gui/stations/sensors-actions.js';

function fixture() {
  const facings=[{arc_id:'port_aux',system_id:'actual-port',label:'Port',hp:75,max_hp:100,online:true,controllable:true,
    operation:{raised:false,protection_fraction:0.25,reserve:{energy:4,capacity:10,heat:2,trip_heat:10},recovery_energy_per_hp:0.2,discharge_hp_per_sec:5,recovery_targets:['actual-fore'],recovery_target:'actual-fore'}},
  {arc_id:'fore',system_id:'actual-fore',label:'Fore',hp:50,max_hp:100,controllable:true,operation:{raised:true,protection_fraction:1,recovery_targets:[]}}];
  document.body.innerHTML='<ph-shield-facings></ph-shield-facings>';
  const ring=document.querySelector('ph-shield-facings');
  ring.state={facings,auto:false};
  const el=ring.shadowRoot.querySelector('ph-shield-operations');
  return {ring,el,facings};
}
afterEach(()=>{document.body.innerHTML='';delete window.activateSemanticAction;});

describe('Shield operations shared console controls',()=>{
  it('shows stored strength independently of reserve, heat and partial protection, and sends donor commands',()=>{
    const activate=vi.fn();window.activateSemanticAction=activate;
    const {el,facings,ring}=fixture();
    const row=el.shadowRoot.querySelector('[data-system-id="actual-port"]');
    const read=key=>row.querySelector(`[data-reading="${key}"]`).textContent;
    expect([read('strength'),read('battery'),read('heat'),read('protection')]).toEqual(['75%','40%','20%','25%']);
    const fore=el.shadowRoot.querySelector('[data-system-id="actual-fore"]');
    expect(fore.querySelector('[data-reading="battery"]').textContent).toBe('\u2014');
    expect(fore.querySelector('[data-reading="heat"]').textContent).toBe('\u2014');
    const [raise,cancel]=row.querySelectorAll('button');
    raise.click();
    expect(activate).toHaveBeenLastCalledWith(SCIENCE_SHIELD_RAISED_ACTION_ID,{source:'control',detail:{system_id:'actual-port',raised:true}});
    expect(raise.getAttribute('aria-pressed')).toBe('false'); // Wait for authoritative update.
    const select=row.querySelector('select');
    expect([...select.options].map(o=>o.value)).toEqual(['','actual-fore']);
    cancel.click();
    expect(activate).toHaveBeenLastCalledWith(SCIENCE_SHIELD_RECOVERY_ACTION_ID,{source:'control',detail:{system_id:'actual-port',target:null}});
    select.value='actual-fore';select.dispatchEvent(new Event('change'));
    expect(activate).toHaveBeenLastCalledWith(SCIENCE_SHIELD_RECOVERY_ACTION_ID,{source:'control',detail:{system_id:'actual-port',target:'actual-fore'}});
    facings[0].operation.reserve.energy=3;
    ring.state={facings,auto:false};
    expect(el.shadowRoot.querySelector('[data-system-id="actual-port"] select')).toBe(select);
    expect(read('battery')).toBe('30%');
  });
  it('hides delegated arcs, keeps explicit feeds read-only, and removes lost observations',()=>{
    const {el,ring,facings}=fixture();
    facings[0].controllable=false;
    facings[1].delegated=true;
    ring.state={facings,auto:false};
    expect(el.shadowRoot.querySelectorAll('.arc')).toHaveLength(1);
    expect(el.shadowRoot.querySelector('.controls').hidden).toBe(true);
    ring.state={facings:[],auto:false};
    expect(el.hidden).toBe(true);
    expect(el.shadowRoot.querySelectorAll('.arc')).toHaveLength(0);
  });
  it('does not let nested operation keyboard events trigger the focus ring',()=>{
    const activate=vi.fn();window.activateSemanticAction=activate;
    const {el,ring}=fixture();
    ring.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true}));
    el.shadowRoot.querySelector('button').dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,composed:true}));
    expect(activate).not.toHaveBeenCalled();
    expect(ring.getAttribute('role')).toBe('group');
  });
  it('explains visible recovery constraints without reading a hidden recipient',()=>{
    const {el,ring,facings}=fixture();
    facings[1].operation.reserve={energy:10,capacity:10,heat:0,trip_heat:10};
    ring.state={facings,auto:false};
    expect(el.shadowRoot.querySelector('.recovery-state').textContent).toContain('Destination battery full');
    facings[1].operation.reserve.thermal_stopped=true;
    ring.state={facings,auto:false};
    expect(el.shadowRoot.querySelector('.recovery-state').textContent).toContain('Destination overheated');
    ring.state={facings:[facings[0]],auto:false};
    expect(el.shadowRoot.querySelector('.recovery-state').textContent).not.toContain('overheated');
    facings[0].operation.reserve.thermal_stopped=true;
    ring.state={facings:[facings[0]],auto:false};
    expect(el.shadowRoot.querySelector('.recovery-state').textContent).toContain('Generator overheated');
  });
});
