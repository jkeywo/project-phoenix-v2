/** The lifecycle a complex GM action shares (issue #1506).
 *
 * A complex action is one that combines several choices before it can be sent —
 * Spawn's type, faction and placement; later, a misclassification's observer and
 * policy, a restore's preflight. It opens as ONE floating draft per action type;
 * invoking it again focuses the draft that is already open rather than opening a
 * second one, because two half-filled copies of the same form is a way to send
 * the wrong one.
 *
 * Finishing is the operator's decision, not the panel's:
 *
 *  - **Keep open** is on the confirm control. A draft the operator wants to send
 *    again stays.
 *  - **Docking implies Keep open.** A docked draft is a tool they put somewhere
 *    on purpose; closing it under them because one send succeeded would be the
 *    dock throwing away an arrangement.
 *  - An undocked, unchecked draft closes on AUTHORITATIVE SUCCESS and at no
 *    other time. Validation, local refusal, authoritative refusal and timeout
 *    all leave it open with its typed feedback, because every one of those is a
 *    thing the operator is about to correct and resend.
 *
 * Nothing here submits, admits or confirms anything: the action's own panel
 * still owns its typed request, its feedback and its confirmation category.
 * This owns only when the draft is on screen and whether it may be discarded.
 */
export function createTemporaryActions({ layout, confirmDiscard = () => true } = {}) {
  const registered = new Map();

  /** Which registered action a panel id belongs to, if any. */
  const entry = panel => registered.get(panel) || null;

  const isOpen = panel => {
    const state = layout?.state?.();
    return !!state && !state.closed.includes(panel);
  };
  const isDocked = panel => {
    const state = layout?.state?.();
    return !!state && !state.closed.includes(panel)
      && !state.floats.some(float => float.panel === panel);
  };

  /** A draft the operator has typed into and not yet sent. */
  const isDirty = panel => {
    const action = entry(panel);
    try { return !!action && action.isDirty() === true; } catch { return false; }
  };

  /** Ask before losing typed work. A clean draft closes without a word. */
  function mayDiscard(panel) {
    if (!isDirty(panel)) return true;
    return confirmDiscard(panel) === true;
  }

  return {
    /**
     * @param panel      the registered dock panel id
     * @param isDirty    () => boolean — has the operator typed anything unsent?
     * @param reset      ({ keepReusable }) => void — clear the draft. On repeat
     *                   the reusable choices stay and the one-shot values go; on
     *                   a fresh open everything returns to its default.
     * @param keepOpen   () => boolean — the confirm control's Keep open state
     * @param focus      () => void — put the caret back in the draft
     */
    register(panel, { isDirty: dirty, reset, keepOpen = () => false, focus = () => {} }) {
      registered.set(panel, { isDirty: dirty, reset, keepOpen, focus });
    },
    isOpen,
    isDocked,
    /** Open the draft, or focus the one already open. */
    open(panel) {
      if (!entry(panel)) return false;
      if (isOpen(panel)) {
        layout?.reveal?.(panel, { reopen: false });
        entry(panel).focus();
        return true;
      }
      // A fresh open starts from the defaults: a draft is not a memory.
      entry(panel).reset({ keepReusable: false });
      if (!layout?.reveal?.(panel)) return false;
      entry(panel).focus();
      return true;
    },
    /** The operator asked to close it. Typed work is confirmed away first. */
    close(panel) {
      const action = entry(panel);
      if (!action || !isOpen(panel)) return false;
      if (!mayDiscard(panel)) return false;
      action.reset({ keepReusable: false });
      layout?.set?.(layout.model.close(layout.state(), panel));
      return true;
    },
    /** Take a draft off the screen without asking. Only an authoritative run
     * boundary uses this: the world the draft was for has gone, so there is
     * nothing left to confirm away. */
    closeSilently(panel) {
      if (!entry(panel) || !isOpen(panel)) return false;
      layout?.set?.(layout.model.close(layout.state(), panel));
      return true;
    },
    /** The action landed. Only an authoritative success reaches this. */
    succeeded(panel) {
      const action = entry(panel);
      if (!action) return false;
      // Repeating is the common case, so the choices that describe WHAT stay and
      // the ones that describe WHERE go: sending the same ship to the same place
      // twice is almost never what was meant.
      action.reset({ keepReusable: true });
      if (isDocked(panel) || action.keepOpen()) {
        action.focus();
        return false;
      }
      layout?.set?.(layout.model.close(layout.state(), panel));
      return true;
    },
    /** Something is about to take the draft away — a reset, or a context change. */
    mayDiscard,
    /** Every open draft must be discardable before a layout reset may proceed. */
    mayReset() {
      return [...registered.keys()].filter(isOpen).every(mayDiscard);
    },
    /** Forget a draft — or every one, for `null` — after the operator agreed to
     * lose it. Separate from `mayDiscard` because agreeing and forgetting are
     * two steps: the caller asks first, then does whatever it was going to do,
     * then tells the drafts it happened. */
    discard(panel = null) {
      for (const [id, action] of registered) {
        if ((panel === null || panel === id) && isOpen(id)) action.reset({ keepReusable: false });
      }
    },
  };
}
