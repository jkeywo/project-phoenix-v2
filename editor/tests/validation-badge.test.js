import { describe, it, expect, beforeEach } from 'vitest';
import { installDom, FakeElement } from './slice-5-helpers.js';
import {
  renderValidationBadge,
  applyValidationResults,
  clearValidationBadges,
} from '../validation-badge.js';

describe('validation-badge', () => {
  beforeEach(() => {
    installDom();
  });

  it('empty results list is a no-op', () => {
    const root = new FakeElement('div');
    const child = new FakeElement('div');
    child.dataset.validationPath = 'foo';
    root.appendChild(child);
    applyValidationResults(root, []);
    expect(root.querySelectorAll('.validation-badge').length).toBe(0);
  });

  it('single error record decorates the matching field with a red badge', () => {
    const root = new FakeElement('div');
    const field = new FakeElement('div');
    field.dataset.validationPath = 'trigger[0].action[0].entity';
    root.appendChild(field);
    applyValidationResults(root, [
      { path: 'trigger[0].action[0].entity', severity: 'error', message: 'unknown entity' },
    ]);
    const badges = root.querySelectorAll('.validation-badge');
    expect(badges.length).toBe(1);
    expect(badges[0].classList.contains('validation-badge-error')).toBe(true);
  });

  it('warning record renders with the warning class', () => {
    const root = new FakeElement('div');
    const field = new FakeElement('div');
    field.dataset.validationPath = 'stations.4.0.next';
    root.appendChild(field);
    applyValidationResults(root, [
      { path: 'stations.4.0.next', severity: 'warning', message: 'dangling' },
    ]);
    const badges = root.querySelectorAll('.validation-badge');
    expect(badges.length).toBe(1);
    expect(badges[0].classList.contains('validation-badge-warning')).toBe(true);
  });

  it('multiple records with identical message+severity de-dupe on the same host', () => {
    const root = new FakeElement('div');
    const field = new FakeElement('div');
    field.dataset.validationPath = 'trigger[0].action[0].entity';
    root.appendChild(field);
    applyValidationResults(root, [
      { path: 'trigger[0].action[0].entity', severity: 'error', message: 'same' },
      { path: 'trigger[0].action[0].entity', severity: 'error', message: 'same' },
      { path: 'trigger[0].action[0].entity', severity: 'error', message: 'different' },
    ]);
    expect(root.querySelectorAll('.validation-badge').length).toBe(2);
  });

  it('clearValidationBadges removes all badges', () => {
    const root = new FakeElement('div');
    const field = new FakeElement('div');
    field.dataset.validationPath = 'x';
    root.appendChild(field);
    applyValidationResults(root, [
      { path: 'x', severity: 'error', message: 'm' },
    ]);
    expect(root.querySelectorAll('.validation-badge').length).toBe(1);
    clearValidationBadges(root);
    expect(root.querySelectorAll('.validation-badge').length).toBe(0);
  });

  it('renderValidationBadge defaults to error severity when none supplied', () => {
    const host = new FakeElement('div');
    renderValidationBadge(host, { message: 'oops' });
    const badges = host.querySelectorAll('.validation-badge');
    expect(badges.length).toBe(1);
    expect(badges[0].classList.contains('validation-badge-error')).toBe(true);
  });

  it('prefix match: a record on trigger[0].action decorates a nested entity field', () => {
    const root = new FakeElement('div');
    const field = new FakeElement('div');
    field.dataset.validationPath = 'trigger[0].action[2].entity';
    root.appendChild(field);
    applyValidationResults(root, [
      { path: 'trigger[0].action', severity: 'error', message: 'malformed action' },
    ]);
    expect(root.querySelectorAll('.validation-badge').length).toBe(1);
  });

  it('re-applying replaces previous badges (idempotent)', () => {
    const root = new FakeElement('div');
    const field = new FakeElement('div');
    field.dataset.validationPath = 'x';
    root.appendChild(field);
    applyValidationResults(root, [{ path: 'x', severity: 'error', message: 'first' }]);
    applyValidationResults(root, [{ path: 'x', severity: 'warning', message: 'second' }]);
    const badges = root.querySelectorAll('.validation-badge');
    expect(badges.length).toBe(1);
    expect(badges[0].classList.contains('validation-badge-warning')).toBe(true);
  });
});
