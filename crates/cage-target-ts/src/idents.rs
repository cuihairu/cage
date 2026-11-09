//! Identifier policy: the JS/TS keyword table and ident sanitizing (split out of lib.rs, LAY-010).

use std::collections::HashSet;
/// JS reserved + strict-mode reserved + TS-specific words that can appear
/// as bindings. Escape = trailing `_`, applied ONLY to binding identifiers
/// (interface/const/function/type-alias/import-alias names), never to
/// property keys — `default: string` is legal TS.
pub(crate) const TS_KEYWORDS: &[&str] = &[
    // JS reserved words
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    // strict-mode reserved
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
    "yield",
    "await",
    // TS reserved/contextual usable as bindings
    "declare",
    "namespace",
    "module",
    "type",
    "abstract",
    "readonly",
    "satisfies",
    "keyof",
    "infer",
    "is",
    "asserts",
    "unique",
    "override",
    "any",
    "boolean",
    "number",
    "string",
    "symbol",
    "unknown",
    "never",
    "object",
    "undefined",
    "bigint",
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

/// Identifier with a trailing `_` for JS/TS keywords (no escape syntax).
pub(crate) fn ts_ident(name: &str) -> String {
    let s = sanitize_ident(name);
    if TS_KEYWORDS.contains(&s.as_str()) {
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
