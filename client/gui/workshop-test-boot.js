import './strings-boot.js';
import { applyToDom, t } from './strings.js';
import { installWorkshopTestChild } from '../editor/workshop-test-child.js';
import { launchWorkshopTest } from '../editor/workshop-test-runtime.js';

applyToDom(document);
const hud = document.getElementById('test-hud');
hud.title = t('workshop.test_heading');
let currentHud = null;
const showHud = payload => { currentHud = payload; hud.contentWindow?.__updateHud?.(payload); };
hud.addEventListener('load', () => { if (currentHud) showHud(currentHud); });
installWorkshopTestChild({ launch: (snapshot, { signal }) => launchWorkshopTest(snapshot, { signal, onHud: showHud }) });
