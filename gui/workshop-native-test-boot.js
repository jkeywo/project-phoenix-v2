import './strings-boot.js';
import { applyToDom, t } from './strings.js';
import { mountWorkshopTestGm } from './workshop-test-gm.js';
import { mountNativeTestDocument } from './workshop-native-test-document.js';

applyToDom(document);
const image = document.getElementById('canvas');
image.alt = t('workshop.test_heading');
const status = document.createElement('p');
status.setAttribute('role', 'status');
status.style.cssText = 'position:absolute;bottom:0;left:0;color:white;background:black';
document.body.append(status);
const view = mountNativeTestDocument({ win: window, image, status,
  hud: document.getElementById('test-hud'), gmRoot: document.getElementById('test-gm'),
  gm: mountWorkshopTestGm(), unavailable: t('workshop.test_presentation_unavailable') });
window.addEventListener('pagehide', () => view.dispose(), { once: true });
