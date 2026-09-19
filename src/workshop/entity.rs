//! Entity template and fragment composition read from exact source
//! (issue #1476).
//!
//! An entity template is authored in two places at once: the local document's
//! own tables, and the ordered `includes` that compose fragments beneath it.
//! This module READS the runtime's answer to "who authored each field" rather
//! than merging a second time — `include_resolve::resolve_template` already
//! returns the composed document and a [`Provenance`] keyed by field address
//! (`hull.hull_integrity`, `system[id=helm-thrust].ai_only`,
//! `station[id=bridge].rating[name=Std].automated_systems`) — and turns it into
//! the catalog a panel renders, with a 1-based line for every LOCAL value.
//!
//! [`Provenance`]: crate::entities::include_resolve::Provenance
//!
//! # The three rules this module is built on
//!
//! 1. **Nothing here re-implements the merge.** Every "who owns this" answer is
//!    `resolve_template`'s provenance, and the keyed-array identities come from
//!    the merge's own table through [`MergePolicy::array_rule`]. A second merge
//!    would drift from the one the runtime spawns from, and the drift would be
//!    invisible: the panel would simply attribute a field to the wrong file.
//! 2. **The supported component list is the runtime's, never a list here.**
//!    `EntityConfig` is `deny_unknown_fields`, so serde already knows its own
//!    field names; [`supported_components`] reads them out of the error a probe
//!    with one unknown key produces, and [`component_skeleton`] asks the
//!    runtime what a component with nothing authored IS by deserialising an
//!    empty table into it and serialising what came back. A component that
//!    cannot answer that is listed with `skeleton: false` rather than silently
//!    missing from the panel.
//! 3. **An edit is refused at edit time with the source untouched**, the way
//!    #1475's composition edits are: [`compose`] applies the exact-source edit
//!    to a COPY, resolves and parses the result, and returns the runtime's own
//!    error when the edited member breaks a rule it did not break before.
//!    Rules the edit does not touch are FINDINGS, not refusals, so a
//!    hand-broken draft can be repaired one edit at a time (this is #1475's
//!    `introduced` rule, restated here over include entries).
//!
//! # Why the parse check compares before with after
//!
//! A fragment is legitimately not a complete entity on its own — the pack gate
//! says so in as many words, and `tests/fixtures/mod-packs/partial-entity-include.zip`
//! is the regression that pins it. So "the resolved template does not parse"
//! cannot be a flat refusal: it would lock every edit out of every fragment.
//! It is refused when the edit INTRODUCES it, and reported as a finding only
//! for a template nothing else includes.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use serde::Serialize;
use toml_edit::{Document, Item, Table, TableLike, Value};

use super::document::{self, Edit, EditRequest, Segment};
use super::source_spans::{is_member, line_at, span_line, value_text};
use super::{WorkshopDependencies, WorkshopFinding};
use crate::entities::config::EntityConfig;
use crate::entities::entity_override::{ArrayRule, MergePolicy};
use crate::entities::include_resolve::{
    canonical_include_path, canonical_template_path, resolve_template, INCLUDES_KEY,
};

/// The one directory an entity template or fragment may live under, for both
/// the pack rules and an `includes` entry.
const ENTITIES: &str = "assets/entities/";
const ORIGIN_DRAFT: &str = "draft";
const ORIGIN_BASE: &str = "base";
/// The layer an entity template composes at — the same constant
/// `include_resolve` merges with, so the identity keys read here are the ones
/// the runtime reconciles by.
const POLICY: MergePolicy = MergePolicy::ComposeFragments;

// ── Catalog types ─────────────────────────────────────────────────────────────

/// One entity template as an author sees it: what it includes, what it
/// composes to, who owns each field, and what may still be added.
#[derive(Clone, Debug, Serialize)]
pub struct EntityComposition {
    pub path: String,
    /// `draft`, `base` or `pack:<id>` — and EMPTY for a path that is in
    /// neither the draft nor its dependencies, which `error` then says.
    pub origin: String,
    pub resolvable: bool,
    /// The resolver's own sentence when the closure does not resolve: an
    /// include cycle, a missing fragment, a malformed `includes` declaration.
    pub error: Option<String>,
    pub includes: Vec<IncludeView>,
    /// The contributing templates in merge order, `provenance.sources()`.
    pub sources: Vec<String>,
    pub components: Vec<ComponentView>,
    pub fields: Vec<FieldView>,
    /// Every top-level key `EntityConfig` knows, read from serde (see
    /// [`supported_components`]).
    pub supported_components: Vec<String>,
    pub fragment_choices: Vec<FragmentChoice>,
    pub findings: Vec<WorkshopFinding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct IncludeView {
    pub index: usize,
    /// Exactly what the entry says, relative to the declaring template.
    pub authored: String,
    /// The lexically canonical path the resolver would read, or the authored
    /// text unchanged when the entry is not resolvable relative to the
    /// declarer at all (empty, root-absolute, drive-absolute) — `origin` is
    /// then `null`.
    pub canonical: String,
    pub line: usize,
    pub origin: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComponentView {
    pub key: String,
    pub local: bool,
    pub local_line: Option<usize>,
    /// The last template beneath this one whose own source declares this
    /// component, or `null` when none does. Independent of `local`: a template
    /// that overrides part of an inherited component — or shadows all of it — is
    /// BOTH, and the panel says so, because removing the local text there leaves
    /// the inherited copy composed.
    pub inherited_from: Option<String>,
    /// Whether the runtime can serialise a default for this component, i.e.
    /// whether [`component_skeleton`] answers.
    pub skeleton: bool,
    /// That default as ONE inline TOML value, which is what an exact-source
    /// `put` needs for its `value_source`; `null` exactly when `skeleton` is
    /// false. The flag alone cannot serve the panel — a bool produces no edit
    /// text and deciding what a component's default IS belongs to the runtime
    /// type (contract D2/D5), so the text travels with the flag.
    pub skeleton_source: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FieldView {
    /// The provenance address, keyed for every array the merge reconciles by
    /// key.
    pub address: String,
    pub source: String,
    pub chain: Vec<String>,
    pub local: bool,
    /// The exact span text when local, and the resolved value serialised to
    /// TOML when inherited.
    pub value_source: String,
    /// `Some` only for a local value, whose line this document owns.
    pub line: Option<usize>,
    /// The resolved value's TOML type.
    pub kind: Option<String>,
    /// Whether [`materialise`] could write this value into the local document
    /// at all — false for a local value, which is already authored here, and for
    /// an inherited one whose address the local document cannot name yet: an
    /// entry of a keyed array this template does not author, a position in an
    /// array the merge appends to, or a value more than one absent table deep.
    /// The panel offers Materialise exactly where it answers, because a control
    /// whose every press is refused is a defect in its own right.
    pub materialisable: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FragmentChoice {
    pub path: String,
    pub origin: String,
}

/// An entity edit is the same all-or-nothing group of structural edits a
/// definition or composition edit is; only the cross-file checks differ.
pub type EntityEditRequest = EditRequest;

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

/// Candidate ∪ dependencies as one flat source map, the draft winning a path —
/// the fragment source the resolver reads, and the overlay order the runtime
/// itself resolves in.
fn sources_of(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
) -> BTreeMap<String, String> {
    let mut sources = beneath_of(dependencies);
    sources.extend(files.clone());
    sources
}

/// Where a path comes from, newest layer first: the draft, then the newest
/// pack that carries it, then the base set.
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

/// Whether `path` may be an entity template or an `includes` target at all.
fn is_entity_path(path: &str) -> bool {
    is_member(path, ENTITIES)
}

// ── Structural reads ──────────────────────────────────────────────────────────

/// The `includes` entries of a template with the line of each, read from the
/// tree so a template the runtime refuses still shows what it declares.
fn includes_of(source: &str) -> Vec<(String, usize)> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    let Some(item) = document.as_table().get(INCLUDES_KEY) else {
        return Vec::new();
    };
    let array_line = span_line(source, item.span()).unwrap_or(1);
    item.as_array()
        .map(|array| {
            array
                .iter()
                .map(|element| {
                    (
                        value_text(source, element),
                        span_line(source, element.span()).unwrap_or(array_line),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The line the `includes` key sits on, for a finding about the closure as a
/// whole rather than about one entry.
fn includes_line(source: &str) -> Option<usize> {
    let document = Document::parse(source).ok()?;
    span_line(source, document.as_table().get(INCLUDES_KEY)?.span())
}

/// The include graph over a flat source map, canonical path to canonical
/// targets, so a cycle is judged by the identity the resolver judges by.
fn include_graph(sources: &BTreeMap<String, String>) -> BTreeMap<String, Vec<String>> {
    sources
        .iter()
        .filter(|(path, _)| is_entity_path(path))
        .map(|(path, source)| {
            let declaring = canonical_template_path(path);
            let targets = includes_of(source)
                .into_iter()
                .filter_map(|(authored, _)| canonical_include_path(&declaring, &authored))
                .collect();
            (declaring, targets)
        })
        .collect()
}

/// Whether `target` is reachable from `start` by following includes. `start`
/// itself counts, so a caller asking "does adding this entry close a cycle"
/// gets `true` for the template itself.
fn reaches(graph: &BTreeMap<String, Vec<String>>, start: &str, target: &str) -> bool {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![start.to_owned()];
    while let Some(node) = stack.pop() {
        if node == target {
            return true;
        }
        if !seen.insert(node.clone()) {
            continue;
        }
        if let Some(children) = graph.get(&node) {
            stack.extend(children.iter().cloned());
        }
    }
    false
}

// ── Provenance addresses ──────────────────────────────────────────────────────

/// One step of a provenance address.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    /// `hull`, or `"a.weird key"` as provenance quotes it.
    Key(String),
    /// `station[id=bridge]` — the entry of a keyed array the merge reconciles
    /// by `by`.
    Keyed {
        name: String,
        by: String,
        value: String,
    },
    /// `system[2]` — provenance's fallback for an entry of a keyed array that
    /// carries no key. There is no stable way to address it in source.
    Indexed { name: String, index: usize },
}

impl Step {
    fn name(&self) -> &str {
        match self {
            Step::Key(name) | Step::Keyed { name, .. } | Step::Indexed { name, .. } => name,
        }
    }
}

/// The address split at unquoted, unbracketed dots.
fn split_address(address: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut depth = 0usize;
    for character in address.chars() {
        match character {
            '"' if depth == 0 => {
                quoted = !quoted;
                current.push(character);
            }
            '[' if !quoted => {
                depth += 1;
                current.push(character);
            }
            ']' if !quoted => {
                depth = depth.checked_sub(1)?;
                current.push(character);
            }
            '.' if !quoted && depth == 0 => out.push(std::mem::take(&mut current)),
            _ => current.push(character),
        }
    }
    if quoted || depth != 0 || current.is_empty() {
        return None;
    }
    out.push(current);
    Some(out)
}

fn unquote(name: &str) -> String {
    name.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(name)
        .to_owned()
}

/// A provenance address as steps, or `None` when it is not one — an address
/// reaches this module from a request, so it is never trusted to be well
/// formed.
fn parse_address(address: &str) -> Option<Vec<Step>> {
    split_address(address)?
        .into_iter()
        .map(|segment| {
            let (name, bracket) = match segment.find('[') {
                Some(at) => (
                    &segment[..at],
                    Some(segment[at + 1..].strip_suffix(']')?.to_owned()),
                ),
                None => (segment.as_str(), None),
            };
            let name = unquote(name);
            if name.is_empty() {
                return None;
            }
            Some(match bracket {
                None => Step::Key(name),
                Some(inner) => match inner.split_once('=') {
                    Some((by, value)) => Step::Keyed {
                        name,
                        by: by.to_owned(),
                        value: value.to_owned(),
                    },
                    None => Step::Indexed {
                        name,
                        index: inner.parse().ok()?,
                    },
                },
            })
        })
        .collect()
}

/// The dotted, index-free path the MERGE judges an array by, which is what
/// [`MergePolicy::array_rule`] is keyed on.
fn merge_path(steps: &[Step]) -> String {
    steps.iter().map(Step::name).collect::<Vec<_>>().join(".")
}

/// The identity key the merge reconciles the array at these steps by, so a
/// refusal names the key an author has to write rather than guessing one.
fn identity_of(steps: &[Step]) -> &'static str {
    match POLICY.array_rule(&merge_path(steps)) {
        ArrayRule::Keyed(by) => by,
        _ => "id",
    }
}

/// The resolved value at an address, walking keyed arrays by their key exactly
/// as provenance addressed them.
fn value_at<'a>(root: &'a toml::Value, steps: &[Step]) -> Option<&'a toml::Value> {
    let mut current = root;
    for step in steps {
        current = match step {
            Step::Key(key) => current.as_table()?.get(key)?,
            Step::Keyed { name, by, value } => current
                .as_table()?
                .get(name)?
                .as_array()?
                .iter()
                .find(|element| element.get(by).and_then(toml::Value::as_str) == Some(value))?,
            Step::Indexed { name, index } => {
                current.as_table()?.get(name)?.as_array()?.get(*index)?
            }
        };
    }
    Some(current)
}

// ── The local syntax tree ─────────────────────────────────────────────────────

/// A position in the local document. A `[[table]]` entry and an inline-array
/// element are not `Item`s, which is why this is not simply `&Item`.
#[derive(Clone, Copy)]
enum Node<'a> {
    Item(&'a Item),
    Table(&'a Table),
    Value(&'a Value),
}

impl<'a> Node<'a> {
    fn table_like(self) -> Option<&'a dyn TableLike> {
        match self {
            Node::Item(Item::Table(table)) => Some(table),
            Node::Item(Item::Value(Value::InlineTable(table))) => Some(table),
            Node::Table(table) => Some(table),
            Node::Value(Value::InlineTable(table)) => Some(table),
            _ => None,
        }
    }

    fn span(self) -> Option<Range<usize>> {
        match self {
            Node::Item(item) => item.span(),
            Node::Table(table) => table.span(),
            Node::Value(value) => value.span(),
        }
    }
}

/// One step down the local tree, with the document path segments it adds. The
/// path is returned rather than pushed so a step that does not resolve leaves
/// the caller's path exactly as it was.
fn descend<'a>(node: Node<'a>, step: &Step) -> Option<(Vec<Segment>, Node<'a>)> {
    let table = node.table_like()?;
    match step {
        Step::Key(key) => Some((vec![Segment::Key(key.clone())], Node::Item(table.get(key)?))),
        Step::Keyed { name, by, value } => match table.get(name)? {
            Item::ArrayOfTables(tables) => {
                let index = tables.iter().position(|entry| {
                    entry.get(by).and_then(Item::as_str) == Some(value.as_str())
                })?;
                Some((
                    vec![Segment::Key(name.clone()), Segment::Index(index)],
                    Node::Table(tables.get(index)?),
                ))
            }
            Item::Value(Value::Array(array)) => {
                let index = array.iter().position(|element| {
                    element
                        .as_inline_table()
                        .and_then(|entry| entry.get(by))
                        .and_then(Value::as_str)
                        == Some(value.as_str())
                })?;
                Some((
                    vec![Segment::Key(name.clone()), Segment::Index(index)],
                    Node::Value(array.get(index)?),
                ))
            }
            _ => None,
        },
        Step::Indexed { name, index } => match table.get(name)? {
            Item::ArrayOfTables(tables) => Some((
                vec![Segment::Key(name.clone()), Segment::Index(*index)],
                Node::Table(tables.get(*index)?),
            )),
            Item::Value(Value::Array(array)) => Some((
                vec![Segment::Key(name.clone()), Segment::Index(*index)],
                Node::Value(array.get(*index)?),
            )),
            _ => None,
        },
    }
}

/// The exact source span and 1-based line an address reaches in the LOCAL
/// document, or `None` when this document does not author it.
fn local_span(
    document: &Document<&str>,
    source: &str,
    steps: &[Step],
) -> Option<(Range<usize>, usize)> {
    let mut node = Node::Item(document.as_item());
    for step in steps {
        node = descend(node, step)?.1;
    }
    let span = node.span()?;
    let line = line_at(source, span.start);
    Some((span, line))
}

/// [`put_path_in`] over source text, for a caller that has no parsed document
/// in hand.
fn put_path(source: &str, steps: &[Step]) -> Result<Vec<Segment>, String> {
    let document = Document::parse(source)
        .map_err(|error| format!("materialise-unknown-address: {}", error.message()))?;
    put_path_in(&document, steps)
}

/// The document path a `put` of this address writes to, and the `put`'s own
/// key. A plain key the local document does not carry yet is CREATED by the
/// put (one level, which is what `document::edit` supports); an entry of a
/// keyed array is not, because the merge reconciles it by a key that has to
/// already be in the source.
fn put_path_in(document: &Document<&str>, steps: &[Step]) -> Result<Vec<Segment>, String> {
    let Some((last, parents)) = steps.split_last() else {
        return Err("materialise-unknown-address: the address is empty".into());
    };
    // A POSITION is not an address. Provenance falls back to `system[2]` for an
    // entry of a keyed array that carries no key, and the merge APPENDS such an
    // entry after the ones it reconciled — so the resolved position and the
    // local one are not the same array. Refused wherever it appears, not only as
    // the last step: an indexed PARENT that happens to resolve locally would
    // otherwise write the value into whichever local entry sits at that
    // position.
    if let Some(at) = steps
        .iter()
        .position(|step| matches!(step, Step::Indexed { .. }))
    {
        return Err(format!(
            "materialise-keyed-entry: {:?} names the entry at position {} of an array the merge \
             reconciles by {:?}; that entry carries no key, so it has no stable address in \
             source. Add a keyed entry locally and materialise into that instead",
            merge_path(steps),
            match &steps[at] {
                Step::Indexed { index, .. } => *index,
                _ => 0,
            },
            identity_of(&steps[..=at])
        ));
    }
    let Step::Key(key) = last else {
        return Err(format!(
            "materialise-keyed-entry: {:?} names an entry of an array the merge reconciles by \
             {:?}, which has no stable address in source; materialise a field inside a local \
             entry instead",
            merge_path(steps),
            identity_of(steps)
        ));
    };
    let mut path = Vec::new();
    let mut node = Some(Node::Item(document.as_item()));
    let mut created = 0usize;
    for (index, step) in parents.iter().enumerate() {
        let next = node.and_then(|current| descend(current, step));
        match next {
            Some((segments, child)) => {
                path.extend(segments);
                node = Some(child);
            }
            // A plain key the local document has not got yet is created by the
            // put itself. An entry of a keyed array is not: the merge finds it
            // by a key that has to be in the source already, so there is
            // nowhere for the value to land.
            None => {
                let Step::Key(name) = step else {
                    return Err(format!(
                        "materialise-keyed-entry: this template authors no {:?} entry keyed \
                         {:?} as {:?}; add the entry locally before materialising a field \
                         inside it",
                        step.name(),
                        identity_of(&parents[..=index]),
                        match step {
                            Step::Keyed { value, .. } => value.clone(),
                            Step::Indexed { index, .. } => index.to_string(),
                            Step::Key(name) => name.clone(),
                        }
                    ));
                };
                path.push(Segment::Key(name.clone()));
                created += 1;
                node = None;
            }
        }
    }
    if created > 1 {
        return Err(format!(
            "materialise-unknown-address: {:?} would need {created} new local tables; \
             materialise the enclosing component first",
            merge_path(steps)
        ));
    }
    path.push(Segment::Key(key.clone()));
    Ok(path)
}

// ── Runtime values as source text ─────────────────────────────────────────────

/// A runtime value as the inline TOML text an exact-source `put` writes. Built
/// through `toml_edit`, so escaping, float spelling and key quoting are the
/// emitter's rather than this module's.
fn edit_value(value: &toml::Value) -> Option<Value> {
    Some(match value {
        toml::Value::String(text) => Value::from(text.as_str()),
        toml::Value::Integer(number) => Value::from(*number),
        toml::Value::Float(number) => Value::from(*number),
        toml::Value::Boolean(flag) => Value::from(*flag),
        toml::Value::Datetime(when) => Value::from(*when),
        toml::Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(edit_value(item)?);
            }
            array.fmt();
            Value::Array(array)
        }
        toml::Value::Table(table) => {
            let mut inline = toml_edit::InlineTable::new();
            for (key, child) in table {
                inline.insert(key, edit_value(child)?);
            }
            inline.fmt();
            Value::InlineTable(inline)
        }
    })
}

/// The TOML source text for a runtime value.
fn value_source_of(value: &toml::Value) -> Option<String> {
    Some(edit_value(value)?.to_string().trim().to_owned())
}

// ── The runtime's own component vocabulary ────────────────────────────────────

/// Every top-level key `EntityConfig` accepts, in the order the struct
/// declares them, read from SERDE rather than listed here.
///
/// `EntityConfig` is `deny_unknown_fields`, so a document carrying one key it
/// does not know is answered with "unknown field `x`, expected one of `a`,
/// `b`, …" — the struct's own field list. Reading it back means a component
/// added to the runtime appears in the Workshop with no edit here, and pinned
/// by a ratchet test so it cannot quietly change either.
///
/// Note what this therefore does NOT list: the keys `EntityConfig::from_toml`
/// consumes before serde sees them (`station`, `system`, `power_groups`,
/// `shield_arc`) and `includes`, which the resolver strips. Those are authored
/// and valid; they are simply not serde's to name, and authoring them
/// structurally is #1481's. Nothing refuses them — the edit-time
/// `component-unsupported` rule asks the RUNTIME whether the edited document
/// parses rather than consulting this list.
pub fn supported_components() -> &'static [String] {
    static COMPONENTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    COMPONENTS.get_or_init(|| {
        // A key no component could ever be called, so the probe cannot collide
        // with a real one and the error is always the unknown-field one.
        let Err(error) = toml::from_str::<EntityConfig>("workshop-probe-unknown-key = true\n")
        else {
            return Vec::new();
        };
        expected_fields(&error.to_string())
    })
}

/// The backtick-quoted name list following serde's "expected one of" in an
/// unknown-field message.
fn expected_fields(message: &str) -> Vec<String> {
    let Some(tail) = message.split_once("expected one of ").map(|(_, tail)| tail) else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    let mut rest = tail;
    while let Some((_, after)) = rest.split_once('`') {
        let Some((name, remainder)) = after.split_once('`') else {
            break;
        };
        fields.push(name.to_owned());
        rest = remainder;
    }
    fields
}

/// What the runtime says this component IS with nothing authored, as inline
/// TOML — or `None` for a component that cannot answer.
///
/// The question is asked of the runtime, not of a table here: deserialise an
/// EMPTY table into the component and serialise back what the runtime built.
/// A component whose fields all carry defaults answers; one that requires an
/// authored field, or that is not a table at all (`name`, `tags`, `mass`),
/// does not, and the catalog then says `skeleton: false` so the panel can tell
/// an author to materialise it from a fragment or write it by hand instead of
/// offering an Add that would be refused.
pub fn component_skeleton(key: &str) -> Option<&'static str> {
    component_skeletons().get(key).map(String::as_str)
}

/// Every component that can answer [`component_skeleton`], asked once.
pub fn component_skeletons() -> &'static BTreeMap<String, String> {
    static SKELETONS: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
    SKELETONS.get_or_init(|| {
        supported_components()
            .iter()
            .filter_map(|key| Some((key.clone(), skeleton_of(key)?)))
            .collect()
    })
}

fn skeleton_of(key: &str) -> Option<String> {
    let config = EntityConfig::from_toml(&format!("{key} = {{}}\n")).ok()?;
    let value = toml::Value::try_from(&config).ok()?;
    value_source_of(value.get(key)?)
}

// ── Catalog ───────────────────────────────────────────────────────────────────

/// Everything an author can see about one entity template: what it includes,
/// what it composes to, who owns each field and what may still be added. The
/// draft's own members are editable; everything beneath is listed read-only
/// under its origin.
pub fn catalog(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
    path: &str,
) -> EntityComposition {
    let sources = sources_of(files, dependencies);
    let canonical = canonical_template_path(path);
    let supported: Vec<String> = supported_components().to_vec();
    let mut view = EntityComposition {
        path: path.to_owned(),
        origin: String::new(),
        resolvable: false,
        error: None,
        includes: Vec::new(),
        sources: Vec::new(),
        components: Vec::new(),
        fields: Vec::new(),
        supported_components: supported.clone(),
        fragment_choices: Vec::new(),
        findings: findings(files, &beneath_of(dependencies)),
    };
    let Some(source) = sources.get(&canonical) else {
        view.error = Some(format!(
            "unknown-document: {path:?} is not a template in the draft or its dependencies"
        ));
        return view;
    };
    view.origin = origin_of(&canonical, files, dependencies)
        .map(|(_, origin)| origin)
        .unwrap_or_default();

    let graph = include_graph(&sources);
    view.includes = includes_of(source)
        .into_iter()
        .enumerate()
        .map(|(index, (authored, line))| {
            let resolved = canonical_include_path(&canonical, &authored);
            IncludeView {
                index,
                origin: resolved
                    .as_deref()
                    .and_then(|target| origin_of(target, files, dependencies))
                    .map(|(_, origin)| origin),
                canonical: resolved.unwrap_or_else(|| authored.clone()),
                authored,
                line,
            }
        })
        .collect();

    // Every other entity member that could be added as an include without
    // closing a cycle: the ones the panel may offer, so no choice it presents
    // is refused the moment it is chosen.
    let mut choices: Vec<(usize, String, String)> = sources
        .keys()
        .filter(|candidate| is_entity_path(candidate))
        .map(|candidate| canonical_template_path(candidate))
        .filter(|candidate| *candidate != canonical && !reaches(&graph, candidate, &canonical))
        .filter_map(|candidate| {
            origin_of(&candidate, files, dependencies)
                .map(|(rank, origin)| (rank, candidate, origin))
        })
        .collect();
    choices.sort();
    view.fragment_choices = choices
        .into_iter()
        .map(|(_, path, origin)| FragmentChoice { path, origin })
        .collect();

    let resolved = match resolve_template(&canonical, &sources) {
        Ok(resolved) => resolved,
        Err(error) => {
            view.error = Some(error.to_string());
            view.components = local_only_components(&supported, source);
            return view;
        }
    };
    view.resolvable = true;
    view.sources = resolved
        .provenance
        .sources()
        .into_iter()
        .map(str::to_owned)
        .collect();

    // Parsed once for every field: the local span of a local value and the put
    // path of an inherited one are both reads of this one tree.
    let tree = Document::parse(source.as_str()).ok();
    for (address, origin) in resolved.provenance.fields() {
        let local = origin.source == canonical;
        let steps = parse_address(address);
        let value = steps
            .as_deref()
            .and_then(|steps| value_at(&resolved.value, steps));
        let exact = if local {
            tree.as_ref()
                .zip(steps.as_deref())
                .and_then(|(tree, steps)| local_span(tree, source, steps))
                .map(|(span, line)| (source[span].to_owned(), line))
        } else {
            None
        };
        let materialisable = !local
            && value.is_some()
            && tree
                .as_ref()
                .zip(steps.as_deref())
                .is_some_and(|(tree, steps)| put_path_in(tree, steps).is_ok());
        let (value_source, line) = match exact {
            Some((text, line)) => (text, Some(line)),
            // An inherited value has no line in this document, and a local one
            // the tree cannot address falls back to the resolved value rather
            // than showing nothing.
            None => (value.and_then(value_source_of).unwrap_or_default(), None),
        };
        view.fields.push(FieldView {
            address: address.clone(),
            source: origin.source.clone(),
            chain: origin.chain.clone(),
            local,
            value_source,
            line,
            kind: value.map(|value| value.type_str().to_owned()),
            materialisable,
        });
    }

    let owners = component_owners(&sources, &resolved.provenance.sources(), &canonical);
    view.components = supported
        .iter()
        .map(|key| {
            let (local, local_line) = local_component(source, key);
            ComponentView {
                key: key.clone(),
                local,
                local_line,
                inherited_from: owners.get(key).cloned(),
                skeleton: component_skeleton(key).is_some(),
                skeleton_source: component_skeleton(key).map(str::to_owned),
            }
        })
        .collect();
    view
}

/// Whether the local document authors this top-level key, and the line it
/// sits on.
fn local_component(source: &str, key: &str) -> (bool, Option<usize>) {
    let Ok(document) = Document::parse(source) else {
        return (false, None);
    };
    match document.as_table().get(key) {
        // A dotted or implicit table (`hull.hull_integrity = 1.0` with no
        // header) has no header span, so its first value's line stands in —
        // the same fallback `source_spans::table_line` applies, without that
        // helper's "line 1" last resort, because a line this document does not
        // own must read as no line at all.
        Some(item) => (
            true,
            span_line(source, item.span()).or_else(|| {
                item.as_table_like()?
                    .iter()
                    .find_map(|(_, child)| span_line(source, child.span()))
            }),
        ),
        None => (false, None),
    }
}

/// The components with no resolved document to read: an unresolvable closure
/// still shows what this file itself authors.
fn local_only_components(supported: &[String], source: &str) -> Vec<ComponentView> {
    supported
        .iter()
        .map(|key| {
            let (local, local_line) = local_component(source, key);
            ComponentView {
                key: key.clone(),
                local,
                local_line,
                inherited_from: None,
                skeleton: component_skeleton(key).is_some(),
                skeleton_source: component_skeleton(key).map(str::to_owned),
            }
        })
        .collect()
}

/// Which template beneath this one AUTHORS each component: the LAST
/// contributor in merge order other than the template itself, which is the one
/// a local override would be overriding.
///
/// # Why this reads the contributing SOURCES and not provenance
///
/// Provenance records the WINNER of each leaf, and "who authored this
/// component" is a structural question about the contributing documents. The
/// two come apart in both directions, and both were real defects:
///
/// * A template that overrides PART of an inherited component wins the leaves
///   it authored while the inherited table is still there underneath — the
///   central case of composition, and 38 (component, fragment) pairs on the
///   shipped composed hulls. Reading the winner of any leaf under the key made
///   such a component read as inherited, which refused its removal.
/// * A template that SHADOWS every leaf a fragment authors wins them all, so no
///   leaf beneath it has a foreign origin and the component read as purely
///   local — while removing the local table would hand the merge the fragment's
///   whole component back (`power` on all four Harrow hulls).
///
/// So the question asked here is the structural one: does a contributing
/// template's own document declare this top-level key at all.
fn component_owners(
    sources: &BTreeMap<String, String>,
    order: &[&str],
    canonical: &str,
) -> BTreeMap<String, String> {
    let mut owners: BTreeMap<String, String> = BTreeMap::new();
    // Merge order, so the last contributor to declare a key is the owner a
    // local override overrides.
    for step in order {
        if *step == canonical {
            continue;
        }
        let Some(text) = sources.get(*step) else {
            continue;
        };
        let Ok(document) = Document::parse(text) else {
            continue;
        };
        for (key, _) in document.as_table().iter() {
            // The resolver's own key, never a component.
            if key == INCLUDES_KEY {
                continue;
            }
            owners.insert(key.to_owned(), (*step).to_owned());
        }
    }
    owners
}

/// Whether anything beneath `canonical` authors this component, and which
/// template does — the question a removal has to ask, because a local table
/// removed off an inherited component leaves the inherited one behind.
fn inherited_owner(
    sources: &BTreeMap<String, String>,
    canonical: &str,
    key: &str,
) -> Option<String> {
    let resolved = resolve_template(canonical, sources).ok()?;
    component_owners(sources, &resolved.provenance.sources(), canonical).remove(key)
}

// ── Rules shared by refusals and findings ─────────────────────────────────────

/// One violated rule: the line it sits on, its category (the finding category,
/// and the rule name a refusal message opens with), a message naming the
/// offending value, and a KEY — the offending value without its array slot —
/// that identifies the violation across an edit which only moves the entry.
///
/// The same shape #1475 uses, and for the same reason: every message names an
/// index, so a violation the member already carried must not read as new at
/// its new index.
struct Issue {
    line: Option<usize>,
    category: &'static str,
    key: String,
    message: String,
}

/// The rules one template's `includes` list breaks, in authored order.
fn include_issues(
    canonical: &str,
    source: &str,
    sources: &BTreeMap<String, String>,
    graph: &BTreeMap<String, Vec<String>>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    for (index, (authored, line)) in includes_of(source).into_iter().enumerate() {
        let issue = |category, key: String, message| Issue {
            line: Some(line),
            category,
            key,
            message,
        };
        let Some(target) = canonical_include_path(canonical, &authored) else {
            issues.push(issue(
                "include-disallowed",
                authored.clone(),
                format!(
                    "includes[{index}] {authored:?} is not resolvable relative to this template"
                ),
            ));
            continue;
        };
        if !is_entity_path(&target) {
            issues.push(issue(
                "include-disallowed",
                target.clone(),
                format!("includes[{index}] {authored:?} is not an {ENTITIES}**.toml path"),
            ));
            continue;
        }
        if target == canonical {
            issues.push(issue(
                "include-self",
                target.clone(),
                format!("includes[{index}] names the template itself"),
            ));
            continue;
        }
        if !sources.contains_key(&target) {
            issues.push(issue(
                "include-missing",
                target.clone(),
                format!("includes[{index}] {authored:?} is not in the draft or its dependencies"),
            ));
            continue;
        }
        if reaches(graph, &target, canonical) {
            issues.push(issue(
                "include-cycle",
                target.clone(),
                format!("includes[{index}] {authored:?} composes a cycle back to {canonical}"),
            ));
        }
    }
    issues
}

/// What the RUNTIME says about this member's whole closure: one issue when it
/// does not resolve or the composed document is not an entity, carrying the
/// runtime's own sentence. `component-unsupported` is the same answer read one
/// step further — serde naming a key `EntityConfig` does not know — so the
/// vocabulary is never listed twice.
fn runtime_issue(
    canonical: &str,
    sources: &BTreeMap<String, String>,
    line: Option<usize>,
    composed_only: bool,
) -> Option<Issue> {
    let (category, message) = match resolve_template(canonical, sources) {
        Err(error) => {
            // A root that is not valid TOML on its own is the plain parse
            // error every source gate already reports; it is not a
            // composition failure and must not be promoted to one here.
            if error.chain.len() == 1 && error.category() == "include-parse" {
                return None;
            }
            (map_resolver_category(error.category()), error.to_string())
        }
        Ok(resolved) => {
            // A template that composed nothing and is not a valid entity is
            // the ordinary source error, reported by the source gate with its
            // own location. Only a COMPOSED document's failure is this
            // module's, because that combination exists in no single authored
            // file — the same asymmetry `composition_finding` applies. A
            // refusal asks without the qualifier, because an edit that breaks
            // an uncomposed template it did not break before is still the
            // edit's doing.
            if composed_only && !resolved.is_composed() {
                return None;
            }
            match resolved.parse() {
                Ok(_) => return None,
                Err(error) => {
                    let message = error.to_string();
                    let category = if message.contains("unknown field") {
                        "component-unsupported"
                    } else {
                        "entity-invalid"
                    };
                    (category, message)
                }
            }
        }
    };
    Some(Issue {
        line,
        category,
        key: category.to_owned(),
        message,
    })
}

/// The resolver's own categories, mapped onto the rules a panel maps to a
/// string. A malformed `includes` declaration is a path the rules do not
/// allow; an unparseable fragment makes the composed template invalid.
fn map_resolver_category(category: &str) -> &'static str {
    match category {
        "include-cycle" => "include-cycle",
        "include-missing" => "include-missing",
        "include-malformed" => "include-disallowed",
        _ => "entity-invalid",
    }
}

/// Every rule the edited member breaks, over the source set it is judged in.
fn member_issues(canonical: &str, source: &str, sources: &BTreeMap<String, String>) -> Vec<Issue> {
    let graph = include_graph(sources);
    let mut issues = include_issues(canonical, source, sources, &graph);
    issues.extend(runtime_issue(
        canonical,
        sources,
        includes_line(source),
        false,
    ));
    issues
}

/// A refusal message: the rule, then the offending value, so the panel can map
/// the rule to a string id and still show what was refused.
fn refusal(issue: &Issue) -> String {
    format!("{}: {}", issue.category, issue.message)
}

/// The rules the edited member would break AFTER the edit that it did not
/// break before it. Rules the edit does not touch are findings, not refusals:
/// an author must be able to fix a hand-broken draft — or edit a fragment that
/// was never a complete entity on its own — one edit at a time. Violations are
/// matched by category and offending value, never by message, and the counts
/// are what make a SECOND copy of a violation the member already had new.
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

// ── Entity edits ──────────────────────────────────────────────────────────────

/// Apply an entity edit to a COPY of the member and refuse it — the error, the
/// source untouched — when the edited member introduces a missing, cyclic,
/// self or disallowed include, a component the runtime does not know, or a
/// composed document that no longer parses.
///
/// Removing a component this template does not author but INHERITS is refused
/// before the edit is even attempted: there is no local text to delete and no
/// tombstone for a whole inherited table, so the removal could only appear to
/// work. The panel is told to materialise it and edit the local copy instead.
/// (A keyed array ENTRY does have a tombstone — `{ id = "…", _remove = true }` —
/// and is an ordinary local edit inside the array, not a top-level removal, so
/// it never reaches this rule.)
///
/// Removing a component this template DOES author is allowed even when a
/// fragment authors it too: dropping a local override so the composed value
/// reverts to the fragment's is an ordinary authoring act, and the panel says
/// which fragment the row reverts to rather than presenting a Remove whose every
/// press is refused. Asking `inherited_owner` without that distinction refused
/// the removal of 38 shipped (component, fragment) pairs — every hull that
/// overrides part of an inherited component.
pub fn compose(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
    request: &EntityEditRequest,
) -> Result<String, String> {
    let path = request.document_path.as_str();
    let source = files
        .get(path)
        .ok_or_else(|| format!("unknown-document: {path:?} is not a draft member"))?;
    let canonical = canonical_template_path(path);
    let before = sources_of(files, dependencies);
    for edit in &request.edits {
        let Edit::Remove { path: target } = edit else {
            continue;
        };
        let [Segment::Key(key)] = target.as_slice() else {
            continue;
        };
        // Local text is this document's own to drop, whoever else authors the
        // component beneath it.
        if local_component(source, key).0 {
            continue;
        }
        if let Some(owner) = inherited_owner(&before, &canonical, key) {
            return Err(format!(
                "component-inherited: the {key:?} component is authored by {owner} and not by \
                 this template, and the merge has no tombstone for a whole inherited table; \
                 materialise it and edit the local copy instead"
            ));
        }
    }
    let edited = document::edit(source, request)?;
    let mut candidate = files.clone();
    candidate.insert(path.to_owned(), edited.clone());
    let after = sources_of(&candidate, dependencies);
    introduced(
        member_issues(&canonical, source, &before),
        member_issues(&canonical, &edited, &after),
    )?;
    Ok(edited)
}

/// Write the resolved value at `address` into the local document as an exact
/// source override (criterion 2).
///
/// This is the ONLY place a runtime value is serialised into source, and it
/// writes NEW local text only: the value is `put` at the address's own path,
/// so every other byte — comments, ordering, unknown keys, the included files
/// themselves — is untouched. It refuses an address that is already local,
/// one the resolved document does not carry, and one naming an entry of an
/// array the merge reconciles by key that the local source has no entry for.
pub fn materialise(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
    path: &str,
    address: &str,
) -> Result<String, String> {
    let source = files
        .get(path)
        .ok_or_else(|| format!("unknown-document: {path:?} is not a draft member"))?;
    let canonical = canonical_template_path(path);
    let sources = sources_of(files, dependencies);
    let resolved = resolve_template(&canonical, &sources)
        .map_err(|error| format!("{}: {error}", map_resolver_category(error.category())))?;
    let steps = parse_address(address).ok_or_else(|| {
        format!("materialise-unknown-address: {address:?} is not a field address")
    })?;
    let origin = resolved.provenance.origin(address).ok_or_else(|| {
        format!("materialise-unknown-address: {address:?} is not a field of the resolved template")
    })?;
    if origin.source == canonical {
        return Err(format!(
            "materialise-local: {address:?} is already authored in this template"
        ));
    }
    let value = value_at(&resolved.value, &steps).ok_or_else(|| {
        format!("materialise-unknown-address: {address:?} is not in the resolved document")
    })?;
    let value_source = value_source_of(value).ok_or_else(|| {
        format!("materialise-unknown-address: {address:?} cannot be written as TOML source")
    })?;
    let put = put_path(source, &steps)?;
    let request = EditRequest {
        document_path: path.to_owned(),
        expected_source: source.clone(),
        edits: vec![Edit::Put {
            path: put,
            value_source,
        }],
    };
    let edited = document::edit(source, &request)?;
    let mut candidate = files.clone();
    candidate.insert(path.to_owned(), edited.clone());
    let after = sources_of(&candidate, dependencies);
    introduced(
        member_issues(&canonical, source, &sources),
        member_issues(&canonical, &edited, &after),
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

/// Include and composition findings over CANDIDATE entity members, resolved
/// against candidate ∪ beneath (candidate wins by path). Deterministic: sorted
/// by file, line, category and message, and deduped.
///
/// What this reports that nothing else does: the include rules at the exact
/// `includes` ENTRY line, for EVERY entity member the draft carries rather
/// than only the ones a manifest-listed world happens to spawn. The pack
/// gate's own composition check (`include_resolve::composition_finding`,
/// reached per spawned instance) names the world and entity that pulled a
/// template in and locates the declaring file by a text search; it cannot
/// point at the array entry, and it never looks at a template no world
/// reaches. Both are wanted, and neither is the other's duplicate.
///
/// `entity-unresolvable` follows the runtime's own asymmetry exactly: a
/// template that composed NOTHING and is not a valid entity is an ordinary
/// partial fragment, reported (if at all) by the source gate with its own
/// location, while a COMPOSED document that does not parse is this module's,
/// because that combination exists in no single authored file. Which member
/// includes which makes no difference to that rule — a hull is checked whether
/// or not some other template composes it.
pub fn findings(
    candidate: &BTreeMap<String, String>,
    beneath: &BTreeMap<String, String>,
) -> Vec<WorkshopFinding> {
    let mut sources = beneath.clone();
    sources.extend(candidate.clone());
    let graph = include_graph(&sources);
    let mut findings = Vec::new();
    for (path, source) in candidate {
        if !is_entity_path(path) {
            continue;
        }
        let canonical = canonical_template_path(path);
        let issues = include_issues(&canonical, source, &sources, &graph);
        let located = !issues.is_empty();
        for issue in issues {
            findings.push(finding(issue.category, path, issue.line, issue.message));
        }
        // One report per member: an entry the rules already named is not
        // restated as a whole-closure failure.
        if located {
            continue;
        }
        if let Some(issue) = runtime_issue(&canonical, &sources, includes_line(source), true) {
            findings.push(finding(
                "entity-unresolvable",
                path,
                issue.line,
                issue.message,
            ));
        }
    }
    sort_findings(&mut findings);
    findings
}

#[cfg(test)]
mod tests;
