/**
 * gui/coordination-popup.js — Shared Coordination presentation renderer.
 *
 * Producers now send a required {title, title_params, body, body_params}
 * envelope beside the typed semantic fact. This module resolves only
 * that envelope plus the generic sender/destination route. It deliberately
 * knows no payload variants: adding a Coordination fact can never require a
 * client-side sentence switch (issue #1255).
 */

import { has, t, wireText } from './strings.js';

/** Resolve a parameter table whether or not localiseTree already ran. */
function resolveParams(params) {
  if (!params || typeof params !== 'object' || Array.isArray(params)) return {};
  return Object.fromEntries(Object.entries(params).map(([key, value]) => [
    key,
    typeof value === 'string' ? wireText(value) : value,
  ]));
}

/**
 * Resolve a String Table id or literal authored string with deterministic
 * interpolation. `localiseTree` normally resolves this at ingress; doing the
 * same here keeps the pure public renderer correct for direct/local callers.
 */
function resolvePresentationText(value, params) {
  if (value === null || value === undefined) return '';
  const text = String(value);
  const resolved = resolveParams(params);
  if (has(text)) return t(text, resolved);
  return text.replace(/\{(\w+)\}/g, (match, key) =>
    Object.prototype.hasOwnProperty.call(resolved, key) ? String(resolved[key]) : match,
  );
}

/**
 * Resolve one producer-owned envelope to the fields shared by both layouts.
 *
 * @param {{title?: string, title_params?: object, body?: string, body_params?: object}} presentation
 * @param {string|null|undefined} senderLabel localised id or literal sender
 * @param {string|null|undefined} targetLabel localised id or literal destination
 * @returns {{sender: string, from: string, to: string, title: string, body: string}}
 */
export function normalizeCoordinationPresentation(presentation, senderLabel, targetLabel) {
  const from = wireText(senderLabel || 'chatter.sender.ai');
  const to = targetLabel ? wireText(targetLabel) : '';
  const envelope = presentation || {};
  return {
    sender: to ? `${from} → ${to}` : from,
    from,
    to,
    title: resolvePresentationText(envelope.title, envelope.title_params),
    body: resolvePresentationText(envelope.body, envelope.body_params),
  };
}

/**
 * Paint the phone's existing route/title/body (two-content-line) layout.
 * Route labels are undecorated display text; this layout owns the brackets.
 */
export function renderCoordinationPopup(doc, presentation, senderLabel, targetLabel) {
  const fromEl = doc.getElementById('popup-from');
  const arrowEl = doc.getElementById('popup-arrow');
  const toEl = doc.getElementById('popup-to');
  const colonEl = doc.getElementById('popup-colon');
  const titleEl = doc.getElementById('popup-title');
  const bodyEl = doc.getElementById('popup-body');
  if (!fromEl || !arrowEl || !toEl || !colonEl || !titleEl || !bodyEl) return null;

  const norm = normalizeCoordinationPresentation(presentation, senderLabel, targetLabel);
  fromEl.textContent = `[${norm.from}]`;
  arrowEl.textContent = norm.to ? ' → ' : '';
  toEl.textContent = norm.to ? `[${norm.to}]` : '';
  colonEl.textContent = ':';
  titleEl.textContent = norm.title;
  bodyEl.textContent = norm.body;
  return norm;
}

/**
 * Build the Viewscreen's existing single-line chatter bubble safely.
 * Route labels are undecorated display text; this layout owns the brackets.
 */
export function buildCoordinationChatterBubble(doc, presentation, senderLabel, targetLabel) {
  const norm = normalizeCoordinationPresentation(presentation, senderLabel, targetLabel);
  const bubble = doc.createElement('div');
  bubble.className = 'chatter-bubble';

  const from = doc.createElement('span');
  from.className = 'chatter-from';
  from.textContent = `[${norm.from}]`;
  bubble.appendChild(from);

  if (norm.to) {
    const arrow = doc.createElement('span');
    arrow.className = 'chatter-arrow';
    arrow.textContent = ' → ';
    bubble.appendChild(arrow);

    const to = doc.createElement('span');
    to.className = 'chatter-to';
    to.textContent = `[${norm.to}]`;
    bubble.appendChild(to);
  }

  const parts = [];
  if (norm.title && norm.title !== norm.from) parts.push(norm.title);
  if (norm.body) parts.push(norm.body);
  const text = doc.createElement('span');
  text.className = 'chatter-text';
  text.textContent = `: ${parts.join(' — ') || norm.title}`;
  bubble.appendChild(text);
  return bubble;
}

// Expose for the non-module inline scripts in client.html and server.html.
if (typeof window !== 'undefined') {
  window.normalizeCoordinationPresentation = normalizeCoordinationPresentation;
  window.renderCoordinationPopup = renderCoordinationPopup;
  window.buildCoordinationChatterBubble = buildCoordinationChatterBubble;
}
