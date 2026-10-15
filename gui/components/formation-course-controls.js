import { t, wireText } from '../strings.js';
import { NAVIGATION_FORMATION_ACTION_ID } from '../stations/navigation-actions.js';
export function mountFormationCourseControls(doc) {
  const node = doc.createElement('section'), request = doc.createElement('select'), leader = doc.createElement('select');
  const button = doc.createElement('button'), status = doc.createElement('p'); button.type = 'button';
  button.textContent = t('navigation.formation.select'); request.setAttribute('aria-label', t('navigation.formation.request'));
  leader.setAttribute('aria-label', t('navigation.formation.leader')); node.append(request, leader, button, status);
  node.dataset.formationCourse = ''; button.dataset.formationSelect = ''; request.dataset.formationRequests = ''; leader.dataset.formationLeaders = '';
  let current = null, requestsKey = null, leadersKey = null;
  const options = (select, rows, value, label) => { const selected = select.value;
    select.replaceChildren(...rows.map(row => { const option = doc.createElement('option'); option.value = value(row); option.textContent = label(row); return option; }));
    if (rows.some(row => value(row) === selected)) select.value = selected;
  };
  button.addEventListener('click', () => {
    if (button.disabled || !current) return;
    doc.defaultView?.activateSemanticAction?.(NAVIGATION_FORMATION_ACTION_ID,
      { source: 'control', context: 'navigation', detail: { request_id: request.value, leader: leader.value } });
  });
  return { node, update(state = {}) {
    current = state; const view = state?.formation, suggestions = view?.suggestions || [], leaders = view?.leaders || [];
    const nextRequests = JSON.stringify(suggestions), nextLeaders = JSON.stringify(leaders);
    if (nextRequests !== requestsKey) { requestsKey = nextRequests; options(request, suggestions, row => row.message_id,
      row => t('navigation.formation.suggestion', { name: wireText(row.source_name), x: row.slot[0], z: row.slot[1] })); }
    if (nextLeaders !== leadersKey) { leadersKey = nextLeaders; options(leader, leaders, row => row.contact, row => row.name ? wireText(row.name) : row.contact); }
    button.disabled = state.navigation_controllable !== true || !!state.navigation_auto || !suggestions.length || !leaders.length;
    request.disabled = leader.disabled = button.disabled;
    status.textContent = view?.course ? t(view.course.paused ? 'navigation.formation.paused' : 'navigation.formation.selected') : '';
    node.hidden = !suggestions.length && !view?.course;
  }, dispose() { current = null; button.disabled = true; node.remove(); } };
}
