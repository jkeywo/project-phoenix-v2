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
    let result =
        keep_final_newline_convention(source, restore_line_endings(source, &document.to_string()));
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

    // ── Structural edits (issue #1474) ────────────────────────────────────

    const FACTION: &str = "assets/factions/rogue.toml";
    const HULL: &str = "assets/entities/probe_hull.toml";
    const ROGUE: &str = "eeeeeeee-5555-4555-8555-eeeeeeeeeeee";
    const ALLIANCE: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";
    const PIRATE: &str = "bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb";
    const HARROW: &str = "cccccccc-3333-4333-8333-cccccccccccc";

    fn faction_source() -> String {
        format!(
            "# Rogue traders\nuuid = \"{ROGUE}\"\nname = 'Rogue' # single quotes kept\nenemies = [\n    \"{ALLIANCE}\", # Alliance\n    \"{PIRATE}\", # Pirate\n]\nbanner = \"unknown extension\"\n\n[compliance]\nhold = \"refuse\"\n"
        )
    }

    fn hull_source() -> String {
        "class = \"cruiser\"\n\n[[station]]\nid = \"captain\"\nname = \"Captain\"\n\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[station.rating]]\nname = \"Simplified\"\nautomated_systems = [\"red-alert\"]\n\n[station.rating.ai_tuning]\ntorpedo_auto_fire = {}\n\n# ── Tactical ──\n[[station]]\nid = \"tactical\"\nname = \"Tactical\"\n\n[[station]]\nid = \"helm\"\nname = \"Helm\"\n\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[system]]\nid = \"red-alert\"\nkind = \"red_alert\"\nstation = \"captain\"\n".into()
    }

    fn key(text: &str) -> Segment {
        Segment::Key(text.into())
    }

    fn request(document_path: &str, source: &str, edits: Vec<Edit>) -> EditRequest {
        EditRequest {
            document_path: document_path.into(),
            expected_source: source.into(),
            edits,
        }
    }

    fn set(path: &[Segment], value_source: &str) -> Edit {
        Edit::Set {
            path: path.to_vec(),
            value_source: value_source.into(),
        }
    }

    fn put(path: &[Segment], value_source: &str) -> Edit {
        Edit::Put {
            path: path.to_vec(),
            value_source: value_source.into(),
        }
    }

    fn insert(path: &[Segment], index: usize, value_source: &str) -> Edit {
        Edit::Insert {
            path: path.to_vec(),
            index,
            value_source: value_source.into(),
        }
    }

    fn remove(path: &[Segment]) -> Edit {
        Edit::Remove {
            path: path.to_vec(),
        }
    }

    fn append(path: &[Segment], fields: &[(&str, &str)]) -> Edit {
        Edit::AppendTable {
            path: path.to_vec(),
            fields: fields
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    /// Every original line that carries none of the touched needles must
    /// reappear byte-for-byte, ending included, and in the original order.
    fn assert_untouched_lines_survive(original: &str, result: &str, touched: &[&str]) {
        let mut remaining = result;
        for (content, ending) in lines_with_endings(original) {
            if touched.iter().any(|needle| content.contains(needle)) {
                continue;
            }
            let line = format!("{content}{ending}");
            let at = remaining
                .find(&line)
                .unwrap_or_else(|| panic!("line {line:?} did not survive in:\n{result}"));
            remaining = &remaining[at + line.len()..];
        }
    }

    #[test]
    fn set_replaces_one_scalar_and_keeps_every_other_byte_on_both_line_endings() {
        for source in [faction_source(), faction_source().replace('\n', "\r\n")] {
            let result = edit(
                &source,
                &request(FACTION, &source, vec![set(&[key("name")], "\"Renamed\"")]),
            )
            .unwrap();
            assert_eq!(result, source.replace("'Rogue'", "\"Renamed\""));
            assert_untouched_lines_survive(&source, &result, &["name = "]);
        }
    }

    #[test]
    fn put_creates_a_root_key_after_the_last_root_value_and_updates_in_place() {
        let source = faction_source();
        let result = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![put(
                    &[key("display_name")],
                    "\"faction.rogue.display_name\"",
                )],
            ),
        )
        .unwrap();
        assert_eq!(
            result,
            source.replace(
                "banner = \"unknown extension\"\n",
                "banner = \"unknown extension\"\ndisplay_name = \"faction.rogue.display_name\"\n"
            )
        );
        let again = edit(
            &result,
            &request(
                FACTION,
                &result,
                vec![
                    put(&[key("display_name")], "\"faction.rogue.other\""),
                    put(&[key("compliance"), key("hold")], "\"comply\""),
                    put(&[key("compliance"), key("decide_secs")], "7"),
                ],
            ),
        )
        .unwrap();
        assert_eq!(
            again,
            result
                .replace("faction.rogue.display_name", "faction.rogue.other")
                .replace(
                    "hold = \"refuse\"\n",
                    "hold = \"comply\"\ndecide_secs = 7\n"
                )
        );
    }

    #[test]
    fn put_materialises_a_missing_parent_as_an_inline_table() {
        let source = hull_source();
        let helm_rule = [
            key("station"),
            Segment::Index(2),
            key("rating"),
            Segment::Index(0),
            key("ai_tuning"),
            key("torpedo_auto_fire"),
        ];
        let result = edit(
            &source,
            &request(HULL, &source, vec![put(&helm_rule, "{}")]),
        )
        .unwrap();
        assert_eq!(
            result,
            source.replace(
                "name = \"Std\"\nautomated_systems = []\n\n[[system]]",
                "name = \"Std\"\nautomated_systems = []\nai_tuning = { torpedo_auto_fire = {} }\n\n[[system]]"
            )
        );
        let removed = edit(&result, &request(HULL, &result, vec![remove(&helm_rule)])).unwrap();
        assert!(removed.contains("ai_tuning = {}\n"), "{removed}");
        let captain_rule = [
            key("station"),
            Segment::Index(0),
            key("rating"),
            Segment::Index(1),
            key("ai_tuning"),
            key("torpedo_auto_fire"),
        ];
        let cleared = edit(
            &source,
            &request(HULL, &source, vec![remove(&captain_rule)]),
        )
        .unwrap();
        assert_eq!(cleared, source.replace("torpedo_auto_fire = {}\n", ""));
        // The emptied `[station.rating.ai_tuning]` header stays a standard
        // table; the next rule goes back into it rather than being refused as
        // a value put over a table.
        let restored = edit(
            &cleared,
            &request(HULL, &cleared, vec![put(&captain_rule, "{}")]),
        )
        .unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn insert_at_the_end_of_a_multi_line_array_without_a_trailing_comma_keeps_the_bracket_line() {
        let tags = [key("tags")];
        let cases = [
            (
                "tags = [\n    \"A\",\n    \"B\"\n]\nafter = 1\n",
                "tags = [\n    \"A\",\n    \"B\",\n    \"C\"\n]\nafter = 1\n",
            ),
            (
                "tags = [\n    \"A\",\n    \"B\" # bye\n]\n",
                "tags = [\n    \"A\",\n    \"B\", # bye\n    \"C\"\n]\n",
            ),
            (
                "tags = [\n  \"A\",\n  \"B\"\n  ]\n",
                "tags = [\n  \"A\",\n  \"B\",\n  \"C\"\n  ]\n",
            ),
            (
                "tags = [\"A\", \"B\"\n]\n",
                "tags = [\"A\", \"B\",\n    \"C\"\n]\n",
            ),
        ];
        for (source, expected) in cases {
            for (source, expected) in [
                (source.to_owned(), expected.to_owned()),
                (source.replace('\n', "\r\n"), expected.replace('\n', "\r\n")),
            ] {
                let result = edit(
                    &source,
                    &request(WORLD, &source, vec![insert(&tags, 2, "\"C\"")]),
                )
                .unwrap();
                assert_eq!(result, expected);
                assert_untouched_lines_survive(&source, &result, &["\"B\""]);
            }
        }
    }

    #[test]
    fn a_document_without_a_final_newline_keeps_that_convention() {
        for source in [
            format!("uuid = \"{ROGUE}\"\nname = \"X\""),
            format!("uuid = \"{ROGUE}\"\r\nname = \"X\""),
        ] {
            let ending = if source.contains('\r') { "\r\n" } else { "\n" };
            let renamed = edit(
                &source,
                &request(FACTION, &source, vec![set(&[key("name")], "\"Y\"")]),
            )
            .unwrap();
            assert_eq!(renamed, source.replace("\"X\"", "\"Y\""));
            let appended = edit(
                &source,
                &request(FACTION, &source, vec![put(&[key("display_name")], "\"d\"")]),
            )
            .unwrap();
            assert_eq!(appended, format!("{source}{ending}display_name = \"d\""));
        }
    }

    #[test]
    fn remove_takes_a_rung_with_its_ai_tuning_table_and_leaves_the_next_station_intact() {
        let source = hull_source();
        let simplified = [
            key("station"),
            Segment::Index(0),
            key("rating"),
            Segment::Index(1),
        ];
        let result = edit(&source, &request(HULL, &source, vec![remove(&simplified)])).unwrap();
        assert_eq!(
            result,
            source.replace(
                "\n[[station.rating]]\nname = \"Simplified\"\nautomated_systems = [\"red-alert\"]\n\n[station.rating.ai_tuning]\ntorpedo_auto_fire = {}\n",
                ""
            )
        );
        assert!(result.contains("# ── Tactical ──\n[[station]]\nid = \"tactical\""));
        // A station's last rung leaves the station rung-less, and a rung can
        // be appended again afterwards.
        let helm = [key("station"), Segment::Index(2), key("rating")];
        let helm_std: Vec<Segment> = helm.iter().cloned().chain([Segment::Index(0)]).collect();
        let bare = edit(&source, &request(HULL, &source, vec![remove(&helm_std)])).unwrap();
        assert_eq!(
            bare,
            source.replace(
                "\n[[station.rating]]\nname = \"Std\"\nautomated_systems = []\n\n[[system]]",
                "\n[[system]]"
            )
        );
        let again = edit(
            &bare,
            &request(
                HULL,
                &bare,
                vec![append(
                    &helm,
                    &[("name", "\"Std\""), ("automated_systems", "[]")],
                )],
            ),
        )
        .unwrap();
        assert_eq!(again, source);
        let missing: Vec<Segment> = helm.iter().cloned().chain([Segment::Index(1)]).collect();
        assert!(edit(&source, &request(HULL, &source, vec![remove(&missing)])).is_err());
    }

    #[test]
    fn insert_keeps_a_multi_line_array_multi_line_and_comments_on_their_lines() {
        let source = faction_source();
        let enemies = [key("enemies")];
        let appended = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![insert(&enemies, 2, &format!("\"{HARROW}\""))],
            ),
        )
        .unwrap();
        assert_eq!(
            appended,
            source.replace(
                &format!("    \"{PIRATE}\", # Pirate\n]"),
                &format!("    \"{PIRATE}\", # Pirate\n    \"{HARROW}\",\n]")
            )
        );
        assert_untouched_lines_survive(&source, &appended, &[]);
        let first = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![insert(&enemies, 0, &format!("\"{HARROW}\""))],
            ),
        )
        .unwrap();
        assert_eq!(
            first,
            source.replace(
                "enemies = [\n",
                &format!("enemies = [\n    \"{HARROW}\",\n")
            )
        );
        let crlf = source.replace('\n', "\r\n");
        let appended = edit(
            &crlf,
            &request(
                FACTION,
                &crlf,
                vec![insert(&enemies, 2, &format!("\"{HARROW}\""))],
            ),
        )
        .unwrap();
        assert!(
            !appended.contains("\",\n"),
            "new text uses CRLF:\n{appended}"
        );
        assert_untouched_lines_survive(&crlf, &appended, &[]);
    }

    #[test]
    fn insert_into_a_single_line_array_stays_on_one_line() {
        let source = format!("uuid = \"{ROGUE}\"\nname = \"Rogue\"\nenemies = [\"{ALLIANCE}\"]\n");
        let enemies = [key("enemies")];
        let appended = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![insert(&enemies, 1, &format!("\"{PIRATE}\""))],
            ),
        )
        .unwrap();
        assert_eq!(
            appended,
            source.replace(
                &format!("[\"{ALLIANCE}\"]"),
                &format!("[\"{ALLIANCE}\", \"{PIRATE}\"]")
            )
        );
        let first = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![insert(&enemies, 0, &format!("\"{PIRATE}\""))],
            ),
        )
        .unwrap();
        assert_eq!(
            first,
            source.replace(
                &format!("[\"{ALLIANCE}\"]"),
                &format!("[\"{PIRATE}\", \"{ALLIANCE}\"]")
            )
        );
        let empty = source.replace(&format!("[\"{ALLIANCE}\"]"), "[]");
        let filled = edit(
            &empty,
            &request(
                FACTION,
                &empty,
                vec![insert(&enemies, 0, &format!("\"{PIRATE}\""))],
            ),
        )
        .unwrap();
        assert_eq!(filled, empty.replace("[]", &format!("[\"{PIRATE}\"]")));
        // Padded brackets keep their padding around the whole list.
        let padded = source.replace(&format!("[\"{ALLIANCE}\"]"), &format!("[ \"{ALLIANCE}\" ]"));
        let last = edit(
            &padded,
            &request(
                FACTION,
                &padded,
                vec![insert(&enemies, 1, &format!("\"{PIRATE}\""))],
            ),
        )
        .unwrap();
        assert_eq!(
            last,
            padded.replace(
                &format!("[ \"{ALLIANCE}\" ]"),
                &format!("[ \"{ALLIANCE}\", \"{PIRATE}\" ]")
            )
        );
        let first = edit(
            &padded,
            &request(
                FACTION,
                &padded,
                vec![insert(&enemies, 0, &format!("\"{PIRATE}\""))],
            ),
        )
        .unwrap();
        assert_eq!(
            first,
            padded.replace(
                &format!("[ \"{ALLIANCE}\" ]"),
                &format!("[ \"{PIRATE}\", \"{ALLIANCE}\" ]")
            )
        );
        let blank = padded.replace(&format!("[ \"{ALLIANCE}\" ]"), "[ ]");
        let one = edit(
            &blank,
            &request(
                FACTION,
                &blank,
                vec![insert(&enemies, 0, &format!("\"{PIRATE}\""))],
            ),
        )
        .unwrap();
        assert_eq!(one, blank.replace("[ ]", &format!("[ \"{PIRATE}\" ]")));
        assert!(edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![insert(&enemies, 2, &format!("\"{PIRATE}\""))]
            )
        )
        .is_err());
    }

    #[test]
    fn remove_takes_an_element_with_its_own_comment_and_no_other() {
        let source = faction_source();
        let first = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![remove(&[key("enemies"), Segment::Index(0)])],
            ),
        )
        .unwrap();
        assert_eq!(
            first,
            source.replace(&format!("    \"{ALLIANCE}\", # Alliance\n"), "")
        );
        let last = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![remove(&[key("enemies"), Segment::Index(1)])],
            ),
        )
        .unwrap();
        assert_eq!(
            last,
            source.replace(&format!("    \"{PIRATE}\", # Pirate\n"), "")
        );
        let none = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![
                    remove(&[key("enemies"), Segment::Index(1)]),
                    remove(&[key("enemies"), Segment::Index(0)]),
                ],
            ),
        )
        .unwrap();
        assert!(none.contains("enemies = []\n"), "{none}");
        assert_untouched_lines_survive(&source, &none, &["enemies", "# Alliance", "# Pirate", "]"]);
        let table_gone = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![remove(&[key("banner")]), remove(&[key("compliance")])],
            ),
        )
        .unwrap();
        assert_eq!(
            table_gone,
            source.replace(
                "banner = \"unknown extension\"\n\n[compliance]\nhold = \"refuse\"\n",
                ""
            )
        );
        assert!(edit(
            &source,
            &request(FACTION, &source, vec![remove(&[key("missing")])])
        )
        .is_err());
        assert!(edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![remove(&[key("enemies"), Segment::Index(2)])]
            )
        )
        .is_err());
    }

    #[test]
    fn append_table_lands_after_that_stations_last_rung_and_before_the_next_station() {
        let source = hull_source();
        let captain = [key("station"), Segment::Index(0), key("rating")];
        let result = edit(
            &source,
            &request(
                HULL,
                &source,
                vec![append(
                    &captain,
                    &[("name", "\"Novice\""), ("automated_systems", "[]")],
                )],
            ),
        )
        .unwrap();
        // After the last rung INCLUDING its `[station.rating.ai_tuning]`
        // sub-table, or the sub-table would attach to the new rung.
        assert_eq!(
            result,
            source.replace(
                "torpedo_auto_fire = {}\n\n# ── Tactical ──",
                "torpedo_auto_fire = {}\n\n[[station.rating]]\nname = \"Novice\"\nautomated_systems = []\n\n# ── Tactical ──"
            )
        );
        assert_untouched_lines_survive(&source, &result, &[]);
        let tactical = [key("station"), Segment::Index(1), key("rating")];
        let bare = edit(
            &source,
            &request(
                HULL,
                &source,
                vec![append(&tactical, &[("name", "\"Std\"")])],
            ),
        )
        .unwrap();
        assert_eq!(
            bare,
            source.replace(
                "name = \"Tactical\"\n\n[[station]]\nid = \"helm\"",
                "name = \"Tactical\"\n\n[[station.rating]]\nname = \"Std\"\n\n[[station]]\nid = \"helm\""
            )
        );
        assert!(
            edit(
                &source,
                &request(
                    HULL,
                    &source,
                    vec![append(&captain, &[("name", "\"Std\"")])]
                )
            )
            .is_err(),
            "a duplicate rung name is refused at edit time"
        );
        assert!(edit(
            &source,
            &request(
                HULL,
                &source,
                vec![append(&captain, &[("automated_systems", "[]")])]
            )
        )
        .is_err());
    }

    #[test]
    fn crlf_documents_get_crlf_new_lines_and_keep_every_other_line() {
        let source = hull_source().replace('\n', "\r\n");
        let captain = [key("station"), Segment::Index(0), key("rating")];
        let result = edit(
            &source,
            &request(
                HULL,
                &source,
                vec![
                    append(
                        &captain,
                        &[("name", "\"Novice\""), ("automated_systems", "[]")],
                    ),
                    put(
                        &[key("station"), Segment::Index(1), key("visiting_rating")],
                        "\"Std\"",
                    ),
                ],
            ),
        )
        .unwrap();
        assert!(
            !result.replace("\r\n", "").contains('\n'),
            "every line ends in CRLF:\n{result}"
        );
        assert!(result
            .contains("[[station.rating]]\r\nname = \"Novice\"\r\nautomated_systems = []\r\n"));
        assert!(result.contains("name = \"Tactical\"\r\nvisiting_rating = \"Std\"\r\n"));
        assert_untouched_lines_survive(&source, &result, &[]);
        let mixed = hull_source().replacen('\n', "\r\n", 3);
        let result = edit(
            &mixed,
            &request(
                HULL,
                &mixed,
                vec![append(&captain, &[("name", "\"Novice\"")])],
            ),
        )
        .unwrap();
        assert_untouched_lines_survive(&mixed, &result, &[]);
        assert!(
            result.contains("[[station.rating]]\nname = \"Novice\"\n"),
            "{result}"
        );
    }

    #[test]
    fn a_refused_edit_in_a_group_applies_nothing_and_stale_or_injected_text_is_refused() {
        let source = faction_source();
        let refused = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![
                    set(&[key("name")], "\"Renamed\""),
                    insert(&[key("enemies")], 0, "\"not-a-uuid\""),
                ],
            ),
        );
        assert!(refused.is_err());
        assert!(edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![put(&[key("compliance"), key("hold")], "\"maybe\"")]
            )
        )
        .is_err());
        assert!(edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![set(&[key("name")], "\"A\"\nextra = 1")]
            )
        )
        .is_err());
        assert!(edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![set(&[key("name")], "\"A\" # comment\n[table]")]
            )
        )
        .is_err());
        assert!(edit(
            &source,
            &request(FACTION, &source, vec![set(&[key("enemies")], "\"scalar\"")])
        )
        .is_err());
        assert!(edit(
            &source,
            &request(FACTION, &source, vec![set(&[key("name")], "4")])
        )
        .is_err());
        assert!(edit(
            &format!("{source}# moved\n"),
            &request(FACTION, &source, vec![set(&[key("name")], "\"A\"")])
        )
        .is_err());
        let hull = hull_source();
        let rung_name = [
            key("station"),
            Segment::Index(0),
            key("rating"),
            Segment::Index(0),
            key("name"),
        ];
        assert!(edit(&hull, &request(HULL, &hull, vec![set(&rung_name, "\"\"")])).is_err());
        let commented = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![set(&[key("name")], "\"A\" # trailing")],
            ),
        )
        .unwrap();
        assert_eq!(
            commented,
            source.replace("'Rogue'", "\"A\""),
            "a trailing comment does not ride in"
        );
    }

    #[test]
    fn a_whole_form_apply_is_one_reparseable_result() {
        let source = faction_source();
        let result = edit(
            &source,
            &request(
                FACTION,
                &source,
                vec![
                    set(&[key("name")], "\"Renamed\""),
                    remove(&[key("enemies"), Segment::Index(1)]),
                    insert(&[key("enemies")], 1, &format!("\"{HARROW}\"")),
                    put(&[key("compliance"), key("divert")], "\"refuse\""),
                ],
            ),
        )
        .unwrap();
        let parsed = crate::ai::faction::parse_faction_config(&result).unwrap();
        assert_eq!(parsed.name, "Renamed");
        assert_eq!(
            parsed
                .enemies
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            vec![ALLIANCE.to_owned(), HARROW.to_owned()]
        );
        let compliance = parsed.compliance.unwrap();
        assert_eq!(compliance.hold, crate::civilian::OrderResponse::Refuse);
        assert_eq!(compliance.divert, crate::civilian::OrderResponse::Refuse);
        assert!(result.contains("# Rogue traders\n"));
        assert!(result.contains("# Alliance\n"));
        assert!(result.contains("banner = \"unknown extension\"\n"));
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
