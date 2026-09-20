import './strings-boot.js';
import { applyToDom, t } from './strings.js';
import { installWorkshopTestChild } from '../editor/workshop-test-child.js';
import { launchWorkshopTest } from '../editor/workshop-test-runtime.js';
import { mountWorkshopTestGm } from './workshop-test-gm.js';

applyToDom(document);
const hud = document.getElementById('test-hud');
hud.title = t('workshop.test_heading');
let currentHud = null;
const showHud = payload => { currentHud = payload; hud.contentWindow?.__updateHud?.(payload); };
hud.addEventListener('load', () => { if (currentHud) showHud(currentHud); });

// The omniscient view of the same run. Mounted once, kept in step by every
// gm_* projection, and simply SHOWN or HIDDEN as the view changes: rebuilding
// it per switch would drop the desk's own arrangement and make a switch feel
// like a restart, which is exactly what this must not be.
const gmRoot = document.getElementById('test-gm');
const gm = mountWorkshopTestGm();
const showGm = visible => {
  if (!gmRoot) return;
  gmRoot.hidden = !visible;
  // The viewscreen keeps drawing underneath either way — the run never pauses
  // for a view change — but a hidden canvas must not be reachable by tab.
  hud.hidden = visible;
  document.getElementById('canvas')?.toggleAttribute('inert', visible);
};
showGm(false);

installWorkshopTestChild({
  launch: (snapshot, { signal }) => launchWorkshopTest(snapshot, {
    signal,
    onHud: showHud,
    onGm: (name, payload) => gm?.channel(name, payload),
    onGmRolePresets: payload => gm?.setRolePresets(payload),
    onView: view => showGm(view?.view === 'game-master'),
  }),
});
