const text = value => typeof value === 'string' && value.trim().length > 0 && new TextEncoder().encode(value).length <= 512 && !/[\x00-\x1f\x7f]/.test(value);
const tick = value => Number.isSafeInteger(value) && value >= 0;
const request = value => value && text(value.message_id) && text(value.origin_id) && text(value.source_name)
  && tick(value.sequence) && value.sequence > 0 && tick(value.accepted_tick)
  && Array.isArray(value.slot) && value.slot.length === 2 && value.slot.every(axis => Number.isInteger(axis) && Math.abs(axis) <= 1e6);
const vector = value => Array.isArray(value) && value.length === 2 && value.every(axis => typeof axis === 'number' && Number.isFinite(Math.fround(axis)));
export function formationNavigation(value) {
  if (!value || !Array.isArray(value.suggestions) || !Array.isArray(value.leaders)) return null;
  const suggestions = value.suggestions.filter(request), leaders = value.leaders.filter(row => row && text(row.contact) && (row.name == null || text(row.name)));
  if (suggestions.length !== value.suggestions.length || leaders.length !== value.leaders.length
    || new Set(suggestions.map(row => row.message_id)).size !== suggestions.length
    || new Set(leaders.map(row => row.contact)).size !== leaders.length) return null;
  let course = value.course;
  if (course != null && (!text(course.owner) || !request(course.request) || !text(course.leader) || !tick(course.selected_tick)
    || course.selected_tick < course.request.accepted_tick || typeof course.paused !== 'boolean'
    || !vector(course.measurement?.position) || !vector(course.measurement?.velocity) || !tick(course.measurement?.observed_tick))) return null;
  return structuredClone({ suggestions, leaders, course: course || null });
}
