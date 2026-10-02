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
mod tests {
    use super::*;
    use crate::workshop::WorkshopDependencyPack;

    fn files(value: &str) -> BTreeMap<String, String> {
        [("same".into(), value.into())].into()
    }

    #[test]
    fn overlays_and_provenance_keep_declared_precedence() {
        let dependencies = WorkshopDependencies {
            base_files: files("base"),
            packs: ["old", "new"]
                .map(|id| WorkshopDependencyPack {
                    id: id.into(),
                    manifest_toml: String::new(),
                    files: files(id),
                    assets: BTreeMap::new(),
                })
                .into(),
            ..Default::default()
        };
        let empty = BTreeMap::new();
        assert_eq!(beneath_of(&dependencies)["same"], "new");
        assert_eq!(
            origin_of("same", &empty, &dependencies),
            Some((3, "pack:new".into()))
        );
        assert_eq!(sources_of(&files("draft"), &dependencies)["same"], "draft");
        assert_eq!(
            origin_of("same", &files("draft"), &dependencies),
            Some((0, "draft".into()))
        );
        assert_eq!(origin_of("missing", &empty, &dependencies), None);
        let base = WorkshopDependencies {
            base_files: files("base"),
            ..Default::default()
        };
        assert_eq!(origin_of("same", &empty, &base), Some((1, "base".into())));
    }

    fn issue(key: &str, line: usize) -> Issue {
        Issue {
            category: "rule",
            key: key.into(),
            line,
            message: format!("line {line}"),
        }
    }

    #[test]
    fn admission_counts_occurrences_and_refuses_first_new_issue() {
        assert_eq!(
            introduced(
                vec![issue("a", 1), issue("b", 2), issue("a", 3)],
                vec![issue("b", 9), issue("a", 8), issue("a", 7)],
                |i| i.message.clone()
            ),
            Ok(())
        );
        assert_eq!(
            introduced(
                vec![issue("a", 1)],
                vec![issue("a", 4), issue("a", 5), issue("b", 6)],
                |i| i.message.clone()
            ),
            Err("line 5".into())
        );
        let absent = Issue {
            line: None::<usize>,
            category: "rule",
            key: "a".into(),
            message: "absent".into(),
        };
        assert_eq!(
            introduced(Vec::new(), vec![absent], |i| format!(
                "{:?}:{}",
                i.line, i.message
            )),
            Err("None:absent".into())
        );
    }

    #[test]
    fn findings_keep_optional_lines_severity_and_exact_sort_order() {
        let a = finding("a", "first", None, "message".into());
        let mut warning = a.clone();
        warning.severity = "warning".into();
        let b = finding("b", "first", Some(2), "message".into());
        let c = finding("a", "last", Some(1), "message".into());
        let mut findings = vec![c.clone(), b.clone(), a.clone(), a.clone(), warning.clone()];
        sort_findings(&mut findings);
        assert_eq!(findings, vec![a, warning, b, c]);
    }
}
