const NAME = /^[A-Za-z_][A-Za-z0-9_:-]{0,127}$/;
const COMPARISONS = new Set(['eq', 'ne', 'ge', 'gt', 'le', 'lt']);

/** Validate and copy the bounded Test-local scenario-state vocabulary. */
export function validateTestBreakpoint(value, worldPaths = []) {
  if (value == null) return null;
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || Object.keys(value).some(key => !['layer', 'condition'].includes(key))) {
    throw new Error('Invalid Test breakpoint');
  }
  const layer = value.layer == null || value.layer === '' ? null : value.layer;
  if (layer !== null && (typeof layer !== 'string' || !worldPaths.includes(layer))) {
    throw new Error('Test breakpoint layer is absent from the exact draft');
  }
  const condition = value.condition;
  if (!condition || typeof condition !== 'object' || !NAME.test(condition.name || '')) {
    throw new Error('Invalid Test breakpoint Flag name');
  }
  if (condition.kind === 'flag') {
    if (typeof condition.value !== 'boolean'
        || Object.keys(condition).some(key => !['kind', 'name', 'value'].includes(key))) {
      throw new Error('Invalid Test Flag breakpoint');
    }
    return { ...(layer ? { layer } : {}), condition: { kind: 'flag', name: condition.name, value: condition.value } };
  }
  if (condition.kind === 'counter') {
    if (!COMPARISONS.has(condition.comparison) || !Number.isSafeInteger(condition.value)
        || Object.keys(condition).some(key => !['kind', 'name', 'comparison', 'value'].includes(key))) {
      throw new Error('Invalid Test counter breakpoint');
    }
    return { ...(layer ? { layer } : {}), condition: { kind: 'counter', name: condition.name,
      comparison: condition.comparison, value: condition.value } };
  }
  throw new Error('Invalid Test breakpoint kind');
}
