//! Identifier policy: the Go keyword table and ident sanitizing (split out of lib.rs, LAY-009).

use std::collections::HashSet;

/// Go keywords (all lowercase). Only the `package` clause value needs the
/// trailing-`_` escape: exported Pascal-case identifiers can never collide
/// with an all-lowercase keyword.
pub(crate) const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];

/// Sanitize a schema name into Go identifier characters (ASCII alnum + `_`).
pub(crate) fn sanitize_go_ident(name: &str) -> String {
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
    out
}

/// Exported Go identifier for a schema name: sanitize, split on `_`, then
/// Pascal-case each segment (first char uppercased, rest untouched —
/// `player_id` → `PlayerId`, `ID` → `ID`, `drop-item` → `DropItem`). The
/// result starts with an ASCII letter (`1st` → `X1st`) and is never empty
/// (`列` → `X`). Go keywords are all-lowercase, so no keyword escaping is
/// needed for exported names.
pub(crate) fn go_ident(name: &str) -> String {
    let mut out = String::new();
    for segment in sanitize_go_ident(name).split('_') {
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() {
        out.push('X');
    } else if !out.starts_with(|c: char| c.is_ascii_alphabetic()) {
        out.insert(0, 'X');
    }
    out
}

/// `package` clause value with a trailing `_` for Go keywords.
pub(crate) fn package_ident(name: &str) -> String {
    if GO_KEYWORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
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
