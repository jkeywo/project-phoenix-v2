// Real host, ordinary transport and authored source overrides only. No injected snapshots.
import { test, expect, readHostPeerId, createTestClient, waitForWasmReady } from './fixtures';

const worldFor = hull => `
[global]
seed = 42
title = "Console ownership acceptance"
description = "Increment 1 ownership and continuation"
[ambient_light]
color = [0.6, 0.55, 0.5]
brightness = 300.0
[[available_ships]]
template_path = "assets/entities/${hull}.toml"
[[entity]]
template_path = "assets/entities/${hull}.toml"
id = "player-ship"
transform = { position = [0.0, 0.0, 0.0] }
spawn_on = "game_start"
`;

async function boot(context, hull = 'alliance_destroyer', duplicate = false) {
  await context.route('**/assets/worlds/default.toml', route =>
    route.fulfill({contentType:'text/plain',body:worldFor(hull)}));
  if (duplicate) await context.route('**/assets/entities/alliance_destroyer.toml', async route => {
    const response = await route.fetch();
    const source = await response.text();
    // This authored variation gives Navigation a human-operated visiting rating.
    // The extra screen is a read-only view of that same permitted publication.
    const block = /\[\[console\]\]\s*id = "navigation"[\s\S]*?(?=\[\[console\.rating\]\])/;
    expect(source).toMatch(block);
    const patched = source.replace(block, text => text.replace('visiting_rating = "Simplified"','visiting_rating = "Std"'));
    expect(patched).not.toBe(source);
    await route.fulfill({response,body:patched + `
[[console]]
id = "navigation-mirror"
name = "Navigation mirror"
home_station = "tactical"
screen = "gui/battleship/navigation.html"
[[console.feed]]
source = "navigation"
kind = "Navigation"
`});
  });
  const server = await context.newPage();
  await server.goto('/?scenario=assets/worlds/default.toml');
  await waitForWasmReady(server);
  return readHostPeerId(server);
}
async function seat(client, station) {
  await client.send('SelectStation',{station});
  await client.page.waitForFunction(token => window.__messages.some(m=>m.type==='StationAssigned' && m.data.token===token),client.token);
}
async function latestSnapshot(client) {
  return client.page.evaluate(()=>window.__messages.filter(m=>m.type==='SimState').at(-1)?.data.snapshot);
}
async function hosted(client, id, host) {
  await expect.poll(async()=> (await latestSnapshot(client))?.console_hosts?.find(c=>c.console===id)?.host).toBe(host);
}
async function feedback(client, correlation) {
  await client.page.waitForFunction(id=>window.__messages.some(m=>m.type==='ActionFeedback' && m.data.correlation===id),correlation);
  return client.page.evaluate(id=>window.__messages.find(m=>m.type==='ActionFeedback' && m.data.correlation===id).data.outcome,correlation);
}
const view = zoom => ({inspected_record:null,comms_thread:null,overlay:null,
  navigation:{zoom,center_x:120,center_y:-45,selected_uuid:null}});

for (const [hull, count] of [['alliance_courier',2],['alliance_destroyer',4],['alliance_cruiser',6],['alliance_battleship',9]]) {
  test(`Console ownership roster: ${hull} retains ${count} claimable seats`, async ({context})=>{
    const host = await boot(context,hull);
    const client = await createTestClient(context,host);
    const welcome = await client.waitForMessage('Welcome');
    const roster = welcome.data.ship_stations;
    expect(roster.stations).toHaveLength(count);
    expect(new Set(roster.stations.map(s=>s.id)).size).toBe(count);
    expect(roster.consoles.length).toBeGreaterThanOrEqual(count);
    const owners = roster.consoles.flatMap(c=>c.systems || []);
    expect(new Set(owners).size).toBe(owners.length);
    for (const console of roster.consoles) {
      if (console.home_station) expect(roster.stations.some(s=>s.id===console.home_station)).toBe(true);
    }
    await client.close();
  });
}

test('same screen URL mounts independent Console instances and real map gestures stay separate', async ({context})=>{
  const host = await boot(context,'alliance_destroyer',true);
  const phone = await context.newPage();
  await phone.setViewportSize({width:1280,height:800});
  await phone.goto(`/client/#${host}`);
  await phone.locator('#station-list .station-row:has-text("Tactical") button.claim-btn').click();
  await phone.locator('#ready-btn').click();
  const navigation = phone.frameLocator('#navigation-iframe');
  const mirror = phone.frameLocator('#navigation-mirror-iframe');
  await expect(navigation.locator('ph-navigation-map')).toBeAttached();
  await expect(mirror.locator('ph-navigation-map')).toBeAttached();
  expect(await phone.locator('#navigation-iframe').getAttribute('src')).toBe(
    await phone.locator('#navigation-mirror-iframe').getAttribute('src'));
  await phone.locator('#station-hero-tabs [data-tab-id="navigation"]').click();
  const before = await phone.evaluate(()=>structuredClone(window.simState.consoleViews['navigation-mirror'] || null));
  await navigation.locator('ph-navigation-map').hover();
  await phone.mouse.wheel(0,-200);
  await expect.poll(()=>phone.evaluate(()=>window.simState.consoleViews.navigation?.navigation?.zoom || 1)).not.toBe(1);
  expect(await phone.evaluate(()=>window.simState.consoleViews['navigation-mirror'] || null)).toEqual(before);
  await phone.locator('#station-hero-tabs [data-tab-id="navigation-mirror"]').click();
  await expect(phone.locator('#navigation-mirror-ui')).toHaveClass(/\bactive\b/);
  expect(await phone.locator('#navigation-iframe').count()).toBe(1);
});

test('working views follow Console handoff and reconnect while stale host writes and commands are refused', async ({context})=>{
  const host = await boot(context,'alliance_destroyer',true);
  const token = 'console-increment-tactical';
  const tactical = await createTestClient(context,host,{token,name:'Tactical'});
  const captain = await createTestClient(context,host,{name:'Captain'});
  await seat(tactical,'Tactical'); await seat(captain,'Captain');
  await tactical.send('SetReady',{ready:true}); await captain.send('SetReady',{ready:true});
  await tactical.waitForMessage('GameStarted');
  await hosted(tactical,'navigation','tactical');
  await tactical.send('SetConsoleView',{console:'navigation',view:view(2)});
  await expect.poll(async()=> (await latestSnapshot(tactical))?.console_views?.navigation).toEqual(view(2));
  await tactical.send('ControlSystemCorrelated',{correlation:'initial-waypoint',target:'navigation',
    payload:{type:'SetNavigationWaypoint',data:{x:240,z:-160}}});
  expect(await feedback(tactical,'initial-waypoint')).toBe('Applied');
  await tactical.send('SetAfk',{afk:true});
  await hosted(captain,'navigation','captain');
  expect((await latestSnapshot(captain)).console_views.navigation).toEqual(view(2));
  await tactical.send('SetConsoleView',{console:'navigation',view:view(7)});
  await tactical.send('ControlSystemCorrelated',{correlation:'stale-waypoint',target:'navigation',
    payload:{type:'SetNavigationWaypoint',data:{x:999,z:999}}});
  // Correlated command acknowledgement is a reliable processing barrier after the view write.
  expect(await feedback(tactical,'stale-waypoint')).toBe('Refused');
  const afterRefusal = await captain.page.evaluate(()=>window.__messages.length);
  await captain.page.waitForFunction(index=>window.__messages.slice(index).some(m=>m.type==='SimState'),afterRefusal);
  expect((await latestSnapshot(captain)).console_views.navigation).toEqual(view(2));
  await captain.send('SetConsoleView',{console:'navigation',view:view(3)});
  await expect.poll(async()=> (await latestSnapshot(captain))?.console_views?.navigation).toEqual(view(3));
  await tactical.send('SetAfk',{afk:false});
  await hosted(tactical,'navigation','tactical');
  expect((await latestSnapshot(tactical)).console_views.navigation).toEqual(view(3));
  await tactical.close();
  await hosted(captain,'navigation','captain');
  const reconnected = await createTestClient(context,host,{token,name:'Tactical'});
  await hosted(reconnected,'navigation','tactical');
  expect((await latestSnapshot(reconnected)).console_views.navigation).toEqual(view(3));
  await reconnected.send('ControlSystemCorrelated',{correlation:'resumed-waypoint',target:'navigation',
    payload:{type:'SetNavigationWaypoint',data:{x:250,z:-170}}});
  expect(await feedback(reconnected,'resumed-waypoint')).toBe('Applied');
  await reconnected.close(); await captain.close();
});
