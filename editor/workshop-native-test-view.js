/** Native Test presentation only. A fresh document belongs to each child run;
 * no simulation, credentials or source are loaded into this iframe. */
export function createNativeWorkshopTestView({ mount, title = '' }) {
  const doc = mount.ownerDocument;
  let frame = null, identity = null, latest = null;
  const clear = () => { frame?.remove(); frame = null; identity = latest = null; };
  const publish = () => {
    if (frame && latest) frame.contentWindow?.postMessage({ type: 'phoenix-native-test-view', run: latest }, doc.location.origin);
  };
  return {
    update(run) {
      if (!run?.running || !run.frame_url || !run.presentation_url) { clear(); return; }
      const next = run.presentation_url;
      if (next !== identity) {
        clear(); identity = next;
        const current = doc.createElement('iframe');
        current.title = title;
        current.className = 'workshop-test-viewscreen';
        current.src = new URL('../workshop-native-test.html', import.meta.url).href;
        current.addEventListener('load', () => { if (frame === current) publish(); });
        frame = current; mount.append(current);
      }
      latest = { running: true, frame_url: run.frame_url, presentation_url: run.presentation_url, view: run.view };
      publish();
    },
    dispose: clear,
  };
}
