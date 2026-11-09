//! Identifier policy: Java keyword tables (hard keywords get a trailing `_`, restricted ones map to `__`) and ident sanitizing (split out of lib.rs, LAY-008).

use std::collections::HashSet;

/// Java hard keywords (JLS §3.9) that get a trailing `_` (Java has no
/// verbatim-identifier escape like C#'s `@`).
pub(crate) const JAVA_KEYWORDS_HARD: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "void",
    "volatile",
    "while",
];

/// Restricted identifiers that cannot be used as type names (`var`, `record`,
/// `yield`, …), the three literals, the module-system contextual keywords,
/// and `_` — illegal as an identifier since Java 9 (mapped to `__`).
pub(crate) const JAVA_KEYWORDS_RESTRICTED: &[&str] = &[
    "_",
    "exports",
    "false",
    "module",
    "null",
    "open",
    "opens",
    "permits",
    "provides",
    "record",
    "requires",
    "sealed",
    "to",
    "transitive",
    "true",
    "uses",
    "var",
    "with",
    "yield",
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

/// Identifier with a trailing `_` for Java keywords (Java has no verbatim
/// escape). Restricted identifiers (`var`, `record`, `yield`, …), the
/// literals, module keywords and the lone `_` (illegal since Java 9, which
/// maps to `__`) are all covered by the same rule.
pub(crate) fn java_ident(name: &str) -> String {
    let s = sanitize_ident(name);
    if JAVA_KEYWORDS_HARD.contains(&s.as_str()) || JAVA_KEYWORDS_RESTRICTED.contains(&s.as_str()) {
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
