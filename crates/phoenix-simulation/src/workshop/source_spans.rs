//! Exact-source helpers shared by the definition and composition catalogs
//! (issues #1474, #1475).
//!
//! The runtime's own whole-file `toml::from_str` never knew a line, so a
//! finding that points at the rung, the enemy entry or the `extra_worlds`
//! path the runtime would refuse has to come from the TOML syntax tree. These
//! helpers turn a `toml_edit` span into the 1-based line a panel shows beside
//! `file:` — one copy, so the two catalogs cannot count lines differently.
use std::ops::Range;

use toml_edit::{Item, Table, TableLike, Value};

/// Whether `path` is a `.toml` member directly or transitively under
/// `directory` (`assets/factions/`, `assets/worlds/`, ...).
pub(super) fn is_member(path: &str, directory: &str) -> bool {
    path.starts_with(directory) && path.ends_with(".toml")
}

/// The 1-based line holding byte `offset` of `source`.
pub(super) fn line_at(source: &str, offset: usize) -> usize {
    source[..offset.min(source.len())]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

pub(super) fn span_line(source: &str, span: Option<Range<usize>>) -> Option<usize> {
    span.map(|span| line_at(source, span.start))
}

/// The header line of a `[[table]]`, or its first value's line for a table
/// the parser gave no header span (an implicit or dotted one).
pub(super) fn table_line(source: &str, table: &Table) -> usize {
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
pub(super) fn value_text(source: &str, value: &Value) -> String {
    value.as_str().map(str::to_owned).unwrap_or_else(|| {
        value
            .span()
            .map(|span| source[span].to_owned())
            .unwrap_or_default()
    })
}

pub(super) fn string_field<'a>(table: &'a dyn TableLike, key: &str) -> Option<&'a str> {
    table.get(key)?.as_str()
}

/// The exact text and 1-based line of one scalar key of a table, or `None`
/// when the table does not carry it (issue #1477). `fallback` is the line a
/// panel should point at for a value whose own span the parser did not record
/// — the entry's header line, which only the caller knows.
pub(super) fn scalar_at(
    source: &str,
    table: &dyn TableLike,
    key: &str,
    fallback: usize,
) -> Option<(String, usize)> {
    let value = table.get(key)?.as_value()?;
    Some((
        value_text(source, value),
        span_line(source, value.span()).unwrap_or(fallback),
    ))
}

/// Every table-like node of a document, depth first, so a trigger action is
/// found wherever the world schema nests it.
pub(super) fn visit_tables<'a>(item: &'a Item, visit: &mut dyn FnMut(&'a dyn TableLike)) {
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
