//! GM role presets and typed mission widgets read from exact source
//! (issue #1477).
//!
//! A `[[gm_role_preset]]` block is WORLD content — the same document family
//! #1475's `extra_worlds` edits — so this module reads and edits members of
//! `assets/worlds/`. It is presentation-only content: a preset is personal
//! browser narrowing a Game Master selects for themselves, and a widget
//! selects among four surfaces this build already draws. Nothing authored here
//! reaches `GmOperator`, a `GmAction`, a snapshot or the sim digest, which is
//! proved rather than asserted by `tests/gm_role_preset_digest_neutrality.rs`.
//!
//! # The runtime owns every rule it has, and this module invents none
//!
//! `GmRolePresetWidget::validate` already refuses an empty id or label, an
//! unknown `type`, a key that belongs to another type, an unknown band or
//! category, an empty ship, an unknown or repeated GM action id and a note
//! whose text is not a String Table id. Its whole-file caller
//! (`world::config::parse_world`) never knew a LINE, and criterion 2 asks for
//! exact source locations — so this module supplies the location and asks the
//! runtime for the judgement:
//!
//! * every widget rule is decided by handing `validate` a PROBE widget that
//!   declares one authored facet and nothing else, and the sentence it returns
//!   is the message a finding and a refusal carry, word for word;
//! * which facets a type OWNS is that same question asked with a value the
//!   runtime accepts, so the ownership table is never copied here either;
//! * `GM_WIDGET_TYPES`, `GM_WIDGET_ACTION_IDS`, `GM_ROLE_PRESET_ALL_ID`,
//!   `GmAttentionBand::from_authored` and `GmAttentionCategory::from_authored`
//!   decide WHICH rule a violation is, so the finding category and the
//!   runtime's own sentence cannot describe different rules.
//!
//! Six sentences are written HERE, for want of a runtime function to ask, and
//! they divide in two:
//!
//! * FOUR repeat `parse_world`'s own words: `preset-empty-id`,
//!   `preset-reserved-id`, `preset-duplicate-id` and `widget-duplicate-id`.
//!   `parse_world` builds those inline in its own loop and returns the first,
//!   so there is no per-entry predicate to call — and its duplicate sentences
//!   name BOTH indices, while a finding points at one line and names one. They
//!   are pinned to the runtime's wording by
//!   `the_preset_sentences_are_pinned_to_parse_worlds_own_words`: a reword in
//!   `world::config` fails that test rather than drifting silently, which is
//!   the guard the widget sentences get for free by being the runtime's.
//! * TWO are Workshop-owned because the runtime has no such rule:
//!   `preset-empty-label` (a preset with no heading is a row an operator cannot
//!   read, and the form authors one) and the reference rules below.
//!
//! # References the runtime cannot check and this module can
//!
//! A widget's `ship` and a preset's `contacts` name world entities by their
//! `[[entity]] name`, exactly as `world::validate::declared_names` reads them.
//! `parse_world` sees one world's text and cannot resolve them; the Workshop
//! holds the whole candidate, so an unknown name is an ERROR with a line here.
//! The set is the selected world's own names plus those of the worlds its
//! `extra_worlds` compose, resolved over candidate ∪ dependencies.
//!
//! These two are the Workshop's own judgement, because the runtime's
//! declarative cross-reference checks are vacuous:
//! `world::validate::collect_entity_references` was emptied when #985 deleted
//! the `[[trigger]]` front-end, and a scripted world's references are resolved
//! by `world::script::validate` instead. An authoring form that offers the
//! world's own entity names should say when an authored one resolves to nothing,
//! and the finding names the LINE so an author sees which reference was judged.
//!
//! But the finding is an ERROR, so it refuses save and export — and refusing
//! content the GAME would load is worse than missing a typo. A name a script
//! mints with `spawn_entity` is authored correctly and appears in no
//! `[[entity]]` block, so [`scripted_names`] counts those too: in a script text
//! that calls `spawn_entity` at all, every `name:` literal is a name that world
//! may mint. The set therefore errs toward ACCEPTING a reference, which is the
//! direction a gate that blocks an export has to err in, while a typo in a world
//! whose scripts never spawn is still caught. A name assembled from a variable
//! is the one case left that nothing structural can see.
//!
//! # The vocabulary this module deliberately does NOT hold
//!
//! `panels`, `quick_actions` and `contacts` are open string vocabularies in
//! Rust on purpose: a preset may already name a panel this build does not draw
//! yet. The list of panel ids and quick-action ids the build DRAWS lives in
//! the browser (`gui/gm-role-presets.js`), and it stays there. This module
//! reports no `preset-panel-not-drawn` / `preset-quick-action-not-drawn`
//! warning and offers no panel or quick-action choice: the panel raises those
//! two warnings from its own vocabulary and renders them in the same findings
//! list. Rust's catalog carries only what Rust owns (widget types, widget
//! action ids, bands, categories, and the world's own entity names).
//!
//! # Refusals
//!
//! Like #1475 and #1476, an edit is REFUSED at edit time with the source
//! untouched: [`compose`] applies the exact-source edit to a COPY, re-reads the
//! preset rules over it, and refuses only what the edit INTRODUCES — a multiset
//! of (category, offending value), so a hand-broken draft can be repaired one
//! edit at a time and a reorder that moves an existing violation is not read as
//! a new one. The same rules are findings with lines in `validate_pack` and
//! `validate_project`, because a hand-edited draft can still carry them and
//! save and export must refuse with a location.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use toml_edit::{ArrayOfTables, Document, DocumentMut, Item, Table, TableLike, Value};

use super::document::{self, EditRequest};
use super::source_spans::{is_member, scalar_at, span_line, string_field, table_line, value_text};
use super::{WorkshopDependencies, WorkshopFinding};
use crate::gm_attention::{GmAttentionBand, GmAttentionCategory};
use crate::world::config::{
    parse_world, GmRolePresetWidget, GM_ROLE_PRESET_ALL_ID, GM_WIDGET_ACTION_IDS, GM_WIDGET_TYPES,
};

const WORLDS: &str = "assets/worlds/";
const ORIGIN_DRAFT: &str = "draft";
const ORIGIN_BASE: &str = "base";
/// The world key holding the presets, and the nested key holding one preset's
/// widgets — the array-of-tables spellings `GmRolePresetEntry` reads.
const PRESETS_KEY: &str = "gm_role_preset";
const WIDGETS_KEY: &str = "widget";
/// Keys `GmRolePresetEntry` reads; anything else is preserved and shown
/// read-only, because the type has no `deny_unknown_fields`.
const PRESET_KEYS: [&str; 6] = [
    "id",
    "label",
    "panels",
    "quick_actions",
    "contacts",
    "widget",
];
/// Keys `GmRolePresetWidget` reads (`type` is the TOML spelling of `kind`).
const WIDGET_KEYS: [&str; 8] = [
    "id", "type", "label", "band", "category", "ship", "actions", "text",
];
/// The id and label a probe widget carries so that only the facet under test
/// can fail the runtime's validator. Never written to a document.
const PROBE_ID: &str = "workshop-probe";
const PROBE_LABEL: &str = "workshop.probe.label";
/// The address an ownership probe is validated under. Only the boolean is read
/// from those calls, never the sentence, so the address is never seen.
const PROBE_AT: &str = "[[gm_role_preset.widget]] probe";

// ── Catalog types ─────────────────────────────────────────────────────────────

/// Every role preset one world authors, as an author sees it: the exact line of
/// each value, which references resolve, and the choices the RUNTIME owns.
#[derive(Clone, Debug, Serialize)]
pub struct PresetCatalog {
    pub path: String,
    /// `draft`, `base` or `pack:<id>` — and EMPTY for a path that is in neither
    /// the draft nor its dependencies, which an `unknown-document` finding then
    /// says.
    pub origin: String,
    pub presets: Vec<PresetView>,
    pub choices: Choices,
    /// The world members a preset may be authored in, draft first: a draft
    /// member is editable, everything beneath is read-only under its origin.
    pub worlds: Vec<WorldChoice>,
    /// Every preset finding over the whole candidate, not only the selected
    /// world, so the panel's list and Check agree.
    pub findings: Vec<WorkshopFinding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PresetView {
    pub index: usize,
    pub id: String,
    pub id_line: usize,
    pub label: String,
    pub label_line: usize,
    pub panels: Vec<Entry>,
    pub quick_actions: Vec<Entry>,
    pub contacts: Vec<Reference>,
    pub widgets: Vec<WidgetView>,
    pub unknown_keys: Vec<String>,
}

/// One element of an open-vocabulary array: exactly what it says and where.
#[derive(Clone, Debug, Serialize)]
pub struct Entry {
    pub index: usize,
    pub value: String,
    pub line: usize,
}

/// One element of an array the runtime or the world can judge. `known: false`
/// is an ERROR finding at this line, never a silent flag.
#[derive(Clone, Debug, Serialize)]
pub struct Reference {
    pub index: usize,
    pub value: String,
    pub line: usize,
    pub known: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Scalar {
    pub value: String,
    pub line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ShipReference {
    pub value: String,
    pub line: usize,
    pub known: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct WidgetView {
    pub index: usize,
    pub id: String,
    pub id_line: usize,
    /// The authored `type`, whatever it says: an unknown one is shown with its
    /// `widget-unknown-type` finding rather than dropped.
    pub kind: String,
    pub kind_line: usize,
    pub label: String,
    pub label_line: usize,
    pub band: Option<Scalar>,
    pub category: Option<Scalar>,
    pub ship: Option<ShipReference>,
    pub actions: Vec<Reference>,
    pub text: Option<Scalar>,
    pub unknown_keys: Vec<String>,
}

/// The vocabularies the RUNTIME owns (see the module header for the two the
/// browser owns and this deliberately omits).
#[derive(Clone, Debug, Serialize)]
pub struct Choices {
    pub widget_types: Vec<String>,
    pub widget_actions: Vec<String>,
    pub bands: Vec<String>,
    pub categories: Vec<String>,
    /// The selected world's own `[[entity]] name`s plus those of the worlds it
    /// composes: what a `ship` or a `contacts` entry may name.
    pub entities: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldChoice {
    pub path: String,
    pub origin: String,
}

/// A preset edit is the same all-or-nothing group of structural edits a
/// definition or composition edit is; only the checks around it differ.
pub type PresetEditRequest = EditRequest;

// ── Effective member set ──────────────────────────────────────────────────────

/// Everything beneath the draft as one map, later packs winning, exactly as
/// `validate_pack` resolves references.
fn beneath_of(dependencies: &WorkshopDependencies) -> BTreeMap<String, String> {
    let mut beneath = dependencies.base_files.clone();
    for pack in &dependencies.packs {
        beneath.extend(pack.files.clone());
    }
    beneath
}

/// Candidate ∪ dependencies as one flat source map, the draft winning a path.
fn sources_of(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
) -> BTreeMap<String, String> {
    let mut sources = beneath_of(dependencies);
    sources.extend(files.clone());
    sources
}

/// Where a path comes from, newest layer first, with the rank the panel lists
/// choices in: the draft, then the newest pack that carries it, then the base.
fn origin_of(
    path: &str,
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
) -> Option<(usize, String)> {
    if files.contains_key(path) {
        return Some((0, ORIGIN_DRAFT.to_owned()));
    }
    for (index, pack) in dependencies.packs.iter().enumerate().rev() {
        if pack.files.contains_key(path) {
            return Some((index + 2, format!("pack:{}", pack.id)));
        }
    }
    if dependencies.base_files.contains_key(path) {
        return Some((1, ORIGIN_BASE.to_owned()));
    }
    None
}

// ── Structural reads ──────────────────────────────────────────────────────────

/// The array-of-tables entries of `key` with the line of each: `[[key]]` blocks
/// and an inline `key = [{ … }]` alike, because the runtime's serde reads both
/// and a widget this module could not see would be a rule it could not check.
fn table_entries<'a>(
    source: &'a str,
    table: &'a dyn TableLike,
    key: &str,
) -> Vec<(&'a dyn TableLike, usize)> {
    match table.get(key) {
        Some(Item::ArrayOfTables(tables)) => tables
            .iter()
            .map(|entry| (entry as &dyn TableLike, table_line(source, entry)))
            .collect(),
        Some(Item::Value(Value::Array(array))) => array
            .iter()
            .filter_map(|element| {
                let line = span_line(source, element.span()).unwrap_or(1);
                Some((element.as_inline_table()? as &dyn TableLike, line))
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The string elements of an array key with the line of each, read from the
/// tree so an entry the runtime would refuse still shows what it says.
fn string_array(
    source: &str,
    table: &dyn TableLike,
    key: &str,
    fallback: usize,
) -> Vec<(String, usize)> {
    table
        .get(key)
        .and_then(Item::as_array)
        .map(|array| {
            array
                .iter()
                .map(|element| {
                    (
                        value_text(source, element),
                        span_line(source, element.span()).unwrap_or(fallback),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The line of an array key itself, for a rule about the array rather than one
/// of its elements.
fn key_line(source: &str, table: &dyn TableLike, key: &str, fallback: usize) -> usize {
    table
        .get(key)
        .and_then(|item| span_line(source, item.span()))
        .unwrap_or(fallback)
}

fn unknown_keys(table: &dyn TableLike, known: &[&str]) -> Vec<String> {
    table
        .iter()
        .map(|(key, _)| key)
        .filter(|key| !known.contains(key))
        .map(str::to_owned)
        .collect()
}

/// The `[[entity]] name`s one world declares — the vocabulary `contacts` and a
/// widget's `ship` name, read exactly as `world::validate::declared_names`
/// reads it off the parsed config.
fn declared_names(source: &str) -> BTreeSet<String> {
    let Ok(document) = Document::parse(source) else {
        return BTreeSet::new();
    };
    table_entries(source, document.as_table(), "entity")
        .into_iter()
        .filter_map(|(table, _)| string_field(table, "name").map(str::to_owned))
        .collect()
}

/// Every script text one world carries: the string values of its inline
/// `[script]` table, and the sibling `.rhai` its root `script` key declares,
/// resolved against the world's own directory.
fn script_texts(path: &str, source: &str, sources: &BTreeMap<String, String>) -> Vec<String> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    let root = document.as_table();
    let mut texts = Vec::new();
    if let Some(script) = root.get("script").and_then(Item::as_table_like) {
        for (_, item) in script.iter() {
            if let Some(body) = item.as_str() {
                texts.push(body.to_owned());
            }
        }
    }
    // A world declares its sibling as `script = "combat.rhai"`, beside itself.
    if let Some(relative) = root.get("script").and_then(Item::as_str) {
        let relative = relative.replace('\\', "/");
        let sibling = match path.rfind('/') {
            Some(index) => format!("{}/{relative}", &path[..index]),
            None => relative,
        };
        if let Some(text) = sources.get(&sibling) {
            texts.push(text.clone());
        }
    }
    texts
}

/// Entity names a SCRIPT mints, best effort.
///
/// `spawn_entity(#{ template_path = "…", name: "watcher", … })` mints a named
/// entity no `[[entity]]` block mentions, so a widget `ship` or a preset
/// `contacts` entry naming one is authored CORRECTLY and must not read as
/// unknown — the finding blocks save and export, and blocking content the game
/// runs is worse than missing a typo.
///
/// Nothing here parses Rhai. In a script text that calls `spawn_entity` at all,
/// every `name:` string literal is taken as a name it may mint. That errs
/// toward accepting a reference, which is the direction a gate that refuses an
/// export has to err in, and a typo in a world whose scripts never spawn is
/// still caught. A name assembled from a variable is not knowable here and is
/// the one case a Workshop author answers for by hand.
fn scripted_names(
    path: &str,
    source: &str,
    sources: &BTreeMap<String, String>,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for text in script_texts(path, source, sources) {
        if !text.contains("spawn_entity") {
            continue;
        }
        let mut rest = text.as_str();
        while let Some(at) = rest.find("name") {
            rest = &rest[at + "name".len()..];
            let after = rest.trim_start();
            let Some(after) = after.strip_prefix(':').or_else(|| after.strip_prefix('=')) else {
                continue;
            };
            let after = after.trim_start();
            let Some(after) = after.strip_prefix('"') else {
                continue;
            };
            if let Some(end) = after.find('"') {
                names.insert(after[..end].to_owned());
            }
        }
    }
    names
}

fn extra_worlds_of(source: &str) -> Vec<String> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    string_array(source, document.as_table(), "extra_worlds", 1)
        .into_iter()
        .map(|(path, _)| path)
        .collect()
}

/// Every entity name the COMPOSED world offers: its own `[[entity]]` names and
/// those of the worlds its `extra_worlds` compose, resolved over candidate ∪
/// dependencies — the set a live GM queue's ship facet draws from.
fn entity_names(path: &str, source: &str, sources: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut names = declared_names(source);
    names.extend(scripted_names(path, source, sources));
    for child in extra_worlds_of(source) {
        if child == path {
            continue;
        }
        if let Some(text) = sources.get(&child) {
            names.extend(declared_names(text));
            names.extend(scripted_names(&child, text, sources));
        }
    }
    names
}

// ── The runtime as the only judge ─────────────────────────────────────────────

/// A probe widget: a valid id, a valid label, the authored type, and nothing
/// else — the caller adds the one facet under test, so the runtime's validator
/// can only fail for that facet.
fn probe(kind: &str) -> GmRolePresetWidget {
    GmRolePresetWidget {
        id: PROBE_ID.into(),
        kind: kind.into(),
        label: PROBE_LABEL.into(),
        ..Default::default()
    }
}

/// The four facet groups `GmRolePresetWidget` assigns to types, named the way
/// the runtime's own refusal names them.
#[derive(Clone, Copy)]
enum Facet {
    BandCategory,
    Ship,
    Actions,
    Text,
}

impl Facet {
    fn key(self) -> &'static str {
        match self {
            Self::BandCategory => "band/category",
            Self::Ship => "ship",
            Self::Actions => "actions",
            Self::Text => "text",
        }
    }

    /// A probe declaring only this facet, carrying a value the runtime
    /// ACCEPTS, so the only thing left that can fail is ownership.
    fn probe(self, kind: &str) -> GmRolePresetWidget {
        let mut widget = probe(kind);
        match self {
            Self::BandCategory => widget.band = Some(GmAttentionBand::Urgent.as_authored().into()),
            Self::Ship => widget.ship = Some(PROBE_ID.into()),
            Self::Actions => widget.actions = vec![GM_WIDGET_ACTION_IDS[0].to_owned()],
            Self::Text => widget.text = Some(PROBE_LABEL.into()),
        }
        widget
    }
}

/// Whether the runtime lets `kind` carry `facet` at all, asked of the runtime
/// rather than answered here: the ownership table lives in one place.
fn owns(kind: &str, facet: Facet) -> bool {
    facet.probe(kind).validate(PROBE_AT).is_ok()
}

/// The vocabulary a runtime enum publishes for AUTHORS as a list: its own
/// `authored_vocabulary()` sentence split back into ids and then re-read
/// through `from_authored`, so a spelling the runtime would refuse can never
/// reach the panel as a choice and no second copy of the list lives here.
fn vocabulary(sentence: &str, accepts: impl Fn(&str) -> bool) -> Vec<String> {
    sentence
        .split(',')
        .map(|part| part.trim().trim_matches('\'').to_owned())
        .filter(|value| accepts(value))
        .collect()
}

fn bands() -> Vec<String> {
    vocabulary(&GmAttentionBand::authored_vocabulary(), |value| {
        GmAttentionBand::from_authored(value).is_some()
    })
}

fn categories() -> Vec<String> {
    vocabulary(&GmAttentionCategory::authored_vocabulary(), |value| {
        GmAttentionCategory::from_authored(value).is_some()
    })
}

// ── Catalog ───────────────────────────────────────────────────────────────────

fn read_widget(
    source: &str,
    index: usize,
    table: &dyn TableLike,
    entry_line: usize,
    names: &BTreeSet<String>,
) -> WidgetView {
    let scalar = |key: &str| scalar_at(source, table, key, entry_line);
    let text = |key: &str| scalar(key).unwrap_or_else(|| (String::new(), entry_line));
    let (id, id_line) = text("id");
    let (kind, kind_line) = text("type");
    let (label, label_line) = text("label");
    WidgetView {
        index,
        id,
        id_line,
        kind,
        kind_line,
        label,
        label_line,
        band: scalar("band").map(|(value, line)| Scalar { value, line }),
        category: scalar("category").map(|(value, line)| Scalar { value, line }),
        ship: scalar("ship").map(|(value, line)| ShipReference {
            known: names.contains(&value),
            value,
            line,
        }),
        actions: string_array(source, table, "actions", entry_line)
            .into_iter()
            .enumerate()
            .map(|(index, (value, line))| Reference {
                known: GM_WIDGET_ACTION_IDS.contains(&value.as_str()),
                index,
                value,
                line,
            })
            .collect(),
        text: scalar("text").map(|(value, line)| Scalar { value, line }),
        unknown_keys: unknown_keys(table, &WIDGET_KEYS),
    }
}

fn read_preset(
    source: &str,
    index: usize,
    table: &dyn TableLike,
    entry_line: usize,
    names: &BTreeSet<String>,
) -> PresetView {
    let text = |key: &str| {
        scalar_at(source, table, key, entry_line).unwrap_or((String::new(), entry_line))
    };
    let (id, id_line) = text("id");
    let (label, label_line) = text("label");
    let open = |key: &str| {
        string_array(source, table, key, entry_line)
            .into_iter()
            .enumerate()
            .map(|(index, (value, line))| Entry { index, value, line })
            .collect()
    };
    PresetView {
        index,
        id,
        id_line,
        label,
        label_line,
        panels: open("panels"),
        quick_actions: open("quick_actions"),
        contacts: string_array(source, table, "contacts", entry_line)
            .into_iter()
            .enumerate()
            .map(|(index, (value, line))| Reference {
                known: names.contains(&value),
                index,
                value,
                line,
            })
            .collect(),
        widgets: table_entries(source, table, WIDGETS_KEY)
            .into_iter()
            .enumerate()
            .map(|(index, (widget, line))| read_widget(source, index, widget, line, names))
            .collect(),
        unknown_keys: unknown_keys(table, &PRESET_KEYS),
    }
}

/// Every role preset one world member authors, with the runtime's own choices
/// and every preset finding the candidate carries. A path in neither the draft
/// nor its dependencies yields an empty catalog and an `unknown-document`
/// finding rather than a silently blank panel.
pub fn catalog(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
    path: &str,
) -> PresetCatalog {
    let sources = sources_of(files, dependencies);
    let mut worlds: Vec<(usize, WorldChoice)> = sources
        .keys()
        .filter(|candidate| is_member(candidate, WORLDS))
        .filter_map(|candidate| {
            origin_of(candidate, files, dependencies).map(|(rank, origin)| {
                (
                    rank,
                    WorldChoice {
                        path: candidate.clone(),
                        origin,
                    },
                )
            })
        })
        .collect();
    worlds.sort_by(|a, b| (a.0, &a.1.path).cmp(&(b.0, &b.1.path)));
    let mut catalog = PresetCatalog {
        path: path.to_owned(),
        origin: String::new(),
        presets: Vec::new(),
        choices: Choices {
            widget_types: GM_WIDGET_TYPES.iter().map(|kind| (*kind).into()).collect(),
            widget_actions: GM_WIDGET_ACTION_IDS
                .iter()
                .map(|action| (*action).into())
                .collect(),
            bands: bands(),
            categories: categories(),
            entities: Vec::new(),
        },
        worlds: worlds.into_iter().map(|(_, world)| world).collect(),
        findings: findings(files, &beneath_of(dependencies)),
    };
    let source = sources.get(path).filter(|_| is_member(path, WORLDS));
    let Some(source) = source else {
        catalog.findings.push(WorkshopFinding {
            severity: "error".into(),
            category: "unknown-document".into(),
            message: format!(
                "{path:?} is not an {WORLDS}*.toml member of the draft or its dependencies"
            ),
            file: path.to_owned(),
            line: None,
        });
        sort_findings(&mut catalog.findings);
        return catalog;
    };
    catalog.origin = origin_of(path, files, dependencies)
        .map(|(_, origin)| origin)
        .unwrap_or_default();
    let names = entity_names(path, source, &sources);
    catalog.choices.entities = names.iter().cloned().collect();
    if let Ok(document) = Document::parse(source.as_str()) {
        catalog.presets = table_entries(source, document.as_table(), PRESETS_KEY)
            .into_iter()
            .enumerate()
            .map(|(index, (preset, line))| read_preset(source, index, preset, line, &names))
            .collect();
    }
    catalog
}

// ── New preset skeleton ───────────────────────────────────────────────────────

/// The `[[gm_role_preset]]` block a new preset is appended as, built through
/// the syntax tree and then ASSERTED by the runtime's own reader, so a block
/// this function emits is a block `parse_world` accepts.
pub fn new_preset_source(id: &str, label: &str) -> Result<String, String> {
    let id = id.trim();
    let label = label.trim();
    if id.is_empty() {
        return Err("A role preset needs an id.".into());
    }
    if id == GM_ROLE_PRESET_ALL_ID {
        return Err(format!(
            "'{GM_ROLE_PRESET_ALL_ID}' is the reserved built-in preset id every Game Master \
             falls back to and may not be authored."
        ));
    }
    if label.is_empty() {
        return Err("A role preset needs a label, which is a strings.csv id.".into());
    }
    let mut entry = Table::new();
    entry.insert("id", toml_edit::value(id));
    entry.insert("label", toml_edit::value(label));
    let mut presets = ArrayOfTables::new();
    presets.push(entry);
    let mut document = DocumentMut::new();
    document.insert(PRESETS_KEY, Item::ArrayOfTables(presets));
    let source = document.to_string();
    parse_world(&source)?;
    Ok(source)
}

// ── Rules shared by refusals and findings ─────────────────────────────────────

/// One violated rule: the line it sits on, its category (the finding category,
/// and the rule name a refusal message opens with), the runtime's own sentence
/// where there is one, and a KEY — the offending VALUE without its array slot —
/// that identifies the violation across an edit which only moves the entry.
///
/// The same shape #1475 and #1476 use, and for the same reason: every message
/// names an index, so a violation the member already carried must not read as
/// new at its new index.
struct Issue {
    line: usize,
    category: &'static str,
    key: String,
    message: String,
}

/// The address `parse_world` builds for a widget, character for character, so
/// the sentence the runtime's validator returns reads as the runtime wrote it.
fn widget_at(preset_index: usize, preset_id: &str, widget_index: usize) -> String {
    format!(
        "[[gm_role_preset]] #{preset_index} '{preset_id}' [[gm_role_preset.widget]] \
         #{widget_index}"
    )
}

/// Push `category` when the runtime's validator objects to a probe carrying
/// this one facet, with the runtime's sentence as the message. The runtime is
/// the only judge: a facet it accepts produces nothing at all.
fn runtime_issue(
    out: &mut Vec<Issue>,
    at: &str,
    category: &'static str,
    key: String,
    line: usize,
    widget: GmRolePresetWidget,
) {
    if let Err(message) = widget.validate(at) {
        out.push(Issue {
            line,
            category,
            key,
            message,
        });
    }
}

/// Every rule one authored widget breaks, each at its own line and each
/// carrying the runtime's own sentence for it.
fn widget_issues(
    source: &str,
    at: &str,
    table: &dyn TableLike,
    entry_line: usize,
    names: &BTreeSet<String>,
) -> Vec<Issue> {
    let mut out = Vec::new();
    let scalar = |key: &str| scalar_at(source, table, key, entry_line);
    let (id, id_line) = scalar("id").unwrap_or((String::new(), entry_line));
    let (kind, kind_line) = scalar("type").unwrap_or((String::new(), entry_line));
    let (label, label_line) = scalar("label").unwrap_or((String::new(), entry_line));
    let band = scalar("band");
    let category = scalar("category");
    let ship = scalar("ship");
    let text = scalar("text");
    let actions = string_array(source, table, "actions", entry_line);
    let actions_line = key_line(source, table, "actions", entry_line);

    // The two keys every widget needs, whatever its type — the order the
    // runtime checks them in.
    if id.trim().is_empty() {
        let mut widget = probe(GM_WIDGET_TYPES[0]);
        widget.id = id.clone();
        runtime_issue(
            &mut out,
            at,
            "widget-empty-id",
            "id".into(),
            id_line,
            widget,
        );
    }
    if label.trim().is_empty() {
        let mut widget = probe(GM_WIDGET_TYPES[0]);
        widget.label = label.clone();
        runtime_issue(
            &mut out,
            at,
            "widget-empty-label",
            "label".into(),
            label_line,
            widget,
        );
    }
    if !GM_WIDGET_TYPES.contains(&kind.as_str()) {
        runtime_issue(
            &mut out,
            at,
            "widget-unknown-type",
            kind.clone(),
            kind_line,
            probe(&kind),
        );
        // Every remaining rule is a rule ABOUT a type. The runtime stops here
        // too, and "a 'wat' widget may not declare text" would be nonsense.
        return out;
    }

    // Which facets this type owns is the runtime's answer: a probe carrying
    // only the facet fails exactly when the type does not own it, and the
    // sentence it returns names the type and the key an author must remove.
    for (facet, line) in [
        (
            Facet::BandCategory,
            band.as_ref().or(category.as_ref()).map(|(_, line)| *line),
        ),
        (Facet::Ship, ship.as_ref().map(|(_, line)| *line)),
        (
            Facet::Actions,
            (!actions.is_empty()).then_some(actions_line),
        ),
        (Facet::Text, text.as_ref().map(|(_, line)| *line)),
    ] {
        let Some(line) = line else {
            continue;
        };
        runtime_issue(
            &mut out,
            at,
            "widget-key-on-wrong-type",
            format!("{kind}:{}", facet.key()),
            line,
            facet.probe(&kind),
        );
    }

    if owns(&kind, Facet::BandCategory) {
        if let Some((value, line)) = &band {
            if GmAttentionBand::from_authored(value).is_none() {
                let mut widget = probe(&kind);
                widget.band = Some(value.clone());
                runtime_issue(
                    &mut out,
                    at,
                    "widget-unknown-band",
                    value.clone(),
                    *line,
                    widget,
                );
            }
        }
        if let Some((value, line)) = &category {
            if GmAttentionCategory::from_authored(value).is_none() {
                let mut widget = probe(&kind);
                widget.category = Some(value.clone());
                runtime_issue(
                    &mut out,
                    at,
                    "widget-unknown-category",
                    value.clone(),
                    *line,
                    widget,
                );
            }
        }
    }
    if owns(&kind, Facet::Ship) {
        if let Some((value, line)) = &ship {
            if value.trim().is_empty() {
                // The runtime has a sentence for an empty ship; a name no
                // world declares is this module's rule, and both are the one
                // category, because both mean "no entity is named".
                let mut widget = probe(&kind);
                widget.ship = Some(value.clone());
                runtime_issue(
                    &mut out,
                    at,
                    "widget-unknown-ship",
                    "ship".into(),
                    *line,
                    widget,
                );
            } else if !names.contains(value) {
                out.push(Issue {
                    line: *line,
                    category: "widget-unknown-ship",
                    key: value.clone(),
                    message: format!(
                        "{at} narrows to ship '{value}', which is not an [[entity]] name this \
                         world or the worlds it composes declare"
                    ),
                });
            }
        }
    }
    if owns(&kind, Facet::Actions) {
        if actions.is_empty() {
            runtime_issue(
                &mut out,
                at,
                "widget-empty-actions",
                "actions".into(),
                actions_line,
                probe(&kind),
            );
        } else {
            let mut widget = probe(&kind);
            widget.actions = actions.iter().map(|(value, _)| value.clone()).collect();
            // The runtime reports the FIRST offending id, so this does too:
            // the message names its index, and fixing it reveals the next.
            if let Err(message) = widget.validate(at) {
                let offender = actions.iter().enumerate().find(|(index, (value, _))| {
                    !GM_WIDGET_ACTION_IDS.contains(&value.as_str())
                        || actions[..*index]
                            .iter()
                            .any(|(earlier, _)| earlier == value)
                });
                if let Some((_, (value, line))) = offender {
                    out.push(Issue {
                        line: *line,
                        category: if GM_WIDGET_ACTION_IDS.contains(&value.as_str()) {
                            "widget-duplicate-action"
                        } else {
                            "widget-unknown-action"
                        },
                        key: value.clone(),
                        message,
                    });
                }
            }
        }
    }
    if owns(&kind, Facet::Text) {
        let mut widget = probe(&kind);
        widget.text = text.as_ref().map(|(value, _)| value.clone());
        if let Err(message) = widget.validate(at) {
            let empty = text
                .as_ref()
                .map(|(value, _)| value.trim().is_empty())
                .unwrap_or(true);
            out.push(Issue {
                line: text.as_ref().map(|(_, line)| *line).unwrap_or(entry_line),
                category: if empty {
                    "widget-empty-text"
                } else {
                    "widget-invalid-text"
                },
                key: text
                    .as_ref()
                    .map(|(value, _)| value.clone())
                    .unwrap_or_else(|| "text".into()),
                message,
            });
        }
    }
    out
}

/// Every rule one world member's `[[gm_role_preset]]` blocks break, in
/// authored order. The preset-level sentences are this module's own (see the
/// header); every widget sentence is the runtime's.
fn preset_issues(source: &str, names: &BTreeSet<String>) -> Vec<Issue> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (index, (preset, entry_line)) in table_entries(source, document.as_table(), PRESETS_KEY)
        .into_iter()
        .enumerate()
    {
        let (id, id_line) =
            scalar_at(source, preset, "id", entry_line).unwrap_or((String::new(), entry_line));
        let (label, label_line) =
            scalar_at(source, preset, "label", entry_line).unwrap_or((String::new(), entry_line));
        if id.trim().is_empty() {
            out.push(Issue {
                line: id_line,
                category: "preset-empty-id",
                key: "id".into(),
                message: format!(
                    "[[gm_role_preset]] #{index} has an empty id; every role preset needs a \
                     stable id for a reconnecting Game Master's identity to name it"
                ),
            });
        } else if id == GM_ROLE_PRESET_ALL_ID {
            out.push(Issue {
                line: id_line,
                category: "preset-reserved-id",
                key: GM_ROLE_PRESET_ALL_ID.into(),
                message: format!(
                    "[[gm_role_preset]] #{index} declares reserved id \
                     '{GM_ROLE_PRESET_ALL_ID}'; that id is the built-in default every Game \
                     Master falls back to and may not be authored"
                ),
            });
        } else if !seen.insert(id.clone()) {
            out.push(Issue {
                line: id_line,
                category: "preset-duplicate-id",
                key: id.clone(),
                message: format!(
                    "duplicate gm_role_preset id '{id}': [[gm_role_preset]] #{index} declares it \
                     again; role preset ids must be unique within a world"
                ),
            });
        }
        if label.trim().is_empty() {
            out.push(Issue {
                line: label_line,
                category: "preset-empty-label",
                key: "label".into(),
                message: format!(
                    "[[gm_role_preset]] #{index} '{id}' has an empty label; a preset's label is \
                     the only heading an operator has for it, and is a strings.csv id"
                ),
            });
        }
        for (slot, (value, line)) in string_array(source, preset, "contacts", entry_line)
            .into_iter()
            .enumerate()
        {
            if !names.contains(&value) {
                out.push(Issue {
                    line,
                    category: "preset-unknown-contact",
                    key: value.clone(),
                    message: format!(
                        "[[gm_role_preset]] #{index} '{id}' contacts[{slot}] names '{value}', \
                         which is not an [[entity]] name this world or the worlds it composes \
                         declare"
                    ),
                });
            }
        }
        let mut widget_ids: BTreeSet<String> = BTreeSet::new();
        for (slot, (widget, widget_line)) in table_entries(source, preset, WIDGETS_KEY)
            .into_iter()
            .enumerate()
        {
            let at = widget_at(index, &id, slot);
            out.extend(widget_issues(source, &at, widget, widget_line, names));
            let (widget_id, line) = scalar_at(source, widget, "id", widget_line)
                .unwrap_or((String::new(), widget_line));
            if !widget_id.trim().is_empty() && !widget_ids.insert(widget_id.clone()) {
                out.push(Issue {
                    line,
                    category: "widget-duplicate-id",
                    key: widget_id.clone(),
                    message: format!(
                        "duplicate widget id '{widget_id}' in [[gm_role_preset]] #{index} \
                         '{id}': [[gm_role_preset.widget]] #{slot} declares it again; a rendered \
                         card is keyed by this id"
                    ),
                });
            }
        }
    }
    out
}

// ── Preset edits ──────────────────────────────────────────────────────────────

/// A refusal message: the rule, then the runtime's own sentence, so the panel
/// can map the rule to a string id and still show exactly what was refused.
fn refusal(issue: &Issue) -> String {
    format!("{}: {}", issue.category, issue.message)
}

/// The rules the edited member would break AFTER the edit that it did not
/// break before it. Rules the edit does not touch are findings, not refusals:
/// an author must be able to repair a hand-broken draft one edit at a time.
/// Violations are matched by category and offending value, never by message —
/// every message names an index, and a reorder shifts every index after the
/// moved entry — and the counts are what make a SECOND copy of a violation the
/// member already had new.
fn introduced(before: Vec<Issue>, after: Vec<Issue>) -> Result<(), String> {
    let mut carried: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for issue in &before {
        *carried
            .entry((issue.category, issue.key.as_str()))
            .or_default() += 1;
    }
    for issue in &after {
        match carried.get_mut(&(issue.category, issue.key.as_str())) {
            Some(count) if *count > 0 => *count -= 1,
            _ => return Err(refusal(issue)),
        }
    }
    Ok(())
}

/// Apply a preset edit to a COPY of the world member and refuse it — the
/// error, the source untouched — when the edit introduces any preset or widget
/// rule the member did not already break. A reorder is `set` edits on the
/// swapped slots for exactly this reason: the entries move, the violations do
/// not, and the comparison is by value rather than by index.
pub fn compose(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
    request: &PresetEditRequest,
) -> Result<String, String> {
    let path = request.document_path.as_str();
    if !is_member(path, WORLDS) {
        return Err(format!(
            "unknown-document: {path:?} is not an {WORLDS}*.toml member; role presets are \
             authored in a world"
        ));
    }
    let source = files
        .get(path)
        .ok_or_else(|| format!("unknown-document: {path:?} is not a draft member"))?;
    let edited = document::edit(source, request)?;
    let before = sources_of(files, dependencies);
    let mut candidate = files.clone();
    candidate.insert(path.to_owned(), edited.clone());
    let after = sources_of(&candidate, dependencies);
    introduced(
        preset_issues(source, &entity_names(path, source, &before)),
        preset_issues(&edited, &entity_names(path, &edited, &after)),
    )?;
    Ok(edited)
}

// ── Findings ──────────────────────────────────────────────────────────────────

fn finding(category: &str, file: &str, line: Option<usize>, message: String) -> WorkshopFinding {
    WorkshopFinding {
        severity: "error".into(),
        category: category.into(),
        message,
        file: file.to_owned(),
        line,
    }
}

fn sort_findings(findings: &mut Vec<WorkshopFinding>) {
    findings.sort_by(|a, b| {
        (&a.file, a.line, &a.category, &a.message).cmp(&(&b.file, b.line, &b.category, &b.message))
    });
    findings.dedup();
}

/// Role-preset findings over CANDIDATE world members, references resolved
/// against candidate ∪ beneath (candidate wins by path). Every category is an
/// ERROR at its exact line, because every one of them is a rule that refuses
/// the load or names a reference that resolves to nothing. Deterministic:
/// sorted by file, line, category and message, and deduped.
///
/// No warning here concerns a panel id or a quick-action id: that vocabulary is
/// the browser's (see the module header), and Rust keeps the string vocabularies
/// open on purpose.
pub fn findings(
    candidate: &BTreeMap<String, String>,
    beneath: &BTreeMap<String, String>,
) -> Vec<WorkshopFinding> {
    let mut sources = beneath.clone();
    sources.extend(candidate.clone());
    let mut findings = Vec::new();
    for (path, source) in candidate {
        if !is_member(path, WORLDS) {
            continue;
        }
        let names = entity_names(path, source, &sources);
        for issue in preset_issues(source, &names) {
            findings.push(finding(
                issue.category,
                path,
                Some(issue.line),
                issue.message,
            ));
        }
    }
    sort_findings(&mut findings);
    findings
}

#[cfg(test)]
mod tests;
