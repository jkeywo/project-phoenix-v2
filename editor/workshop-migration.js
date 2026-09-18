/** Reviewing a proposed change to older supported content before accepting it.
 *
 * The one such change that exists today is the content pin. A pack declares
 * `[pack.requires] content_id` and `content_epoch`, and the runtime REFUSES a
 * pack pinned to an epoch the installed base content has moved past
 * (`src/world/mod_pack.rs`) — with no remediation. An author's only recourse is
 * to hand-edit the manifest, which is exactly the kind of blind edit the
 * Workshop exists to replace.
 *
 * So this proposes, it never applies. It produces the exact before and after
 * text for the member it would rewrite, and acceptance is the caller's — as one
 * undoable history entry, because half an accepted migration is source nobody
 * reviewed.
 *
 * Deliberately narrow: it rewrites ONE scalar in one member, leaving every
 * comment, key order and line ending around it untouched. A migration that
 * reserialised the manifest would be a migration that silently normalised
 * source the author never asked it to touch.
 */

/** The pin a pack declares, read without parsing the whole manifest. */
// `.` does not match a carriage return in JavaScript, so a CRLF manifest needs
// the comment class spelled out and the trailing \r absorbed by \s — otherwise
// every pin in a CRLF pack reads as absent.
const EPOCH = /^(\s*content_epoch\s*=\s*)(-?\d+)(\s*(?:#[^\r\n]*)?\s*)$/;
const CONTENT_ID = /^\s*content_id\s*=\s*"([^"]*)"/;

/**
 * Propose a migration for one draft against the base content it must run on.
 *
 * Returns `null` when there is nothing to propose — which is the ordinary case
 * and must stay silent rather than nagging.
 */
export function proposeContentPinMigration(source, base) {
  // `base` is null until the dependency snapshot has been read, which is most
  // of a session: destructuring it would make "not known yet" a crash instead
  // of the silence it should be.
  const contentId = base?.contentId ?? null;
  const contentEpoch = base?.contentEpoch;
  if (typeof source !== 'string' || typeof contentEpoch !== 'number') return null;
  const lines = source.split('\n');
  let declaredId = null;
  let epochLine = -1;
  let declaredEpoch = null;
  lines.forEach((line, index) => {
    const id = CONTENT_ID.exec(line);
    if (id && declaredId === null) declaredId = id[1];
    const epoch = EPOCH.exec(line);
    if (epoch && epochLine < 0) { epochLine = index; declaredEpoch = Number(epoch[2]); }
  });
  if (epochLine < 0 || declaredEpoch === null) return null;
  // A pack pinned to DIFFERENT content is not out of date, it is for something
  // else. Re-pinning it would quietly claim it works here.
  if (contentId != null && declaredId !== null && declaredId !== contentId) return null;
  if (declaredEpoch === contentEpoch) return null;
  // Only forward. A pack pinned ahead of the installed content is a pack built
  // against something newer, and moving it backwards would lose the authored
  // intent rather than repair it.
  if (declaredEpoch > contentEpoch) return null;

  const after = [...lines];
  after[epochLine] = lines[epochLine].replace(EPOCH, `$1${contentEpoch}$3`);
  return {
    kind: 'content-epoch',
    from: declaredEpoch,
    to: contentEpoch,
    line: epochLine + 1,
    before: source,
    after: after.join('\n'),
  };
}

/** The member a content-pin migration rewrites. */
export const MIGRATION_MEMBER = 'scenarios.toml';

/**
 * Propose whatever migration this draft needs, or `null`.
 *
 * One entry point so a caller never has to know which migrations exist; adding
 * a second kind changes this function and nothing that calls it.
 */
export function proposeWorkshopMigration(draft, base) {
  if (!draft || typeof draft.read !== 'function') return null;
  const source = draft.read(MIGRATION_MEMBER);
  const proposed = proposeContentPinMigration(source, base);
  return proposed ? { ...proposed, path: MIGRATION_MEMBER } : null;
}
