import './strings-boot.js';
import { applyToDom } from './strings.js';
import { mountWorkshopAuthoring } from './workshop-authoring.js';

applyToDom(document);
mountWorkshopAuthoring({ root: document.getElementById('workshop') });
