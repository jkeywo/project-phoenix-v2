//! The authored join-code table, read by a host that ISSUES codes
//! (issue #1353).
//!
//! # Why this exists at all, given what `core::rendezvous` says
//!
//! [`crate::core::rendezvous`] states plainly that a native host does no
//! minting, no parsing and no normalisation, because a host is *issued* a code
//! by the rendezvous service and only ever prints it. That was true of every
//! native host up to #1353, and it is still true of the one that dials out to
//! the cloud service with `--rendezvous`.
//!
//! A host that accepts LAN joins **directly** is the service for its own game
//! (see [`crate::native_host::direct_join`]), and a service is exactly the
//! party that mints the code and resolves the one a joiner types. So the two
//! operations `core::rendezvous` refused to grow live here instead — in the
//! native-only module that needs them, behind the `host` feature, where the
//! shared frame vocabulary cannot pick them up.
//!
//! # It is a READER, not a second table
//!
//! Nothing about the scheme is written down in Rust. The alphabet, the
//! confusable map, the strip set, the deny-list, both project GUIDs, the
//! release GUID and every bound are read from `assets/join/join-codes.toml`,
//! which that file's own header names as "the ONE place the join-identifier
//! scheme is written down". `gui/join-code.js` reads the generated JSON twin of
//! the same table and `worker-rendezvous/src/registry.js` bundles it; this is
//! the third reader, and a designer retuning the table retunes all three
//! without a native release.
//!
//! What IS mirrored is the small amount of *logic* around the data — the
//! canonicalisation order, the `PROJECT_VERSION_SUFFIX` composition, the
//! structured-vs-suffix shape test, and the lookup's three typed failures. Each
//! of those is a handful of lines against `gui/join-code.js` and
//! `worker-rendezvous/src/registry.js`'s `lookup()`, and each is named in the
//! doc comment of the function that mirrors it.
//!
//! # `[ai]` — why the host resolves rather than parses
//!
//! A single-game host holds exactly ONE record, so it never needs the
//! registry's map: "is this the code I minted" answers every question the
//! registry answers with a lookup, and answers it with the same three reasons
//! (`wrong-type`, `version-mismatch`, `unknown`) because those three are
//! decided by comparing a typed code's parts against a record's, which is what
//! [`JoinCodeTable::resolve`] does against its one record.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

/// The table revision this module reads. The Rust twin of
/// `JOIN_CODE_FORMAT_VERSION` in `gui/join-code.js`, which likewise refuses a
/// table it does not implement rather than reading fields that may have moved.
pub const JOIN_CODE_FORMAT_VERSION: u32 = 1;

/// The typed namespace a phone or console joins in.
pub const NAMESPACE_CLIENT: &str = "client";
/// The typed namespace a ship host joins a fleet in (issue #1114).
pub const NAMESPACE_SERVER: &str = "server";

/// The separator between a full code's three parts — `PART_SEPARATOR` in
/// `gui/join-code.js`, where it is also one of the authored `strip` characters.
const PART_SEPARATOR: char = '_';

/// The authored table, as it is on disk.
#[derive(Debug, Deserialize)]
struct RawTable {
    format_version: u32,
    suffix: RawSuffix,
    namespaces: BTreeMap<String, String>,
    version: RawVersion,
    #[serde(default)]
    limits: RawLimits,
    #[serde(default)]
    deny: Vec<RawDeny>,
}

#[derive(Debug, Deserialize)]
struct RawSuffix {
    length: usize,
    alphabet: String,
    #[serde(default)]
    strip: String,
    #[serde(default)]
    normalise: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct RawVersion {
    guid: String,
}

#[derive(Debug, Deserialize)]
struct RawDeny {
    word: String,
}

/// The authored bounds this host applies, with the same parse-time defaults
/// `worker-rendezvous/src/registry.js` and `src/relay.js` carry, so an older
/// table still loads.
#[derive(Clone, Debug, Deserialize)]
pub struct RawLimits {
    #[serde(default = "default_max_lookups")]
    pub max_lookups_per_connection: usize,
    #[serde(default = "default_max_peers")]
    pub max_peers_per_record: usize,
    #[serde(default = "default_max_code_length")]
    pub max_code_length: usize,
    #[serde(default = "default_max_relay_peers")]
    pub max_relay_peers_per_record: usize,
    #[serde(default = "default_max_relay_frame_bytes")]
    pub max_relay_frame_bytes: usize,
    #[serde(default = "default_max_relay_queue_reliable")]
    pub max_relay_queue_reliable: usize,
    #[serde(default = "default_max_relay_queue_snapshot")]
    pub max_relay_queue_snapshot: usize,
    #[serde(default = "default_max_relay_send_buffer_bytes")]
    pub max_relay_send_buffer_bytes: usize,
}

fn default_max_lookups() -> usize {
    60
}
fn default_max_peers() -> usize {
    32
}
fn default_max_code_length() -> usize {
    160
}
fn default_max_relay_peers() -> usize {
    8
}
fn default_max_relay_frame_bytes() -> usize {
    262144
}
fn default_max_relay_queue_reliable() -> usize {
    256
}
fn default_max_relay_queue_snapshot() -> usize {
    32
}
fn default_max_relay_send_buffer_bytes() -> usize {
    262144
}

impl Default for RawLimits {
    fn default() -> Self {
        Self {
            max_lookups_per_connection: default_max_lookups(),
            max_peers_per_record: default_max_peers(),
            max_code_length: default_max_code_length(),
            max_relay_peers_per_record: default_max_relay_peers(),
            max_relay_frame_bytes: default_max_relay_frame_bytes(),
            max_relay_queue_reliable: default_max_relay_queue_reliable(),
            max_relay_queue_snapshot: default_max_relay_queue_snapshot(),
            max_relay_send_buffer_bytes: default_max_relay_send_buffer_bytes(),
        }
    }
}

/// The authored table, ready to mint and resolve against.
#[derive(Clone, Debug)]
pub struct JoinCodeTable {
    suffix_length: usize,
    alphabet: Vec<char>,
    strip: BTreeSet<char>,
    normalise: BTreeMap<char, char>,
    /// Deny-list entries, already canonicalised — so `HELLO`, `HEIIO` and
    /// `he110` are one entry, exactly as `deniedSuffixes` folds them.
    denied: BTreeSet<String>,
    client_project: String,
    server_project: String,
    version: String,
    /// The authored bounds. Public because the direct service applies them.
    pub limits: RawLimits,
}

/// Why a typed code is not this host's.
///
/// The variants are spelled with the service's own machine reasons, because
/// they travel on an `error` frame and `gui/join-code.js`'s `reasonStringId`
/// maps each to a `strings.csv` id. A reason this host invented would render as
/// the misleading `unknown` fallback on the phone.
pub type CodeRefusal = &'static str;

impl JoinCodeTable {
    /// Read and check `path` (normally `assets/join/join-codes.toml`).
    ///
    /// A table this build does not implement is an error rather than a partial
    /// read, the same answer `checkJoinCodeFormat` gives: a host that minted a
    /// code from half a schema would print five letters no phone can resolve.
    pub fn read(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read the join-code table {}: {e}", path.display()))?;
        Self::parse(&text)
            .map_err(|e| format!("the join-code table {} is unusable: {e}", path.display()))
    }

    /// Parse authored TOML. Separate from [`Self::read`] so the whole of the
    /// scheme is testable without a file.
    pub fn parse(toml_text: &str) -> Result<Self, String> {
        let raw: RawTable = toml::from_str(toml_text).map_err(|e| e.to_string())?;
        if raw.format_version != JOIN_CODE_FORMAT_VERSION {
            return Err(format!(
                "format_version {} is not the {JOIN_CODE_FORMAT_VERSION} this build reads",
                raw.format_version
            ));
        }
        let client_project = raw
            .namespaces
            .get(NAMESPACE_CLIENT)
            .cloned()
            .ok_or_else(|| "no [namespaces] client project GUID".to_string())?;
        let server_project = raw
            .namespaces
            .get(NAMESPACE_SERVER)
            .cloned()
            .unwrap_or_default();
        let normalise = raw
            .suffix
            .normalise
            .iter()
            .filter_map(|(from, to)| Some((from.chars().next()?, to.chars().next()?)))
            .collect();
        let mut table = Self {
            suffix_length: raw.suffix.length,
            alphabet: raw.suffix.alphabet.chars().collect(),
            strip: raw.suffix.strip.chars().collect(),
            normalise,
            denied: BTreeSet::new(),
            client_project,
            server_project,
            version: raw.version.guid,
            limits: raw.limits,
        };
        if table.suffix_length == 0 || table.alphabet.is_empty() {
            return Err("the [suffix] block authors no length or no alphabet".to_string());
        }
        table.denied = raw
            .deny
            .iter()
            .map(|entry| table.canonicalise(&entry.word))
            .collect();
        Ok(table)
    }

    /// This build's release GUID — what a minted code's middle part carries.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The project GUID for a namespace name, or `None` — `projectGuidFor`.
    pub fn project_for(&self, namespace: &str) -> Option<&str> {
        match namespace {
            NAMESPACE_CLIENT => Some(&self.client_project),
            NAMESPACE_SERVER if !self.server_project.is_empty() => Some(&self.server_project),
            _ => None,
        }
    }

    /// The namespace a project GUID belongs to, or `None` — `namespaceOf`.
    pub fn namespace_of(&self, project: &str) -> Option<&'static str> {
        let guid = project.to_ascii_lowercase();
        if guid == self.client_project.to_ascii_lowercase() {
            Some(NAMESPACE_CLIENT)
        } else if !self.server_project.is_empty()
            && guid == self.server_project.to_ascii_lowercase()
        {
            Some(NAMESPACE_SERVER)
        } else {
            None
        }
    }

    /// Fold typed input to the one spelling a record is stored under.
    ///
    /// Uppercase, drop the authored strip characters, then apply the authored
    /// confusable map — in that order, which is `canonicaliseSuffix`'s order and
    /// is load-bearing: `strip` contains `_`, and the map turns `0`/`1`/`L` into
    /// `O`/`I`/`I` only after the case fold. Characters outside the alphabet
    /// survive, so [`Self::validate_suffix`] can tell "you typed a digit" from
    /// "you typed too few letters".
    pub fn canonicalise(&self, raw: &str) -> String {
        let mut out = String::new();
        for ch in raw.to_uppercase().chars() {
            if self.strip.contains(&ch) {
                continue;
            }
            out.push(*self.normalise.get(&ch).unwrap_or(&ch));
        }
        out
    }

    /// Judge typed suffix input — `validateSuffix`, reason for reason and in
    /// the same order (`empty`, `length`, `charset`, `denied`).
    pub fn validate_suffix(&self, raw: &str) -> Result<String, CodeRefusal> {
        let suffix = self.canonicalise(raw);
        if suffix.is_empty() {
            return Err("empty");
        }
        if suffix.chars().count() != self.suffix_length {
            return Err("length");
        }
        if suffix.chars().any(|ch| !self.alphabet.contains(&ch)) {
            return Err("charset");
        }
        if self.denied.contains(&suffix) {
            return Err("denied");
        }
        Ok(suffix)
    }

    /// `PROJECT_VERSION_SUFFIX` — `composeJoinCode`.
    pub fn compose(&self, project: &str, version: &str, suffix: &str) -> String {
        format!("{project}{PART_SEPARATOR}{version}{PART_SEPARATOR}{suffix}")
    }

    /// Draw a fresh suffix from the authored alphabet, skipping the deny-list.
    ///
    /// `draw(n)` yields an index below `n` — injected rather than reached for,
    /// so the mint is testable without a scripted RNG global, exactly as
    /// `mintSuffix`'s `randomInt` seam is. Nothing here touches the simulation's
    /// RNG: a join code is transport-plane and must never draw from a stream a
    /// replay depends on.
    ///
    /// `None` only when every attempt landed on a denied word, which for a
    /// 25^5 space and a deny-list of tens of entries means a broken `draw`.
    pub fn mint_suffix(&self, mut draw: impl FnMut(usize) -> usize) -> Option<String> {
        // The same ceiling `mintSuffix` uses, and for the same reason: a draw
        // that keeps landing on denied words must end in an answer rather than
        // in a loop nobody can see.
        const MAX_ATTEMPTS: usize = 64;
        for _ in 0..MAX_ATTEMPTS {
            let suffix: String = (0..self.suffix_length)
                .map(|_| self.alphabet[draw(self.alphabet.len()) % self.alphabet.len()])
                .collect();
            if self.denied.contains(&suffix) {
                continue;
            }
            return Some(suffix);
        }
        None
    }

    /// Mint a whole crew code for this host: a fresh suffix in the `client`
    /// namespace, composed with this build's release GUID.
    pub fn mint_client_code(
        &self,
        draw: impl FnMut(usize) -> usize,
    ) -> Option<crate::core::rendezvous::JoinCode> {
        let suffix = self.mint_suffix(draw)?;
        Some(crate::core::rendezvous::JoinCode {
            full: self.compose(&self.client_project, &self.version, &suffix),
            suffix,
            project: self.client_project.clone(),
            version: self.version.clone(),
            namespace: NAMESPACE_CLIENT.to_string(),
        })
    }

    /// Is `typed` the code `record` names, and may `asked` namespace have it?
    ///
    /// The single-record collapse of `parseJoinCode` + the registry's
    /// `lookup()`. The three typed failures are kept apart exactly as the
    /// registry keeps them, because they are three different sentences on a
    /// phone: `wrong-type` ("that is the other kind of code"),
    /// `version-mismatch` ("that code is from another release") and `unknown`
    /// ("no ship is using that code").
    pub fn resolve(
        &self,
        typed: &str,
        asked: &str,
        record: &crate::core::rendezvous::JoinCode,
    ) -> Result<(), CodeRefusal> {
        // Bound before parsing, like `resolveRequest`: a megabyte of "code" is a
        // request to spend this host's CPU, not to join.
        if typed.len() > self.limits.max_code_length {
            return Err("malformed");
        }
        // A URL (a QR scan pasted whole) carries the code in its fragment.
        let text = typed.trim();
        let fragment = match text.split_once('#') {
            Some((_, after)) => after,
            None => text,
        };
        if fragment.trim().is_empty() {
            return Err("empty");
        }
        // The SHAPE is decided before any canonicalisation, because `_` is both
        // the separator and an authored strip character: `QU_ARK` is a player
        // spacing out five letters, while `<guid>_<guid>_QUARK` is a pasted
        // identifier, and folding first would report the former as malformed.
        let parts: Vec<&str> = fragment.split(PART_SEPARATOR).map(str::trim).collect();
        let structured = parts.len() == 3 && is_code_head(parts[0]) && is_code_head(parts[1]);

        let (project, version, raw_suffix) = if structured {
            (parts[0].to_string(), parts[1].to_string(), parts[2])
        } else {
            if parts.len() > 1 && parts.iter().any(|p| is_code_head(p)) {
                return Err("malformed");
            }
            // A bare suffix is composed under the namespace it was TYPED into
            // and this build's release — `joinCodeForSuffix`.
            let project = self.project_for(asked).ok_or("unknown-namespace")?;
            (project.to_string(), self.version.clone(), fragment)
        };
        let suffix = self.validate_suffix(raw_suffix)?;
        let namespace = self.namespace_of(&project).ok_or("unknown-project")?;

        let same_project = project.eq_ignore_ascii_case(&record.project);
        let same_version = version.eq_ignore_ascii_case(&record.version);
        let same_suffix = suffix == record.suffix;

        if same_project && same_version && same_suffix {
            // The record exists and is joinable; the only remaining question is
            // whether the ASKER is asking in the record's own namespace. A fleet
            // code is a perfectly good record, just not one a phone may attach
            // to (issue #1114's reason for the parameter).
            if namespace != record.namespace || asked != record.namespace {
                return Err("wrong-type");
            }
            return Ok(());
        }
        if same_suffix && !same_project {
            // The same five letters registered under the other typed namespace:
            // the operator typed a fleet code into the crew field, or the
            // reverse.
            return Err("wrong-type");
        }
        if same_suffix && same_project {
            return Err("version-mismatch");
        }
        Err("unknown")
    }
}

/// Is this part of a split input one of a full code's two GUID heads?
///
/// `CODE_HEAD` in `gui/join-code.js`, and deliberately as loose: hosts and tests
/// do register identifier-shaped versions that are not canonical GUIDs, but
/// nothing a five-letter code can be broken into is ever this long.
fn is_code_head(part: &str) -> bool {
    let mut chars = part.chars();
    // `^[0-9a-z]` under the `i` flag: any ASCII letter or digit.
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    // `[0-9a-z-]{7,}$`, i.e. at least eight characters in total.
    let rest = chars.as_str();
    rest.len() >= 7 && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A `draw` for [`JoinCodeTable::mint_suffix`] backed by the OS entropy pool.
///
/// The join code is the one secret a LAN game has — anybody who can reach the
/// delivery port and guess five letters is in the mission — so it is drawn from
/// `rand`'s OS-seeded generator rather than from a clock, and never from any
/// stream the simulation replays.
pub fn os_draw(n: usize) -> usize {
    use rand::Rng;
    rand::rng().random_range(0..n.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> JoinCodeTable {
        JoinCodeTable::read(std::path::Path::new("assets/join/join-codes.toml"))
            .expect("the authored table is checked in")
    }

    #[test]
    fn the_shipped_table_is_the_one_this_build_reads() {
        // The whole justification for a third reader: it must load the file the
        // designer edits, not a copy. A `format_version` bump lands here first.
        let t = table();
        assert_eq!(t.version().len(), 36, "an authored release GUID");
        assert_eq!(t.project_for(NAMESPACE_CLIENT).map(str::len), Some(36));
        assert_eq!(t.project_for(NAMESPACE_SERVER).map(str::len), Some(36));
        assert!(t.limits.max_relay_frame_bytes >= 65536);
    }

    #[test]
    fn a_table_from_another_revision_is_refused_rather_than_half_read() {
        let bad = "format_version = 99\n[suffix]\nlength = 5\nalphabet = \"AB\"\n\
                   [namespaces]\nclient = \"c\"\n[version]\nguid = \"v\"\n";
        assert!(JoinCodeTable::parse(bad).is_err());
    }

    #[test]
    fn canonicalisation_folds_exactly_what_the_client_folds() {
        // The confusables that make a code typeable off a viewscreen: `0`→`O`,
        // `1`→`I`, `L`→`I`, with `J` deliberately untouched, and the punctuation
        // a player adds while reading it aloud dropped.
        let t = table();
        assert_eq!(t.canonicalise("qu-ark"), "QUARK");
        assert_eq!(t.canonicalise("he110"), "HEIIO");
        assert_eq!(t.canonicalise("j o k e r"), "JOKER");
        assert_eq!(t.canonicalise("QU_ARK"), "QUARK");
    }

    #[test]
    fn a_denied_word_is_refused_in_every_confusable_spelling() {
        let t = table();
        assert_eq!(t.validate_suffix("ADMIN"), Err("denied"));
        assert_eq!(t.validate_suffix("ADM1N"), Err("denied"));
        assert_eq!(t.validate_suffix(""), Err("empty"));
        assert_eq!(t.validate_suffix("ABC"), Err("length"));
        assert_eq!(t.validate_suffix("ABC$E"), Err("charset"));
        assert_eq!(t.validate_suffix("quark"), Ok("QUARK".to_string()));
    }

    #[test]
    fn a_minted_code_is_one_the_clients_own_parser_would_accept() {
        // Composition is `PROJECT_VERSION_SUFFIX` and nothing else; the parts
        // are the authored GUIDs. Asserted here because a code that does not
        // round-trip through `parseJoinCode` is five letters nobody can use.
        let t = table();
        let mut n = 0;
        let code = t
            .mint_client_code(|len| {
                n += 1;
                n % len
            })
            .expect("a code is mintable");
        assert_eq!(code.suffix.chars().count(), 5);
        assert_eq!(code.namespace, NAMESPACE_CLIENT);
        assert_eq!(
            code.full,
            format!("{}_{}_{}", code.project, code.version, code.suffix)
        );
        assert!(!t.denied.contains(&code.suffix), "never a denied word");
    }

    #[test]
    fn the_mint_never_draws_a_denied_word_even_when_the_draw_insists() {
        // A `draw` pinned to the letters of a deny-listed word must not produce
        // it — `mintSuffix`'s `continue`, and the reason the ceiling exists.
        let t = table();
        let denied: Vec<char> = "ADMIN".chars().collect();
        let mut i = 0;
        let minted = t.mint_suffix(|_| {
            let ch = denied[i % denied.len()];
            i += 1;
            t.alphabet.iter().position(|c| *c == ch).unwrap()
        });
        assert!(
            minted.is_none(),
            "64 denied draws produce no code, not ADMIN"
        );
    }

    fn record(t: &JoinCodeTable) -> crate::core::rendezvous::JoinCode {
        crate::core::rendezvous::JoinCode {
            full: t.compose(
                t.project_for(NAMESPACE_CLIENT).unwrap(),
                t.version(),
                "QUARK",
            ),
            suffix: "QUARK".to_string(),
            project: t.project_for(NAMESPACE_CLIENT).unwrap().to_string(),
            version: t.version().to_string(),
            namespace: NAMESPACE_CLIENT.to_string(),
        }
    }

    #[test]
    fn the_full_code_a_qr_carries_resolves_and_so_does_the_bare_suffix() {
        // The two routes into the same record: a scanned QR sends the whole
        // structured code, and a guest reading the viewscreen types five
        // letters. Both must land on the same answer or half the room cannot
        // join.
        let t = table();
        let rec = record(&t);
        assert_eq!(t.resolve(&rec.full, NAMESPACE_CLIENT, &rec), Ok(()));
        assert_eq!(t.resolve("QUARK", NAMESPACE_CLIENT, &rec), Ok(()));
        assert_eq!(t.resolve("qu-ark", NAMESPACE_CLIENT, &rec), Ok(()));
        assert_eq!(
            t.resolve(
                &format!("http://host/client/index.html#{}", rec.full),
                NAMESPACE_CLIENT,
                &rec
            ),
            Ok(()),
            "a whole scanned URL carries its code in the fragment"
        );
    }

    #[test]
    fn the_three_typed_refusals_stay_three_answers() {
        // What the registry's `lookup` keeps apart, kept apart here: they are
        // three different sentences on a phone, and collapsing them sends a
        // guest back to re-type five letters that were already right.
        let t = table();
        let rec = record(&t);
        assert_eq!(t.resolve("XYZAB", NAMESPACE_CLIENT, &rec), Err("unknown"));
        let other_release = t.compose(
            &rec.project,
            "00000000-0000-4000-8000-000000000000",
            "QUARK",
        );
        assert_eq!(
            t.resolve(&other_release, NAMESPACE_CLIENT, &rec),
            Err("version-mismatch")
        );
        let fleet = t.compose(
            t.project_for(NAMESPACE_SERVER).unwrap(),
            t.version(),
            "QUARK",
        );
        assert_eq!(t.resolve(&fleet, NAMESPACE_CLIENT, &rec), Err("wrong-type"));
        // A phone asking in the fleet namespace for this crew record is the
        // same refusal from the other direction (issue #1114's parameter).
        assert_eq!(
            t.resolve(&rec.full, NAMESPACE_SERVER, &rec),
            Err("wrong-type")
        );
    }

    #[test]
    fn a_code_that_is_not_a_code_is_refused_before_any_lookup() {
        let t = table();
        let rec = record(&t);
        assert_eq!(t.resolve("", NAMESPACE_CLIENT, &rec), Err("empty"));
        assert_eq!(t.resolve("ABC", NAMESPACE_CLIENT, &rec), Err("length"));
        assert_eq!(
            t.resolve(
                &"A".repeat(t.limits.max_code_length + 1),
                NAMESPACE_CLIENT,
                &rec
            ),
            Err("malformed"),
            "a bounded identifier, bounded before it is parsed"
        );
        // A stale `#<32 hex peer id>` bookmark from the PeerJS era.
        assert_eq!(
            t.resolve("0123456789abcdef0123456789abcdef", NAMESPACE_CLIENT, &rec),
            Err("length")
        );
        // Two GUID-shaped parts and no third is a paste that went wrong, not a
        // spaced-out suffix.
        assert_eq!(
            t.resolve(
                &format!("{}_{}", rec.project, rec.version),
                NAMESPACE_CLIENT,
                &rec
            ),
            Err("malformed")
        );
    }

    #[test]
    fn a_head_is_told_apart_from_a_player_spacing_out_five_letters() {
        assert!(is_code_head("2f6b0a11-9c4e-4d7a-8f31-5b90c2d47e18"));
        assert!(is_code_head("release-1"));
        assert!(!is_code_head("QU"));
        assert!(!is_code_head("ARK"));
        assert!(!is_code_head(""));
        assert!(!is_code_head("has space"));
    }
}
