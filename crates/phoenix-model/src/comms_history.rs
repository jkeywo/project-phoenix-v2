//! Own issued hails, scoped to the actual ship that issued them.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HailHistory(BTreeMap<String, BTreeSet<String>>);

impl HailHistory {
    pub fn record(&mut self, source: &str, target: &str) {
        self.0
            .entry(source.into())
            .or_default()
            .insert(target.into());
    }

    pub fn contains(&self, source: &str, target: &str) -> bool {
        self.0
            .get(source)
            .is_some_and(|targets| targets.contains(target))
    }

    pub fn clear_source(&mut self, source: &str) {
        self.0.remove(source);
    }

    pub fn retain_source(&mut self, source: &str, mut keep: impl FnMut(&str) -> bool) {
        if let Some(targets) = self.0.get_mut(source) {
            targets.retain(|target| keep(target));
            if targets.is_empty() {
                self.0.remove(source);
            }
        }
    }

    pub fn remove_actor(&mut self, actor: &str) {
        self.0.remove(actor);
        self.retain_targets(|target| target != actor);
    }

    pub fn retain_targets(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.0.retain(|_, targets| {
            targets.retain(|target| keep(target));
            !targets.is_empty()
        });
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().flat_map(|(source, targets)| {
            targets
                .iter()
                .map(move |target| (source.as_str(), target.as_str()))
        })
    }

    pub fn len(&self) -> usize {
        self.0.values().map(BTreeSet::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let valid = |id: &str| {
            !id.trim().is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
        };
        if self.0.iter().any(|(source, targets)| {
            !valid(source) || targets.is_empty() || targets.iter().any(|target| !valid(target))
        }) {
            return Err("hail history requires bounded source and target identities and nonempty source rows");
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "comms_history_tests.rs"]
mod tests;
