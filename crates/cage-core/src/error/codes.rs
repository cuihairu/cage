//! Error code definitions for Cage diagnostics.
//! Each error code follows the pattern: E{category}{number}
//! Categories:
//! - 0xxx: Parse/Syntax errors (L0)
//! - 1xxx: Schema/Type/Value/Table errors (L1-L4)
//! - 4xxx: Reference errors (L5)
//! - 5xxx: Semantic errors (L6)
//! - 6xxx: Game Rule errors (L7)
//! - 9xxx: Internal/System errors

/// Parse/Syntax errors (L0)
pub mod parse {
    /// E0001 - Generic syntax error in source file
    pub const E0001: &str = "E0001";
    /// E0002 - Unexpected EOF
    pub const E0002: &str = "E0002";
    /// E0003 - Invalid character encoding
    pub const E0003: &str = "E0003";
    /// E0004 - Malformed structure (e.g., YAML mapping error)
    pub const E0004: &str = "E0004";
}

/// Schema validation errors (L1)
pub mod schema {
    /// E1001 - Missing required field
    pub const E1001: &str = "E1001";
    /// E1002 - Unknown field not defined in schema
    pub const E1002: &str = "E1002";
    /// E1003 - Duplicate field definition in schema
    pub const E1003: &str = "E1003";
    /// E1004 - Invalid schema definition (e.g., circular reference in schema)
    pub const E1004: &str = "E1004";
    /// E1005 - Schema file not found or unreadable
    pub const E1005: &str = "E1005";
}

/// Type validation errors (L2)
pub mod type_val {
    /// E1101 - Type mismatch (expected vs actual)
    pub const E1101: &str = "E1101";
    /// E1102 - Cannot coerce value to target type
    pub const E1102: &str = "E1102";
    /// E1103 - Overflow/underflow for numeric type
    pub const E1103: &str = "E1103";
    /// E1104 - Invalid enum variant
    pub const E1104: &str = "E1104";
}

/// Value constraint errors (L3)
pub mod value {
    /// E1201 - Value out of range (min/max)
    pub const E1201: &str = "E1201";
    /// E1202 - String length out of bounds (`min_length/max_length`)
    pub const E1202: &str = "E1202";
    /// E1203 - Value does not match regex pattern
    pub const E1203: &str = "E1203";
    /// E1204 - Value not in allowed enum values
    pub const E1204: &str = "E1204";
    /// E1205 - Array length out of bounds
    pub const E1205: &str = "E1205";
}

/// Table-level validation errors (L4)
pub mod table {
    /// E1301 - Duplicate primary key
    pub const E1301: &str = "E1301";
    /// E1302 - Duplicate composite unique key
    pub const E1302: &str = "E1302";
    /// E1303 - Required row missing (e.g., referenced ID not present)
    pub const E1303: &str = "E1303";
    /// E1304 - Row ordering violation
    pub const E1304: &str = "E1304";
    /// E1305 - Empty table where data required
    pub const E1305: &str = "E1305";
}

/// Reference validation errors (L5)
pub mod reference {
    /// E1401 - Referenced target does not exist
    pub const E1401: &str = "E1401";
    /// E1402 - Reference points to deleted/removed entity
    pub const E1402: &str = "E1402";
    /// E1403 - Circular reference detected
    pub const E1403: &str = "E1403";
    /// E1404 - Reference cardinality violation (e.g., one-to-many exceeded)
    pub const E1404: &str = "E1404";
    /// E1410 - Referenced object exists but fails semantic predicate (type/compatibility)
    pub const E1410: &str = "E1410";
    /// E1411 - Referenced object field constraint violation
    pub const E1411: &str = "E1411";
}

/// Semantic validation errors (L6)
pub mod semantic {
    /// E1501 - Assertion expression evaluated to false
    pub const E1501: &str = "E1501";
    /// E1502 - Cross-field constraint violation
    pub const E1502: &str = "E1502";
    /// E1503 - Expression evaluation error (division by zero, etc.)
    pub const E1503: &str = "E1503";
}

/// Game Rule validation errors (L7)
pub mod gamerule {
    /// E1601 - Custom validator plugin returned error
    pub const E1601: &str = "E1601";
    /// E1602 - Validator plugin not found
    pub const E1602: &str = "E1602";
    /// E1603 - Validator plugin execution failed (panic, timeout, etc.)
    pub const E1603: &str = "E1603";
}

/// Editor interchange errors (Web UI / Schema Editor, third phase)
pub mod editor {
    /// E1701 - Editor document does not parse into a Schema (malformed
    /// editor JSON / canonical YAML; the editor interchange contract lives
    /// in `cage_core::edit`)
    pub const E1701: &str = "E1701";
}

/// Registry errors (Configuration Registry, third phase)
pub mod registry {
    /// E1801 - Registry publish version conflict (same version republished
    /// with different bytes) or invalid package/version name; nothing is
    /// written on any of these paths
    pub const E1801: &str = "E1801";
    /// E1802 - Registry reference cannot be resolved (invalid spec, package
    /// or version not found, corrupt or missing index)
    pub const E1802: &str = "E1802";
    /// E1803 - Registry entry failed ledger verification (tampered /
    /// missing files) — refused at publish and at resolve
    pub const E1803: &str = "E1803";
}

/// Remote Source errors (S series, design §45): read-only fetch of
/// Google Sheets / MySQL / PostgreSQL / HTTP API sources. All six are
/// wired: E1901/E1902 at the fetch stage of every adapter, E1903 in the
/// Sheets shape gate, E1904 in credential resolution, E1905 in the DB
/// query whitelist, E1906 as the offline-fallback warning.
pub mod remote {
    /// E1901 - Remote source fetch failed: network / DNS / timeout after
    /// bounded retries, or a non-auth HTTP error status
    pub const E1901: &str = "E1901";
    /// E1902 - Remote source authentication / authorization rejected
    /// (HTTP 401 / 403)
    pub const E1902: &str = "E1902";
    /// E1903 - Remote source response shape invalid (not a row set /
    /// missing header) — reserved until a source adapter needs it
    pub const E1903: &str = "E1903";
    /// E1904 - Remote source credential missing (configured env var
    /// unset, credential file unreadable) — reserved until the DB /
    /// Sheets sources land
    pub const E1904: &str = "E1904";
    /// E1905 - Remote source query invalid (non-SELECT / multi-statement
    /// named query)
    pub const E1905: &str = "E1905";
    /// E1906 - Remote source unreachable but a previous cached copy was
    /// served instead (offline fallback warning — not fatal; `--no-cache`
    /// disables the fallback and turns it back into a hard E1901)
    pub const E1906: &str = "E1906";
}

/// Build/Transform errors (Target generation)
pub mod build {
    /// E9001 - Target generator not found for format
    pub const E9001: &str = "E9001";
    /// E9002 - Target generation failed
    pub const E9002: &str = "E9002";
    /// E9003 - Deterministic build violation (non-reproducible output)
    pub const E9003: &str = "E9003";
    /// E9004 - Manifest generation failed
    pub const E9004: &str = "E9004";
    /// E9005 - Profile not found
    pub const E9005: &str = "E9005";
    /// E9006 - Field visibility conflict
    pub const E9006: &str = "E9006";
}

/// Internal/System errors
pub mod internal {
    /// E9901 - Internal invariant violation (bug)
    pub const E9901: &str = "E9901";
    /// E9902 - I/O error during source reading
    pub const E9902: &str = "E9902";
    /// E9903 - Configuration error (invalid CLI args, missing files)
    pub const E9903: &str = "E9903";
    /// E9904 - Plugin loading failed
    pub const E9904: &str = "E9904";
}

/// Get human-readable title for an error code
pub fn error_title(code: &str) -> Option<&'static str> {
    match code {
        // Parse
        parse::E0001 => Some("Syntax Error"),
        parse::E0002 => Some("Unexpected End of Input"),
        parse::E0003 => Some("Invalid Encoding"),
        parse::E0004 => Some("Malformed Structure"),
        // Schema
        schema::E1001 => Some("Missing Required Field"),
        schema::E1002 => Some("Unknown Field"),
        schema::E1003 => Some("Duplicate Field Definition"),
        schema::E1004 => Some("Invalid Schema Definition"),
        schema::E1005 => Some("Schema Not Found"),
        // Type
        type_val::E1101 => Some("Type Mismatch"),
        type_val::E1102 => Some("Type Coercion Failed"),
        type_val::E1103 => Some("Numeric Overflow"),
        type_val::E1104 => Some("Invalid Enum Variant"),
        // Value
        value::E1201 => Some("Value Out of Range"),
        value::E1202 => Some("String Length Violation"),
        value::E1203 => Some("Pattern Mismatch"),
        value::E1204 => Some("Enum Value Not Allowed"),
        value::E1205 => Some("Array Length Violation"),
        // Table
        table::E1301 => Some("Duplicate Primary Key"),
        table::E1302 => Some("Duplicate Composite Key"),
        table::E1303 => Some("Missing Referenced Row"),
        table::E1304 => Some("Row Ordering Violation"),
        table::E1305 => Some("Empty Table"),
        // Reference
        reference::E1401 => Some("Reference Target Not Found"),
        reference::E1402 => Some("Reference To Deleted Entity"),
        reference::E1403 => Some("Circular Reference"),
        reference::E1404 => Some("Cardinality Violation"),
        reference::E1410 => Some("Reference Semantic Mismatch"),
        reference::E1411 => Some("Reference Field Constraint Violation"),
        // Semantic
        semantic::E1501 => Some("Assertion Failed"),
        semantic::E1502 => Some("Cross-Field Constraint Violation"),
        semantic::E1503 => Some("Expression Evaluation Error"),
        // Game Rule
        gamerule::E1601 => Some("Game Rule Validation Failed"),
        gamerule::E1602 => Some("Validator Plugin Not Found"),
        gamerule::E1603 => Some("Validator Plugin Execution Failed"),
        // Editor
        editor::E1701 => Some("Editor Interchange Invalid"),
        // Registry
        registry::E1801 => Some("Registry Publish Conflict"),
        registry::E1802 => Some("Registry Reference Unresolved"),
        registry::E1803 => Some("Registry Entry Verification Failed"),
        // Remote Source
        remote::E1901 => Some("Remote Source Fetch Failed"),
        remote::E1902 => Some("Remote Source Auth Rejected"),
        remote::E1903 => Some("Remote Source Response Invalid"),
        remote::E1904 => Some("Remote Source Credential Missing"),
        remote::E1905 => Some("Remote Source Query Invalid"),
        // Build
        build::E9001 => Some("Target Generator Not Found"),
        build::E9002 => Some("Target Generation Failed"),
        build::E9003 => Some("Non-Deterministic Build"),
        build::E9004 => Some("Manifest Generation Failed"),
        build::E9005 => Some("Profile Not Found"),
        build::E9006 => Some("Field Visibility Conflict"),
        // Internal
        internal::E9901 => Some("Internal Error"),
        internal::E9902 => Some("I/O Error"),
        internal::E9903 => Some("Configuration Error"),
        internal::E9904 => Some("Plugin Load Failed"),
        _ => None,
    }
}

/// Get default severity for an error code
pub fn default_severity(code: &str) -> Severity {
    // Parse errors are always errors
    if code.starts_with("E0") {
        return Severity::Error;
    }
    // Schema/Type/Value/Table/Reference/Semantic/GameRule are errors by default
    if code.starts_with("E1")
        || code.starts_with("E4")
        || code.starts_with("E5")
        || code.starts_with("E6")
    {
        return Severity::Error;
    }
    // Build errors
    if code.starts_with("E90") {
        return Severity::Error;
    }
    // Internal errors
    if code.starts_with("E99") {
        return Severity::Error;
    }
    Severity::Error
}

/// Diagnostic severity levels
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Severity {
    /// Informational, does not block build
    Info,
    /// Warning, may indicate potential issues
    Warning,
    /// Error, blocks build
    #[default]
    Error,
}

impl Severity {
    /// Uppercase label used in rendered output (INFO / WARNING / ERROR)
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARNING",
            Severity::Error => "ERROR",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_titles() {
        assert_eq!(error_title("E0001"), Some("Syntax Error"));
        assert_eq!(error_title("E1001"), Some("Missing Required Field"));
        assert_eq!(error_title("E1401"), Some("Reference Target Not Found"));
        assert_eq!(error_title("E1410"), Some("Reference Semantic Mismatch"));
        assert_eq!(error_title("E9999"), None);
    }

    #[test]
    fn test_default_severity() {
        assert_eq!(default_severity("E0001"), Severity::Error);
        assert_eq!(default_severity("E1001"), Severity::Error);
        assert_eq!(default_severity("E1401"), Severity::Error);
        assert_eq!(default_severity("E9001"), Severity::Error);
    }

    #[test]
    fn default_severity_prefix_families_and_unknown_fallback() {
        // every family prefix the function special-cases, including the
        // E99 internal branch that only runs once E0/E1/E4/E5/E6/E90 miss
        for code in [
            "E0001", "E1001", "E4001", "E5001", "E6001", "E9001", "E9901",
        ] {
            assert_eq!(default_severity(code), Severity::Error, "family {code}");
        }
        // codes outside every known prefix fall through to the same default
        assert_eq!(default_severity("X0001"), Severity::Error);
        assert_eq!(default_severity(""), Severity::Error);
    }

    #[test]
    fn error_title_resolves_every_code_family() {
        let samples = [
            ("E0001", "Syntax Error"),
            ("E1004", "Invalid Schema Definition"),
            ("E1101", "Type Mismatch"),
            ("E1201", "Value Out of Range"),
            ("E1301", "Duplicate Primary Key"),
            ("E1401", "Reference Target Not Found"),
            ("E1501", "Assertion Failed"),
            ("E1601", "Game Rule Validation Failed"),
            ("E1701", "Editor Interchange Invalid"),
            ("E1802", "Registry Reference Unresolved"),
            ("E1901", "Remote Source Fetch Failed"),
            ("E9001", "Target Generator Not Found"),
            ("E9901", "Internal Error"),
        ];
        for (code, title) in samples {
            assert_eq!(error_title(code), Some(title), "title of {code}");
        }
        // unknown codes have no title (rendered without the em-dash suffix)
        assert_eq!(error_title("E7777"), None);
        assert_eq!(error_title(""), None);
    }

    #[test]
    fn severity_display_uses_uppercase_label() {
        assert_eq!(Severity::Info.to_string(), "INFO");
        assert_eq!(Severity::Warning.to_string(), "WARNING");
        assert_eq!(Severity::Error.to_string(), "ERROR");
        assert_eq!(format!("{}", Severity::Info), Severity::Info.as_str());
        // the default severity is the build-blocking one
        assert_eq!(Severity::default(), Severity::Error);
    }
}
