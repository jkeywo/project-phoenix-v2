import { t, wireText } from '../strings.js';
import { PRODUCTION_ACTION_IDS } from '../stations/production-actions.js';
import { activateEngineeringAction } from '../stations/engineering-action-control.js';
/** Human standing orders; every input is retained across progress snapshots. */
export class PhProductionControls extends HTMLElement {
  constructor() {
    super(); const root = this.attachShadow({ mode: 'open' }), doc = this.ownerDocument;
    const el = (tag, key) => { const node = doc.createElement(tag); if (key) node.textContent = t('production.' + key); return node; };
    this.toggle = el('button'); this.toggle.type = 'button'; this.recipe = el('select');
    this.target = el('input'); this.target.type = 'number'; this.target.min = '0'; this.target.step = '1';
    this.applyRecipe = el('button', 'select-recipe'); this.applyRecipe.type = 'button';
    this.applyTarget = el('button', 'set-target'); this.applyTarget.type = 'button';
    this.recipeInfo = el('p'); this.progress = el('p'); this.blocked = el('p'); this.blocked.setAttribute('role', 'status');
    const recipeLabel = el('label', 'recipe'), targetLabel = el('label', 'target'); recipeLabel.append(this.recipe); targetLabel.append(this.target);
    const style = el('style'); style.textContent = ':host{display:block}label{display:flex;gap:.5em;flex-wrap:wrap}button,select,input{min-height:44px}p{overflow-wrap:anywhere}';
    root.append(style, el('h2', 'title'), this.toggle, recipeLabel, el('p', 'recipe-change-hint'), this.applyRecipe, targetLabel, this.applyTarget, this.recipeInfo, this.progress, this.blocked);
    const activate = (name, detail) => activateEngineeringAction(null, PRODUCTION_ACTION_IDS[name], detail);
    this.toggle.addEventListener('click', () => activate('enabled', { enabled: !this._state.enabled }));
    this.applyRecipe.addEventListener('click', () => activate('recipe', { recipe: this.recipe.value || null, target: this.recipe.value ? this.target.valueAsNumber : 0 }));
    this.applyTarget.addEventListener('click', () => activate('target', { amount: this.target.valueAsNumber }));
    this.recipe.addEventListener('change', () => this.paintSelection());
    this.target.addEventListener('keydown', event => event.stopPropagation()); this.recipe.addEventListener('keydown', event => event.stopPropagation());
  }
  paintSelection() {
    const state = this._state; if (!state) return;
    const recipe = state.recipes?.find(row => row.id === this.recipe.value);
    this.target.max = String(recipe?.ceiling ?? 0);
    this.applyTarget.disabled = !!state.production_auto || state.production_controllable === false || this.recipe.value !== (state.selected_recipe || '');
    this.recipeInfo.textContent = recipe ? t('production.recipe-summary', { inputs: recipe.inputs.map(row => `${row.amount} ${wireText(row.store)}`).join(' + '),
      outputs: recipe.outputs.map(row => `${row.amount} ${wireText(row.store)}`).join(' + '), seconds: recipe.work_secs }) : '';
  }
  set state(value) {
    const previous = this._state; this._state = value; this.hidden = !value?.system_id; if (this.hidden) return;
    const identityChanged = previous?.system_id !== value.system_id;
    const current = this.recipe.value, ids = ['', ...(value.recipes || []).map(row => row.id)];
    if (JSON.stringify(ids) !== this._options) { this._options = JSON.stringify(ids); this.recipe.replaceChildren();
      for (const id of ids) { const option = this.ownerDocument.createElement('option'); option.value = id; option.textContent = id ? wireText(id) : t('production.none'); this.recipe.append(option); }
      this.recipe.value = !identityChanged && ids.includes(current) ? current : value.selected_recipe || '';
    }
    if (identityChanged || (previous?.selected_recipe !== value.selected_recipe && this.shadowRoot.activeElement !== this.recipe)) this.recipe.value = value.selected_recipe || '';
    if (identityChanged || this.shadowRoot.activeElement !== this.target) this.target.value = value.target;
    for (const node of [this.toggle, this.recipe, this.target, this.applyRecipe]) node.disabled = !!value.production_auto || value.production_controllable === false;
    this.toggle.textContent = t(value.enabled ? 'production.pause' : 'production.enable'); this.paintSelection();
    this.progress.textContent = value.active ? t('production.progress', { recipe: wireText(value.active.recipe), progress: Math.round(value.active.progress * 100) }) : t('production.idle');
    this.blocked.textContent = value.blocked ? t(value.blocked) : '';
  }
  get state() { return this._state; }
}
if (!customElements.get('ph-production-controls')) customElements.define('ph-production-controls', PhProductionControls);
