import { installSessionToken as install } from '../packages/session/src/session-token.js';
export * from '../packages/session/src/session-token.js';
export const REGISTRY_KEY = 'phoenix-live-tabs';
export function installSessionToken(win = globalThis.window) {
  return install(win, { registryKey: REGISTRY_KEY });
}
if (typeof window !== 'undefined') window.installSessionToken = installSessionToken;
