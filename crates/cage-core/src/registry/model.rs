//! Registry data model: index entries, indexes, and publish/export/import
//! reports. Split out of the registry module (LAY-002).

use serde::{Deserialize, Serialize};

/// One published version of a package, as recorded in `index.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexEntry {
    /// Published version (dotted-numeric order used for sorting)
    pub version: String,
    /// Deterministic build fingerprint of the entry (from its ledger)
    pub build_id: String,
    /// Deterministic content fingerprint of the entry (from its ledger)
    pub content_hash: String,
    /// Number of files the ledger covers
    pub files: usize,
}

/// Per-package registry index. JSON is written pretty-printed, entries
/// sorted by dotted-numeric version order — byte-deterministic.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RegistryIndex {
    /// Package name this file indexes
    pub package: String,
    /// Published versions, ascending
    pub entries: Vec<IndexEntry>,
}

/// Result of `publish`: what changed in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    /// Destination entry directory (registry-relative from publish root)
    pub entry_path: String,
    /// `true` when a byte-identical version already existed and nothing
    /// was rewritten (idempotent re-publish), `false` when newly written
    pub already_identical: bool,
}

/// Container format of an exported bundle (A1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleCompression {
    /// The deterministic tar bytes, unwrapped — the default and the only
    /// format `export_bundle` wrote before compression landed.
    Plain,
    /// The deterministic tar wrapped in a single zstd frame at the fixed
    /// level [`super::BUNDLE_ZSTD_LEVEL`]. `import_bundle` sniffs the frame magic,
    /// so both forms load identically; the trust ledger hashes uncompressed
    /// content, so a wrapped bundle verifies exactly like its plain form.
    Zstd,
}

/// Result of `export_bundle`: what was packed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// Package the bundle was exported from
    pub package: String,
    /// Entry version packed (explicitly requested, or dotted-numeric latest
    /// when the version was omitted)
    pub version: String,
    /// Entry files packed (the bundle also carries the package index excerpt)
    pub files: usize,
}

/// Result of `import_bundle`: what entered (or would enter) the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// Package imported from the bundle's index excerpt
    pub package: String,
    /// Entry version imported
    pub version: String,
    /// Entry files staged and ledger-verified
    pub files: usize,
    /// `true` when the exact bytes were already in the registry (idempotent
    /// re-import, nothing rewritten), `false` when newly written
    pub already_identical: bool,
    /// `true` when the import ran as a report-only dry run (nothing written)
    pub dry_run: bool,
}
