/**
 * gui/roving-tabindex.js — the one roving-tabindex rule, shared (issue #1170).
 *
 * A composite widget — a toolbar of fire buttons, a scope's ring of contacts —
 * must be ONE Tab stop, not one per control, or a dense console becomes a
 * hundred-press tab gauntlet. The pattern that fixes this is roving tabindex:
 * exactly one member of the group is in the tab order (`tabindex="0"`) at a
 * time, the rest are `tabindex="-1"`, and the arrow keys move both the focus
 * and that single `0` between them. Tab then enters the group once and leaves
 * it once.
 *
 * The Hero Bar (gui/hero-bar.js `heroBarKeyTarget`) already did this for its
 * station tabs — an id-keyed, horizontal, wrapping ring. This module is that
 * same rule generalised so every composite in the keyboard sweep (PRD #1168)
 * reuses it rather than each re-deriving "which control does ArrowDown land
 * on": index-keyed rather than id-keyed (a composite's controls are a live
 * list, not a stable id set), and orientation-aware (a vertical toolbar answers
 * to Up/Down, a horizontal one to Left/Right).
 *
 * `rovingKeyTarget` is pure — it is the tested core, and it decides nothing
 * about the DOM. `installRovingTabindex` is the thin DOM binding that most
 * callers want; a caller whose items are redrawn every frame (the weapon
 * toolbars reconcile their rows on each state push) calls `syncRovingTabindex`
 * after a render to re-establish the single `0` over the new element set.
 */

/**
 * The index the arrow/Home/End key moves to, or -1 for a key this group does
 * not consume.
 *
 * Wrapping is deliberate and matches the Hero Bar: ArrowDown past the last
 * control returns to the first, so a group is a ring the operator can spin in
 * either direction without hunting for its end.
 *
 * @param {number} count        how many controls are in the group
 * @param {number} currentIndex the focused control's index (<0 ⇒ treat as 0)
 * @param {string} key          KeyboardEvent.key
 * @param {'both'|'horizontal'|'vertical'} [orientation='both']
 * @returns {number} the target index, or -1 if `key` is not a navigation key
 */
export function rovingKeyTarget(count, currentIndex, key, orientation = 'both') {
  if (!count || count < 1) return -1;
  const i = currentIndex < 0 || currentIndex >= count ? 0 : currentIndex;
  const horizontal = orientation === 'both' || orientation === 'horizontal';
  const vertical = orientation === 'both' || orientation === 'vertical';
  if (key === 'Home') return 0;
  if (key === 'End') return count - 1;
  if ((horizontal && key === 'ArrowRight') || (vertical && key === 'ArrowDown')) {
    return (i + 1) % count;
  }
  if ((horizontal && key === 'ArrowLeft') || (vertical && key === 'ArrowUp')) {
    return (i - 1 + count) % count;
  }
  return -1;
}

/**
 * Whether the ring may land its single tab stop on `el`.
 *
 * A `disabled` native control and an `aria-disabled="true"` composite are both
 * unfocusable — `.focus()` is a no-op on them — so if an arrow step parked the
 * lone `tabindex="0"` there, focus would silently fail to move and the next Tab
 * would skip the stranded stop entirely. Roving therefore steps past them and
 * never parks on one; they stay `tabindex="-1"` like every other non-active
 * item.
 *
 * @param {Element} el
 * @returns {boolean}
 */
export function isRovable(el) {
  if (!el) return false;
  if (el.disabled) return false;
  if (typeof el.getAttribute === 'function' && el.getAttribute('aria-disabled') === 'true') return false;
  return true;
}

/**
 * Make exactly `items[activeIndex]` the group's single tab stop.
 *
 * Every other item becomes `tabindex="-1"`; a missing/out-of-range/disabled
 * active index defaults to the first ENABLED item, so a freshly-rendered group
 * is always left with one — and only one — reachable, operable control in the
 * tab order. Returns the index it settled on so a caller can remember where the
 * ring is (or -1 when the group has no rovable control at all).
 *
 * @param {Element[]} items
 * @param {number} [activeIndex] the item to keep tabbable; defaults to the one
 *   already at `tabindex="0"`, else the first enabled item
 * @returns {number} the resolved active index (or -1 for an empty group)
 */
export function syncRovingTabindex(items, activeIndex) {
  const list = Array.from(items || []).filter(Boolean);
  if (list.length === 0) return -1;
  let active = typeof activeIndex === 'number' ? activeIndex : -1;
  // The single tab stop must be an enabled control; never park it on a disabled
  // one, which is unfocusable and would strand the group's only entry point.
  if (active < 0 || active >= list.length || !isRovable(list[active])) {
    const already = list.findIndex((el) => el.tabIndex === 0 && isRovable(el));
    active = already >= 0 ? already : list.findIndex(isRovable);
  }
  list.forEach((el, idx) => { el.tabIndex = idx === active ? 0 : -1; });
  return active;
}

/**
 * Bind arrow-key roving to a composite's `keydown`.
 *
 * The listener sits on `host` and reads the current items fresh on every press
 * (via `getItems`), so a group whose controls come and go between renders needs
 * no re-binding — only a `syncRovingTabindex` call after each render to keep the
 * single `0` correct. The focused control is found by walking the event's
 * composed path, so it works whether the items are in `host`'s light DOM or its
 * shadow root.
 *
 * Enter/Space are left entirely alone: the family's controls are native
 * `<button>`s, which already activate on both, so binding them here would be
 * the behaviour fork the keyboard contract forbids.
 *
 * @param {EventTarget} host
 * @param {{
 *   getItems: () => Element[],
 *   orientation?: 'both'|'horizontal'|'vertical',
 * }} opts
 * @returns {() => void} an uninstall function
 */
export function installRovingTabindex(host, { getItems, orientation = 'vertical' } = {}) {
  if (!host || typeof getItems !== 'function') return () => {};
  const onKeyDown = (event) => {
    const all = Array.from(getItems() || []).filter(Boolean);
    // Step only over the enabled controls, so an arrow always lands on the next
    // OPERABLE one and never stalls on a disabled control it cannot focus.
    const items = all.filter(isRovable);
    if (items.length === 0) return;
    const path = typeof event.composedPath === 'function' ? event.composedPath() : [];
    const focused = path.find((node) => items.includes(node)) || event.target;
    const current = items.indexOf(focused);
    const next = rovingKeyTarget(items.length, current, event.key, orientation);
    if (next < 0) return;
    event.preventDefault();
    const target = items[next];
    // Re-lay the single `0` across the FULL set (so any disabled item is cleared
    // to -1, not left holding a stale stop), then move focus to the enabled one.
    syncRovingTabindex(all, all.indexOf(target));
    if (target && typeof target.focus === 'function') target.focus();
  };
  host.addEventListener('keydown', onKeyDown);
  return () => host.removeEventListener('keydown', onKeyDown);
}

// Expose for any non-module inline script, matching the window.* convention the
// other shared gui modules (hero-bar.js, action-map.js) follow.
if (typeof window !== 'undefined') {
  window.rovingKeyTarget = rovingKeyTarget;
  window.isRovable = isRovable;
  window.syncRovingTabindex = syncRovingTabindex;
  window.installRovingTabindex = installRovingTabindex;
}
