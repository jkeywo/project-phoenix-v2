/**
 * comms-editor.js
 *
 * Pure data model for editing comms templates ([[comms]] blocks in world TOML).
 *
 * Each template has:
 *  - from: entity name (sender)
 *  - trigger: { kind, entity } or { kind, after_secs }
 *  - node: { body, responses[] }
 *    - each response: { text, actions[], follow_up? }
 *
 * Nodes are accessed via nodePath — an array of response indices.
 * [] = root node, [0] = first response's follow_up, [0,1] = second response
 * under the first response's follow-up.
 *
 * No DOM manipulation — fully testable in Node.
 */

function deepClone(obj) {
  return JSON.parse(JSON.stringify(obj));
}

export class CommsEditor {
  constructor() {
    this._templates = [];
  }

  load(comms) {
    this._templates = [];
    if (!Array.isArray(comms)) return;
    for (const t of comms) {
      this._templates.push(deepClone(t));
    }
  }

  getTemplates() {
    return this._templates.map((t) => deepClone(t));
  }

  addTemplate(from, triggerKind, entityName) {
    const trigger =
      triggerKind === 'on_timer'
        ? { kind: triggerKind, after_secs: 10.0 }
        : { kind: triggerKind, entity: entityName };
    this._templates.push({
      from,
      trigger,
      node: { body: '', responses: [] },
    });
  }

  removeTemplate(index) {
    if (index >= 0 && index < this._templates.length) {
      this._templates.splice(index, 1);
    }
  }

  setTemplateField(index, field, value) {
    const t = this._templates[index];
    if (!t) return;
    if (field === 'from') {
      t.from = value;
    } else if (field === 'trigger.kind') {
      t.trigger.kind = value;
    } else if (field === 'trigger.entity') {
      t.trigger.entity = value;
    } else if (field === 'trigger.after_secs') {
      t.trigger.after_secs = value;
    }
  }

  _resolveNode(templateIndex, nodePath) {
    const t = this._templates[templateIndex];
    if (!t) return null;
    let node = t.node;
    for (const idx of nodePath) {
      if (!node.responses[idx] || !node.responses[idx].follow_up) return null;
      node = node.responses[idx].follow_up;
    }
    return node;
  }

  getNode(templateIndex, nodePath) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return null;
    return deepClone({
      body: node.body,
      responses: node.responses,
    });
  }

  setNodeBody(templateIndex, nodePath, text) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return;
    node.body = text;
  }

  addResponse(templateIndex, nodePath) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return;
    node.responses.push({ text: '', actions: [] });
  }

  removeResponse(templateIndex, nodePath, responseIndex) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return;
    if (responseIndex >= 0 && responseIndex < node.responses.length) {
      node.responses.splice(responseIndex, 1);
    }
  }

  setResponseText(templateIndex, nodePath, responseIndex, text) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return;
    const resp = node.responses[responseIndex];
    if (!resp) return;
    resp.text = text;
  }

  getResponseActions(templateIndex, nodePath, responseIndex) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return [];
    const resp = node.responses[responseIndex];
    if (!resp) return [];
    return [...(resp.actions || [])];
  }

  addResponseAction(templateIndex, nodePath, responseIndex, action) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return;
    const resp = node.responses[responseIndex];
    if (!resp) return;
    resp.actions.push({ ...action });
  }

  removeResponseAction(templateIndex, nodePath, responseIndex, actionIndex) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return;
    const resp = node.responses[responseIndex];
    if (!resp) return;
    if (actionIndex >= 0 && actionIndex < resp.actions.length) {
      resp.actions.splice(actionIndex, 1);
    }
  }

  addFollowUp(templateIndex, nodePath, responseIndex) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return;
    const resp = node.responses[responseIndex];
    if (!resp) return;
    resp.follow_up = { body: '', responses: [] };
  }

  removeFollowUp(templateIndex, nodePath, responseIndex) {
    const node = this._resolveNode(templateIndex, nodePath);
    if (!node) return;
    const resp = node.responses[responseIndex];
    if (!resp) return;
    delete resp.follow_up;
  }

  toComms() {
    return this._templates.map((t) => deepClone(t));
  }
}
