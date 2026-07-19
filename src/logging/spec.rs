//! Parsing for the log spec string shared by the CLI flag and the URL param.
//!
//! One parser, two front ends — `phoenix-headless --log ai=debug,admit=trace`
//! and `server.html?log=ai=debug,admit=trace` produce the same
//! [`LogFilterConfig`].

use super::{empty_per_cat, EntityFilter, LevelFilter, LogCat, LogFilterConfig};
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
    let mut cfg = LogFilterConfig {
        per_cat: empty_per_cat(),
        ..Default::default()
    };

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
mod tests {
    use super::*;

    #[test]
    fn empty_spec_is_the_default() {
        let cfg = parse_log_spec("").unwrap();
        assert_eq!(cfg.default_level, LevelFilter::Warn);
        assert!(cfg.per_cat.is_empty());
    }

    #[test]
    fn bare_level_sets_the_default() {
        let cfg = parse_log_spec("debug").unwrap();
        assert_eq!(cfg.default_level, LevelFilter::Debug);
    }

    #[test]
    fn mixed_spec_sets_default_and_overrides() {
        let cfg = parse_log_spec("info,ai=debug,admit=trace,physics=off").unwrap();
        assert_eq!(cfg.default_level, LevelFilter::Info);
        assert_eq!(cfg.per_cat[&LogCat::Ai], LevelFilter::Debug);
        assert_eq!(cfg.per_cat[&LogCat::Admit], LevelFilter::Trace);
        assert_eq!(cfg.per_cat[&LogCat::Physics], LevelFilter::Off);
        assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Debug));
        assert!(!cfg.cat_enabled(LogCat::Physics, LevelFilter::Error));
        // Unlisted category falls back to the spec's default.
        assert!(cfg.cat_enabled(LogCat::Helm, LevelFilter::Info));
    }

    #[test]
    fn whitespace_and_trailing_commas_are_tolerated() {
        let cfg = parse_log_spec(" info , ai = debug , ").unwrap();
        assert_eq!(cfg.default_level, LevelFilter::Info);
        assert_eq!(cfg.per_cat[&LogCat::Ai], LevelFilter::Debug);
    }

    #[test]
    fn category_and_level_are_case_insensitive() {
        let cfg = parse_log_spec("AI=DEBUG").unwrap();
        assert_eq!(cfg.per_cat[&LogCat::Ai], LevelFilter::Debug);
    }

    #[test]
    fn unknown_category_is_an_error() {
        assert_eq!(
            parse_log_spec("warpcore=debug").unwrap_err(),
            LogSpecError::UnknownCategory("warpcore".into())
        );
    }

    #[test]
    fn unknown_level_is_an_error() {
        assert_eq!(
            parse_log_spec("ai=chatty").unwrap_err(),
            LogSpecError::UnknownLevel("chatty".into())
        );
    }

    #[test]
    fn double_equals_is_malformed() {
        assert!(matches!(
            parse_log_spec("ai=debug=trace").unwrap_err(),
            LogSpecError::Malformed(_)
        ));
    }

    #[test]
    fn entity_list_parses_and_trims() {
        let f = parse_log_entities("Ironveil, Ashrender").unwrap();
        assert_eq!(f.names, vec!["Ironveil", "Ashrender"]);
    }

    #[test]
    fn empty_entity_list_means_no_filtering() {
        assert!(parse_log_entities("").is_none());
        assert!(parse_log_entities("  , ").is_none());
    }

    #[test]
    fn every_category_round_trips_through_its_target_string() {
        use strum::IntoEnumIterator;
        for cat in LogCat::iter() {
            let spec = format!("{}=trace", cat.target());
            let cfg = parse_log_spec(&spec)
                .unwrap_or_else(|e| panic!("category {cat:?} failed to parse: {e}"));
            assert_eq!(cfg.per_cat[&cat], LevelFilter::Trace);
        }
    }
}
