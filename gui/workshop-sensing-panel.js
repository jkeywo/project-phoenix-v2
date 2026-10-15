import { t } from './strings.js';
import { inspectSensing, applySensingOperation, applyEnquiryObservationOperation, sensingRoles, focusedAims, enquiryEffects } from '../editor/workshop-sensing.js';
import { runWorkshopMutation } from './workshop-edit-session.js';

/** Instruments edit the accepted source and retain ordinary Workshop history. */
export function mountWorkshopSensing({ root, provider, runtime, draft: getDraft, busy, setBusy, changed = () => {}, attach = true }) {
  const doc = root.ownerDocument, prefix = 'workshop-sensing-';
  const el = (tag, id, key) => { const node = doc.createElement(tag); if (id) node.id = prefix + id; if (key) node.textContent = t('workshop.sensing.' + key); return node; };
  const input = (id, type = 'text') => { const node = el('input', id); node.type = type; if (type === 'number') { node.min = '0'; node.step = 'any'; } return node; };
  const label = (control, key) => { const node = el('label', null, key); node.htmlFor = control.id; node.append(control); return node; };
  const fill = (select, values, previous = select.value) => { select.replaceChildren(); for (const value of values) { const option = el('option'); option.value = value; option.textContent = value || t('workshop.sensing.none'); select.append(option); } select.value = values.includes(previous) ? previous : values[0] || ''; };
  const node = el('section', 'panel'), template = el('select', 'template'), materialise = input('materialise', 'checkbox'), status = el('p', 'status'); status.setAttribute('role', 'status');
  const console = el('select', 'console'), group = el('select', 'group'), room = el('select', 'room'), newId = input('new-id'); newId.value = 'passive-receiver';
  const bindings = Object.fromEntries(["combat_lock","science_target"].map(key=>[key,el("select",key)]));
  const rows = {}; let inspection = null, dependencies = null, disposed = false, loading = false, observed = null, generation = 0;
  const navOwner=el('select','navigation-owner'),navShare=input('navigation-share','checkbox'),navReceive=input('navigation-receive','checkbox');
  node.append(el('h3', null, 'title'), el('p', null, 'hint'), label(template, 'template'), label(materialise, 'materialise'), ...Object.entries(bindings).map(([key,control])=>label(control,key)), status);
  const defaults = { focused: { reading_ticks: 120, work_secs: 1, history_limit: 32, unfinished_retention_secs: 0 }, broad: { reading_ticks: 120, sample_interval_secs: .5, identity_secs: 1, classification_secs: 3, assessment_secs: 6, position_error: 2, drift_error_per_sec: .5, track_retention_secs: 60 }, passive: { sample_interval_secs: .5, min_signal_strength: .01, bearing_error_rad: .15, mount: { facing_deg: 0, arc_deg: 360, range: 100 } } };
  const hosts = {}, sequences = {};
  const observationHost=el('fieldset','observation'), observationRows=new Map(); let observationSequence=0;
  observationHost.append(el('legend',null,'observation'));
  function focusedControls(container,key,value) {
    const defaultAim=el('select',key+'-default-aim'); fill(defaultAim,focusedAims,value.default_aim || 'general');
    const forecast=input(key+'-forecast','checkbox'), horizon=input(key+'-forecast-horizon','number');
    forecast.checked=!!value.forecast; horizon.value=value.forecast?.horizon_secs ?? 120;
    container.append(label(defaultAim,'default-aim'),label(forecast,'forecast'),label(horizon,'forecast-horizon'));
    const aims={};
    for (const aim of focusedAims) {
      const prior=value.aims?.find(row=>row.aim===aim), group=el('fieldset');
      const legend=el('legend'); legend.textContent=t('component.scan.aim.'+aim); group.append(legend);
      const enabled=input(key+'-aim-'+aim+'-enabled','checkbox'); enabled.checked=aim==='general' || !!prior;
      if (aim==='general') enabled.dataset.fixed='true';
      const intrusive=input(key+'-aim-'+aim+'-intrusive','checkbox'); intrusive.checked=!!prior?.intrusive;
      const work=input(key+'-aim-'+aim+'-work','number'), cost=input(key+'-aim-'+aim+'-cost','number');
      work.value=prior?.work_multiplier ?? 1; cost.value=prior?.resource_multiplier ?? 1;
      group.append(label(enabled,'aim-enabled'),label(work,'aim-work'),label(cost,'aim-cost'),label(intrusive,'aim-intrusive')); container.append(group);
      aims[aim]={enabled,intrusive,work,cost,prior};
    }
    return {defaultAim,forecast,horizon,aims,prior:value};
  }
  function readFocused(row) {
    const controls=row.focused, result={};
    if (controls.forecast.checked) result.forecast={horizon_secs:controls.horizon.valueAsNumber};
    if (controls.defaultAim.value!=='general' || controls.prior.default_aim!=null) result.default_aim=controls.defaultAim.value;
    const aims=Object.entries(controls.aims).filter(([aim,value])=>aim==='general'
      ? value.prior || value.intrusive.checked || value.work.valueAsNumber!==1 || value.cost.valueAsNumber!==1
      : value.enabled.checked).map(([aim,value])=>({aim,work_multiplier:value.work.valueAsNumber,resource_multiplier:value.cost.valueAsNumber,
        ...(value.intrusive.checked || value.prior?.intrusive!=null ? {intrusive:value.intrusive.checked} : {})}));
    if (aims.length || controls.prior.aims!=null) result.aims=aims;
    return result;
  }
  function observationChoices() {
    const value=inspection.resolved.value, choices=[];
    if (value.shape) {
      choices.push(['field_exposure',t('component.scan.quantity.field_exposure')]);
      for (const effect of enquiryEffects) choices.push(['effect:'+effect,t('component.scan.effect.'+effect)]);
    }
    for (const capacity of value.infrastructure?.capacity || []) if (capacity.label) choices.push(['capacity:'+capacity.id,t(capacity.label)]);
    return choices;
  }
  function addObservation(value={}) {
    const key=observationSequence++, group=el('fieldset'), property=el('select','observation-'+key+'-property');
    const selected=value.property?.kind==='capacity' ? 'capacity:'+value.property.capacity : value.property?.kind==='effect_signature' ? 'effect:'+value.property.effect : 'field_exposure';
    const choices=observationChoices(); fill(property,choices.map(([id])=>id),selected);
    for (const option of property.options) option.textContent=choices.find(([id])=>id===option.value)?.[1] || '';
    const normal=input('observation-'+key+'-normal','checkbox'), intrusive=input('observation-'+key+'-intrusive','checkbox');
    normal.checked=!!value.normal; intrusive.checked=!!value.intrusive;
    const point=el('select','observation-'+key+'-point'); fill(point,['receiver','observed_subject'],value.point || 'receiver');
    for (const option of point.options) option.textContent=t('workshop.sensing.point.'+option.value);
    const step=input('observation-'+key+'-step','number'); step.value=value.step ?? 1;
    const remove=el('button','observation-'+key+'-remove','remove-instrument'); remove.type='button';
    remove.addEventListener('click',()=>{observationRows.delete(key);group.remove();});
    group.append(label(property,'property'),label(normal,'normal'),label(intrusive,'aim-intrusive'),label(point,'point'),label(step,'precision'),remove);
    observationRows.set(key,{property,normal,intrusive,point,step}); observationHost.append(group);
  }
  function readObservation() {
    return {property:[...observationRows.values()].map(row=>{
      const selected=row.property.value, property=selected.startsWith('capacity:') ? {kind:'capacity',capacity:selected.slice(9)} : selected.startsWith('effect:') ? {kind:'effect_signature',effect:selected.slice(7)} : {kind:selected};
      return {property,normal:row.normal.checked,intrusive:row.intrusive.checked,point:row.point.value,step:row.step.valueAsNumber};
    })};
  }
  function addInstrument(role, value = defaults[role]) {
    const spec = sensingRoles[role], index = sequences[role]++, key = index === 0 ? role : role + '-' + index;
    const container = el('fieldset'), system = el('select', key + '-system'), fields = {};
    container.append(label(system, 'system'));
    fill(system, ['', ...inspection.systems.filter(system => (spec.kinds || [spec.kind]).includes(system.kind)).map(system => system.id)], value.system || '');
    for (const field of [...spec.fields, ...(role === 'passive' ? ['facing_deg', 'arc_deg', 'range'] : [])]) {
      const control = input(key + '-' + field, 'number'); control.value = value[field] ?? value.mount?.[field] ?? defaults[role][field] ?? defaults[role].mount?.[field];
      if (field === 'facing_deg') control.removeAttribute('min');
      if (field === 'reading_ticks') { control.step = '1'; control.max = '4294967295'; }
      if (field === 'history_limit') { control.step = '1'; control.min = '1'; control.max = '65535'; }
      fields[field] = control; container.append(label(control, field));
    }
    if (role === 'broad') {
      const initial = inspection.sensing?.sharing?.find(row => row.system === value.system);
      for (const [key, checked] of [['share_own_faction', initial?.share_own_faction ?? false], ['receive', initial?.receive ?? true]]) {
        fields[key] = input(key + '-' + index, 'checkbox'); fields[key].checked = checked;
        container.append(label(fields[key], key));
      }
      container.append(el('p', null, 'sharing-hint'));
    }
    const remove = el('button', key + '-remove', 'remove-instrument'); remove.type = 'button';
    remove.addEventListener('click', () => { rows[role].delete(key); container.remove(); });
    const focused=role==='focused' ? focusedControls(container,key,value) : null;
    container.append(remove); rows[role].set(key, { system, fields, focused }); hosts[role].append(container);
    system.addEventListener('change',controls);
  }
  for (const role of Object.keys(sensingRoles)) {
    hosts[role] = el('fieldset', role); hosts[role].append(el('legend', null, role)); rows[role] = new Map(); sequences[role] = 0;
    const add = el('button', role + '-add', 'add-instrument'); add.type = 'button'; add.addEventListener('click', () => { addInstrument(role); controls(); }); node.append(hosts[role], add);
  }
  function controls() {
    const blocked=disposed || loading || busy() || !inspection;
    for (const control of node.querySelectorAll('input,select,button')) control.disabled = blocked || control.dataset.fixed==='true';
    for(const row of rows.broad?.values() || []) {
      const eligible=inspection?.systems.some(system=>system.id===row.system.value && ['sensor_radar','tactical_radar'].includes(system.kind));
      for(const key of ['share_own_faction','receive']) { row.fields[key].parentElement.hidden=!eligible;row.fields[key].disabled=blocked||!eligible; }
    }
    navSave.disabled=blocked || !inspection?.resolved.value.navigation_console || !navOwner.value;
  }
  function paint() {
    const sharing=inspection.resolved.value.navigation_console?.sharing;
    fill(navOwner,['',...inspection.systems.filter(system=>system.kind==='navigation').map(system=>system.id)],sharing?.system || inspection.systems.find(system=>system.kind==='navigation')?.id || '');
    navShare.checked=sharing?.share_own_faction ?? false;navReceive.checked=sharing?.receive ?? true;
    for(const [binding,kind] of [["combat_lock","tactical_radar"],["science_target","sensors"]]) fill(bindings[binding],["",...inspection.systems.filter(system=>system.kind===kind).map(system=>system.id)],inspection.sensing?.control?.[binding] || "");
    for (const role of Object.keys(sensingRoles)) {
      for (const child of [...hosts[role].children].slice(1)) child.remove(); rows[role].clear(); sequences[role] = 0;
      for (const value of inspection.sensing?.[role] || []) addInstrument(role, value);
      // An empty editable row also provides the visible values for fitting new hardware.
      if (!rows[role].size) addInstrument(role);
    }
    for (const child of [...observationHost.children].slice(1)) child.remove(); observationRows.clear(); observationSequence=0;
    for (const value of inspection.resolved.value.enquiry_observation?.property || []) addObservation(value);
    fill(console, (inspection.resolved.value.console || []).map(row => row.id)); fill(group, (inspection.resolved.value.energy?.group || []).map(row => row.id)); fill(room, (inspection.resolved.value.interior?.compartment || []).map(row => row.id)); controls();
  }
  function readRow(role, row, system = row.system.value) { return { system,
    ...Object.fromEntries(sensingRoles[role].fields.map(field => [field, row.fields[field].valueAsNumber])),
    ...(role === 'passive' ? { mount: Object.fromEntries(['facing_deg', 'arc_deg', 'range'].map(field => [field, row.fields[field].valueAsNumber])) } : {}),
    ...(role === 'focused' ? readFocused(row) : {}),
  }; }
  function read() { return {
    control:Object.fromEntries(Object.entries(bindings).filter(([,control])=>control.value).map(([key,control])=>[key,control.value])),
    sharing:[...rows.broad.values()].filter(row=>inspection.systems.some(system=>system.id===row.system.value && ['sensor_radar','tactical_radar'].includes(system.kind)))
      .map(row=>({system:row.system.value,share_own_faction:row.fields.share_own_faction.checked,receive:row.fields.receive.checked})),
    ...Object.fromEntries(Object.keys(sensingRoles).map(role => [role, [...rows[role].values()].filter(row => row.system.value).map(row => readRow(role, row))]))}; }
  async function apply(extra = {}) {
    if (busy() || !inspection) return; const token = ++generation, value = read();
    if (extra.type) {
      const editable = [...rows.passive.values()], values = editable.find(row => !row.system.value) || editable.at(-1);
      if (!values) return;
      value.passive.push(readRow('passive', values, newId.value));
    }
    const operation = { path: template.value, value, materialise: materialise.checked, ...extra };
    return runWorkshopMutation({ setBusy, current: () => !disposed && token === generation, start: controls,
      invoke: () => applySensingOperation({ draft: getDraft(), provider, runtime, dependencies, operation, current: () => !disposed && token === generation }),
      success: () => { changed(operation.path); inspect(true); status.textContent = t('workshop.changed'); },
      error: error => { status.textContent = t('workshop.sensing.refused', { reason: error.report?.findings?.[0]?.message || error.message }); }, release: controls });
  }
  const save = el('button', 'save', 'save'); save.type = 'button'; save.addEventListener('click', () => void apply());
  const navSave=el('button','navigation-save','save');navSave.type='button';navOwner.addEventListener('change',controls);
  navSave.addEventListener('click',()=>{
    if(busy() || !inspection)return;const token=++generation;
    const operation={type:'navigation-sharing',path:template.value,materialise:materialise.checked,value:{system:navOwner.value,share_own_faction:navShare.checked,receive:navReceive.checked}};
    void runWorkshopMutation({setBusy,current:()=>!disposed&&token===generation,start:controls,
      invoke:()=>applySensingOperation({draft:getDraft(),provider,runtime,dependencies,operation,current:()=>!disposed&&token===generation}),
      success:()=>{changed(operation.path);inspect(true);status.textContent=t('workshop.changed');},error:error=>{status.textContent=t('workshop.sensing.refused',{reason:error.message});},release:controls});
  });
  const nav=el('fieldset');nav.append(el('legend',null,'navigation-sharing'),label(navOwner,'system'),label(navShare,'share_own_faction'),label(navReceive,'receive'),navSave);node.append(nav);
  const fit = el('button', 'fit', 'fit'); fit.type = 'button'; fit.addEventListener('click', () => void apply({ type: 'sensing-fit-passive', new_system: newId.value, console: console.value, group: group.value, compartment: room.value }));
  node.append(save, el('h4', null, 'fit'), label(newId, 'new-id'), label(console, 'console'), label(group, 'group'), label(room, 'room'), fit);
  const addObservationButton=el('button','observation-add','add-property'); addObservationButton.type='button'; addObservationButton.addEventListener('click',()=>{addObservation();controls();});
  const saveObservation=el('button','observation-save','save-observation'); saveObservation.type='button';
  saveObservation.addEventListener('click',()=>{
    if (busy() || !inspection) return;
    const token=++generation, operation={path:template.value,value:readObservation(),materialise:materialise.checked};
    void runWorkshopMutation({setBusy,current:()=>!disposed && token===generation,start:controls,
      invoke:()=>applyEnquiryObservationOperation({draft:getDraft(),provider,runtime,dependencies,operation,current:()=>!disposed && token===generation}),
      success:()=>{changed(operation.path);inspect(true);status.textContent=t('workshop.changed');},
      error:error=>{status.textContent=t('workshop.sensing.refused',{reason:error.report?.findings?.[0]?.message || error.message});},release:controls});
  });
  node.append(observationHost,addObservationButton,saveObservation);
  function inspect(force = false) {
    const draft = getDraft(); if (!draft || !dependencies) { inspection = null; controls(); return; }
    fill(template, draft.paths().filter(path => path.startsWith('assets/entities/') && path.endsWith('.toml') && !draft.isBinary(path)).sort());
    const key = JSON.stringify([draft.sourceRevision, template.value, dependencies]); if (!force && observed === key) { controls(); return; }
    try { inspection = inspectSensing(draft, dependencies, template.value); observed = key; paint(); materialise.checked = false; status.textContent = ''; }
    catch (error) { inspection = null; status.textContent = t('workshop.sensing.refused', { reason: error.message }); } controls();
  }
  async function load() {
    if (disposed || loading || dependencies || typeof runtime.dependencies !== 'function') return;
    loading = true; controls(); const token = ++generation;
    try { const value = await runtime.dependencies(); if (!disposed && token === generation) dependencies = value; }
    catch (error) { if (!disposed) status.textContent = t('workshop.sensing.refused', { reason: error.message }); }
    finally { if (!disposed && token === generation) { loading = false; inspect(true); } }
  }
  template.addEventListener('change', () => inspect(true)); if (attach) root.append(node); controls(); void load();
  return { node, refresh({ hidden = false } = {}) { node.hidden = hidden; if (!hidden) { void load(); inspect(); } }, dispose() { disposed = true; generation++; node.remove(); } };
}
