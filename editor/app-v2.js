import { isSupported, pickProjectRoot, getProjectRoot, readFile, writeFile } from './project-root.js';
import { ModeShell } from './mode-shell.js';
import { parseWorldToml, stringifyWorldToml, validateWorldToml } from './world-toml.js';
import { parseEntityToml, stringifyEntityToml, validateEntityToml } from './entity-toml.js';

const $ = (id) => document.getElementById(id);

const modeShell = new ModeShell();
let currentFilePath = null;

async function init() {
  if (!isSupported()) {
    showBanner();
    return;
  }

  setupModeSwitcher();
  setupPickRoot();
  setupChangeRoot();
  setupOpenFile();
  setupSaveFile();

  // V1 map editor (canvas + layers) is the default view.
  // V2 text editor stays hidden; it's shown only when the user triggers it
  // from the V1 toolbar. The root handle (if persisted) is available for
  // V2's File System Access API read/write when needed.
  await getProjectRoot();
}

function showBanner() {
  $('browser-not-supported').classList.remove('hidden');
}

function showPicker() {
  $('root-picker').classList.remove('hidden');
}

function showEditor() {
  $('v2-editor').classList.remove('hidden');
  $('v2-root-label').textContent = 'Project root: selected';
}

function setupModeSwitcher() {
  document.querySelectorAll('.v2-mode-tab').forEach((tab) => {
    tab.addEventListener('click', () => {
      const mode = tab.dataset.mode;
      modeShell.switchMode(mode);
      document.querySelectorAll('.v2-mode-tab').forEach((t) => t.classList.remove('active'));
      tab.classList.add('active');
    });
  });
}

function setupPickRoot() {
  $('pickRootBtn').addEventListener('click', async () => {
    try {
      await pickProjectRoot();
      showEditor();
    } catch (err) {
      $('v2-status').textContent = `Error picking root: ${err.message}`;
    }
  });
}

function setupChangeRoot() {
  $('v2-change-root-btn').addEventListener('click', async () => {
    try {
      await pickProjectRoot();
      $('v2-status').textContent = 'Root changed';
    } catch (err) {
      $('v2-status').textContent = `Error changing root: ${err.message}`;
    }
  });
}

function setupOpenFile() {
  const fileInput = document.createElement('input');
  fileInput.type = 'file';
  fileInput.accept = '.toml';
  fileInput.multiple = false;
  fileInput.style.display = 'none';
  document.body.appendChild(fileInput);

  fileInput.addEventListener('change', async (e) => {
    const file = e.target.files?.[0];
    if (!file) return;
    try {
      const content = await file.text();
      currentFilePath = file.name;
      $('v2-file-content').value = content;
      $('v2-status').textContent = `Loaded: ${file.name}`;
    } catch (err) {
      $('v2-status').textContent = `Error: ${err.message}`;
    }
    fileInput.value = '';
  });

  $('v2-open-btn').addEventListener('click', () => {
    fileInput.click();
  });
}

function setupSaveFile() {
  $('v2-save-btn').addEventListener('click', async () => {
    if (!currentFilePath) {
      $('v2-status').textContent = 'No file open';
      return;
    }

    try {
      const content = $('v2-file-content').value;
      await writeFile(currentFilePath, content);
      $('v2-status').textContent = `Saved: ${currentFilePath}`;
    } catch (err) {
      $('v2-status').textContent = `Error: ${err.message}`;
    }
  });
}

document.addEventListener('DOMContentLoaded', init);
