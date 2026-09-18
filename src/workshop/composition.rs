//! World composition and scenario entry points read from exact source
//! (issue #1475).
//!
//! Composition is authored in three places: the manifest's `[[scenario]]`
//! roots, each world's `extra_worlds`, and script-driven `load_world` /
//! `unload_world` references. The first two are edited here; the third is
//! LISTED with its origin and validated, because editing Rhai is #1478's.
//!
//! Unlike the definition forms (#1474), a composition edit is checked
//! cross-file at edit time and REFUSED with the source untouched: a missing,
//! cyclic, duplicate or disallowed reference cannot enter the draft through
//! the panel, which is what "refuse without mutating the draft" asks for. The
//! same rules are also findings with lines in `validate_pack` and
//! `validate_project`, because a hand-edited draft can still violate them and
//! save and export must refuse it with a location; a Test clears the subset
//! its selected root composes (`selection_findings`) before it starts.
//!
//! Script references are a best-effort scan, and say so: TOML actions whose
//! `type` is `load_world`/`unload_world` are read structurally wherever the
//! schema nests them, while Rhai bodies — the string values of an inline
//! `[script]` table and the sibling `.rhai` a world declares — have their
//! comments blanked (`//` to the end of the line, nested `/* */`, string
//! literals left alone) and are then scanned line by line for a call carrying
//! a string literal, plainly quoted or `\"`-escaped inside a one-line TOML
//! body. A path built from a variable cannot be judged here and is not
//! listed; a call spelled inside another string literal is.
//!
//! The loader reads `extra_worlds` ONE level deep: a child's own
//! `extra_worlds` are not followed, so a cycle is never an infinite load, but
//! it is still refused here because the author's intent (a composition graph)
//! and the runtime's behaviour (a flat list) have silently parted.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use toml_edit::{Document, DocumentMut, Item, Table};

use super::document::{self, EditRequest};
use super::source_spans::{
    is_member, line_at, span_line, string_field, table_line, value_text, visit_tables,
};
use super::{Sources, WorkshopDependencies, WorkshopFinding};
use crate::entities::loader::TemplateLoader;
use crate::world::manifest::{build_catalog, parse_manifest};
use crate::world::mod_pack::is_allowed_content_path;

const WORLDS: &str = "assets/worlds/";
const ENTITIES: &str = "assets/entities/";
/// A mod pack's manifest sits at the archive root; a project's under assets.
const PACK_MANIFEST: &str = "scenarios.toml";
const PROJECT_MANIFEST: &str = "assets/scenarios.toml";
const ORIGIN_DRAFT: &str = "draft";
const ORIGIN_BASE: &str = "base";
/// Top-level manifest keys the runtime reads; anything else is preserved and
/// shown read-only.
const MANIFEST_KEYS: [&str; 3] = ["pack", "content", "scenario"];
/// The TOML action types and script host calls that compose a world at
/// runtime, with the short kind the catalog reports for each.
const LOAD_ACTIONS: [(&str, &str); 2] = [("load_world", "load"), ("unload_world", "unload")];

// ── Catalog types ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
pub struct CompositionCatalog {
    /// `None` when the candidate carries neither `scenarios.toml` nor
    /// `assets/scenarios.toml`: the panel then offers no roots to edit rather
    /// than a manifest that does not exist and an Apply it would refuse.
    pub manifest: Option<ManifestView>,
    pub worlds: Vec<WorldView>,
    pub members: Vec<MemberView>,
    pub choices: Choices,
    pub catalogue: Vec<CatalogueEntry>,
    pub findings: Vec<WorkshopFinding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ManifestView {
    pub path: String,
    /// `pack` when a `[pack]` header is present, else `project`.
    pub kind: String,
    pub pack: Option<PackHeader>,
    pub content: Option<ContentHeader>,
    pub scenarios: Vec<ScenarioView>,
    pub unknown_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PackHeader {
    pub id: String,
    pub name: String,
    pub version: String,
    pub line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ContentHeader {
    pub id: String,
    pub epoch: Option<i64>,
    pub line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScenarioView {
    pub index: usize,
    pub id: String,
    pub id_line: usize,
    pub world: String,
    pub world_line: usize,
    pub world_origin: Option<String>,
    pub label: Option<String>,
    pub ships: Vec<ScenarioShip>,
    /// Every `[[available_ships]]` template the referenced world offers.
    pub offered_ships: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScenarioShip {
    pub path: String,
    pub line: usize,
    pub offered: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorldView {
    pub path: String,
    pub origin: String,
    pub title: Option<String>,
    pub line: usize,
    pub extra_worlds: Vec<ExtraWorld>,
    pub script_refs: Vec<ScriptRef>,
    pub available_ships: Vec<AvailableShip>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExtraWorld {
    pub index: usize,
    pub path: String,
    pub line: usize,
    /// `None` when the path resolves to nothing in the candidate or beneath.
    pub origin: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScriptRef {
    pub path: String,
    pub line: usize,
    /// `load` or `unload`.
    pub kind: String,
    /// `trigger` for a TOML action, `inline-script` for a `[script]` body, or
    /// the sibling `.rhai` path the line refers to.
    pub source: String,
    pub origin: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AvailableShip {
    pub template_path: String,
    pub line: usize,
    pub origin: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemberView {
    pub path: String,
    pub origin: String,
    /// Whether a mod pack may carry the member at all.
    pub allowed: bool,
    /// The manifest and worlds that name this member.
    pub referenced_by: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Choices {
    pub worlds: Vec<Choice>,
    pub templates: Vec<Choice>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Choice {
    pub path: String,
    pub origin: String,
}

/// The serialisable mirror of `world::manifest::ScenarioCatalogEntry`: what
/// the lobby, a Test and a saved or exported catalogue would list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CatalogueEntry {
    pub id: String,
    pub world: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub ships: Vec<CatalogueShip>,
    pub origin: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CatalogueShip {
    pub template_path: String,
    pub label: Option<String>,
}

/// A composition edit is the same all-or-nothing group of structural edits a
/// definition edit is; only the cross-file checks around it differ.
pub type ComposeRequest = EditRequest;

// ── Effective member set ──────────────────────────────────────────────────────

/// One path of candidate ∪ dependencies with the origin that supplies it.
struct Member<'a> {
    path: &'a str,
    origin: String,
    source: &'a str,
    /// Draft first, then base, then each pack oldest first: the order the
    /// panel lists choices in.
    rank: usize,
}

/// Every path in candidate ∪ dependencies, the draft winning a path, then the
/// newest pack, then the base set — the order the runtime overlay resolves
/// in, so the origin shown is the file the runtime would actually read.
fn effective_members<'a>(
    files: &'a BTreeMap<String, String>,
    dependencies: &'a WorkshopDependencies,
) -> Vec<Member<'a>> {
    let mut members: BTreeMap<&str, Member<'a>> = BTreeMap::new();
    for (path, source) in &dependencies.base_files {
        members.insert(
            path,
            Member {
                path,
                origin: ORIGIN_BASE.to_owned(),
                source,
                rank: 1,
            },
        );
    }
    for (index, pack) in dependencies.packs.iter().enumerate() {
        for (path, source) in &pack.files {
            members.insert(
                path,
                Member {
                    path,
                    origin: format!("pack:{}", pack.id),
                    source,
                    rank: index + 2,
                },
            );
        }
    }
    for (path, source) in files {
        members.insert(
            path,
            Member {
                path,
                origin: ORIGIN_DRAFT.to_owned(),
                source,
                rank: 0,
            },
        );
    }
    let mut ordered: Vec<Member<'a>> = members.into_values().collect();
    ordered.sort_by(|a, b| (a.rank, a.path).cmp(&(b.rank, b.path)));
    ordered
}

/// Everything beneath the draft as one map, later packs winning, exactly as
/// `validate_pack` resolves references.
fn beneath_of(dependencies: &WorkshopDependencies) -> BTreeMap<String, String> {
    let mut beneath = dependencies.base_files.clone();
    for pack in &dependencies.packs {
        beneath.extend(pack.files.clone());
    }
    beneath
}

fn origin_of(members: &[Member<'_>], path: &str) -> Option<String> {
    members
        .iter()
        .find(|member| member.path == path)
        .map(|member| member.origin.clone())
}

/// `assets/worlds/<name>.toml`, directly under the directory, as both the
/// pack rules and the loader's flat layout expect.
fn is_world_path(path: &str) -> bool {
    path.strip_prefix(WORLDS).is_some_and(|name| {
        name.ends_with(".toml") && name.len() > ".toml".len() && !name.contains('/')
    })
}

fn is_world_script(path: &str) -> bool {
    path.starts_with(WORLDS) && path.ends_with(".rhai")
}

/// The manifest member the candidate carries: a pack's at the root, a
/// project's under assets. A pack is judged by its root manifest even when a
/// project-style one is also present.
fn manifest_member(candidate: &BTreeMap<String, String>) -> Option<(&'static str, &String)> {
    [PACK_MANIFEST, PROJECT_MANIFEST]
        .into_iter()
        .find_map(|path| candidate.get(path).map(|source| (path, source)))
}

// ── Structural reads ──────────────────────────────────────────────────────────

/// The `extra_worlds` entries of a world with the line of each, read from the
/// tree so a world the runtime refuses still shows what it declares.
fn extra_worlds_of(source: &str) -> Vec<(String, usize)> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    let root = document.as_table();
    let Some(item) = root.get("extra_worlds") else {
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

/// The `[[available_ships]]` template paths of a world with their lines.
fn available_ships_of(source: &str) -> Vec<(String, usize)> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    document
        .as_table()
        .get("available_ships")
        .and_then(Item::as_array_of_tables)
        .map(|ships| {
            ships
                .iter()
                .filter_map(|ship| {
                    let value = ship.get("template_path").and_then(Item::as_value)?;
                    Some((
                        value_text(source, value),
                        span_line(source, value.span()).unwrap_or_else(|| table_line(source, ship)),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The sibling `.rhai` a top-level `script = "file.rhai"` names, relative to
/// the world's directory exactly as `world::script::load` resolves it (that
/// helper is compiled for the browser and tests only).
fn declared_sibling(world_path: &str, source: &str) -> Option<String> {
    let document = Document::parse(source).ok()?;
    let relative = document
        .as_table()
        .get("script")?
        .as_str()?
        .replace('\\', "/");
    Some(match world_path.rfind('/') {
        Some(index) => format!("{}/{relative}", &world_path[..index]),
        None => relative,
    })
}

fn title_of(source: &str) -> Option<String> {
    let document = Document::parse(source).ok()?;
    let global = document.as_table().get("global")?.as_table_like()?;
    string_field(global, "title").map(str::to_owned)
}

/// TOML actions of `type = "load_world" | "unload_world"` wherever the world
/// schema nests them: `(path, line, kind)`.
fn trigger_refs(source: &str) -> Vec<(String, usize, &'static str)> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    visit_tables(document.as_item(), &mut |table| {
        let Some(kind) = string_field(table, "type")
            .and_then(|action| LOAD_ACTIONS.iter().find(|(name, _)| *name == action))
            .map(|(_, kind)| *kind)
        else {
            return;
        };
        let Some(path) = table.get("path").and_then(Item::as_value) else {
            return;
        };
        let line = span_line(source, path.span())
            .or_else(|| {
                table
                    .get("type")
                    .and_then(Item::as_value)
                    .and_then(|value| span_line(source, value.span()))
            })
            .unwrap_or(1);
        refs.push((value_text(source, path), line, kind));
    });
    refs
}

/// The one string literal of a `load_world("…")` call starting at `text`:
/// plainly quoted, or `\"`-escaped as a one-line TOML basic string body
/// writes it.
fn literal(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let (rest, closer) = match text.strip_prefix("\\\"") {
        Some(rest) => (rest, "\\\""),
        None => (text.strip_prefix('"')?, "\""),
    };
    let end = rest.find(closer)?;
    Some(&rest[..end])
}

/// The byte length of the nested block comment opening `text`, or the whole
/// text when it never closes.
fn block_comment_end(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"/*") {
            depth += 1;
            index += 2;
        } else if bytes[index..].starts_with(b"*/") {
            depth = depth.saturating_sub(1);
            index += 2;
            if depth == 0 {
                return index;
            }
        } else {
            index += 1;
        }
    }
    text.len()
}

/// The byte length of the string literal opening `text`, if one does: `"…"`
/// with backslash escapes, `` `…` `` raw, `'c'` (one character, escaped or
/// not), or `\"…\"` with its escapes doubled as a one-line TOML basic string
/// body carries it. An unterminated literal runs to the end of the text.
fn string_end(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let (opener, closer, escapes): (usize, &[u8], bool) = if text.starts_with("\\\"") {
        (2, b"\\\"", true)
    } else if text.starts_with('"') {
        (1, b"\"", true)
    } else if text.starts_with('`') {
        (1, b"`", false)
    } else if let Some(rest) = text.strip_prefix('\'') {
        // A char literal is exactly one (possibly escaped) character wide;
        // any other apostrophe is not a literal and is left as text.
        let close = if rest.as_bytes().first() == Some(&b'\\') {
            3
        } else {
            1 + rest.chars().next().map_or(0, char::len_utf8)
        };
        return (bytes.get(close) == Some(&b'\'')).then_some(close + 1);
    } else {
        return None;
    };
    let mut index = opener;
    while index < bytes.len() {
        if bytes[index..].starts_with(closer) {
            return Some(index + closer.len());
        }
        // An escape pair (`\n`, `\\`, an escaped quote) is skipped whole so
        // the quote it protects cannot close the literal.
        index += if escapes && bytes[index] == b'\\' {
            2
        } else {
            1
        };
    }
    Some(text.len())
}

/// Rhai source with every comment blanked to spaces — `//` to the end of the
/// line and `/* … */`, nested as Rhai allows — so a commented-out call is
/// never listed and never refuses a Test, a save or an export. Newlines
/// survive, so line numbers counted over the result are the file's own.
/// String literals are copied intact and a comment cannot open inside one,
/// so a `//` in a URL is text.
fn strip_rhai_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(first) = rest.chars().next() {
        if rest.starts_with("//") {
            let end = rest.find('\n').unwrap_or(rest.len());
            out.extend(rest[..end].chars().map(|_| ' '));
            rest = &rest[end..];
        } else if rest.starts_with("/*") {
            let end = block_comment_end(rest);
            out.extend(
                rest[..end]
                    .chars()
                    .map(|ch| if ch == '\n' { '\n' } else { ' ' }),
            );
            rest = &rest[end..];
        } else if let Some(end) = string_end(rest) {
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        } else {
            out.push(first);
            rest = &rest[first.len_utf8()..];
        }
    }
    out
}

/// Script host calls carrying a literal path, one per line of the
/// comment-stripped text, as `(path, line, kind)`. `unload_world(` contains
/// `load_world(`, so a match preceded by an identifier byte belongs to the
/// longer name.
fn literal_refs(text: &str) -> Vec<(String, usize, &'static str)> {
    let stripped = strip_rhai_comments(text);
    let mut refs = Vec::new();
    for (index, line) in stripped.lines().enumerate() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        for (action, kind) in LOAD_ACTIONS {
            let call = format!("{action}(");
            for (offset, _) in line.match_indices(call.as_str()) {
                let preceded_by_identifier = line[..offset]
                    .bytes()
                    .last()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                if preceded_by_identifier {
                    continue;
                }
                if let Some(path) = literal(&line[offset + call.len()..]) {
                    refs.push((path.to_owned(), index + 1, kind));
                }
            }
        }
    }
    refs
}

/// The text between a TOML string's delimiters (`"""`, `'''`, `"` or `'`).
fn body_of(raw: &str) -> &str {
    ["\"\"\"", "'''", "\"", "'"]
        .into_iter()
        .find_map(|delimiter| {
            raw.strip_prefix(delimiter)
                .and_then(|rest| rest.strip_suffix(delimiter))
        })
        .unwrap_or(raw)
}

/// The calls in a world's inline `[script]` bodies: every string value of
/// that table, scanned as the raw source between its delimiters so the line
/// of each call is the file's own line — and only there, because a call
/// spelled in a description or an anchor name is prose, not a composition.
fn script_body_refs(source: &str) -> Vec<(String, usize, &'static str)> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    let Some(script) = document
        .as_table()
        .get("script")
        .and_then(Item::as_table_like)
    else {
        return Vec::new();
    };
    let mut refs = Vec::new();
    for (_, item) in script.iter() {
        let Some(span) = item
            .as_value()
            .filter(|value| value.is_str())
            .and_then(|value| value.span())
        else {
            continue;
        };
        let first_line = line_at(source, span.start);
        refs.extend(
            literal_refs(body_of(&source[span]))
                .into_iter()
                .map(|(path, line, kind)| (path, first_line + line - 1, kind)),
        );
    }
    refs
}

/// Every path the manifest's roots name: each world and each curated ship.
fn manifest_references(source: &str) -> Vec<String> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if let Some(scenarios) = document
        .as_table()
        .get("scenario")
        .and_then(Item::as_array_of_tables)
    {
        for scenario in scenarios.iter() {
            if let Some(world) = scenario.get("world").and_then(Item::as_value) {
                paths.push(value_text(source, world));
            }
            if let Some(ships) = scenario.get("ships").and_then(Item::as_array) {
                paths.extend(ships.iter().map(|ship| value_text(source, ship)));
            }
        }
    }
    paths
}

/// Every path a world names: its extra worlds, the worlds its actions and
/// scripts (inline and the declared sibling) load or unload, and the hulls
/// it offers.
fn world_references(
    path: &str,
    source: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut paths: Vec<String> = extra_worlds_of(source)
        .into_iter()
        .map(|(child, _)| child)
        .collect();
    paths.extend(
        trigger_refs(source)
            .into_iter()
            .map(|(target, _, _)| target),
    );
    paths.extend(
        script_body_refs(source)
            .into_iter()
            .map(|(target, _, _)| target),
    );
    if let Some(text) = declared_sibling(path, source).and_then(|sibling| lookup(&sibling)) {
        paths.extend(literal_refs(&text).into_iter().map(|(target, _, _)| target));
    }
    paths.extend(
        available_ships_of(source)
            .into_iter()
            .map(|(template, _)| template),
    );
    paths
}

/// Which manifest and worlds name each path: the one reading the Members
/// section's "referenced by" and the `member-disallowed` finding share, so
/// the two cannot disagree about who depends on a member.
fn referrers<'a>(
    worlds: impl Iterator<Item = (&'a str, &'a str)>,
    manifest: Option<(&'a str, &'a str)>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut referenced: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    if let Some((path, source)) = manifest {
        for target in manifest_references(source) {
            referenced
                .entry(target)
                .or_default()
                .insert(path.to_owned());
        }
    }
    for (path, source) in worlds {
        for target in world_references(path, source, lookup) {
            referenced
                .entry(target)
                .or_default()
                .insert(path.to_owned());
        }
    }
    referenced
}

// ── Catalog ───────────────────────────────────────────────────────────────────

fn read_manifest(
    path: &str,
    source: &str,
    members: &[Member<'_>],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> ManifestView {
    let mut view = ManifestView {
        path: path.to_owned(),
        kind: "project".into(),
        pack: None,
        content: None,
        scenarios: Vec::new(),
        unknown_keys: Vec::new(),
    };
    let Ok(document) = Document::parse(source) else {
        return view;
    };
    let root = document.as_table();
    let header_line = |item: &Item| span_line(source, item.span()).unwrap_or(1);
    if let Some(item) = root.get("pack") {
        if let Some(pack) = item.as_table_like() {
            let text = |key: &str| string_field(pack, key).unwrap_or_default().to_owned();
            view.kind = "pack".into();
            view.pack = Some(PackHeader {
                id: text("id"),
                name: text("name"),
                version: text("version"),
                line: header_line(item),
            });
        }
    }
    if let Some(item) = root.get("content") {
        if let Some(content) = item.as_table_like() {
            view.content = Some(ContentHeader {
                id: string_field(content, "id").unwrap_or_default().to_owned(),
                epoch: content.get("epoch").and_then(Item::as_integer),
                line: header_line(item),
            });
        }
    }
    if let Some(scenarios) = root.get("scenario").and_then(Item::as_array_of_tables) {
        for (index, scenario) in scenarios.iter().enumerate() {
            let entry_line = table_line(source, scenario);
            let scalar = |key: &str| -> (String, usize) {
                match scenario.get(key).and_then(Item::as_value) {
                    Some(value) => (
                        value_text(source, value),
                        span_line(source, value.span()).unwrap_or(entry_line),
                    ),
                    None => (String::new(), entry_line),
                }
            };
            let (id, id_line) = scalar("id");
            let (world, world_line) = scalar("world");
            let offered_ships: Vec<String> = lookup(&world)
                .map(|text| {
                    available_ships_of(&text)
                        .into_iter()
                        .map(|(template, _)| template)
                        .collect()
                })
                .unwrap_or_default();
            let ships = scenario
                .get("ships")
                .and_then(Item::as_array)
                .map(|array| {
                    array
                        .iter()
                        .map(|element| {
                            let path = value_text(source, element);
                            ScenarioShip {
                                offered: offered_ships.contains(&path),
                                path,
                                line: span_line(source, element.span()).unwrap_or(entry_line),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            view.scenarios.push(ScenarioView {
                index,
                id,
                id_line,
                world_origin: origin_of(members, &world),
                world,
                world_line,
                label: string_field(scenario, "label").map(str::to_owned),
                ships,
                offered_ships,
            });
        }
    }
    view.unknown_keys = root
        .iter()
        .map(|(key, _)| key)
        .filter(|key| !MANIFEST_KEYS.contains(key))
        .map(str::to_owned)
        .collect();
    view
}

fn read_world(
    member: &Member<'_>,
    members: &[Member<'_>],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> WorldView {
    let source = member.source;
    let mut script_refs: Vec<ScriptRef> = trigger_refs(source)
        .into_iter()
        .map(|(path, line, kind)| ScriptRef {
            origin: origin_of(members, &path),
            path,
            line,
            kind: kind.into(),
            source: "trigger".into(),
        })
        .chain(
            script_body_refs(source)
                .into_iter()
                .map(|(path, line, kind)| ScriptRef {
                    origin: origin_of(members, &path),
                    path,
                    line,
                    kind: kind.into(),
                    source: "inline-script".into(),
                }),
        )
        .collect();
    // The sibling file the world declares, resolved the way the loader
    // resolves it; a missing one is already `script-file-missing`.
    if let Some(sibling) = declared_sibling(member.path, source) {
        if let Some(text) = lookup(&sibling) {
            script_refs.extend(literal_refs(&text).into_iter().map(|(path, line, kind)| {
                ScriptRef {
                    origin: origin_of(members, &path),
                    path,
                    line,
                    kind: kind.into(),
                    source: sibling.clone(),
                }
            }));
        }
    }
    WorldView {
        path: member.path.to_owned(),
        origin: member.origin.clone(),
        title: title_of(source),
        line: 1,
        extra_worlds: extra_worlds_of(source)
            .into_iter()
            .enumerate()
            .map(|(index, (path, line))| ExtraWorld {
                index,
                origin: origin_of(members, &path),
                path,
                line,
            })
            .collect(),
        script_refs,
        available_ships: available_ships_of(source)
            .into_iter()
            .map(|(template_path, line)| AvailableShip {
                origin: origin_of(members, &template_path),
                template_path,
                line,
            })
            .collect(),
    }
}

/// Every composition an author can see: the manifest, each world with what
/// it composes, the members and the runtime's own choices, the draft's
/// members editable and everything beneath listed read-only under its origin.
pub fn catalog(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
) -> CompositionCatalog {
    let members = effective_members(files, dependencies);
    let beneath = beneath_of(dependencies);
    let lookup =
        |path: &str| -> Option<String> { files.get(path).or_else(|| beneath.get(path)).cloned() };
    let manifest_source = manifest_member(files);
    let manifest =
        manifest_source.map(|(path, source)| read_manifest(path, source, &members, &lookup));
    let worlds: Vec<WorldView> = members
        .iter()
        .filter(|member| is_member(member.path, WORLDS))
        .map(|member| read_world(member, &members, &lookup))
        .collect();
    let referenced_by = referrers(
        members
            .iter()
            .filter(|member| is_member(member.path, WORLDS))
            .map(|member| (member.path, member.source)),
        manifest_source.map(|(path, source)| (path, source.as_str())),
        &lookup,
    );

    // A hull choice is what a Test may select: a complete composed template
    // with a ship configuration, resolved over candidate ∪ beneath.
    let mut merged = beneath.clone();
    merged.extend(files.clone());
    let loader = Sources(merged);
    let templates = members
        .iter()
        .filter(|member| is_member(member.path, ENTITIES))
        .filter(|member| {
            loader
                .load_template(member.path)
                .is_some_and(|hull| hull.class.is_some() && hull.ship_config.is_some())
        })
        .map(|member| Choice {
            path: member.path.to_owned(),
            origin: member.origin.clone(),
        })
        .collect();

    let mut findings = findings(files, &beneath);
    // A draft member the tree cannot read has nothing to show; say why here
    // so the panel is not simply missing it. Check reports the same syntax
    // error through the runtime parsers, which is the gate.
    let manifest_path = manifest_source.map(|(path, _)| path);
    for (path, source) in files {
        if Some(path.as_str()) == manifest_path || is_member(path, WORLDS) {
            if let Err(error) = Document::parse(source.as_str()) {
                findings.push(WorkshopFinding {
                    severity: "error".into(),
                    category: "runtime-source-invalid".into(),
                    message: error.message().to_owned(),
                    file: path.clone(),
                    line: error.span().map(|span| line_at(source, span.start)),
                });
            }
        }
    }
    sort_findings(&mut findings);

    let member_views = members
        .iter()
        .map(|member| MemberView {
            path: member.path.to_owned(),
            origin: member.origin.clone(),
            allowed: is_allowed_content_path(member.path),
            referenced_by: referenced_by
                .get(member.path)
                .map(|paths| paths.iter().cloned().collect())
                .unwrap_or_default(),
        })
        .collect();
    // Only a world the rules accept as a root or a child is offered: a
    // nested one (a project workspace admits any assets/**.toml) would be
    // refused as disallowed the moment it was chosen.
    let world_choices = members
        .iter()
        .filter(|member| is_world_path(member.path))
        .map(|member| Choice {
            path: member.path.to_owned(),
            origin: member.origin.clone(),
        })
        .collect();
    CompositionCatalog {
        catalogue: scenario_catalogue(files, &beneath),
        manifest,
        worlds,
        members: member_views,
        choices: Choices {
            worlds: world_choices,
            templates,
        },
        findings,
    }
}

// ── Same validated catalogue ──────────────────────────────────────────────────

/// The catalogue the lobby would list for this candidate: the runtime's own
/// `build_catalog` over the candidate manifest, worlds resolved through
/// candidate ∪ beneath. `validate_pack` and `validate_project` read the same
/// candidate, so a pack and a project of identical members catalogue alike.
pub fn scenario_catalogue(
    candidate: &BTreeMap<String, String>,
    beneath: &BTreeMap<String, String>,
) -> Vec<CatalogueEntry> {
    let Some((_, text)) = manifest_member(candidate) else {
        return Vec::new();
    };
    let Ok(manifest) = parse_manifest(text) else {
        return Vec::new();
    };
    build_catalog(&manifest, |path| {
        candidate.get(path).or_else(|| beneath.get(path)).cloned()
    })
    .scenarios
    .into_iter()
    .map(|entry| CatalogueEntry {
        id: entry.id,
        world: entry.world,
        label: entry.label,
        description: entry.description,
        ships: entry
            .ships
            .into_iter()
            .map(|ship| CatalogueShip {
                template_path: ship.template_path,
                label: ship.label,
            })
            .collect(),
        origin: entry.origin,
    })
    .collect()
}

// ── New world skeleton ────────────────────────────────────────────────────────

/// A minimal world the runtime's own reader accepts: `[global]` with the
/// title. Built through the syntax tree rather than by serialising
/// `GlobalConfig`, which would pin every tuning default into the file.
pub fn new_world_source(title: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("A world needs a title.".into());
    }
    let mut global = Table::new();
    global.insert("title", toml_edit::value(title));
    let mut document = DocumentMut::new();
    document.insert("global", Item::Table(global));
    let source = document.to_string();
    crate::world::config::parse_world(&source)?;
    Ok(source)
}

// ── Rules shared by refusals and findings ─────────────────────────────────────

/// One violated composition rule: the line it sits on, its category (the
/// finding category, and the rule name a refusal message opens with), a
/// message naming the offending value, and a KEY — the offending value(s)
/// without the array slot — that identifies the violation across an edit
/// which only moves the entry.
struct Issue {
    line: usize,
    category: &'static str,
    key: String,
    message: String,
}

/// The `extra_worlds` graph over candidate ∪ beneath, the candidate winning a
/// path, as edges from each world to what it declares.
fn extra_worlds_graph(
    candidate: &BTreeMap<String, String>,
    beneath: &BTreeMap<String, String>,
) -> BTreeMap<String, Vec<String>> {
    let mut graph = BTreeMap::new();
    for (path, source) in beneath.iter().chain(candidate) {
        if is_member(path, WORLDS) {
            graph.insert(
                path.clone(),
                extra_worlds_of(source)
                    .into_iter()
                    .map(|(child, _)| child)
                    .collect(),
            );
        }
    }
    graph
}

/// The path `start -> ... -> target` through the graph, depth first in
/// authored order, or `None` when `target` is unreachable.
fn path_to(
    graph: &BTreeMap<String, Vec<String>>,
    start: &str,
    target: &str,
) -> Option<Vec<String>> {
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![vec![start.to_owned()]];
    while let Some(path) = stack.pop() {
        let Some(node) = path.last() else {
            continue;
        };
        if node == target {
            return Some(path);
        }
        if !visited.insert(node.clone()) {
            continue;
        }
        let Some(children) = graph.get(node) else {
            continue;
        };
        // Reversed so the first authored child is explored first.
        for child in children.iter().rev() {
            let mut next = path.clone();
            next.push(child.clone());
            stack.push(next);
        }
    }
    None
}

/// The rules one world's `extra_worlds` list breaks, in authored order. A
/// repeated entry carries only the duplicate rule; the first occurrence is
/// the one judged for existence and cycles.
fn extra_world_issues(
    path: &str,
    source: &str,
    exists: &dyn Fn(&str) -> bool,
    graph: &BTreeMap<String, Vec<String>>,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, (entry, line)) in extra_worlds_of(source).into_iter().enumerate() {
        let issue = |category, message| Issue {
            line,
            category,
            key: entry.clone(),
            message,
        };
        if !is_world_path(&entry) {
            issues.push(issue(
                "extra-worlds-disallowed",
                format!("extra_worlds[{index}] {entry:?} is not an assets/worlds/*.toml path"),
            ));
            continue;
        }
        if entry == path {
            issues.push(issue(
                "extra-worlds-self",
                format!("extra_worlds[{index}] names the world itself"),
            ));
            continue;
        }
        if !seen.insert(entry.clone()) {
            issues.push(issue(
                "extra-worlds-duplicate",
                format!("extra_worlds[{index}] {entry:?} is already listed"),
            ));
            continue;
        }
        if !exists(&entry) {
            issues.push(issue(
                "extra-worlds-missing",
                format!("extra_worlds[{index}] {entry:?} is not in the draft or its dependencies"),
            ));
            continue;
        }
        if let Some(cycle) = path_to(graph, &entry, path) {
            let mut listed = vec![path.to_owned()];
            listed.extend(cycle);
            issues.push(issue(
                "extra-worlds-cycle",
                format!(
                    "extra_worlds[{index}] {entry:?} composes a cycle: {}",
                    listed.join(" -> ")
                ),
            ));
        }
    }
    issues
}

/// The rules the manifest's `[[scenario]]` entries break, in authored order.
/// Only `scenario-world-disallowed` is new; the rest restate what
/// `validate_manifest` reports, so an edit can be refused before the manifest
/// carries them.
fn manifest_issues(source: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Vec<Issue> {
    let Ok(document) = Document::parse(source) else {
        return Vec::new();
    };
    let Some(scenarios) = document
        .as_table()
        .get("scenario")
        .and_then(Item::as_array_of_tables)
    else {
        return Vec::new();
    };
    let mut issues = Vec::new();
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for (index, scenario) in scenarios.iter().enumerate() {
        let entry_line = table_line(source, scenario);
        let scalar = |key: &str| -> (String, usize) {
            match scenario.get(key).and_then(Item::as_value) {
                Some(value) => (
                    value_text(source, value),
                    span_line(source, value.span()).unwrap_or(entry_line),
                ),
                None => (String::new(), entry_line),
            }
        };
        let (id, id_line) = scalar("id");
        let (world, world_line) = scalar("world");
        if id.trim().is_empty() {
            issues.push(Issue {
                line: id_line,
                category: "invalid-manifest-entry",
                key: "id".into(),
                message: format!("scenario[{index}] has an empty id"),
            });
        } else if !ids.insert(id.trim().to_owned()) {
            issues.push(Issue {
                line: id_line,
                category: "duplicate-scenario-id",
                key: id.trim().to_owned(),
                message: format!("scenario id {:?} is declared more than once", id.trim()),
            });
        }
        if world.trim().is_empty() {
            issues.push(Issue {
                line: world_line,
                category: "invalid-manifest-entry",
                key: "world".into(),
                message: format!("scenario[{index}] has an empty world path"),
            });
            continue;
        }
        if !is_world_path(&world) {
            issues.push(Issue {
                line: world_line,
                category: "scenario-world-disallowed",
                key: world.clone(),
                message: format!(
                    "scenario[{index}] world {world:?} is not an assets/worlds/*.toml path"
                ),
            });
            continue;
        }
        let Some(world_source) = lookup(&world) else {
            issues.push(Issue {
                line: world_line,
                category: "missing-scenario-world",
                key: world.clone(),
                message: format!(
                    "scenario[{index}] world {world:?} is not in the draft or its dependencies"
                ),
            });
            continue;
        };
        let offered: Vec<String> = available_ships_of(&world_source)
            .into_iter()
            .map(|(template, _)| template)
            .collect();
        if let Some(ships) = scenario.get("ships").and_then(Item::as_array) {
            for element in ships.iter() {
                let ship = value_text(source, element);
                if !offered.contains(&ship) {
                    issues.push(Issue {
                        line: span_line(source, element.span()).unwrap_or(entry_line),
                        category: "unknown-scenario-ship",
                        // The rule is relational: the same ship against
                        // another world is another violation.
                        key: format!("{ship} for {world}"),
                        message: format!("scenario[{index}] curates ship {ship:?} which world {world:?} does not offer"),
                    });
                }
            }
        }
    }
    issues
}

// ── Composition edits ─────────────────────────────────────────────────────────

/// A refusal message: the rule, then the offending value, so the panel can
/// map the rule to a string id and still show what was refused.
fn refusal(issue: &Issue) -> String {
    format!("{}: {}", issue.category, issue.message)
}

/// The rules the edited member would break AFTER the edit that it did not
/// break before it. Rules the edit does not touch are findings, not
/// refusals: an author must be able to fix a hand-broken draft one edit at a
/// time. Violations are matched by category and offending value, never by
/// message: every message names the entry's array slot, and removing or
/// reordering an earlier entry shifts the slots after it, so a violation the
/// member already carried must not read as new at its new index. The counts
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

/// Apply a composition edit to a COPY of the member and refuse it — the
/// error, the source untouched — when the edited member introduces a
/// missing, cyclic, duplicate or disallowed reference over candidate ∪
/// dependencies (the candidate winning a path). Rules that do not concern the
/// edited member are not re-checked here; they are findings.
pub fn compose(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
    request: &ComposeRequest,
) -> Result<String, String> {
    let path = request.document_path.as_str();
    let source = files
        .get(path)
        .ok_or_else(|| format!("unknown-document: {path:?} is not a draft member"))?;
    let edited = document::edit(source, request)?;
    let beneath = beneath_of(dependencies);
    let mut candidate = files.clone();
    candidate.insert(path.to_owned(), edited.clone());
    let lookup = |path: &str| -> Option<String> {
        candidate.get(path).or_else(|| beneath.get(path)).cloned()
    };
    if path == PACK_MANIFEST || path == PROJECT_MANIFEST {
        // The headers carry the identity every gate reads first; an edit
        // that drops one would only be told at Check.
        let had = |text: &str, key: &str| {
            Document::parse(text).is_ok_and(|document| document.as_table().contains_key(key))
        };
        for header in ["pack", "content"] {
            if had(source, header) && !had(&edited, header) {
                return Err(format!(
                    "manifest-header-removed: the [{header}] table must stay"
                ));
            }
        }
        introduced(
            manifest_issues(source, &lookup),
            manifest_issues(&edited, &lookup),
        )?;
    } else if is_member(path, WORLDS) {
        let exists = |target: &str| lookup(target).is_some();
        let graph_before = extra_worlds_graph(files, &beneath);
        let graph_after = extra_worlds_graph(&candidate, &beneath);
        introduced(
            extra_world_issues(path, source, &exists, &graph_before),
            extra_world_issues(path, &edited, &exists, &graph_after),
        )?;
    }
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

/// Composition findings over CANDIDATE members, resolved against candidate ∪
/// beneath (candidate wins by path). Lines refer to candidate sources.
/// Deterministic: sorted by file, line, category, message and deduped.
///
/// `member-disallowed` is a warning for a project workspace only, and only
/// for a member the manifest or a world NAMES. A pack already refuses such a
/// member through the archive gate's `disallowed-path`; a project workspace
/// legitimately carries members a pack never could (audio catalogues, join
/// codes, an alternative manifest), and every member's allowed flag is
/// already on the Members section, so a warning on each would be noise on
/// every Check of the shipped project. What a project is told is which of
/// the members its ROOTS AND WORLDS depend on could not follow them into a
/// pack. A candidate carrying a root `scenarios.toml` is a pack.
pub fn findings(
    candidate: &BTreeMap<String, String>,
    beneath: &BTreeMap<String, String>,
) -> Vec<WorkshopFinding> {
    let lookup = |path: &str| -> Option<String> {
        candidate.get(path).or_else(|| beneath.get(path)).cloned()
    };
    let exists = |path: &str| candidate.contains_key(path) || beneath.contains_key(path);
    let graph = extra_worlds_graph(candidate, beneath);
    let mut findings = Vec::new();
    for (path, source) in candidate {
        if is_member(path, WORLDS) {
            for issue in extra_world_issues(path, source, &exists, &graph) {
                findings.push(finding(
                    issue.category,
                    path,
                    Some(issue.line),
                    issue.message,
                ));
            }
        }
        let references = if is_member(path, WORLDS) {
            let mut references = trigger_refs(source);
            references.extend(script_body_refs(source));
            references
        } else if is_world_script(path) {
            literal_refs(source)
        } else {
            continue;
        };
        for (target, line, kind) in references {
            if !exists(&target) {
                findings.push(finding(
                    "world-missing-load-reference",
                    path,
                    Some(line),
                    format!(
                        "{kind}_world reference {target:?} is not in the draft or its dependencies"
                    ),
                ));
            }
        }
    }
    if let Some((path, source)) = manifest_member(candidate) {
        for issue in manifest_issues(source, &lookup) {
            if issue.category == "scenario-world-disallowed" {
                findings.push(finding(
                    issue.category,
                    path,
                    Some(issue.line),
                    issue.message,
                ));
            }
        }
    }
    if !candidate.contains_key(PACK_MANIFEST) {
        let referenced = referrers(
            candidate
                .iter()
                .filter(|(path, _)| is_member(path, WORLDS))
                .map(|(path, source)| (path.as_str(), source.as_str())),
            manifest_member(candidate).map(|(path, source)| (path, source.as_str())),
            &lookup,
        );
        for (path, named_by) in &referenced {
            if candidate.contains_key(path) && !is_allowed_content_path(path) {
                findings.push(WorkshopFinding {
                    severity: "warning".into(),
                    category: "member-disallowed".into(),
                    message: format!(
                        "Member {path:?} is named by {} but is outside the paths a mod pack may carry",
                        named_by.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                    file: path.clone(),
                    line: None,
                });
            }
        }
    }
    sort_findings(&mut findings);
    findings
}

/// The composition findings a Test selection must clear before it starts:
/// the rules broken by the selected root, by the worlds its `extra_worlds`
/// compose and by the sibling scripts those declare, over the captured set
/// (one flat map, the draft already laid over its dependencies). A Test
/// runs the exact candidate, so a root that composes a duplicate, cyclic or
/// dangling reference is refused here as save and export refuse it; what
/// the selection does not compose stays with Check.
pub fn selection_findings(files: &BTreeMap<String, String>, root: &str) -> Vec<WorkshopFinding> {
    let Some(source) = files.get(root) else {
        return Vec::new();
    };
    let mut scope: BTreeSet<String> = BTreeSet::from([root.to_owned()]);
    scope.extend(extra_worlds_of(source).into_iter().map(|(child, _)| child));
    for world in scope.clone() {
        if let Some(sibling) = files
            .get(&world)
            .and_then(|text| declared_sibling(&world, text))
        {
            scope.insert(sibling);
        }
    }
    findings(files, &BTreeMap::new())
        .into_iter()
        .filter(|finding| scope.contains(&finding.file))
        .collect()
}

#[cfg(test)]
mod tests;
