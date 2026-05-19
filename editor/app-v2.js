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

  const root = await getProjectRoot();
  if (root) {
    showEditor();
  } else {
    showPicker();
  }
}

function hideV1() {
  const app = document.getElementById('app');
  if (app) app.style.display = 'none';
}

function showBanner() {
  $('browser-not-supported').classList.remove('hidden');
  hideV1();
}

function showPicker() {
  $('root-picker').classList.remove('hidden');
  hideV1();
}

function showEditor() {
  $('v2-editor').classList.remove('hidden');
  $('v2-root-label').textContent = 'Project root: selected';
  hideV1();
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
  $('v2-open-btn').addEventListener('click', async () => {
    const path = prompt('Enter relative file path (e.g., assets/worlds/default.toml):');
    if (!path) return;

    try {
      const content = await readFile(path);
      currentFilePath = path;
      $('v2-file-content').value = content;
      $('v2-status').textContent = `Loaded: ${path}`;
    } catch (err) {
      $('v2-status').textContent = `Error: ${err.message}`;
    }
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
