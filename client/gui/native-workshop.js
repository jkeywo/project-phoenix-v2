/** Shared offline UI on the native host's private Authoring bridge. The shell
 * calls this module; the ordinary browser boot never imports native authority. */
import { createNativeWorkshopProvider, createWorkshopBridge } from '../editor/workshop-provider.js';
import { mountWorkshopAuthoring } from './workshop-authoring.js';

export function mountNativeWorkshop({ root, send, win = window }) {
  const bridge = createWorkshopBridge({ send });
  const provider = createNativeWorkshopProvider({ request: bridge.request });
  const authoring = mountWorkshopAuthoring({ root, win, provider });
  return { ready: authoring.ready, receive: bridge.receive,
    dispose() { authoring.dispose(); bridge.dispose(); } };
}
