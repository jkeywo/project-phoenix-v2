import {test,expect} from '@playwright/test';
// Actual shipped Science document/action mapper, supplied permitted phone data.
// This test makes no Rust/WASM emitter, sensor or resource producer claim.
const systems=[{system:'radio',display_name:'Radio receiver',cost:2,home_station:'comms'},{system:'radar',display_name:'Measured radar',cost:2,home_station:'science'},{system:'passive',display_name:'Passive receiver',cost:1,home_station:'science'}];
const view={enabled:true,priorities:['radio','radar'],capacity:3,systems,active:true,protected_systems:['radio'],controllable:true};
const row={system_id:'own-protection',display_name:'Signal protection',availability:'normal',reasons:[],delegated:false,signal_protection:view};
const state={system_ids:['sensors','own-protection'],system_families:{sensors:'sensors','own-protection':'sensors'},system_status:[row]};
const publish=(page,value)=>page.evaluate(state=>window.__updateConsole('science',JSON.stringify(state)),value);
test('Science edits ordered own protection and Station presets through correlated controls',{tag:'@core'},async({page})=>{
 await page.goto('/gui/cruiser/science.html');await page.waitForFunction(()=>typeof window.__updateConsole==='function');
 await page.evaluate(async()=>{const{dispatchConsoleAction}=await import('/gui/action-map.js');window.__wire=[];window.__sendAction=json=>dispatchConsoleAction(JSON.parse(json),(type,data)=>window.__wire.push({type,...data}));});
 await publish(page,state);await page.locator('.system-status-strip > summary').click();await page.locator('[data-system-id="own-protection"]').click();
 const controls=page.locator('[data-signal-protection]');await expect(controls).toBeVisible();await expect(controls).toContainText('Paid protection: Radio receiver');await expect(controls).toContainText('receiver effects unknown');
 await controls.getByLabel('Protection priorities',{exact:true}).selectOption('radar');
 await controls.getByRole('button',{name:'Move up',exact:true}).click();await controls.getByRole('button',{name:'Apply priorities',exact:true}).click();
 await controls.getByRole('combobox',{name:'Station preset',exact:true}).selectOption('science');await controls.getByRole('button',{name:'Use Station preset',exact:true}).click();await controls.getByRole('button',{name:'Apply priorities',exact:true}).click();
 await controls.getByRole('button',{name:'Disable protection',exact:true}).focus();await page.keyboard.press('Enter');
 expect(await page.evaluate(()=>window.__wire)).toEqual([
 {type:'ControlSystemCorrelated',target:'own-protection',correlation:expect.any(String),payload:{type:'SetSignalProtectionPriorities',data:{systems:['radar','radio']}}},
 {type:'ControlSystemCorrelated',target:'own-protection',correlation:expect.any(String),payload:{type:'SetSignalProtectionPriorities',data:{systems:['radar','passive']}}},
 {type:'ControlSystemCorrelated',target:'own-protection',correlation:expect.any(String),payload:{type:'SetSignalProtectionEnabled',data:{enabled:false}}}]);
 await publish(page,{...state,system_status:[{...row,delegated:true,signal_protection:{...view,controllable:false}}]});
 await page.locator('.system-status-dialog').getByRole('button',{name:'Close',exact:true}).click();await page.locator('[data-delegated-summary]').click();await expect(controls.locator('button:visible')).toHaveCount(0);
 await page.evaluate(()=>window.activateSemanticAction('sensors.signal-protection-enabled',{context:'protection',detail:{owner:'foreign',enabled:true}}));expect(await page.evaluate(()=>window.__wire.length)).toBe(3);
});
