//! Source-preserving authoring fields. Runtime-owned schemas supply types and
//! defaults; the TOML syntax tree supplies exact source spans for every authored
//! scalar, including extension fields. No serializer rewrites the document.
use crate::inspector::{FieldDescriptor, FieldOrigin, LiveMutability};
use serde::{Deserialize, Serialize};
use toml_edit::{Document, Item, Value};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Segment {
    Key(String),
    Index(usize),
}

#[derive(Clone, Debug, Serialize)]
pub struct Field {
    pub path: Vec<Segment>,
    #[serde(flatten)]
    pub descriptor: FieldDescriptor,
    /// Exact TOML value text, which preserves large integers without JS loss.
    pub source: String,
    pub line: usize,
    pub runtime_owned: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Patch {
    pub document_path: String,
    pub path: Vec<Segment>,
    /// Optimistic concurrency is over the exact entire document, not a parsed
    /// model that might have forgotten an intervening comment or unknown key.
    pub expected_source: String,
    pub value_source: String,
}

pub(super) trait ScalarField {
    const KIND: &'static str;
    fn default_source(&self) -> Option<String>;
    fn descriptor(&self) -> (&'static str, Option<String>) {
        (Self::KIND, self.default_source())
    }
}

macro_rules! numeric_field {
    ($ty:ty, $kind:literal) => {
        impl ScalarField for $ty {
            const KIND: &'static str = $kind;
            fn default_source(&self) -> Option<String> {
                Some(self.to_string())
            }
        }
    };
}
numeric_field!(f32, "float");
numeric_field!(u32, "integer");
numeric_field!(u64, "integer");
impl ScalarField for String {
    const KIND: &'static str = "string";
    fn default_source(&self) -> Option<String> {
        Some(Value::from(self.clone()).to_string())
    }
}
impl<T: ScalarField> ScalarField for Option<T> {
    const KIND: &'static str = T::KIND;
    fn default_source(&self) -> Option<String> {
        self.as_ref().and_then(ScalarField::default_source)
    }
}

fn global_field(path: &[Segment]) -> Option<(&'static str, Option<String>)> {
    let [Segment::Key(table), Segment::Key(key)] = path else {
        return None;
    };
    if table != "global" {
        return None;
    }
    let config = crate::entities::config::GlobalConfig::default();
    // Each descriptor is inferred from the actual Rust field type and default.
    // A malformed authored value does not get to redefine its expected type.
    Some(match key.as_str() {
        "seed" => config.seed.descriptor(),
        "title" => config.title.descriptor(),
        "description" => config.description.descriptor(),
        "sim_tick_hz" => config.sim_tick_hz.descriptor(),
        "ai_tick_hz" | "ai_helm_tick_hz" => config.ai_tick_hz.descriptor(),
        "ai_snapshot_hz" => config.ai_snapshot_hz.descriptor(),
        "autosave_interval_secs" => config.autosave_interval_secs.descriptor(),
        "intent_break_off_hull_fraction" => config.intent_break_off_hull_fraction.descriptor(),
        "attacked_memory_secs" => config.attacked_memory_secs.descriptor(),
        "station_activity_bucket_secs" => config.station_activity_bucket_secs.descriptor(),
        "trigger_fire_history_depth" => config.trigger_fire_history_depth.descriptor(),
        "gm_activity_history_depth" => config.gm_activity_history_depth.descriptor(),
        "command_delay_ticks" => config.command_delay_ticks.descriptor(),
        _ => return None,
    })
}

pub fn fields(source: &str, document_path: &str) -> Result<Vec<Field>, String> {
    let document = Document::parse(source).map_err(|e| e.to_string())?;
    let mut fields = Vec::new();
    walk_item(document.as_item(), &mut Vec::new(), source, &mut fields);
    for field in &mut fields {
        field.descriptor.origin.document = Some(document_path.into());
    }
    if document_path.starts_with("assets/worlds/") && document_path.ends_with(".toml") {
        for field in &mut fields {
            if let Some((kind, default)) = global_field(&field.path) {
                field.descriptor.kind = kind.into();
                field.runtime_owned = true;
                field.descriptor.default_source = default;
            }
        }
    }
    for field in &mut fields {
        if let Some((kind, default)) = super::model_fields::descriptor(document_path, &field.path) {
            field.descriptor.kind = kind.into();
            field.runtime_owned = true;
            field.descriptor.default_source = default;
        }
    }
    Ok(fields)
}

fn walk_item(item: &Item, path: &mut Vec<Segment>, source: &str, fields: &mut Vec<Field>) {
    match item {
        Item::Value(value) => walk_value(value, path, source, fields),
        Item::Table(table) => {
            for (key, item) in table.iter() {
                path.push(Segment::Key(key.into()));
                walk_item(item, path, source, fields);
                path.pop();
            }
        }
        Item::ArrayOfTables(tables) => {
            for (index, table) in tables.iter().enumerate() {
                path.push(Segment::Index(index));
                for (key, item) in table.iter() {
                    path.push(Segment::Key(key.into()));
                    walk_item(item, path, source, fields);
                    path.pop();
                }
                path.pop();
            }
        }
        Item::None => {}
    }
}

fn walk_value(value: &Value, path: &mut Vec<Segment>, source: &str, fields: &mut Vec<Field>) {
    match value {
        Value::Array(array) => {
            for (index, value) in array.iter().enumerate() {
                path.push(Segment::Index(index));
                walk_value(value, path, source, fields);
                path.pop();
            }
        }
        Value::InlineTable(table) => {
            for (key, value) in table.iter() {
                path.push(Segment::Key(key.into()));
                walk_value(value, path, source, fields);
                path.pop();
            }
        }
        _ => {
            if let Some(span) = value.span() {
                fields.push(Field {
                    path: path.clone(),
                    descriptor: FieldDescriptor {
                        kind: value.type_name().into(),
                        default_source: None,
                        live_mutability: LiveMutability::RecreateRequired,
                        origin: FieldOrigin {
                            schema_path: path
                                .iter()
                                .map(|part| match part {
                                    Segment::Key(key) => key.clone(),
                                    Segment::Index(index) => format!("[{index}]"),
                                })
                                .collect::<Vec<_>>()
                                .join("."),
                            document: None,
                            line: Some(
                                source[..span.start].bytes().filter(|b| *b == b'\n').count() + 1,
                            ),
                            layer: None,
                        },
                        validation: vec!["inspector.validation.source_document".into()],
                    },
                    source: source[span.clone()].into(),
                    line: source[..span.start]
                        .bytes()
                        .filter(|byte| *byte == b'\n')
                        .count()
                        + 1,
                    runtime_owned: false,
                });
            }
        }
    }
}

/// Replace one scalar span and preserve every other byte, including CRLF,
/// comments, ordering, arrays, inline tables and unknown extension fields.
pub fn patch(source: &str, patch: &Patch) -> Result<String, String> {
    if source != patch.expected_source {
        return Err("The document changed; inspect it again before applying this field.".into());
    }
    let document = Document::parse(source).map_err(|e| e.to_string())?;
    let field = fields(source, &patch.document_path)?
        .into_iter()
        .find(|field| field.path == patch.path)
        .ok_or("The field no longer exists or is not a scalar.")?;
    let replacement =
        Document::parse(format!("value = {}", patch.value_source)).map_err(|e| e.to_string())?;
    if replacement.iter().count() != 1 {
        return Err("A field edit must contain exactly one value.".into());
    }
    let value = replacement
        .get("value")
        .and_then(Item::as_value)
        .ok_or("Expected a scalar value.")?;
    let numeric = field.descriptor.kind == "float" && value.is_integer();
    if value.type_name() != field.descriptor.kind && !numeric {
        return Err("The value has a different type from this field.".into());
    }
    super::model_fields::validate(&patch.document_path, &patch.path, &patch.value_source)?;
    let span =
        find_span(document.as_item(), &patch.path).ok_or("The source location is unavailable.")?;
    let replacement_span = value
        .span()
        .ok_or("The replacement source location is unavailable.")?;
    let replacement_source = &replacement.raw()[replacement_span];
    let mut result = source.to_owned();
    result.replace_range(span, replacement_source);
    Document::parse(result.as_str()).map_err(|e| e.to_string())?;
    Ok(result)
}

fn find_span(item: &Item, path: &[Segment]) -> Option<std::ops::Range<usize>> {
    if let Item::Value(value) = item {
        return value_span(value, path);
    }
    let (head, tail) = path.split_first()?;
    match (item, head) {
        (Item::Table(table), Segment::Key(key)) => find_span(table.get(key)?, tail),
        (Item::ArrayOfTables(tables), Segment::Index(index)) => {
            let (Segment::Key(key), tail) = tail.split_first()? else {
                return None;
            };
            find_span(tables.get(*index)?.get(key)?, tail)
        }
        _ => None,
    }
}

fn value_span(value: &Value, path: &[Segment]) -> Option<std::ops::Range<usize>> {
    let Some((head, tail)) = path.split_first() else {
        return value.span();
    };
    match (value, head) {
        (Value::Array(array), Segment::Index(index)) => value_span(array.get(*index)?, tail),
        (Value::InlineTable(table), Segment::Key(key)) => value_span(table.get(key)?, tail),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const WORLD: &str = "assets/worlds/test.toml";
    #[test]
    fn nested_field_edit_retains_all_other_source_bytes() {
        let source = "# 文書\r\n[global] # keep\r\ntitle = 'Original' # retained\n[[entity]]\r\nposition = [1, 2, 3] # coordinates\r\ncustom = { other = 'unknown' }\n";
        let fields = fields(source, WORLD).unwrap();
        let coordinate = fields
            .iter()
            .find(|field| {
                field.path
                    == vec![
                        Segment::Key("entity".into()),
                        Segment::Index(0),
                        Segment::Key("position".into()),
                        Segment::Index(1),
                    ]
            })
            .unwrap();
        let result = patch(
            source,
            &Patch {
                document_path: WORLD.into(),
                path: coordinate.path.clone(),
                expected_source: source.into(),
                value_source: "8".into(),
            },
        )
        .unwrap();
        assert_eq!(result, source.replace("[1, 2, 3]", "[1, 8, 3]"));
        assert!(fields
            .iter()
            .any(|field| field.runtime_owned && field.descriptor.default_source.is_none()));
        assert!(fields
            .iter()
            .any(|field| !field.runtime_owned && field.source == "'unknown'"));
    }
    #[test]
    fn stale_or_type_changing_edits_are_refused() {
        let source = "[global]\ntitle = 'Before'\n";
        let mut change = Patch {
            document_path: WORLD.into(),
            path: fields(source, WORLD).unwrap()[0].path.clone(),
            expected_source: source.into(),
            value_source: "10".into(),
        };
        assert!(patch(source, &change).is_err());
        change.value_source = "'After'".into();
        assert!(patch(&format!("{source}# external change\n"), &change).is_err());
        change.value_source = "'After'\nother = 'injected'".into();
        assert!(patch(source, &change).is_err());
    }

    #[test]
    fn runtime_field_type_and_default_do_not_come_from_malformed_source() {
        let source = "[global]\ntitle = 4\nsim_tick_hz = 'invalid'\n";
        let descriptors = fields(source, WORLD).unwrap();
        assert_eq!(descriptors[0].descriptor.kind, "string");
        assert_eq!(descriptors[1].descriptor.kind, "float");
        assert_eq!(
            descriptors[1].descriptor.default_source,
            Some(
                crate::entities::config::GlobalConfig::default()
                    .sim_tick_hz
                    .to_string()
            )
        );
        let fixed = patch(
            source,
            &Patch {
                document_path: WORLD.into(),
                path: descriptors[0].path.clone(),
                expected_source: source.into(),
                value_source: "'Repaired'".into(),
            },
        )
        .unwrap();
        assert!(fixed.contains("title = 'Repaired'"));
    }

    #[test]
    fn manifest_extension_keys_do_not_inherit_an_unrelated_world_schema() {
        let source = "[global]\ntitle = 4\n";
        let field = fields(source, "scenarios.toml").unwrap().remove(0);
        assert!(!field.runtime_owned);
        assert_eq!(field.descriptor.kind, "integer");
        assert_eq!(
            patch(
                source,
                &Patch {
                    document_path: "scenarios.toml".into(),
                    path: field.path,
                    expected_source: source.into(),
                    value_source: "5".into()
                }
            )
            .unwrap(),
            source.replace('4', "5")
        );
    }
}
