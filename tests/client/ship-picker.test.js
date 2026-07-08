import { describe, it, expect } from 'vitest';
import { resolveShipSelection } from '../../gui/ship-picker.js';

describe('resolveShipSelection', () => {
  it('returns legacy-fallback for empty list', () => {
    expect(resolveShipSelection([])).toEqual({ action: 'legacy-fallback' });
  });

  it('returns legacy-fallback for null/undefined', () => {
    expect(resolveShipSelection(null)).toEqual({ action: 'legacy-fallback' });
    expect(resolveShipSelection(undefined)).toEqual({ action: 'legacy-fallback' });
  });

  it('returns auto-select with templatePath for single ship', () => {
    const ships = [{ template_path: 'assets/entities/alliance_cruiser.toml', label: 'Default' }];
    expect(resolveShipSelection(ships)).toEqual({
      action: 'auto-select',
      templatePath: 'assets/entities/alliance_cruiser.toml',
    });
  });

  it('returns show-picker with ships list for multiple ships', () => {
    const ships = [
      { template_path: 'assets/entities/ship_scout.toml', label: 'Scout' },
      { template_path: 'assets/entities/ship_cruiser.toml', label: 'Cruiser' },
    ];
    const result = resolveShipSelection(ships);
    expect(result.action).toBe('show-picker');
    expect(result.ships).toBe(ships);
  });
});
