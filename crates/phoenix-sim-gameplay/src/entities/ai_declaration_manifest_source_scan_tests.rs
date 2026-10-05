//! Reading the crate's own source, so the declared tables are re-derived
//! rather than trusted. Same technique as
//! `crate::entities::ai_flag_hosts::tests`.

use std::path::PathBuf;

pub fn crate_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Drop everything from the file's `#[cfg(test)] mod ...` marker onwards, so
/// fixtures in unit tests never masquerade as production call sites.
pub fn strip_test_module(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start() != "#[cfg(test)]" {
            continue;
        }
        if lines
            .get(i + 1)
            .is_some_and(|next| next.trim_start().starts_with("mod "))
        {
            return lines[..i].join("\n");
        }
    }
    src.to_string()
}

pub fn read_non_test_source(rel: &str) -> String {
    let path = crate_root().join(rel);
    let src = crate::repo_fixtures::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("scanned file {rel} must be readable: {e}"));
    strip_test_module(&src)
}

/// The body of `fn <name>`, by brace counting from the signature's `{`.
pub fn function_body<'a>(src: &'a str, func: &str) -> &'a str {
    let needle = format!("fn {func}");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("no `{needle}` in the scanned source"));
    let open = start
        + src[start..]
            .find('{')
            .unwrap_or_else(|| panic!("`{needle}` has no body"));
    let mut depth = 0usize;
    for (offset, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &src[open..open + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("`{needle}` body is unbalanced");
}

/// Ship initialization is one dedicated module, including its concrete helpers.
/// Other attachment sites (such as Comms conversion) remain individual functions.
pub fn spawn_site_source(file: &str, func: &str) -> String {
    let src = read_non_test_source(file);
    if file == "crates/phoenix-simulation/src/entities/ship_spawn.rs" && func == "install" {
        src
    } else {
        function_body(&src, func).to_string()
    }
}

/// Every `default_*_ai_config` / `default_*_target_selector_config` name
/// mentioned in `src`, deduplicated.
///
/// Deliberately name-shaped rather than call-shaped: it catches a definition
/// as readily as a call, which is what
/// `every_synthesiser_definition_belongs_to_a_kind` needs.
pub fn synthesiser_names(src: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for suffix in ["_ai_config", "_target_selector_config"] {
        let mut from = 0usize;
        while let Some(hit) = src[from..].find(suffix) {
            let end = from + hit + suffix.len();
            // Walk back to the start of the identifier.
            let start = src[..from + hit]
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map(|i| i + 1)
                .unwrap_or(0);
            let name = &src[start..end];
            if name.starts_with("default_") {
                out.push(name.to_string());
            }
            from = end;
        }
    }
    out.sort();
    out.dedup();
    out
}
