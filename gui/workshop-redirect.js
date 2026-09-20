import { legacyWorkshopUrl } from '../editor/workshop-launch.js';

const kind = document.documentElement.dataset.workshopRedirect;
const link = document.getElementById('workshop-migration-link');
const target = legacyWorkshopUrl(window.location, kind);
link.href = target;
window.location.replace(target);
