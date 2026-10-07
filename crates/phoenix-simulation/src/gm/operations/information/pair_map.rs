//! Absolute observer/target edits and disappearance cleanup, independent of meaning.
use std::collections::BTreeMap;

pub(crate) fn set<V, P: PartialEq>(
    rows: &mut BTreeMap<String, BTreeMap<String, V>>,
    observer: &str,
    target: &str,
    requested: Option<P>,
    value: impl FnOnce(P) -> V,
    policy: impl FnOnce(&V) -> &P,
) -> bool {
    if rows
        .get(observer)
        .and_then(|rows| rows.get(target))
        .map(policy)
        == requested.as_ref()
    {
        return false;
    }
    if let Some(requested) = requested {
        rows.entry(observer.into())
            .or_default()
            .insert(target.into(), value(requested));
    } else if let Some(bucket) = rows.get_mut(observer) {
        bucket.remove(target);
        if bucket.is_empty() {
            rows.remove(observer);
        }
    }
    true
}

pub(crate) fn prune<V>(
    rows: &mut BTreeMap<String, BTreeMap<String, V>>,
    live: &BTreeMap<&str, bool>,
) {
    rows.retain(|observer, bucket| {
        if live.get(observer.as_str()) != Some(&true) {
            return false;
        }
        bucket.retain(|target, _| live.contains_key(target.as_str()));
        !bucket.is_empty()
    });
}

#[cfg(test)]
#[path = "pair_map_tests.rs"]
mod tests;
