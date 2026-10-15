import {test,expect} from '@playwright/test';
// Actual shipped Comms document and correlated action mapper. Publications are
// already permitted phone data; this case does not claim a Rust/WASM producer.
const publish=(page,state)=>page.evaluate(value=>window.__updateConsole('comms',JSON.stringify(value)),state);
const view={enabled:false,target:null,emitting:false,controllable:true,targets:[{contact:'held-contact',name:'Observed relay'}]};
const row={system_id:'actual-jammer',display_name:'Interference equipment',availability:'normal',reasons:[],delegated:false,jamming:view};
const base={system_ids:['comms','actual-jammer'],system_families:{comms:'comms','actual-jammer':'jamming'},comms_auto:false,channels:[{id:'public',range:200,work_units:1,protected:false}],contacts:[],messages:[],system_status:[row]};
test('Comms operator selects its own measured jammer aim and stops emission through correlated controls',{tag:'@core'},async({page})=>{
 await page.goto('/gui/cruiser/comms.html');await page.waitForFunction(()=>typeof window.__updateConsole==='function');
 await page.evaluate(async()=>{const{dispatchConsoleAction}=await import('/gui/action-map.js');window.__wire=[];window.__sendAction=json=>dispatchConsoleAction(JSON.parse(json),(type,data)=>window.__wire.push({type,...data}));});
 await publish(page,{...base,receiver_interference:true});
 await expect(page.locator('ph-comms-text-composer #interference')).toBeVisible();
 await page.locator('.system-status-strip > summary').click();await page.locator('[data-system-id="actual-jammer"]').click();
 const controls=page.locator('[data-jamming]');await expect(controls).toBeVisible();
 await controls.getByRole('button',{name:'Select observed aim',exact:true}).click();
 await controls.getByRole('button',{name:'Enable emitter',exact:true}).focus();await page.keyboard.press('Enter');
 const emitted=await page.evaluate(()=>window.__wire);expect(emitted).toEqual([
 {type:'ControlSystemCorrelated',target:'actual-jammer',correlation:expect.any(String),payload:{type:'SetJammingTarget',data:{target_uuid:'held-contact'}}},
 {type:'ControlSystemCorrelated',target:'actual-jammer',correlation:expect.any(String),payload:{type:'SetJammingEnabled',data:{enabled:true}}},
 ]);
 await publish(page,{...base,receiver_interference:true,system_status:[{...row,jamming:{...view,enabled:true,emitting:true,target:'held-contact'}}]});
 await expect(controls).toContainText('receiver effects unknown');await controls.getByRole('button',{name:'Disable emitter',exact:true}).click();
 expect((await page.evaluate(()=>window.__wire))[2].payload).toEqual({type:'SetJammingEnabled',data:{enabled:false}});
 await publish(page,{...base,receiver_interference:false,system_status:[{...row,jamming:{...view,controllable:false},delegated:true}]});
 await expect(page.locator('ph-comms-text-composer #interference')).toBeHidden();
 await page.locator('.system-status-dialog').getByRole('button',{name:'Close',exact:true}).click();await page.locator('[data-delegated-summary]').click();
 await expect(page.locator('[data-jamming] button:visible')).toHaveCount(0);expect(await page.evaluate(()=>window.__wire.length)).toBe(3);
});
