//! Shared source precedence and repairable-draft admission; domain rules stay local.
use std::collections::BTreeMap;

use super::{WorkshopDependencies, WorkshopFinding};

pub(super) fn beneath_of(dependencies: &WorkshopDependencies) -> BTreeMap<String, String> {
    let mut sources = dependencies.base_files.clone();
    for pack in &dependencies.packs {
        sources.extend(pack.files.clone());
    }
    sources
}

pub(super) fn sources_of(
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
) -> BTreeMap<String, String> {
    let mut sources = beneath_of(dependencies);
    sources.extend(files.clone());
    sources
}

pub(super) fn origin_of(
    path: &str,
    files: &BTreeMap<String, String>,
    dependencies: &WorkshopDependencies,
) -> Option<(usize, String)> {
    if files.contains_key(path) {
        return Some((0, "draft".into()));
    }
    for (index, pack) in dependencies.packs.iter().enumerate().rev() {
        if pack.files.contains_key(path) {
            return Some((index + 2, format!("pack:{}", pack.id)));
        }
    }
    dependencies
        .base_files
        .contains_key(path)
        .then(|| (1, "base".into()))
}

// Entity diagnostics allow absent source lines; Composition and Presets require one.
pub(super) struct Issue<L = usize> {
    pub line: L,
    pub category: &'static str,
    pub key: String,
    pub message: String,
}

pub(super) fn introduced<L>(
    before: Vec<Issue<L>>,
    after: Vec<Issue<L>>,
    refusal: impl Fn(&Issue<L>) -> String,
) -> Result<(), String> {
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

pub(super) fn finding(
    category: &str,
    file: &str,
    line: Option<usize>,
    message: String,
) -> WorkshopFinding {
    WorkshopFinding {
        severity: "error".into(),
        category: category.into(),
        message,
        file: file.to_owned(),
        line,
    }
}

pub(super) fn sort_findings(findings: &mut Vec<WorkshopFinding>) {
    findings.sort_by(|a, b| {
        (&a.file, a.line, &a.category, &a.message).cmp(&(&b.file, b.line, &b.category, &b.message))
    });
    findings.dedup();
}
#[cfg(test)]
#[path = "validation_support_tests.rs"]
mod tests;
