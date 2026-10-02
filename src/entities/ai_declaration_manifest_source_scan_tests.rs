//! Reading the crate's own source, so the declared tables are re-derived
//! rather than trusted. Same technique as
//! `crate::entities::ai_flag_hosts::tests`.

use std::path::PathBuf;

pub fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
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
    let src = std::fs::read_to_string(&path)
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

/// The source a spawn-site attachment scan should search for `func` in
/// `file`.
///
/// Normally the body of `fn func`. Two decomposed spawn paths are the
/// exceptions:
///
/// * `entities/spawner.rs` / `spawn_entity` (since #1238): `spawn_entity` no
///   longer inserts sections itself — it walks `SPAWN_SECTIONS`, and the
///   `SpawnSection` impls beside it (in the same module) hold the actual
///   `insert`s. That module is wholly a spawn path, so the whole non-test
///   module IS the source; narrowing to `fn spawn_entity`'s body would see
///   only the dispatch loop and miss every attachment.
/// * `server_app/world_setup.rs` / `spawn_game_start_entities` (since #1200):
///   the player game-start spawn delegates to `configure_player_ship` and the
///   `insert_player_*` builders beside it. That module ALSO holds world-setup
///   code that is not part of this path, so the source is those functions
///   gathered by name — see [`player_game_start_spawn_path`] — rather than the
///   whole module.
pub fn spawn_site_source(file: &str, func: &str) -> String {
    let src = read_non_test_source(file);
    if file.ends_with("entities/spawner.rs") && func == "spawn_entity" {
        src
    } else if file.ends_with("server_app/world_setup.rs") && func == "spawn_game_start_entities" {
        player_game_start_spawn_path(&src)
    } else {
        function_body(&src, func).to_string()
    }
}

/// The player game-start attachment path: the body of
/// `spawn_game_start_entities` concatenated with the bodies of
/// `configure_player_ship` and every `insert_player_*` builder it fans out to
/// (issue #1200's decomposition). A function joins the path by name, so a new
/// `insert_player_*` builder is covered automatically and no non-spawn
/// function in the module can mask a missing attachment.
pub fn player_game_start_spawn_path(src: &str) -> String {
    let mut out = String::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find("fn ") {
        let name_start = from + rel + "fn ".len();
        let name_end = src[name_start..]
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map(|i| name_start + i)
            .unwrap_or(src.len());
        let name = &src[name_start..name_end];
        let on_path = (name == "spawn_game_start_entities"
            || name == "configure_player_ship"
            || name.starts_with("insert_player_"))
            && src[name_end..].starts_with('(');
        if on_path {
            out.push_str(function_body(src, name));
            out.push('\n');
        }
        from = name_end;
    }
    out
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
