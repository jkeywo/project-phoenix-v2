import '../strings-boot.js';
import { t } from '../strings.js';
import { PhElement, phDefine } from './ph-element.js';

export class PhHelmMovement extends PhElement {
  template() {
    return `<style>
      :host{display:block;font:var(--text-sm) 'JetBrains Mono',monospace;color:var(--ink)}
      :host([hidden]){display:none}.axes{display:grid;gap:4px}.axis{display:flex;align-items:center;gap:4px;flex-wrap:wrap}
      .axis span{flex:1}button,input,select{font:inherit;max-width:100%}input{width:7rem}
      details{margin-top:4px}label{display:flex;flex-wrap:wrap;gap:4px;margin:4px 0}p{margin:4px 0;overflow-wrap:anywhere}
    </style><p>${t('helm.movement.hint')}</p><div id="axes" class="axes"></div>
    <details id="assist"><summary>${t('helm.movement.assistance')}</summary>
      <label>${t('helm.movement.speed')}<input id="speed" type="number" step="any"><button id="set-speed" type="button">${t('helm.movement.set')}</button></label>
      <label>${t('helm.movement.heading')}<input id="heading" type="number" step="any"><button id="set-heading" type="button">${t('helm.movement.set')}</button></label>
      <label>${t('helm.movement.match')}<select id="contact"></select><button id="match" type="button">${t('helm.movement.match')}</button><button id="cancel" type="button">${t('helm.movement.cancel')}</button></label>
      <p id="held" role="status"></p>
    </details>`;
  }
  onTemplate() {
    this.rows = new Map(); this.options = new Map();
    this.placeholder = this.ownerDocument.createElement('option');
    this.placeholder.value = ''; this.placeholder.textContent = t('helm.movement.choose');
    this.$('contact').append(this.placeholder);
    for (const kind of ['keydown', 'keyup']) this.shadowRoot.addEventListener(kind, event => event.stopPropagation());
    const axis = role => this.state?.axes?.find(axis => axis.role === role);
    const emit = (id, role, detail) => {
      const owner = axis(role);
      if (!owner?.controllable) return;
      window.activateSemanticAction?.(id, { context: 'helm', source: 'control', detail: { systemId: owner.system_id, ...detail } });
    };
    this.$('set-speed').addEventListener('click', () => emit('helm.desired-speed', 'thrust', { speed: this.$('speed').valueAsNumber }));
    this.$('set-heading').addEventListener('click', () => emit('helm.desired-heading', 'steering', { heading_rad: this.$('heading').valueAsNumber * Math.PI / 180 }));
    this.$('match').addEventListener('click', () => emit('helm.match-speed', 'thrust', { contact: this.$('contact').value }));
    this.$('cancel').addEventListener('click', () => emit('helm.match-speed', 'thrust', { contact: null }));
    this.$('contact').addEventListener('change', () => this.render(this.state));
  }
  render(view) {
    this.hidden = !Array.isArray(view?.axes) || !view.axes.length;
    const axes = Array.isArray(view?.axes) ? view.axes : [];
    const present = new Set(axes.map(axis => axis.system_id));
    for (const [id, row] of this.rows) if (!present.has(id)) { row.node.remove(); this.rows.delete(id); }
    for (const axis of axes) {
      let row = this.rows.get(axis.system_id);
      if (!row) {
        const node = this.ownerDocument.createElement('div'); node.className = 'axis';
        const label = this.ownerDocument.createElement('span'), value = this.ownerDocument.createElement('output'), button = this.ownerDocument.createElement('button');
        button.type = 'button'; button.dataset.systemId = axis.system_id;
        button.addEventListener('click', () => {
          const current = this.state?.axes?.find(candidate => candidate.system_id === button.dataset.systemId);
          if (current?.controllable) window.activateSemanticAction?.('helm.mode', { context: 'helm', source: 'control',
            detail: { systemId: current.system_id, mode: current.mode === 'direct' ? 'assisted' : 'direct' } });
        });
        const follow=this.ownerDocument.createElement('button');follow.type='button';follow.dataset.followNavigation=axis.system_id;
        let armed=null;
        const arm=()=>{const current=this.state?.axes?.find(candidate=>candidate.system_id===axis.system_id);armed={generation:this.state?.navigation_generation,following:current?.following_generation};};
        follow.addEventListener('pointerdown',arm);follow.addEventListener('keydown',event=>{if(event.key==='Enter'||event.key===' ')arm();});
        follow.addEventListener('click',()=>{
          const current=this.state?.axes?.find(candidate=>candidate.system_id===axis.system_id);
          if(!current?.controllable||follow.disabled)return;
          if(armed&&(armed.generation!==this.state?.navigation_generation||armed.following!==current.following_generation)){armed=null;return;}
          armed=null;window.activateSemanticAction?.('helm.follow-navigation',{context:'helm',source:'control',detail:{systemId:current.system_id,generation:current.following_generation!==null?null:this.state.navigation_generation}});
        });
        node.append(label, value, button,follow); this.$('axes').append(node); row = { node, label, value, button,follow }; this.rows.set(axis.system_id, row);
      }
      row.label.textContent = t('helm.movement.axis.' + axis.role)
        + (!axis.available ? ` · ${t('helm.movement.unavailable')}` : !axis.controllable ? ` · ${t('helm.movement.delegated')}` : '');
      row.button.textContent = t('helm.movement.mode.' + axis.mode); row.button.disabled = !axis.controllable;
      row.follow.textContent=t(axis.following_generation!==null?'helm.movement.stop_navigation':'helm.movement.follow_navigation');
      row.follow.disabled=!axis.controllable||(axis.following_generation===null&&(!axis.navigation_available||!axis.available));
      row.follow.setAttribute('aria-pressed',String(axis.following_generation!==null));

      const intent = axis.intent, demand = intent?.demand;
      row.value.textContent = axis.following_generation!==null ? t(axis.navigation_available && axis.following_generation === this.state.navigation_generation ? 'helm.movement.navigation_selected' : 'helm.movement.navigation_paused')
        : demand?.kind === 'heading' ? t('helm.movement.heading_goal', { value: (demand.heading_rad * 180 / Math.PI).toFixed(1) })
        : ['absolute', 'match'].includes(demand?.kind) ? t('helm.movement.speed_goal', { value: demand.speed.toFixed(1) })
        : intent ? t('helm.movement.stick_goal', { value: ((intent.force ?? intent.rate ?? intent.value ?? demand?.value ?? 0) * 100).toFixed(0) }) : '';
    }
    const thrust = axes.find(axis => axis.role === 'thrust'), steering = axes.find(axis => axis.role === 'steering');
    for (const id of ['speed', 'set-speed', 'contact']) this.$(id).disabled = !thrust?.controllable;
    for (const id of ['heading', 'set-heading']) this.$(id).disabled = !steering?.controllable;
    const contacts = Array.isArray(view?.contacts) ? view.contacts : [];
    const wanted = new Set(contacts.map(row => row.contact));
    for (const [id, option] of this.options) if (!wanted.has(id)) { option.remove(); this.options.delete(id); }
    for (const contact of contacts) {
      let option = this.options.get(contact.contact);
      if (!option) { option = this.ownerDocument.createElement('option'); option.value = contact.contact; this.options.set(contact.contact, option); this.$('contact').append(option); }
      option.textContent = t('helm.movement.contact', { name: contact.name, speed: contact.speed.toFixed(1), tick: contact.observed_tick });
    }
    this.$('match').disabled = !thrust?.controllable || !wanted.has(this.$('contact').value);
    this.$('cancel').disabled = !thrust?.controllable || !view?.match;
    this.$('held').textContent = view?.match ? t(view.match.observation_available ? 'helm.movement.holding' : 'helm.movement.stale', {
      speed: view.match.speed.toFixed(1), tick: view.match.observed_tick,
      age: view.match.age ?? t('observation.unknown'),
    }) : '';
  }
}
phDefine('ph-helm-movement', PhHelmMovement);
