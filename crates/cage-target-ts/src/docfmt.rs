//! JSDoc-shaped member model and comment-scanning safety (split out of lib.rs, LAY-010).

use cage_core::schema::FieldType;
/// How a resolved enum type is spelled inside one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnumStyle {
    /// Type-only import binding (TS mode and `.d.ts` files), possibly
    /// aliased when it collides with a local declaration.
    Local,
    /// `JSDoc` inline `import("…").X` type (JS mode table files).
    InlineImport,
}

/// One interface/`JSDoc` property, in schema name order.
pub(crate) struct Member<'a> {
    /// Property key: sanitized + uniqued, never keyword-escaped.
    pub(crate) prop: String,
    /// Field type (rendered per file style).
    pub(crate) ft: &'a FieldType,
    /// Constraint summary comment, when there is anything to say.
    pub(crate) doc: Option<String>,
    /// Whether the field must be present.
    pub(crate) required: bool,
    /// Rendered default literal, when the default maps to one.
    pub(crate) default: Option<String>,
}

impl Member<'_> {
    /// Optional members carry no renderable default (see the optionality
    /// group rule); in `JSDoc` they get `[name]` brackets.
    pub(crate) fn is_optional(&self) -> bool {
        !self.required && self.default.is_none()
    }

    /// `JSDoc` property name: optional members get `[name]` brackets.
    pub(crate) fn jsdoc_name(&self) -> String {
        if self.is_optional() {
            format!("[{}]", self.prop)
        } else {
            self.prop.clone()
        }
    }
}

/// Whether a `@property` description may follow `[name] ` on the same
/// line. The `JSDoc` scanner re-lexes what comes after the closing bracket,
/// so the description must start with a cleanly-scannable token: quotes,
/// backslashes and backticks open unterminated literals, non-ASCII
/// punctuation is an “Invalid character” (TS1127), and a number glued to
/// an identifier (`1st`) is TS1351. Anything else moves the doc to its
/// own continuation line — same text, still a valid comment.
pub(crate) fn jsdoc_desc_safe(doc: &str) -> bool {
    let Some(first) = doc.chars().next() else {
        return true;
    };
    match first {
        '"' | '\\' | '`' => false,
        c if c.is_ascii_digit() => digit_leading_word_ok(doc),
        c if c.is_ascii() => c.is_ascii_graphic(),
        c => c.is_alphabetic(),
    }
}

/// For a digit-leading description, the leading numeric literal must not
/// be glued to an identifier (`1st` → TS1351 after `[name] `).
pub(crate) fn digit_leading_word_ok(doc: &str) -> bool {
    let word = doc.split_whitespace().next().unwrap_or("");
    let mut rest = word;
    if let Some(hex) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
        if hex.is_empty() {
            return false;
        }
        let end = hex
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(hex.len());
        rest = &hex[end..];
    } else {
        let end = word
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(word.len());
        rest = &word[end..];
        if let Some(exp) = rest.strip_prefix(['e', 'E']) {
            let digits = exp.strip_prefix(['+', '-']).unwrap_or(exp);
            if digits.starts_with(|c: char| c.is_ascii_digit()) {
                let end = digits
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(digits.len());
                rest = &digits[end..];
            }
        }
    }
    // An identifier character right after the number is glued to it.
    !rest.starts_with(|c: char| c == '_' || c == '$' || c.is_alphabetic())
}
