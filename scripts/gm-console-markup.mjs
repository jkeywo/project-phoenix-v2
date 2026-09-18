// Extract the GM console subtree from server.html, for the disposable Test page.
//
// ONE source of GM markup. The Test's omniscient view has to be the ordinary GM
// workspace mounted over the ordinary markup — a Workshop-only substitute would
// be a second surface to keep correct, and would stop being evidence about the
// real one the moment it drifted. The native GM does the same thing at runtime
// (src/native_host/native_gm/document.rs); this does it at build time, because
// a Test page must not fetch anything it was not given.
export function gmConsoleMarkup(hostHtml) {
  const open = hostHtml.indexOf('<main id="gm-console"');
  if (open < 0) throw new Error('server.html has no #gm-console to extract');
  // Balance <main> tags: the console contains nested sections but no nested
  // <main>, so counting that one tag is enough and is honest about what it
  // assumes.
  let depth = 0;
  let index = open;
  for (;;) {
    const nextOpen = hostHtml.indexOf('<main', index + 1);
    const nextClose = hostHtml.indexOf('</main>', index + 1);
    if (nextClose < 0) throw new Error('server.html has an unbalanced #gm-console');
    if (nextOpen >= 0 && nextOpen < nextClose) { depth += 1; index = nextOpen; continue; }
    if (depth === 0) {
      const markup = hostHtml.slice(open, nextClose + '</main>'.length);
      if (/<script/i.test(markup)) throw new Error('#gm-console must carry no script');
      return markup;
    }
    depth -= 1;
    index = nextClose;
  }
}
