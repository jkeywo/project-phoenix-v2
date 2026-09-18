//! Faction and complexity definitions read from exact source (issue #1474).
//!
//! The runtime types own the vocabulary: `FactionConfig` says which keys a
//! faction has, `OrderResponse`'s serde spelling says what a compliance verb
//! may answer, `console_ai::server` says which AI rules exist and
//! `ComplianceDisposition::default()` says what an unauthored table means. The
//! TOML syntax tree supplies the line of every reference, so a finding can
//! point at the rung or the enemy entry the runtime would refuse — the
//! runtime's own whole-file `toml::from_str` never knew a line.
//!
//! Cross-file validity (does the enemy exist? is the system owned?) is a
//! finding, never an edit-time refusal: an author adds the enemy first and the
//! faction second, and the Check button says what is still dangling.
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use toml_edit::{Document, Item, Table, TableLike, Value};
use uuid::Uuid;

use super::document::Segment;
use super::{WorkshopDependencies, WorkshopFinding};
use crate::ai::faction::FactionConfig;
use crate::civilian::{ComplianceDisposition, OrderResponse};
use crate::core::messages::{StationId, SystemId};
use crate::ship::config::ShipConfigError;

const FACTIONS: &str = "assets/factions/";
const ENTITIES: &str = "assets/entities/";
const WORLDS: &str = "assets/worlds/";
const ORIGIN_DRAFT: &str = "draft";
const ORIGIN_BASE: &str = "base";
/// Top-level keys `FactionConfig` reads; anything else is preserved and shown
/// read-only, because the type has no `deny_unknown_fields`.
const FACTION_KEYS: [&str; 5] = ["uuid", "name", "display_name", "enemies", "compliance"];
const FACTION_ENEMY_ACTIONS: [&str; 2] = ["add_faction_enemy", "remove_faction_enemy"];

#[derive(Clone, Debug, Serialize)]
pub struct DefinitionCatalog {
    pub factions: Vec<FactionDefinition>,
    pub hulls: Vec<HullDefinition>,
    pub choices: Choices,
    pub defaults: Defaults,
    pub findings: Vec<WorkshopFinding>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Choices {
    pub factions: Vec<FactionChoice>,
    pub order_responses: Vec<String>,
    pub ai_rules: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct FactionChoice {
    pub uuid: String,
    pub name: String,
    pub origin: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Defaults {
    pub compliance: ComplianceDefaults,
}

#[derive(Clone, Debug, Serialize)]
pub struct ComplianceDefaults {
    pub ack_secs: i64,
    pub decide_secs: i64,
    pub hold: String,
    pub divert: String,
    pub dock: String,
}

/// Exact TOML value text and its 1-based line.
#[derive(Clone, Debug, Serialize)]
pub struct SourceSpan {
    pub source: String,
    pub line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct FactionDefinition {
    pub path: String,
    pub origin: String,
    pub uuid: Option<String>,
    pub uuid_line: Option<usize>,
    pub name: Option<String>,
    pub name_line: Option<usize>,
    pub display_name: Option<String>,
    pub display_name_line: Option<usize>,
    pub enemies: Vec<EnemyReference>,
    pub compliance: Option<BTreeMap<String, SourceSpan>>,
    pub unknown_keys: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EnemyReference {
    pub uuid: String,
    pub line: usize,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct HullDefinition {
    pub path: String,
    pub origin: String,
    pub stations: Vec<StationDefinition>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StationDefinition {
    pub id: String,
    pub name: String,
    pub line: usize,
    pub human_seeking: bool,
    pub visiting_rating: Option<SourceSpan>,
    pub systems: Vec<SystemReference>,
    pub ratings: Vec<RatingDefinition>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemReference {
    pub id: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct RatingDefinition {
    pub index: usize,
    pub name: String,
    pub line: usize,
    pub automated_systems: Vec<AutomatedSystem>,
    pub ai_tuning: Vec<AiRule>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AutomatedSystem {
    pub id: String,
    pub line: usize,
    pub owned: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AiRule {
    pub rule: String,
    pub line: usize,
}

// ── Source helpers ────────────────────────────────────────────────────────────

fn is_member(path: &str, directory: &str) -> bool {
    path.starts_with(directory) && path.ends_with(".toml")
}

fn line_at(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn span_line(source: &str, span: Option<Range<usize>>) -> Option<usize> {
    span.map(|span| line_at(source, span.start))
}

/// The header line of a `[[table]]`, or its first value's line for a table
/// the parser gave no header span (an implicit or dotted one).
fn table_line(source: &str, table: &Table) -> usize {
    span_line(source, table.span())
        .or_else(|| {
            table
                .iter()
                .find_map(|(_, item)| span_line(source, item.span()))
        })
        .unwrap_or(1)
}

/// The string content of a string value, or the exact source text of any
/// other value, so a mistyped `uuid = 42` still shows what was written.
fn value_text(source: &str, value: &Value) -> String {
    value.as_str().map(str::to_owned).unwrap_or_else(|| {
        value
            .span()
            .map(|span| source[span].to_owned())
            .unwrap_or_default()
    })
}

fn source_span(source: &str, value: &Value) -> Option<SourceSpan> {
    let span = value.span()?;
    Some(SourceSpan {
        source: source[span.clone()].to_owned(),
        line: line_at(source, span.start),
    })
}

fn string_field<'a>(table: &'a dyn TableLike, key: &str) -> Option<&'a str> {
    table.get(key)?.as_str()
}

// ── Effective faction set ─────────────────────────────────────────────────────

/// One faction file as the runtime would see it, read structurally so a file
/// the runtime type refuses still names what it declares.
struct EffectiveFaction {
    path: String,
    candidate: bool,
    uuid: Option<Uuid>,
    name: Option<String>,
}

fn effective_factions(
    candidate: &BTreeMap<String, String>,
    beneath: &BTreeMap<String, String>,
) -> Vec<EffectiveFaction> {
    let mut members: BTreeMap<&str, (&str, bool)> = BTreeMap::new();
    for (path, source) in beneath {
        if is_member(path, FACTIONS) {
            members.insert(path, (source, false));
        }
    }
    for (path, source) in candidate {
        if is_member(path, FACTIONS) {
            members.insert(path, (source, true));
        }
    }
    members
        .into_iter()
        .filter_map(|(path, (source, candidate))| {
            let document = Document::parse(source).ok()?;
            let root = document.as_table();
            Some(EffectiveFaction {
                path: path.to_owned(),
                candidate,
                uuid: string_field(root, "uuid").and_then(|text| Uuid::parse_str(text).ok()),
                name: string_field(root, "name").map(str::to_owned),
            })
        })
        .collect()
}

/// The name each uuid resolves to over `effective` in PRECEDENCE order: the
/// last declaration wins, as the runtime registry's insert does, so the
/// caller orders base before packs (oldest first) before the candidate.
fn names_by_uuid(effective: &[EffectiveFaction]) -> BTreeMap<Uuid, String> {
    let mut names = BTreeMap::new();
    for faction in effective {
        if let (Some(uuid), Some(name)) = (faction.uuid, &faction.name) {
            names.insert(uuid, name.clone());
        }
    }
    names
}

// ── Catalog ───────────────────────────────────────────────────────────────────

fn read_faction(
    path: &str,
    origin: &str,
    source: &str,
    names: &BTreeMap<Uuid, String>,
) -> Option<FactionDefinition> {
    let document = Document::parse(source).ok()?;
    let root = document.as_table();
    let scalar = |key: &str| -> (Option<String>, Option<usize>) {
        match root.get(key).and_then(Item::as_value) {
            Some(value) => (
                Some(value_text(source, value)),
                span_line(source, value.span()),
            ),
            None => (None, None),
        }
    };
    let (uuid, uuid_line) = scalar("uuid");
    let (name, name_line) = scalar("name");
    let (display_name, display_name_line) = scalar("display_name");
    let enemies_line = root
        .get("enemies")
        .and_then(|item| span_line(source, item.span()));
    let enemies = root
        .get("enemies")
        .and_then(Item::as_array)
        .map(|array| {
            array
                .iter()
                .map(|element| EnemyReference {
                    uuid: value_text(source, element),
                    line: span_line(source, element.span())
                        .or(enemies_line)
                        .unwrap_or(1),
                    name: element
                        .as_str()
                        .and_then(|text| Uuid::parse_str(text).ok())
                        .and_then(|uuid| names.get(&uuid).cloned()),
                })
                .collect()
        })
        .unwrap_or_default();
    let compliance = root
        .get("compliance")
        .and_then(Item::as_table_like)
        .map(|table| {
            table
                .get_values()
                .into_iter()
                .filter_map(|(keys, value)| {
                    let key = keys.last()?.get();
                    let key = match key {
                        "refusal_reason" => "refusal",
                        "ack_secs" | "decide_secs" | "hold" | "divert" | "dock" => key,
                        _ => return None,
                    };
                    Some((key.to_owned(), source_span(source, value)?))
                })
                .collect()
        });
    let unknown_keys = root
        .iter()
        .map(|(key, _)| key)
        .filter(|key| !FACTION_KEYS.contains(key))
        .map(str::to_owned)
        .collect();
    Some(FactionDefinition {
        path: path.to_owned(),
        origin: origin.to_owned(),
        uuid,
        uuid_line,
        name,
        name_line,
        display_name,
        display_name_line,
        enemies,
        compliance,
        unknown_keys,
    })
}

/// The `[[system]]` owners of a hull, by system id; `None` is an ownerless
/// (`ai_only`) system, which no station's rung may automate.
fn system_owners(root: &Table) -> BTreeMap<String, Option<String>> {
    let mut owners = BTreeMap::new();
    if let Some(systems) = root.get("system").and_then(Item::as_array_of_tables) {
        for system in systems.iter() {
            let Some(id) = string_field(system, "id") else {
                continue;
            };
            owners
                .entry(id.to_owned())
                .or_insert_with(|| string_field(system, "station").map(str::to_owned));
        }
    }
    owners
}

fn read_hull(path: &str, origin: &str, source: &str) -> Option<HullDefinition> {
    let document = Document::parse(source).ok()?;
    let root = document.as_table();
    let stations = root.get("station").and_then(Item::as_array_of_tables)?;
    let systems: Vec<(String, String, Option<String>, usize)> = root
        .get("system")
        .and_then(Item::as_array_of_tables)
        .map(|systems| {
            systems
                .iter()
                .filter_map(|system| {
                    Some((
                        string_field(system, "id")?.to_owned(),
                        string_field(system, "kind").unwrap_or_default().to_owned(),
                        string_field(system, "station").map(str::to_owned),
                        table_line(source, system),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let stations = stations
        .iter()
        .map(|station| {
            let id = string_field(station, "id").unwrap_or_default().to_owned();
            let ratings = station
                .get("rating")
                .and_then(Item::as_array_of_tables)
                .map(|ratings| {
                    ratings
                        .iter()
                        .enumerate()
                        .map(|(index, rating)| RatingDefinition {
                            index,
                            name: string_field(rating, "name").unwrap_or_default().to_owned(),
                            line: table_line(source, rating),
                            automated_systems: rating
                                .get("automated_systems")
                                .and_then(Item::as_array)
                                .map(|array| {
                                    array
                                        .iter()
                                        .map(|element| {
                                            let system = value_text(source, element);
                                            let owned =
                                                systems.iter().any(|(candidate, _, owner, _)| {
                                                    candidate == &system
                                                        && owner.as_deref() == Some(id.as_str())
                                                });
                                            AutomatedSystem {
                                                id: system,
                                                line: span_line(source, element.span())
                                                    .unwrap_or_else(|| table_line(source, rating)),
                                                owned,
                                            }
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                            ai_tuning: rating
                                .get("ai_tuning")
                                .and_then(Item::as_table_like)
                                .map(|table| {
                                    table
                                        .iter()
                                        .map(|(rule, item)| AiRule {
                                            rule: rule.to_owned(),
                                            line: table
                                                .get_key_value(rule)
                                                .and_then(|(key, _)| span_line(source, key.span()))
                                                .or_else(|| span_line(source, item.span()))
                                                .unwrap_or_else(|| table_line(source, rating)),
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            StationDefinition {
                name: string_field(station, "name").unwrap_or_default().to_owned(),
                line: table_line(source, station),
                human_seeking: station
                    .get("human_seeking")
                    .and_then(Item::as_bool)
                    .unwrap_or(false),
                visiting_rating: station
                    .get("visiting_rating")
                    .and_then(Item::as_value)
                    .and_then(|value| source_span(source, value)),
                systems: systems
                    .iter()
                    .filter(|(_, _, owner, _)| owner.as_deref() == Some(id.as_str()))
                    .map(|(system, kind, _, line)| SystemReference {
                        id: system.clone(),
                        kind: kind.clone(),
                        line: *line,
                    })
                    .collect(),
                ratings,
                id,
            }
        })
        .collect();
    Some(HullDefinition {
        path: path.to_owned(),
        origin: origin.to_owned(),
        stations,
    })
}

fn spelling<T: Serialize>(value: T) -> String {
    // The runtime enum's serde contract is the only spelling the parser accepts.
    toml::Value::try_from(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn order_responses() -> Vec<String> {
    // Listed by hand, but the match keeps the list honest: a variant added to
    // the runtime enum is a compile error here rather than a missing choice.
    [OrderResponse::Comply, OrderResponse::Refuse]
        .into_iter()
        .map(|response| {
            match response {
                OrderResponse::Comply | OrderResponse::Refuse => {}
            }
            spelling(response)
        })
        .collect()
}

fn ai_rules() -> Vec<String> {
    vec![crate::console_ai::server::AI_RULE_TORPEDO_AUTO_FIRE.to_owned()]
}

fn compliance_defaults() -> ComplianceDefaults {
    let defaults = ComplianceDisposition::default();
    ComplianceDefaults {
        ack_secs: defaults.ack_secs,
        decide_secs: defaults.decide_secs,
        hold: spelling(defaults.hold),
        divert: spelling(defaults.divert),
        dock: spelling(defaults.dock),
    }
}

/// Every faction and hull an author can see, with the draft's own members
/// editable and everything beneath listed read-only under its origin.
pub fn catalog(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
) -> DefinitionCatalog {
    let mut members: BTreeMap<String, (String, &str)> = BTreeMap::new();
    for (path, source) in &dependencies.base_files {
        members.insert(path.clone(), (ORIGIN_BASE.to_owned(), source));
    }
    for pack in &dependencies.packs {
        for (path, source) in &pack.files {
            members.insert(path.clone(), (format!("pack:{}", pack.id), source));
        }
    }
    let beneath: BTreeMap<String, String> = members
        .iter()
        .map(|(path, (_, source))| (path.clone(), (*source).to_owned()))
        .collect();
    for (path, source) in files {
        members.insert(path.clone(), (ORIGIN_DRAFT.to_owned(), source));
    }
    // A uuid declared twice resolves the way the runtime registry resolves it
    // — the base set first, then each pack in stack order, then the draft,
    // the latest insert winning — so the choice label and a resolved enemy
    // name are the same faction. Stable, so equal ranks keep path order.
    let rank = |path: &str| -> usize {
        match members.get(path).map(|(origin, _)| origin.as_str()) {
            Some(ORIGIN_DRAFT) => dependencies.packs.len() + 1,
            Some(origin) => origin
                .strip_prefix("pack:")
                .and_then(|id| dependencies.packs.iter().position(|pack| pack.id == id))
                .map(|position| position + 1)
                .unwrap_or(0),
            None => 0,
        }
    };
    let mut effective = effective_factions(files, &beneath);
    effective.sort_by_key(|faction| rank(&faction.path));
    let names = names_by_uuid(&effective);
    let mut choices: BTreeMap<Uuid, FactionChoice> = BTreeMap::new();
    for faction in &effective {
        let (Some(uuid), Some(name)) = (faction.uuid, &faction.name) else {
            continue;
        };
        let origin = members
            .get(&faction.path)
            .map(|(origin, _)| origin.clone())
            .unwrap_or_default();
        choices.insert(
            uuid,
            FactionChoice {
                uuid: uuid.to_string(),
                name: name.clone(),
                origin,
            },
        );
    }
    let mut choices: Vec<FactionChoice> = choices.into_values().collect();
    choices.sort_by(|a, b| (&a.name, &a.uuid).cmp(&(&b.name, &b.uuid)));

    let mut findings = findings(files, &beneath);
    // A draft member the tree cannot read has no fields to show; say why
    // here so the panel is not simply missing a file. Check reports the same
    // syntax error through the runtime parsers, which is the gate.
    for (path, source) in files {
        if is_member(path, FACTIONS) || is_member(path, ENTITIES) {
            if let Err(error) = Document::parse(source.as_str()) {
                findings.push(WorkshopFinding {
                    severity: "error".into(),
                    category: "runtime-source-invalid".into(),
                    message: error.message().to_owned(),
                    file: path.clone(),
                    line: error.span().map(|span| line_at(source, span.start)),
                });
            } else if is_member(path, FACTIONS) {
                if let Err(error) = crate::ai::faction::parse_faction_config(source) {
                    findings.push(WorkshopFinding {
                        severity: "error".into(),
                        category: "runtime-source-invalid".into(),
                        message: error.to_string(),
                        file: path.clone(),
                        line: None,
                    });
                }
            }
        }
    }
    sort_findings(&mut findings);

    DefinitionCatalog {
        factions: members
            .iter()
            .filter(|(path, _)| is_member(path, FACTIONS))
            .filter_map(|(path, (origin, source))| read_faction(path, origin, source, &names))
            .collect(),
        hulls: members
            .iter()
            .filter(|(path, _)| is_member(path, ENTITIES))
            .filter_map(|(path, (origin, source))| read_hull(path, origin, source))
            .collect(),
        choices: Choices {
            factions: choices,
            order_responses: order_responses(),
            ai_rules: ai_rules(),
        },
        defaults: Defaults {
            compliance: compliance_defaults(),
        },
        findings,
    }
}

// ── New faction skeleton ──────────────────────────────────────────────────────

/// A faction file whose key set and spelling come from the runtime type.
pub fn new_faction_source(name: &str, uuid: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A faction needs a name.".into());
    }
    let uuid = Uuid::parse_str(uuid.trim()).map_err(|error| error.to_string())?;
    let config = FactionConfig {
        uuid,
        name: name.to_owned(),
        display_name: None,
        enemies: Vec::new(),
        compliance: None,
    };
    toml::to_string(&config).map_err(|error| error.to_string())
}

// ── Definition-aware value checks ─────────────────────────────────────────────

#[derive(Deserialize)]
struct ScalarInput<T> {
    value: T,
}

/// The same serde implementation as the runtime field decides, exactly as
/// `model_fields::accepts` does for rig sidecars; that helper is private to its
/// domain, and sharing one five-line function is not worth coupling the two.
fn accepts<T: DeserializeOwned>(source: &str) -> bool {
    toml::from_str::<ScalarInput<T>>(&format!("value = {source}"))
        .map(|input| input.value)
        .is_ok()
}

fn non_empty_string(source: &str) -> bool {
    toml::from_str::<ScalarInput<String>>(&format!("value = {source}"))
        .is_ok_and(|input| !input.value.trim().is_empty())
}

fn refuse(message: &str) -> Result<(), String> {
    Err(message.to_owned())
}

/// Whether `source` may stand at `path` in the document, judged by the runtime
/// type that will read it. Cross-file validity is deliberately not checked
/// here (see the module doc).
pub(super) fn check_value(
    document_path: &str,
    path: &[Segment],
    source: &str,
) -> Result<(), String> {
    use Segment::{Index, Key};
    let accepted = if is_member(document_path, FACTIONS) {
        match path {
            [Key(key)] if key == "uuid" => accepts::<Uuid>(source),
            [Key(key)] if key == "enemies" => accepts::<Vec<Uuid>>(source),
            [Key(key), Index(_)] if key == "enemies" => accepts::<Uuid>(source),
            [Key(key)] if key == "name" || key == "display_name" => accepts::<String>(source),
            [Key(key)] if key == "compliance" => accepts::<ComplianceDisposition>(source),
            [Key(table), Key(key)] if table == "compliance" => match key.as_str() {
                "hold" | "divert" | "dock" => accepts::<OrderResponse>(source),
                "ack_secs" | "decide_secs" => accepts::<i64>(source),
                "refusal_reason" => accepts::<String>(source),
                _ => false,
            },
            _ => true,
        }
    } else if is_member(document_path, ENTITIES) {
        match path {
            [Key(station), Index(_), Key(rating), Index(_), Key(key)]
                if station == "station" && rating == "rating" =>
            {
                match key.as_str() {
                    "name" => non_empty_string(source),
                    "automated_systems" => accepts::<Vec<String>>(source),
                    _ => true,
                }
            }
            [Key(station), Index(_), Key(rating), Index(_), Key(key), Index(_)]
                if station == "station" && rating == "rating" && key == "automated_systems" =>
            {
                accepts::<String>(source)
            }
            [Key(station), Index(_), Key(key)] if station == "station" => match key.as_str() {
                "visiting_rating" => accepts::<String>(source),
                "human_seeking" => accepts::<bool>(source),
                _ => true,
            },
            _ => true,
        }
    } else {
        true
    };
    if accepted {
        Ok(())
    } else {
        refuse("The value is not accepted by this runtime field type.")
    }
}

/// The fields of a table appended to an array of tables, checked as the
/// values they will become. A rating without a name has nothing a station
/// can be rated by, so it is refused before it exists.
pub(super) fn check_table_fields(
    document_path: &str,
    path: &[Segment],
    fields: &[(String, String)],
) -> Result<(), String> {
    let is_rating = is_member(document_path, ENTITIES)
        && matches!(path, [Segment::Key(station), Segment::Index(_), Segment::Key(rating)] if station == "station" && rating == "rating");
    if is_rating && !fields.iter().any(|(key, _)| key == "name") {
        return refuse("A rating needs a name.");
    }
    for (key, source) in fields {
        let mut field_path = path.to_vec();
        // The new entry's index is irrelevant to the type check; any index
        // resolves to the same rule.
        field_path.push(Segment::Index(0));
        field_path.push(Segment::Key(key.clone()));
        check_value(document_path, &field_path, source)?;
    }
    Ok(())
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

fn warning(category: &str, file: &str, message: String) -> WorkshopFinding {
    WorkshopFinding {
        severity: "warning".into(),
        ..finding(category, file, None, message)
    }
}

fn sort_findings(findings: &mut Vec<WorkshopFinding>) {
    findings.sort_by(|a, b| {
        (&a.file, a.line, &a.category, &a.message).cmp(&(&b.file, b.line, &b.category, &b.message))
    });
    findings.dedup();
}

/// Definition-level semantic findings over CANDIDATE members, resolved against
/// candidate ∪ beneath (candidate wins by path). Lines refer to candidate
/// sources. Deterministic: sorted by file, line, category.
pub fn findings(
    candidate: &BTreeMap<String, String>,
    beneath: &BTreeMap<String, String>,
) -> Vec<WorkshopFinding> {
    let effective = effective_factions(candidate, beneath);
    let mut findings = Vec::new();
    faction_findings(candidate, &effective, &mut findings);
    dependency_findings(&effective, &mut findings);
    entity_findings(candidate, &effective, &mut findings);
    world_findings(candidate, &effective, &mut findings);
    sort_findings(&mut findings);
    findings
}

/// Two READ-ONLY files beneath the draft declaring one uuid. Nothing the
/// author can edit here caused it, so it is a warning rather than a refusal,
/// but it is said: the Live host takes the newest pack's declaration while a
/// disposable Test's staged directory takes the last by file name, so which
/// one a Test runs is not the one Live would. A candidate sharing the uuid
/// already carries the error above.
fn dependency_findings(effective: &[EffectiveFaction], findings: &mut Vec<WorkshopFinding>) {
    let mut by_uuid: BTreeMap<Uuid, Vec<&EffectiveFaction>> = BTreeMap::new();
    for faction in effective {
        if let Some(uuid) = faction.uuid {
            by_uuid.entry(uuid).or_default().push(faction);
        }
    }
    for (uuid, declared) in by_uuid {
        if declared.len() < 2 || declared.iter().any(|faction| faction.candidate) {
            continue;
        }
        let paths: Vec<&str> = declared
            .iter()
            .map(|faction| faction.path.as_str())
            .collect();
        findings.push(warning(
            "dependency-duplicate-uuid",
            paths[paths.len() - 1],
            format!(
                "Faction uuid {uuid} is declared by {} beneath this draft; Live and Test may resolve it differently",
                paths.join(", ")
            ),
        ));
    }
}

/// Whether `member` should carry the duplicate finding for a shared key. Two
/// candidate files sharing one: the second by path says so, the first is the
/// original. A candidate sharing with a read-only file: the candidate, because
/// it is the one that can change.
fn is_duplicate(path: &str, others: &[&EffectiveFaction]) -> bool {
    if others.is_empty() {
        return false;
    }
    let first_of_candidates = others
        .iter()
        .all(|other| other.candidate && other.path.as_str() > path);
    !first_of_candidates
}

fn faction_findings(
    candidate: &BTreeMap<String, String>,
    effective: &[EffectiveFaction],
    findings: &mut Vec<WorkshopFinding>,
) {
    let known: BTreeSet<Uuid> = effective
        .iter()
        .filter_map(|faction| faction.uuid)
        .collect();
    for (path, source) in candidate {
        if !is_member(path, FACTIONS) {
            continue;
        }
        let Ok(document) = Document::parse(source.as_str()) else {
            continue;
        };
        let root = document.as_table();
        let own_uuid = match root.get("uuid").and_then(Item::as_value) {
            Some(value) => match value.as_str().and_then(|text| Uuid::parse_str(text).ok()) {
                Some(uuid) => Some(uuid),
                None => {
                    findings.push(finding(
                        "faction-invalid-uuid",
                        path,
                        span_line(source, value.span()),
                        format!("Faction uuid {} is not a uuid", value_text(source, value)),
                    ));
                    None
                }
            },
            None => None,
        };
        if let Some(uuid) = own_uuid {
            let others: Vec<&EffectiveFaction> = effective
                .iter()
                .filter(|other| other.uuid == Some(uuid) && &other.path != path)
                .collect();
            if is_duplicate(path, &others) {
                findings.push(finding(
                    "faction-duplicate-uuid",
                    path,
                    root.get("uuid")
                        .and_then(|item| span_line(source, item.span())),
                    format!(
                        "Faction uuid {uuid} is also declared by {}",
                        others
                            .iter()
                            .map(|other| other.path.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        if let Some(name) = string_field(root, "name") {
            let others: Vec<&EffectiveFaction> = effective
                .iter()
                .filter(|other| other.name.as_deref() == Some(name) && &other.path != path)
                .collect();
            if is_duplicate(path, &others) {
                findings.push(finding(
                    "faction-duplicate-name",
                    path,
                    root.get("name").and_then(|item| span_line(source, item.span())),
                    format!(
                        "Faction name {name:?} is also declared by {}; a world trigger naming it would resolve to the lowest uuid",
                        others
                            .iter()
                            .map(|other| other.path.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                ));
            }
        }
        if let Some(enemies) = root.get("enemies").and_then(Item::as_array) {
            for element in enemies.iter() {
                let line = span_line(source, element.span());
                match element.as_str().and_then(|text| Uuid::parse_str(text).ok()) {
                    None => findings.push(finding(
                        "faction-invalid-uuid",
                        path,
                        line,
                        format!("Enemy {} is not a uuid", value_text(source, element)),
                    )),
                    Some(enemy) if Some(enemy) == own_uuid => findings.push(finding(
                        "faction-self-enemy",
                        path,
                        line,
                        format!("Faction {enemy} lists itself as an enemy"),
                    )),
                    Some(enemy) if !known.contains(&enemy) => findings.push(finding(
                        "faction-unknown-enemy",
                        path,
                        line,
                        format!("Enemy {enemy} names no faction"),
                    )),
                    Some(_) => {}
                }
            }
        }
    }
}

fn entity_findings(
    candidate: &BTreeMap<String, String>,
    effective: &[EffectiveFaction],
    findings: &mut Vec<WorkshopFinding>,
) {
    let known: BTreeSet<Uuid> = effective
        .iter()
        .filter_map(|faction| faction.uuid)
        .collect();
    for (path, source) in candidate {
        if !is_member(path, ENTITIES) {
            continue;
        }
        let Ok(document) = Document::parse(source.as_str()) else {
            continue;
        };
        let root = document.as_table();
        if let Some(value) = root.get("faction").and_then(Item::as_value) {
            let resolved = value
                .as_str()
                .and_then(|text| Uuid::parse_str(text).ok())
                .filter(|uuid| known.contains(uuid));
            if resolved.is_none() {
                findings.push(finding(
                    "entity-unknown-faction",
                    path,
                    span_line(source, value.span()),
                    format!("Faction {} names no faction", value_text(source, value)),
                ));
            }
        }
        let Some(stations) = root.get("station").and_then(Item::as_array_of_tables) else {
            continue;
        };
        let owners = system_owners(root);
        for station in stations.iter() {
            let station_id = string_field(station, "id").unwrap_or_default().to_owned();
            let ratings: Vec<&Table> = station
                .get("rating")
                .and_then(Item::as_array_of_tables)
                .map(|ratings| ratings.iter().collect())
                .unwrap_or_default();
            let mut names = BTreeSet::new();
            for &rating in &ratings {
                let name = string_field(rating, "name").unwrap_or_default().to_owned();
                if !names.insert(name.clone()) {
                    findings.push(finding(
                        "rating-duplicate-name",
                        path,
                        Some(table_line(source, rating)),
                        ShipConfigError::DuplicateRatingName {
                            station: StationId(station_id.clone()),
                            rating: name.clone(),
                        }
                        .to_string(),
                    ));
                }
                let Some(automated) = rating.get("automated_systems").and_then(Item::as_array)
                else {
                    continue;
                };
                for element in automated.iter() {
                    let system = value_text(source, element);
                    let line = span_line(source, element.span());
                    match owners.get(&system) {
                        None => findings.push(finding(
                            "rating-unknown-system",
                            path,
                            line,
                            ShipConfigError::DanglingRatingReference {
                                station: StationId(station_id.clone()),
                                rating: name.clone(),
                                system: SystemId(system.clone()),
                            }
                            .to_string(),
                        )),
                        Some(owner) if owner.as_deref() != Some(station_id.as_str()) => {
                            findings.push(finding(
                                "rating-unowned-system",
                                path,
                                line,
                                ShipConfigError::RatingReferencesUnownedSystem {
                                    station: StationId(station_id.clone()),
                                    rating: name.clone(),
                                    system: SystemId(system.clone()),
                                    owner: owner.clone().map(StationId),
                                }
                                .to_string(),
                            ));
                        }
                        Some(_) => {}
                    }
                }
            }
            let human_seeking = station
                .get("human_seeking")
                .and_then(Item::as_bool)
                .unwrap_or(false);
            // A visiting rating or a host order only means something on a
            // station that seats a human; the runtime refuses the hull
            // otherwise, so the panel says so at the line that carries it.
            if !human_seeking {
                let orders = station
                    .get("host_order")
                    .and_then(Item::as_array)
                    .filter(|hosts| !hosts.is_empty())
                    .map(|hosts| ("station-host-order-without-human-seeking", hosts.span()));
                let visiting =
                    station
                        .get("visiting_rating")
                        .and_then(Item::as_value)
                        .map(|value| {
                            (
                                "station-visiting-rating-without-human-seeking",
                                value.span(),
                            )
                        });
                for (category, span) in orders.into_iter().chain(visiting) {
                    findings.push(finding(
                        category,
                        path,
                        span_line(source, span).or_else(|| Some(table_line(source, station))),
                        ShipConfigError::HostOrderWithoutHumanSeeking {
                            station: StationId(station_id.clone()),
                        }
                        .to_string(),
                    ));
                }
            }
            match station.get("visiting_rating").and_then(Item::as_value) {
                Some(value) => {
                    let visiting = value_text(source, value);
                    if !names.contains(&visiting) {
                        findings.push(finding(
                            "station-unknown-visiting-rating",
                            path,
                            span_line(source, value.span()),
                            ShipConfigError::UnknownVisitingRating {
                                station: StationId(station_id.clone()),
                                rating: visiting,
                            }
                            .to_string(),
                        ));
                    }
                }
                None if human_seeking => findings.push(finding(
                    "station-missing-visiting-rating",
                    path,
                    Some(table_line(source, station)),
                    ShipConfigError::MissingVisitingRating {
                        station: StationId(station_id.clone()),
                    }
                    .to_string(),
                )),
                None => {}
            }
        }
    }
}

/// Every table-like node of a document, depth first, so a trigger action is
/// found wherever the world schema nests it.
fn visit_tables<'a>(item: &'a Item, visit: &mut dyn FnMut(&'a dyn TableLike)) {
    match item {
        Item::Table(table) => {
            visit(table);
            for (_, child) in table.iter() {
                visit_tables(child, visit);
            }
        }
        Item::ArrayOfTables(tables) => {
            for table in tables.iter() {
                visit(table);
                for (_, child) in table.iter() {
                    visit_tables(child, visit);
                }
            }
        }
        Item::Value(value) => visit_values(value, visit),
        Item::None => {}
    }
}

fn visit_values<'a>(value: &'a Value, visit: &mut dyn FnMut(&'a dyn TableLike)) {
    match value {
        Value::InlineTable(table) => {
            visit(table);
            for (_, child) in table.iter() {
                visit_values(child, visit);
            }
        }
        Value::Array(array) => {
            for element in array.iter() {
                visit_values(element, visit);
            }
        }
        _ => {}
    }
}

/// The two string literals of a scripted `add_faction_enemy("A", "B")` call
/// starting at `text`, when both are literals. A call built from variables
/// cannot be judged here and is left to the runtime's own warning.
fn literal_pair(text: &str) -> Option<(&str, &str)> {
    let mut rest = text.trim_start();
    let mut names = Vec::new();
    for _ in 0..2 {
        rest = rest.strip_prefix('"')?;
        let end = rest.find('"')?;
        names.push(&rest[..end]);
        rest = rest[end + 1..].trim_start();
        if names.len() == 1 {
            rest = rest.strip_prefix(',')?.trim_start();
        }
    }
    Some((names[0], names[1]))
}

fn world_findings(
    candidate: &BTreeMap<String, String>,
    effective: &[EffectiveFaction],
    findings: &mut Vec<WorkshopFinding>,
) {
    let names: BTreeSet<&str> = effective
        .iter()
        .filter_map(|faction| faction.name.as_deref())
        .collect();
    let unknown = |path: &str, line: Option<usize>, name: &str| {
        finding(
            "world-unknown-faction",
            path,
            line,
            format!("Faction {name:?} names no faction"),
        )
    };
    for (path, source) in candidate {
        let scripted = path.starts_with(WORLDS) && path.ends_with(".rhai");
        if !scripted && !is_member(path, WORLDS) {
            continue;
        }
        if !scripted {
            if let Ok(document) = Document::parse(source.as_str()) {
                visit_tables(document.as_item(), &mut |table| {
                    let is_action = string_field(table, "type")
                        .is_some_and(|kind| FACTION_ENEMY_ACTIONS.contains(&kind));
                    if !is_action {
                        return;
                    }
                    for key in ["faction", "enemy"] {
                        if let Some(value) = table.get(key).and_then(Item::as_value) {
                            let name = value_text(source, value);
                            if !names.contains(name.as_str()) {
                                findings.push(unknown(
                                    path,
                                    span_line(source, value.span()),
                                    &name,
                                ));
                            }
                        }
                    }
                });
            }
        }
        // Script effects name factions the same way; a literal pair is judged,
        // a comment line is not.
        for (index, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            for action in FACTION_ENEMY_ACTIONS {
                let call = format!("{action}(");
                for (offset, _) in line.match_indices(call.as_str()) {
                    let Some((faction, enemy)) = literal_pair(&line[offset + call.len()..]) else {
                        continue;
                    };
                    for name in [faction, enemy] {
                        if !names.contains(name) {
                            findings.push(unknown(path, Some(index + 1), name));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
