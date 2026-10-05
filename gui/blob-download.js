/** Start a download and release its temporary DOM and URL after the click. */
export function downloadBlob(doc, view, name, blob) {
  const href = view.URL.createObjectURL(blob);
  let anchor;
  try {
    anchor = doc.createElement('a');
    anchor.href = href;
    anchor.download = name;
    anchor.style.display = 'none';
    doc.body.appendChild(anchor);
    anchor.click();
  } finally {
    anchor?.remove();
    if (typeof view.setTimeout === 'function') {
      view.setTimeout(() => {
        try { view.URL.revokeObjectURL(href); }
        catch (_) { /* the download already took it */ }
      }, 0);
    }
  }
}
