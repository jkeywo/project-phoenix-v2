//! Parsing for the log spec string shared by the CLI flag and the URL param.
//!
//! One parser, two front ends — `phoenix-headless --log ai=debug,admit=trace`
//! and `server.html?log=ai=debug,admit=trace` produce the same
//! [`LogFilterConfig`].

use super::{EntityFilter, LevelFilter, LogCat, LogFilterConfig};
use std::str::FromStr;

/// A log spec that could not be parsed. Carries the offending fragment so the
/// caller can print something actionable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogSpecError {
    UnknownCategory(String),
    UnknownLevel(String),
    /// More than one `=` in a single comma-separated entry.
    Malformed(String),
}

impl std::fmt::Display for LogSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCategory(s) => write!(f, "unknown log category {s:?}"),
            Self::UnknownLevel(s) => write!(f, "unknown log level {s:?}"),
            Self::Malformed(s) => write!(f, "malformed log spec entry {s:?}"),
        }
    }
}

impl std::error::Error for LogSpecError {}

fn parse_level(s: &str) -> Result<LevelFilter, LogSpecError> {
    match s.trim().to_lowercase().as_str() {
        "off" | "none" => Ok(LevelFilter::Off),
        "error" => Ok(LevelFilter::Error),
        "warn" | "warning" => Ok(LevelFilter::Warn),
        "info" => Ok(LevelFilter::Info),
        "debug" => Ok(LevelFilter::Debug),
        "trace" => Ok(LevelFilter::Trace),
        other => Err(LogSpecError::UnknownLevel(other.to_string())),
    }
}

/// Parse a spec like `"info,ai=debug,admit=trace,physics=off"`.
///
/// A bare level (no `=`) sets the default for every category; later bare levels
/// override earlier ones. Entries with `=` set one category. Empty spec yields
/// the default config.
pub fn parse_log_spec(spec: &str) -> Result<LogFilterConfig, LogSpecError> {
    let mut cfg = LogFilterConfig::default();

    for entry in spec.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let mut parts = entry.splitn(3, '=');
        let first = parts.next().unwrap_or_default().trim();
        match (parts.next(), parts.next()) {
            // Bare level: `info`
            (None, _) => cfg.default_level = parse_level(first)?,
            // `cat=level`
            (Some(level), None) => {
                let cat = LogCat::from_str(&first.to_lowercase())
                    .map_err(|_| LogSpecError::UnknownCategory(first.to_string()))?;
                cfg.per_cat.insert(cat, parse_level(level)?);
            }
            // `a=b=c`
            (Some(_), Some(_)) => return Err(LogSpecError::Malformed(entry.to_string())),
        }
    }

    Ok(cfg)
}

/// Parse a comma-separated entity name list into an [`EntityFilter`].
///
/// Returns `None` for an empty or whitespace-only list, which means "no entity
/// filtering" rather than "filter matching nothing".
pub fn parse_log_entities(names: &str) -> Option<EntityFilter> {
    let names: Vec<String> = names
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        None
    } else {
        Some(EntityFilter::new(names))
    }
}

#[cfg(test)]
#[path = "spec_tests.rs"]
mod tests;
