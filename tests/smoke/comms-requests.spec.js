import {test,expect} from '@playwright/test';

// Actual shipped browser Console and typed action dispatch. These are already
// permitted phone publications; native owner/full-host tests prove physical
// radio and receiving AI. This browser case does not claim a Rust/WASM host.
const publish=(page,state)=>page.evaluate(state=>window.__updateConsole('comms',JSON.stringify(state)),state);
const base={system_ids:['comms'],system_families:{comms:'comms'},comms_auto:false,
  channels:[{id:'public',range:200,work_units:1,protected:false}],contacts:[{uuid:'known-peer',name:'Known peer',in_range:true}],messages:[]};
const request={id:'incoming-request',sender_uuid:'known-peer',sender_name:'Known peer',thread_id:'request-thread',body:'Report status.',is_read:false,
  sender_in_range:false,responses:[{text:'Report',available:true,important:false},{text:'Decline',available:true,important:false}],
  transmission:{origin_id:'original',channel:'public',audience:{kind:'contact',target_uuid:'this-ship'},emitted_tick:10,received_tick:12,content:{kind:'request',request:'report_status'}},
  request_progress:{facts:{read_tick:null,decision:null,report:null},last_returned:null,dismissed:false}};

test('human sends and reads a closed Comms request, then inspects its same-Console cleared history',{tag:'@core'},async({page})=>{
  await page.goto('/gui/cruiser/comms.html');
  await page.waitForFunction(()=>typeof window.__updateConsole==='function');
  await page.evaluate(async()=>{
    const {dispatchConsoleAction}=await import('/gui/action-map.js');window.__wire=[];
    window.__sendAction=json=>dispatchConsoleAction(JSON.parse(json),(type,data)=>window.__wire.push({type,...data}));
  });
  await publish(page,base);
  await page.locator('ph-comms-text-composer #request').click();
  expect((await page.evaluate(()=>window.__wire))[0]).toEqual({type:'ControlSystemCorrelated',correlation:expect.any(String),target:'comms',payload:{type:'SendCommsRequest',data:{channel:'public',audience:{kind:'fleet'},request:'report_status'}}});
  await publish(page,{...base,messages:[request]});
  await page.locator('ph-comms-current-message #read-request').focus();await page.keyboard.press('Enter');
  expect((await page.evaluate(()=>window.__wire))[1].payload).toEqual({type:'MarkCommsRead',data:{message_id:request.id}});
  const read={...request,is_read:true,request_progress:{...request.request_progress,facts:{read_tick:32,decision:null,report:null}}};
  await publish(page,{...base,messages:[read]});
  await page.locator('ph-comms-current-message').getByRole('button',{name:'Decline',exact:true}).click();
  expect((await page.evaluate(()=>window.__wire))[2]).toEqual({type:'ControlSystemCorrelated',correlation:expect.any(String),target:'comms',payload:{type:'RespondToMessage',data:{message_id:request.id,response_index:1}}});
  const cleared={...read,selected_response:1,request_progress:{...read.request_progress,dismissed:true,facts:{read_tick:32,decision:{choice:'decline',tick:35},report:null}}};
  await publish(page,{...base,messages:[cleared]});
  await expect(page.locator('ph-comms-hail-list').getByRole('option')).toHaveCount(0);
  await page.locator('ph-comms-text-composer #cleared').check();
  await expect(page.locator('ph-comms-hail-list').getByRole('option')).toHaveCount(1);
  await expect(page.locator('ph-comms-current-message .request-status')).toContainText('Read at tick 32');
  await expect(page.locator('ph-comms-current-message .request-status')).toContainText('Declined at tick 35');
  expect(await page.evaluate(()=>window.__wire.length)).toBe(3,'history filtering sends no command or decision');
  await page.locator('ph-comms-text-composer #cleared').uncheck();
  await expect(page.locator('ph-comms-hail-list').getByRole('option')).toHaveCount(0);
});
