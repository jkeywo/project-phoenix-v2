// @vitest-environment jsdom
import { expect, it, vi } from 'vitest';
import { bindFormMoves } from '../../gui/ordered-form-controls.js';

it('rebuilds before focus, preserves directional fallbacks and ignores refused moves', () => {
  const up = document.createElement('button'), down = document.createElement('button');
  const order = [], move = vi.fn(direction => direction < 0 ? 3 : null);
  bindFormMoves(up, down, move, () => order.push('rebuild'), (...args) => order.push(args));
  up.click(); down.click();
  expect(move.mock.calls).toEqual([[-1], [1]]);
  expect(order).toEqual(['rebuild', [3, 'up', 'down']]);
});
