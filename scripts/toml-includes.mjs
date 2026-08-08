/**
 * scripts/toml-includes.mjs — read a top-level key through an entity
 * template's `includes` closure.
 *
 * `assets/entities/*.toml` compose: a hull declares `includes = [...]` and
 * inherits every top-level key it does not author itself
 * (`src/entities/include_resolve.rs`). A checker that reads only a file's OWN
 * keys therefore sees `class` on the four alliance hulls — which author it —
 * and nothing at all on a hull composed from fragments, even though Rust
 * resolves one and the client badges it. Issues #875 / #878 migrate the
 * shipped hulls to composition, so that blind spot is on the path the data is
 * taking.
 *
 * Precedence mirrors `include_resolve`'s merge order exactly:
 * `((a's closure) ⊕ (b's closure)) ⊕ self`. For a single scalar key that is:
 *
 *   1. the file's own value, if it authors one;
 *   2. otherwise the LAST include (in declared order) whose own closure yields
 *      one — "includes merge in declared order, so the later one wins";
 *   3. otherwise nothing.
 *
 * Reading is injected rather than imported so this module stays pure and the
 * closure semantics can be tested against a fake tree.
 */

/** A TOML table header on a line of its own — everything after it is nested. */
const TABLE_HEADER = /^\[\[?[^\]]+\]\]?$/;

/** Strip a `#` comment, honouring quoted strings so a `#` inside one survives. */
function stripComment(line) {
  let quoted = false;
  for (let i = 0; i < line.length; i += 1) {
    const c = line[i];
    if (c === '"') quoted = !quoted;
    else if (c === '#' && !quoted) return line.slice(0, i);
  }
  return line;
}

/**
 * The value of a top-level `key = "…"`, before any table header.
 *
 * @param {string} src TOML source
 * @param {string} key
 * @returns {{ value: string, lineNo: number } | null}
 */
export function topLevelString(src, key) {
  let lineNo = 0;
  for (const raw of src.split('\n')) {
    lineNo += 1;
    const line = stripComment(raw).trim();
    if (line === '') continue;
    if (TABLE_HEADER.test(line)) return null;
    const m = line.match(/^([A-Za-z0-9_-]+)\s*=\s*"([^"]*)"/);
    if (m && m[1] === key) return { value: m[2], lineNo };
  }
  return null;
}

/**
 * The template paths in a top-level `includes = [...]`, in declared order.
 *
 * Handles both the one-line form and the multi-line form the shipped hulls
 * use. An `includes` inside a table is not the composition key and is ignored.
 *
 * @param {string} src TOML source
 * @returns {string[]}
 */
export function topLevelIncludes(src) {
  const out = [];
  let collecting = false;
  for (const raw of src.split('\n')) {
    const line = stripComment(raw).trim();
    if (line === '') continue;
    if (!collecting) {
      if (TABLE_HEADER.test(line)) return out;
      if (!/^includes\s*=/.test(line)) continue;
      collecting = true;
      // Fall through so a single-line `includes = ["a.toml"]` is read here.
    }
    for (const m of line.matchAll(/"([^"]*)"/g)) out.push(m[1]);
    if (line.includes(']')) return out;
  }
  return out;
}

/**
 * Resolve an include reference against the directory of the file declaring it.
 *
 * Include paths are relative to the DECLARING template, not to the root hull,
 * and `..` is collapsed against that directory — see
 * `include_paths_resolve_relative_to_the_declaring_template` and
 * `a_nested_fragment_resolves_its_own_includes_relative_to_itself`.
 *
 * @param {string} fromFile path of the file that declared the include
 * @param {string} reference the authored include path
 */
export function resolveInclude(fromFile, reference) {
  const parts = fromFile.replace(/\\/g, '/').split('/');
  parts.pop();
  for (const segment of reference.replace(/\\/g, '/').split('/')) {
    if (segment === '' || segment === '.') continue;
    else if (segment === '..') parts.pop();
    else parts.push(segment);
  }
  return parts.join('/');
}

/**
 * Read a top-level scalar key through `start`'s include closure.
 *
 * @param {string} start path of the template to resolve
 * @param {string} key the top-level key to read
 * @param {(file: string) => Promise<string | null>} read
 *   returns the file's source, or null when it does not exist — a dangling
 *   include is `world::validate`'s finding to report, not this module's.
 * @param {Set<string>} [seen] cycle guard; a template already on the stack
 *   contributes nothing rather than recursing forever.
 * @returns {Promise<{ value: string, file: string, lineNo: number } | null>}
 */
export async function resolveThroughIncludes(start, key, read, seen = new Set()) {
  if (seen.has(start)) return null;
  seen.add(start);

  const src = await read(start);
  if (src == null) return null;

  const own = topLevelString(src, key);
  if (own) return { value: own.value, file: start, lineNo: own.lineNo };

  // Later includes override earlier ones, so the last one that yields a value
  // is the one the resolved document ends up with.
  const includes = topLevelIncludes(src);
  for (let i = includes.length - 1; i >= 0; i -= 1) {
    const found = await resolveThroughIncludes(
      resolveInclude(start, includes[i]),
      key,
      read,
      seen,
    );
    if (found) return found;
  }
  return null;
}
