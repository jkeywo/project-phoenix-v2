//! Source-preserving authoring fields. Runtime-owned schemas supply types and
//! defaults; the TOML syntax tree supplies exact source spans for every authored
//! scalar, including extension fields. No serializer rewrites the document.
use crate::inspector::{FieldDescriptor, FieldOrigin, LiveMutability};
use serde::{Deserialize, Serialize};
use toml_edit::{
    Array, ArrayOfTables, Document, DocumentMut, InlineTable, Item, RawString, Table, Value,
};

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

// ── Structural edits (issue #1474) ────────────────────────────────────────────

/// A group of structural edits applied all-or-nothing to one document, so a
/// form's Apply is one history entry however many keys it touched.
#[derive(Clone, Debug, Deserialize)]
pub struct EditRequest {
    pub document_path: String,
    /// The same optimistic rule as [`Patch`]: the exact entire document.
    pub expected_source: String,
    pub edits: Vec<Edit>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Edit {
    /// Replace an existing scalar, under the same type rule as [`patch`].
    Set {
        path: Vec<Segment>,
        value_source: String,
    },
    /// Set or create a key in an existing table or inline table.
    Put {
        path: Vec<Segment>,
        value_source: String,
    },
    /// Insert into an array value at `index` (0 ≤ index ≤ len).
    Insert {
        path: Vec<Segment>,
        index: usize,
        value_source: String,
    },
    /// Remove an array element, a table key or an array-of-tables entry.
    Remove { path: Vec<Segment> },
    /// Append an entry to an array of tables, keys in the given order, placed
    /// after that array's last entry rather than at the end of the file.
    AppendTable {
        path: Vec<Segment>,
        fields: Vec<(String, String)>,
    },
}

/// Apply every edit in order to the parsed document and re-emit it. Untouched
/// bytes survive: `toml_edit` keeps every comment, key order and unknown key,
/// and [`restore_line_endings`] gives each surviving line its own ending back
/// (the emitter itself writes LF only). A refused edit returns the untouched
/// source's error; nothing partial is ever returned.
pub fn edit(source: &str, request: &EditRequest) -> Result<String, String> {
    if source != request.expected_source {
        return Err("The document changed; inspect it again before applying this edit.".into());
    }
    let mut document = Document::parse(source)
        .map_err(|error| error.to_string())?
        .into_mut();
    for edit in &request.edits {
        apply(&mut document, &request.document_path, edit)?;
    }
    let mut emitted = document.to_string();
    // toml_edit consumes a source BOM. Restore it before line matching, or the
    // unchanged first line also loses its own newline convention.
    if source.starts_with('\u{feff}') && !emitted.starts_with('\u{feff}') {
        emitted.insert(0, '\u{feff}');
    }
    let result = keep_final_newline_convention(source, restore_line_endings(source, &emitted));
    Document::parse(result.as_str()).map_err(|error| error.to_string())?;
    Ok(result)
}

/// The emitter ends every line, so a document whose last line had no ending
/// would gain one on any edit — a `\ No newline at end of file` diff line
/// under an otherwise exact change. The document keeps its own convention.
fn keep_final_newline_convention(source: &str, mut result: String) -> String {
    if !source.is_empty() && !source.ends_with('\n') {
        for ending in ["\r\n", "\n"] {
            if let Some(kept) = result.strip_suffix(ending) {
                result.truncate(kept.len());
                break;
            }
        }
    }
    result
}

/// A mutable position in the tree. Tables inside an array of tables are not
/// wrapped in an [`Item`], which is why this is not simply `&mut Item`.
enum Cursor<'a> {
    Table(&'a mut Table),
    Tables(&'a mut ArrayOfTables),
    Value(&'a mut Value),
}

fn locate<'a>(root: &'a mut Table, path: &[Segment]) -> Option<Cursor<'a>> {
    let mut cursor = Cursor::Table(root);
    for segment in path {
        cursor = match (cursor, segment) {
            (Cursor::Table(table), Segment::Key(key)) => match table.get_mut(key)? {
                Item::Table(table) => Cursor::Table(table),
                Item::ArrayOfTables(tables) => Cursor::Tables(tables),
                Item::Value(value) => Cursor::Value(value),
                Item::None => return None,
            },
            (Cursor::Tables(tables), Segment::Index(index)) => {
                Cursor::Table(tables.get_mut(*index)?)
            }
            (Cursor::Value(value), Segment::Key(key)) => {
                Cursor::Value(value.as_inline_table_mut()?.get_mut(key)?)
            }
            (Cursor::Value(value), Segment::Index(index)) => {
                Cursor::Value(value.as_array_mut()?.get_mut(*index)?)
            }
            _ => return None,
        };
    }
    Some(cursor)
}

fn split_key(path: &[Segment]) -> Result<(&[Segment], &str), String> {
    match path.split_last() {
        Some((Segment::Key(key), parent)) => Ok((parent, key)),
        _ => Err("The path must end in a key.".into()),
    }
}

/// Parse `value = <text>` and take the one value out with its exact spelling
/// and no surrounding decor, so a trailing comment or a second key in the
/// text cannot ride into the document.
fn parse_value(value_source: &str) -> Result<Value, String> {
    let replacement =
        Document::parse(format!("value = {value_source}")).map_err(|error| error.to_string())?;
    if replacement.iter().count() != 1 {
        return Err("A field edit must contain exactly one value.".into());
    }
    let mut replacement = replacement.into_mut();
    let mut value = replacement
        .as_table_mut()
        .remove("value")
        .and_then(|item| item.into_value().ok())
        .ok_or("Expected a value.")?;
    value.decor_mut().clear();
    Ok(value)
}

fn replace_keeping_decor(current: &mut Value, value: Value) {
    let decor = current.decor().clone();
    *current = value;
    *current.decor_mut() = decor;
}

fn raw(raw: Option<&RawString>) -> &str {
    raw.and_then(RawString::as_str).unwrap_or("")
}

fn prefix_of(value: &Value) -> String {
    raw(value.decor().prefix()).to_owned()
}

fn suffix_of(value: &Value) -> String {
    raw(value.decor().suffix()).to_owned()
}

/// Whether the array spans lines. The newline before `]` is the array's
/// trailing text after a trailing comma but the LAST ELEMENT'S SUFFIX when
/// there is none (`toml_edit` stores it that way), so both are consulted.
fn is_multiline(array: &Array) -> bool {
    raw(Some(array.trailing())).contains('\n')
        || array
            .iter()
            .any(|element| raw(element.decor().prefix()).contains('\n'))
        || array
            .iter()
            .last()
            .is_some_and(|last| suffix_of(last).contains('\n'))
}

/// The indentation of a multi-line array's elements: what follows the last
/// newline of the first element that starts a line.
fn indent_of(array: &Array) -> String {
    array
        .iter()
        .map(prefix_of)
        .find(|prefix| prefix.contains('\n'))
        .map(|prefix| prefix[prefix.rfind('\n').unwrap_or(0) + 1..].to_owned())
        .unwrap_or_else(|| "    ".into())
}

fn insert_element(array: &mut Array, index: usize, mut value: Value) -> Result<(), String> {
    let length = array.len();
    if index > length {
        return Err("The array index is out of range.".into());
    }
    if is_multiline(array) {
        let indent = indent_of(array);
        if index < length {
            // The displaced element's prefix carries the comment that ended the
            // line before it, so the newcomer takes that prefix and the
            // displaced element starts a fresh line.
            let previous = prefix_of(array.get(index).ok_or("The array index is out of range.")?);
            if previous.contains('\n') {
                if let Some(displaced) = array.get_mut(index) {
                    displaced.decor_mut().set_prefix(format!("\n{indent}"));
                }
            }
            value.decor_mut().set_prefix(previous);
        } else {
            // The text between the last element and `]` — its comment, the
            // newline and the indentation before the bracket — is the array's
            // trailing text after a trailing comma but the last element's
            // suffix without one; the comma the newcomer adds must come before
            // that comment, so the suffix moves out of the element either way.
            // Split so the comment stays on the element it describes and the
            // newcomer takes the bracket's line.
            let mut trailing = raw(Some(array.trailing())).to_owned();
            if !trailing.contains('\n') {
                if let Some(last) = length.checked_sub(1).and_then(|at| array.get_mut(at)) {
                    let suffix = suffix_of(last);
                    if suffix.contains('\n') {
                        last.decor_mut().set_suffix("");
                        trailing = format!("{suffix}{trailing}");
                    }
                }
            }
            match trailing.rfind('\n') {
                Some(newline) => {
                    value
                        .decor_mut()
                        .set_prefix(format!("{}\n{indent}", &trailing[..newline]));
                    array.set_trailing(format!("\n{}", &trailing[newline + 1..]));
                }
                None => value.decor_mut().set_prefix(format!("\n{indent}")),
            }
        }
        value.decor_mut().set_suffix("");
    } else if length == 0 {
        // `[ ]` keeps its padding on both sides of the one element.
        value
            .decor_mut()
            .set_prefix(raw(Some(array.trailing())).to_owned());
        value.decor_mut().set_suffix("");
    } else if index == 0 {
        value
            .decor_mut()
            .set_prefix(array.get(0).map(prefix_of).unwrap_or_default());
        value.decor_mut().set_suffix("");
        if let Some(first) = array.get_mut(0) {
            first.decor_mut().set_prefix(" ");
        }
    } else {
        value.decor_mut().set_prefix(" ");
        value.decor_mut().set_suffix("");
        if index == length {
            // The padding before `]` (`[ "A" ]`) belongs after the newcomer,
            // not between the last element and the comma.
            if let Some(last) = array.get_mut(length - 1) {
                let suffix = suffix_of(last);
                last.decor_mut().set_suffix("");
                value.decor_mut().set_suffix(suffix);
            }
        }
    }
    array.insert_formatted(index, value);
    Ok(())
}

fn remove_element(array: &mut Array, index: usize) -> Result<(), String> {
    let length = array.len();
    if index >= length {
        return Err("The array index is out of range.".into());
    }
    let removed = array.remove(index);
    let removed_prefix = prefix_of(&removed);
    if length == 1 {
        if is_multiline(array) || removed_prefix.contains('\n') {
            array.set_trailing("");
            array.set_trailing_comma(false);
        }
    } else if index < length - 1 {
        // The element that moved up keeps the removed one's place on the page,
        // so the comment that ended the previous line stays where it was.
        if let Some(next) = array.get_mut(index) {
            next.decor_mut().set_prefix(removed_prefix);
        }
    } else if let Some(newline) = removed_prefix.rfind('\n') {
        // The removed element's prefix ended the previous element's line; the
        // trailing text ended the removed element's own line and goes with it.
        let trailing = raw(Some(array.trailing())).to_owned();
        let closing = trailing
            .rfind('\n')
            .map(|at| &trailing[at + 1..])
            .unwrap_or("");
        array.set_trailing(format!("{}\n{closing}", &removed_prefix[..newline]));
    } else {
        // The padding before `]` is the LAST element's suffix (`[ "a", "b" ]`),
        // so removing the last element would take the page's own spacing with
        // it: it belongs to the element that is last now. The mirror of
        // `insert_element`'s `index == length` branch. Only horizontal padding
        // moves — a comment is the removed element's own and goes with it.
        let suffix = suffix_of(&removed);
        if !suffix.is_empty() && suffix.chars().all(|c| c == ' ' || c == '\t') {
            if let Some(last) = array.get_mut(length - 2) {
                last.decor_mut().set_suffix(suffix);
            }
        }
    }
    Ok(())
}

fn apply(document: &mut DocumentMut, document_path: &str, edit: &Edit) -> Result<(), String> {
    let root = document.as_table_mut();
    match edit {
        Edit::Set { path, value_source } => {
            let value = parse_value(value_source)?;
            super::definitions::check_value(document_path, path, &value.to_string())?;
            super::model_fields::validate(document_path, path, value_source)?;
            let Some(Cursor::Value(current)) = locate(root, path) else {
                return Err("The field no longer exists or is not a scalar.".into());
            };
            if current.is_array() || current.is_inline_table() {
                return Err("The field no longer exists or is not a scalar.".into());
            }
            let numeric = current.is_float() && value.is_integer();
            if value.type_name() != current.type_name() && !numeric {
                return Err("The value has a different type from this field.".into());
            }
            replace_keeping_decor(current, value);
        }
        Edit::Put { path, value_source } => {
            let value = parse_value(value_source)?;
            super::definitions::check_value(document_path, path, &value.to_string())?;
            let (parent, key) = split_key(path)?;
            if locate(root, parent).is_none() {
                // A missing parent is created as an inline table when its own
                // parent is a table: `ai_tuning.<rule> = {}` on a rung that
                // has no `ai_tuning` yet is the ordinary first rule.
                let (grandparent, parent_key) = split_key(parent)?;
                match locate(root, grandparent) {
                    Some(Cursor::Table(table)) => {
                        table.insert(
                            parent_key,
                            Item::Value(Value::InlineTable(InlineTable::new())),
                        );
                    }
                    Some(Cursor::Value(holder)) => {
                        holder
                            .as_inline_table_mut()
                            .ok_or("The parent is not a table.")?
                            .insert(parent_key, Value::InlineTable(InlineTable::new()));
                    }
                    _ => return Err("The parent table does not exist.".into()),
                }
            }
            match locate(root, parent) {
                Some(Cursor::Table(table)) => match table.get_mut(key) {
                    Some(Item::Value(current)) => replace_keeping_decor(current, value),
                    Some(_) => return Err("The key holds a table, not a value.".into()),
                    None => {
                        table.insert(key, Item::Value(value));
                    }
                },
                Some(Cursor::Value(holder)) => {
                    let table = holder
                        .as_inline_table_mut()
                        .ok_or("The parent is not a table.")?;
                    match table.get_mut(key) {
                        Some(current) => replace_keeping_decor(current, value),
                        None => {
                            table.insert(key, value);
                        }
                    }
                }
                _ => return Err("The parent table does not exist.".into()),
            }
        }
        Edit::Insert {
            path,
            index,
            value_source,
        } => {
            let value = parse_value(value_source)?;
            let mut element_path = path.clone();
            element_path.push(Segment::Index(*index));
            super::definitions::check_value(document_path, &element_path, &value.to_string())?;
            let Some(Cursor::Value(holder)) = locate(root, path) else {
                return Err("The array does not exist.".into());
            };
            let array = holder.as_array_mut().ok_or("The path is not an array.")?;
            insert_element(array, *index, value)?;
        }
        Edit::Remove { path } => match path.split_last() {
            Some((Segment::Key(key), parent)) => match locate(root, parent) {
                Some(Cursor::Table(table)) => {
                    table.remove(key).ok_or("The key does not exist.")?;
                }
                Some(Cursor::Value(holder)) => {
                    holder
                        .as_inline_table_mut()
                        .ok_or("The parent is not a table.")?
                        .remove(key)
                        .ok_or("The key does not exist.")?;
                }
                _ => return Err("The parent table does not exist.".into()),
            },
            Some((Segment::Index(index), parent)) => match locate(root, parent) {
                Some(Cursor::Tables(tables)) => {
                    if *index >= tables.len() {
                        return Err("The table index is out of range.".into());
                    }
                    tables.remove(*index);
                }
                Some(Cursor::Value(holder)) => {
                    let array = holder.as_array_mut().ok_or("The path is not an array.")?;
                    remove_element(array, *index)?;
                }
                _ => return Err("The array does not exist.".into()),
            },
            None => return Err("The path is empty.".into()),
        },
        Edit::AppendTable { path, fields } => {
            super::definitions::check_table_fields(document_path, path, fields)?;
            let (parent, key) = split_key(path)?;
            let Some(Cursor::Table(parent)) = locate(root, parent) else {
                return Err("The parent table does not exist.".into());
            };
            if parent.get(key).is_none() {
                parent.insert(key, Item::ArrayOfTables(ArrayOfTables::new()));
            }
            let tables = parent
                .get_mut(key)
                .and_then(Item::as_array_of_tables_mut)
                .ok_or("The key is not an array of tables.")?;
            let mut table = Table::new();
            for (field, value_source) in fields {
                table.insert(field, Item::Value(parse_value(value_source)?));
            }
            // DuplicateRatingName is a hard runtime refusal with no ordering
            // excuse, so it is the one cross-entry rule checked at edit time.
            if key == "rating" {
                if let Some(name) = table.get("name").and_then(Item::as_str) {
                    if tables
                        .iter()
                        .any(|existing| existing.get("name").and_then(Item::as_str) == Some(name))
                    {
                        return Err(format!(
                            "A rating named {name:?} already exists on this station."
                        ));
                    }
                }
            }
            tables.push(table);
        }
    }
    Ok(())
}

fn split_line(line: &str) -> (&str, &str) {
    if let Some(content) = line.strip_suffix("\r\n") {
        (content, "\r\n")
    } else if let Some(content) = line.strip_suffix('\n') {
        (content, "\n")
    } else {
        (line, "")
    }
}

/// Lines with their own endings, the last one possibly ending in nothing.
fn lines_with_endings(text: &str) -> Vec<(&str, &str)> {
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find('\n').map(|at| at + 1).unwrap_or(rest.len());
        lines.push(split_line(&rest[..end]));
        rest = &rest[end..];
    }
    lines
}

/// Longest common subsequence match of `b` onto `a`: for each line of `b`,
/// the index in `a` it corresponds to. Bounded so a pathological pair falls
/// back to "everything is new" rather than a quadratic table.
fn align(a: &[(&str, &str)], b: &[(&str, &str)]) -> Vec<Option<usize>> {
    const LIMIT: usize = 4_000_000;
    let mut matched = vec![None; b.len()];
    if a.is_empty() || b.is_empty() || a.len().saturating_mul(b.len()) > LIMIT {
        return matched;
    }
    let width = b.len() + 1;
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] = if a[i].0 == b[j].0 {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].0 == b[j].0 {
            matched[j] = Some(i);
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    matched
}

/// Give every line of the emitted document the ending it had in `original`,
/// and new lines the document's own convention. `toml_edit` drops carriage
/// returns on output, which would otherwise rewrite every line of a CRLF file.
fn restore_line_endings(original: &str, output: &str) -> String {
    if !original.contains('\r') {
        return output.to_owned();
    }
    let before = lines_with_endings(original);
    let after = lines_with_endings(output);
    let crlf = before
        .iter()
        .filter(|(_, ending)| *ending == "\r\n")
        .count();
    let lf = before.iter().filter(|(_, ending)| *ending == "\n").count();
    let convention = if crlf >= lf { "\r\n" } else { "\n" };
    // Edits are local, so only the changed middle needs the quadratic match.
    let prefix = before
        .iter()
        .zip(after.iter())
        .take_while(|(a, b)| a.0 == b.0)
        .count();
    let suffix = before[prefix..]
        .iter()
        .rev()
        .zip(after[prefix..].iter().rev())
        .take_while(|(a, b)| a.0 == b.0)
        .count();
    let middle = align(
        &before[prefix..before.len() - suffix],
        &after[prefix..after.len() - suffix],
    );
    let mut result = String::with_capacity(output.len() + after.len());
    for (index, (content, ending)) in after.iter().enumerate() {
        let original_ending = if index < prefix {
            Some(before[index].1)
        } else if index >= after.len() - suffix {
            Some(before[before.len() - (after.len() - index)].1)
        } else {
            middle[index - prefix].map(|at| before[prefix + at].1)
        };
        result.push_str(content);
        result.push_str(match (ending.is_empty(), original_ending) {
            (true, _) => "",
            (false, Some(original)) if !original.is_empty() => original,
            (false, _) => convention,
        });
    }
    result
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
