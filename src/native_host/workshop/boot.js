import { mountNativeWorkshop } from './gui/native-workshop.js';
import { parseWorkshopLaunch } from './editor/workshop-launch.js';
import './gui/strings-boot.js';
import { applyToDom } from './gui/strings.js';
window.__phoenixNativeWorkshopKey = record => {
  const event = new KeyboardEvent(record.pressed ? 'keydown' : 'keyup', {
    code: record.code, key: record.key, ctrlKey: record.ctrlKey, shiftKey: record.shiftKey,
    altKey: record.altKey, metaKey: record.metaKey, repeat: record.repeat, bubbles: true, cancelable: true,
  });
  return !(document.activeElement || document.body).dispatchEvent(event);
};
const profileKey = 'phoenix-operator-profile-v1';
let profile = null;
let localeChoice = null;
const withLocale = json => {
  let next; try { next = JSON.parse(json || 'null'); } catch { next = null; }
  if (!next || typeof next !== 'object') next = {kind:'project-phoenix/operator-profile',version:1};
  if (localeChoice) next.locale = localeChoice;
  return JSON.stringify(next);
};
const loaded = new Promise(resolve => {
  window.__phoenixOperatorReply = reply => {
    if (reply.operation === 'load') {
      profile = typeof reply.profile === 'string' ? reply.profile : null;
      try { localeChoice = JSON.parse(profile || 'null')?.locale || null; } catch { localeChoice = null; }
      resolve();
    }
    window.PhoenixOperatorStorageStatus = reply.status === 'error' ? reply : null;
    window.dispatchEvent(new Event('phoenix-operator-storage-status'));
  };
});
window.PhoenixOperatorStorage = {
  getItem: key => key === profileKey ? profile : null,
  setItem(key, value) {
    if (key !== profileKey) throw new Error('Unsupported Workshop preference');
    profile = withLocale(String(value));
    window.__phoenixNativeWorkshopSend(JSON.stringify({ type: 'NativeOperator', operation: 'save', profile }));
  },
};
window.PhoenixLocaleStorage = {
  getItem: () => localeChoice,
  setItem(_key, value) {
    if (typeof value !== 'string' || !/^[A-Za-z0-9-]{1,35}$/.test(value)) throw Error('Invalid private language');
    localeChoice = value;
    profile = withLocale(profile);
    window.__phoenixNativeWorkshopSend(JSON.stringify({ type: 'NativeOperator', operation: 'save', profile }));
  },
};
window.__phoenixNativeWorkshopSend(JSON.stringify({ type: 'NativeOperator', operation: 'load' }));
await loaded;
applyToDom(document);
const workspace = mountNativeWorkshop({ root: document.getElementById('workshop'), send: window.__phoenixNativeWorkshopSend,
  launch: parseWorkshopLaunch(new URLSearchParams(window.location.hash.slice(1))) });
window.__phoenixNativeWorkshopReply = response => workspace.receive(response);
window.addEventListener('pagehide', () => workspace.dispose(), { once: true });
await workspace.ready;
window.__phoenixNativeWorkshopSend('NativeWorkshopReady');
