/** Authored rungs are most-detailed first; names remain wire identities. */
export function stationRatingStars(ratings, name) {
  if (name === 'Backfill') return 0;
  const index = (ratings || []).indexOf(name);
  return index < 0 ? null : ratings.length - index;
}

export function stationRatingLabel(ratings, name, label = name) {
  const stars = stationRatingStars(ratings, name);
  return stars == null ? label : `${'★'.repeat(stars) || '☆'} ${label}`;
}
