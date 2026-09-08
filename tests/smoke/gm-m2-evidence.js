import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

/** Observe the actual GM submit seam and modal; never submit an action here. */
export async function observeGm(page, label) {
  await page.evaluate(label => {
    if (window.__m2Evidence) throw new Error('M2 observer already installed');
    const record = window.__m2Evidence = { label, requests: [], confirmations: [] };
    const tick = () => window.wasm_sim_tick();
    const profile = () => JSON.parse(window.__hostGmConfirmationProfile.exportProfile());
    const submit = window.wasm_submit_gm_action;
    if (typeof submit !== 'function') throw new Error('Missing production GM submit seam');
    window.wasm_submit_gm_action = function (...args) {
      const row = { tick: tick(), request: JSON.parse(args[0]), profile: profile() };
      record.requests.push(row);
      try {
        const result = Reflect.apply(submit, this, args);
        row.acceptedAtIngress = result !== false;
        return result;
      } catch (error) {
        row.error = String(error);
        throw error;
      }
    };
    const dialog = document.getElementById('gm-action-confirmation');
    if (!dialog) throw new Error('Missing shared GM confirmation');
    const state = () => ({ tick: tick(), category: dialog.dataset.category,
      description: dialog.querySelector('[data-confirmation-description]')?.textContent,
      preview: dialog.querySelector('[data-confirmation-preview]')?.textContent,
      operator: window.__hostLocalGm(), profile: profile() });
    let previous = '';
    let active = null;
    const sample = () => {
      if (dialog.hidden) {
        if (active) record.confirmations.push({ ...active, tick: tick(), event: 'closed',
          reason: 'without-observed-human-decision' });
        active = null;
        previous = '';
        return;
      }
      const row = state();
      const signature = JSON.stringify([row.category, row.description, row.preview]);
      if (signature !== previous) {
        record.confirmations.push({ event: active ? 'updated' : 'shown', ...row });
        previous = signature;
      }
      active = row;
    };
    const decision = (event, reason) => {
      if (dialog.hidden) return;
      // Capture synchronously: a show and click can share one mutation batch.
      sample();
      record.confirmations.push({ ...active, event, reason });
      active = null;
      previous = '';
    };
    const observer = new MutationObserver(sample);
    observer.observe(dialog, { attributes: true, childList: true, subtree: true, characterData: true });
    // The actual controller focuses Cancel on opening, before observers flush.
    dialog.addEventListener('focusin', sample, true);
    dialog.addEventListener('click', event => {
      const button = event.target.closest('[data-confirmation-accept],[data-confirmation-cancel]');
      if (button && !button.disabled) decision(
        button.hasAttribute('data-confirmation-accept') ? 'accepted' : 'cancelled', 'button');
      else if (event.target === dialog) decision('cancelled', 'backdrop');
    }, true);
    window.addEventListener('keydown', event => {
      if (event.key === 'Escape' && !dialog.hidden) decision('cancelled', 'escape');
    }, true);
    sample();
  }, label);
}

/** Install after initial crew choice. Even transient forbidden input is retained. */
export async function observeCrew(client) {
  await client.page.evaluate(() => {
    if (window.__m2CrewSent) throw new Error('M2 crew observer already installed');
    const send = window.__conn.send;
    window.__m2CrewSent = [];
    window.__conn.send = function (...args) {
      window.__m2CrewSent.push(JSON.parse(args[0]));
      return Reflect.apply(send, this, args);
    };
  });
}

export async function readGmEvidence(page) {
  return page.evaluate(() => {
    const states = {};
    for (const family of ['Session', 'Mission', 'Spawn', 'Objective', 'Station',
      'Knowledge', 'Activity', 'Effect', 'Contact', 'Despawn', 'System', 'Npc', 'Comms', 'RolePresets']) {
      const read = window[`__hostGm${family}State`];
      if (typeof read === 'function') states[family] = read();
    }
    return { ...window.__m2Evidence, tick: window.wasm_sim_tick(), phase: window.__saveSlotsPhase,
      operator: window.__hostLocalGm(), states };
  });
}

export async function assertGmOnlyCrewWitness(client, expect) {
  const sent = await client.page.evaluate(() => window.__m2CrewSent);
  // Readiness and heartbeat traffic do not operate the simulation. Any other
  // outgoing client message requires the broader attended replay contract.
  const allowed = new Set(['SetReady', 'Ping', 'Pong']);
  expect(sent.filter(message => !allowed.has(message.type)),
    'GM-only replay cannot discard crew commands or temporary seat/rating changes').toEqual([]);
  return sent;
}

export async function retainEvidence(testInfo, name, content, contentType = 'application/json') {
  const output = testInfo.outputPath(name);
  await mkdir(path.dirname(output), { recursive: true });
  const body = typeof content === 'string' ? content : JSON.stringify(content, null, 2);
  await writeFile(output, body);
  await testInfo.attach(name, { path: output, contentType });
  return output;
}
