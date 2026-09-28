/** The omniscient view of a disposable Workshop Test.
 *
 * The ORDINARY GM workspace over the ORDINARY GM markup, fed by the ordinary
 * `gm_*` Host Channel projections of the run that is already going. Nothing
 * here is a Workshop-only rendering of GM state: a substitute would be a second
 * surface to keep correct, and would stop being evidence about the real one the
 * moment it drifted.
 *
 * READ ONLY, and provably so. A GM action reaches the simulation through a
 * `window.__host*` binding that a live GM page installs over its admitted
 * command route. This page installs those names as REFUSALS instead. Leaving
 * them undefined would crash a panel on the first press and would leave "is
 * there a mutation path here?" to be answered by reading every panel; a stub
 * that refuses answers it in one place, and a Test that somehow acquired a
 * route would fail the assertion rather than quietly work.
 */
import { mountGmWorkspace } from './gm-workspace.js';
import { createHostChannel } from './host-channel.js';
import { t, applyToDom } from './strings.js';

import { GM_ACTION_NAMES, refusingGmActionAdapter, installGmActionBindings } from './gm-action-bindings.js';

export const TEST_GM_REFUSED_ACTIONS = GM_ACTION_NAMES;

export function mountWorkshopTestGm({ win = window, doc = win.document } = {}) {
  const root = doc.getElementById('gm-console');
  if (!root) return null;
  win.__phoenixGmPage = true;
  // The reused host markup carries launch chrome whose own boot never runs here.
  for (const id of ['landing-panel', 'scenario-panel', 'wasm-spinner']) {
    doc.getElementById(id)?.remove();
  }
  const refused = () => { throw new Error(t('workshop.test_gm_read_only')); };
  const bindings = installGmActionBindings(win, refusingGmActionAdapter(refused));
  // A Test has no operator identity, because it admits nothing.
  const noOperator = () => null;
  win.__hostLocalGm = noOperator;

  applyToDom(doc);
  // Isolated: this page may read nothing its capture did not hand it. The
  // ordinary mount would fetch the private-feedback manifest, the cue catalogue
  // and their samples from the server — project assets outside the capture —
  // which is the one thing a disposable Test must never do.
  const workspace = mountGmWorkspace({ win, doc, isolated: true });
  const dispatch = createHostChannel({ handlers: workspace.handlers, strings: { t } });
  return {
    /** Fold one `gm_*` projection of the running Test. */
    channel(name, payload) { dispatch(name, payload); },
    /** Exact role/widget descriptors parsed from this captured run, through
     * the ordinary GM role-preset and widget controllers. */
    setRolePresets(payload) { workspace.setRolePresets(payload); },
    rolePresetState() { return workspace.rolePresetState(); },
    dispose() {
      workspace.dispose?.();
      bindings.dispose();
      if (win.__hostLocalGm === noOperator) {
        delete win.__hostLocalGm;
        delete win.__phoenixGmPage;
      }
    },
  };
}
