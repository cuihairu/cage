//! Lua identifier rules: the reserved-word list, schema-name sanitizing
//! and the unique-name allocator (split out of lib.rs, LAY-022).

use std::collections::HashSet;

/// Lua keywords that get a trailing `_` (Lua has no `@`-style escape).
pub(crate) const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Sanitize a schema name into a valid identifier (ASCII alnum + `_`,
/// never starting with a digit).
pub(crate) fn sanitize_ident(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// Identifier with a trailing `_` for Lua keywords (no escape syntax exists).
pub(crate) fn lua_ident(name: &str) -> String {
    let s = sanitize_ident(name);
    if LUA_KEYWORDS.contains(&s.as_str()) {
        format!("{s}_")
    } else {
        s
    }
}

/// First-unique wins: colliding identifiers get `_` appended until free
/// (deterministic because callers iterate in name order).
pub(crate) fn unique_ident(base: String, used: &mut HashSet<String>) -> String {
    let mut candidate = base;
    while !used.insert(candidate.clone()) {
        candidate.push('_');
    }
    candidate
}
