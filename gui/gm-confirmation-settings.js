/** The private confirmation matrix inside the ordinary host Settings panel. */
import { GM_CONFIRMATION_CATEGORIES } from './gm-confirmation.js';
import { GM_CONFIRMATION_MODES, OPERATOR_PROFILE_FILENAME } from './operator-profile.js';
import { downloadArtifact, readFileText } from './snapshot-transfer.js';

export function renderGmConfirmationSettings({ doc, target, profile, t,
  download = downloadArtifact, read = readFileText, onImport = () => {} }) {
  const section = doc.createElement('section');
  section.className = 'server-settings-section';
  section.dataset.gmConfirmations = '';
  const heading = doc.createElement('h2');
  heading.textContent = t('settings.gm.confirmation.heading');
  section.append(heading);
  const status = doc.createElement('p');
  status.setAttribute('role', 'status');
  const selects = new Map();
  const report = (result) => {
    status.textContent = t(result.status === 'rejected'
      ? 'settings.gm.confirmation.save_failed' : 'settings.gm.confirmation.saved');
  };
  for (const category of GM_CONFIRMATION_CATEGORIES) {
    const row = doc.createElement('label');
    row.className = 'server-settings-row';
    row.textContent = t(category.labelId);
    const select = doc.createElement('select');
    select.dataset.gmConfirmationCategory = category.id;
    for (const mode of GM_CONFIRMATION_MODES) {
      const option = doc.createElement('option');
      option.value = mode;
      option.textContent = t(`settings.gm.confirmation.mode.${mode}`);
      select.append(option);
    }
    select.value = profile.mode(category.id);
    select.addEventListener('change', () => {
      report(profile.setMode(category.id, select.value));
      select.value = profile.mode(category.id);
    });
    row.append(select);
    selects.set(category.id, select);
    section.append(row);
  }
  const exportButton = doc.createElement('button');
  exportButton.type = 'button';
  exportButton.dataset.gmProfileExport = '';
  exportButton.textContent = t('settings.controls.profile.export');
  exportButton.addEventListener('click', () => {
    report({ status: download(doc, OPERATOR_PROFILE_FILENAME, profile.exportProfile()) ? 'saved' : 'rejected' });
  });
  const label = doc.createElement('label');
  label.textContent = t('settings.controls.profile.import');
  const input = doc.createElement('input');
  input.type = 'file';
  input.accept = '.json,application/json';
  input.dataset.gmProfileImport = '';
  input.addEventListener('change', async () => {
    const file = input.files?.[0];
    if (!file) return;
    try {
      const result = profile.importProfile(await read(file));
      report(result);
      if (result.status !== 'rejected') {
        for (const [id, select] of selects) select.value = profile.mode(id);
        onImport();
      }
    } catch (_) { report({ status: 'rejected' }); }
    input.value = '';
  });
  label.append(input);
  section.append(exportButton, label, status);
  target.append(section);
  return section;
}
