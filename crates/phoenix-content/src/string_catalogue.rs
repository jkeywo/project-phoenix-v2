//! Partial String Table validation shared by content admission.
use crate::archive::{MANIFEST_PATH, STRING_CATALOGUE_PATH};
use crate::findings::{Severity, SourceLocation, WorldFinding};
use std::collections::BTreeSet;

/// Build a single archive-scoped finding (no line lookup — the archive is not a
/// single source file).
pub fn archive_finding(
    severity: Severity,
    category: &'static str,
    reference: &str,
    message: String,
) -> WorldFinding {
    WorldFinding {
        severity,
        category,
        message,
        source: SourceLocation {
            file: MANIFEST_PATH.to_string(),
            line: None,
            reference: reference.to_string(),
        },
    }
}

/// An archive-scoped ERROR finding (blocks acceptance).
pub fn archive_error(category: &'static str, reference: &str, message: String) -> WorldFinding {
    archive_finding(Severity::Error, category, reference, message)
}

/// A decoded member (including an unchanged dependency) failed validation.
/// Attribute it to the file the author must repair, not the pack manifest.
pub fn member_error(category: &'static str, path: &str, message: String) -> WorldFinding {
    let mut finding = archive_error(category, path, message);
    finding.source.file = path.to_owned();
    finding
}

pub fn member_warning(category: &'static str, path: &str, message: String) -> WorldFinding {
    let mut finding = archive_finding(Severity::Warning, category, path, message);
    finding.source.file = path.to_owned();
    finding
}

fn parse_catalogue_csv(source: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = source.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
        } else if ch == '"' && field.is_empty() {
            quoted = true;
        } else if ch == ',' {
            row.push(std::mem::take(&mut field));
        } else if ch == '\n' {
            row.push(std::mem::take(&mut field));
            if !(row.len() == 1 && row[0].is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else if ch != '\r' {
            field.push(ch);
        }
    }
    if quoted {
        return Err("unterminated quoted field".into());
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

fn catalogue_locale(name: &str) -> bool {
    if name == "id" {
        return false;
    }
    let mut parts = name.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    let language_ok =
        (2..=3).contains(&language.len()) && language.chars().all(|ch| ch.is_ascii_lowercase());
    let region_ok = match parts.next() {
        None => true,
        Some(region) => region.len() == 2 && region.chars().all(|ch| ch.is_ascii_uppercase()),
    };
    language_ok && region_ok && parts.next().is_none()
}

fn catalogue_placeholders(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('}') else { break };
        let value = &rest[..close];
        if !value.is_empty()
            && value
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            found.push(value);
        }
        rest = &rest[close + 1..];
    }
    found.sort_unstable();
    found
}

pub fn validate_string_catalogue(source: &str) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let rows = match parse_catalogue_csv(source) {
        Ok(rows) if !rows.is_empty() => rows,
        Ok(_) => {
            return vec![member_error(
                "string-catalogue-schema",
                STRING_CATALOGUE_PATH,
                "String Table is empty".into(),
            )];
        }
        Err(error) => {
            return vec![member_error(
                "malformed-string-catalogue",
                STRING_CATALOGUE_PATH,
                format!("String Table CSV is malformed: {error}"),
            )];
        }
    };
    let header = &rows[0];
    let id_col = header.iter().position(|name| name == "id");
    if id_col.is_none() {
        findings.push(member_error(
            "string-catalogue-schema",
            STRING_CATALOGUE_PATH,
            "String Table is missing required 'id' column".into(),
        ));
    }
    let locales: Vec<_> = header
        .iter()
        .filter(|name| catalogue_locale(name))
        .collect();
    if locales.is_empty() {
        findings.push(member_error(
            "string-catalogue-schema",
            STRING_CATALOGUE_PATH,
            "String Table has no locale columns".into(),
        ));
    }
    let mut columns = BTreeSet::new();
    if header.iter().any(|name| !columns.insert(name)) {
        findings.push(member_error(
            "string-catalogue-schema",
            STRING_CATALOGUE_PATH,
            "String Table has duplicate column names".into(),
        ));
    }
    for name in header {
        if let Some(locale) = name
            .strip_suffix("_source")
            .or_else(|| name.strip_suffix("_provenance"))
        {
            if !locales.iter().any(|candidate| candidate.as_str() == locale) {
                findings.push(member_error(
                    "string-catalogue-schema",
                    STRING_CATALOGUE_PATH,
                    format!("{name} has no matching {locale} locale column"),
                ));
            }
        }
    }
    let mut ids = BTreeSet::new();
    for (index, row) in rows.iter().enumerate().skip(1) {
        let id = id_col
            .and_then(|col| row.get(col))
            .map_or("", String::as_str)
            .trim();
        if row.len() != header.len() {
            findings.push(member_error(
                "malformed-string-catalogue",
                STRING_CATALOGUE_PATH,
                format!(
                    "row {} ({id}) has {} fields; header has {}",
                    index + 1,
                    row.len(),
                    header.len()
                ),
            ));
            continue;
        }
        if id.is_empty() || !ids.insert(id) {
            findings.push(member_error(
                "string-catalogue-schema",
                STRING_CATALOGUE_PATH,
                if id.is_empty() {
                    format!("row {} has a blank id", index + 1)
                } else {
                    format!("String Table has duplicate id {id:?}")
                },
            ));
        }
        for locale in locales.iter().filter(|locale| locale.as_str() != "en") {
            let value = &row[header
                .iter()
                .position(|name| name == locale.as_str())
                .unwrap()];
            if value.trim().is_empty() {
                findings.push(member_warning(
                    "translation-fallback",
                    STRING_CATALOGUE_PATH,
                    format!("{id}: {locale} is blank; players use effective English"),
                ));
                continue;
            }
            let source_name = format!("{locale}_source");
            let source = header
                .iter()
                .position(|name| name == &source_name)
                .map(|col| row[col].as_str())
                .unwrap_or("");
            if source.is_empty() {
                findings.push(member_warning(
                    "translation-fallback",
                    STRING_CATALOGUE_PATH,
                    format!(
                        "{id}: {source_name} is missing or blank; players use effective English"
                    ),
                ));
                continue;
            }
            let english = header
                .iter()
                .position(|name| name == "en")
                .map(|col| row[col].as_str())
                .filter(|value| !value.is_empty())
                .unwrap_or(source);
            if catalogue_placeholders(english) != catalogue_placeholders(value) {
                findings.push(member_warning("invalid-translation-placeholders", STRING_CATALOGUE_PATH,
                    format!("{id}: {locale} placeholders do not match its English source; players use effective English")));
            }
        }
    }
    findings
}
