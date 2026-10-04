/** Positions stay in authored order; removed source rows retain their slots. */
export const liveFormPositions = list => list.map((entry, at) => entry.removed ? null : at).filter(at => at != null);

export function formNeighbours(list, position, movable = () => true, dense = false) {
  const live = dense ? list.map((_, at) => at) : liveFormPositions(list);
  const rank = live.indexOf(position), before = live[rank - 1], after = live[rank + 1];
  const blocked = at => at != null && !movable(list[at]);
  const selfBlocked = rank < 0 || !movable(list[position]);
  return { before, after, up: !selfBlocked && before != null && !blocked(before),
    down: !selfBlocked && after != null && !blocked(after), selfBlocked,
    neighbourBlocked: blocked(before) || blocked(after) };
}

export function swapFormNeighbour(list, position, direction, movable = () => true, dense = false) {
  const live = dense ? list.map((_, at) => at) : liveFormPositions(list);
  const rank = dense ? position : live.indexOf(position);
  if (rank < 0 || (dense && position >= list.length)) return null;
  const target = dense ? position + direction : live[rank + direction];
  if (target == null || (dense && (target < 0 || target >= list.length))
      || !movable(list[position]) || !movable(list[target])) return null;
  [list[position], list[target]] = [list[target], list[position]];
  return target;
}

/** Ratings deliberately skip the shifted row at the removed position. */
export function survivingFormPosition(list, position, strict = false) {
  const live = liveFormPositions(list);
  return live.find(at => strict ? at > position : at >= position) ?? live.filter(at => at < position).pop();
}
