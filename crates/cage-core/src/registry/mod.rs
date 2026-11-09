//! Configuration Registry（R1）: local registry + source resolution.
//!
//! A registry is a root directory versioning published configuration
//! packages (design §29). Each published entry is a **self-verifying
//! snapshot** — the exact `cage snapshot` payload (manifest.json /
//! schema.json / data/… / generated/… / HASHES.json ledger) — so publish
//! and resolve both gate on ledger verification: nothing enters the
//! registry unverified, nothing leaves it unverified.
//!
//! Layout (all bytes deterministic — same inputs → same entry → same index):
//!
//! ```text
//! <root>/
//!   <package>/                       # package name: [A-Za-z0-9._-]+
//!     index.json                     # deterministic index (sorted versions)
//!     <version>/                     # version: [A-Za-z0-9._-]+, no path segs
//!       manifest.json schema.json HASHES.json data/… generated/…
//! ```
//!
//! Version ordering is dotted-numeric (1.9 < 1.10); a resolve spec without a
//! version picks the highest. Consumers declare the registry root in their
//! project config (`[registry] path = "…"`) and reference entries as source
//! roots with the `registry:<package>[@<version>]` syntax — resolution
//! verifies the entry's ledger, then yields its `data/` directory as the
//! source root.

use crate::error::codes::distribution::{E2101, E2102, E2103, E2104, E2105, E2106, E2107};
use crate::error::codes::registry::{E1801, E1802, E1803};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

mod model;
mod push;
mod signing;
mod store;
#[cfg(test)]
mod tests;
mod verify;
mod version;

pub use model::{
    BundleCompression, ExportReport, ImportReport, IndexEntry, PublishReport, RegistryIndex,
};
pub use push::{parse_presign_map, push_entry, push_entry_presigned, PresignMap, PushReport};
pub use signing::{
    generate_signing_key, key_material, read_bundle_signature, sidecar_path, sign_bundle_bytes,
    signing_key_from_env, verify_bundle_bytes, verifying_key_from_env, write_bundle_signature,
    BundleSignature, BUNDLE_SIG_ALGORITHM, BUNDLE_SIG_SUFFIX,
};
pub use store::{
    cache_key, export_bundle, import_bundle, packages, publish, resolve, resolve_pinned,
};
pub use verify::{gc_registry, remove_entry, verify_registry, GcReport, RegistryReport};
pub use version::{
    parse_spec, parse_version_req, satisfies, select_version, CmpOp, Comparator, VersionReq,
};

pub(crate) use store::{entry_record, read_index, resolve_entry, write_index};
pub(crate) use version::{latest_version, version_key};

/// Ledger file name inside a published entry (the snapshot trust root).
const LEDGER_FILE: &str = "HASHES.json";
/// Registry index file name inside a package directory.
const INDEX_FILE: &str = "index.json";

/// Fixed zstd compression level for `--compress zstd` bundles (A1).
///
/// Level 19 — the top of zstd's standard (documented 1–19) range. The
/// compression-time cost is paid once per export, while bundles are small
/// configuration artifacts where the ratio wins on every wire/store hop —
/// the same trade as the release profile's `opt-level = "z"`. The ultra
/// levels above 19 cost extreme time and window memory for marginal gain,
/// so they stay out of the contract. The level is pinned in code (not
/// user-selectable) so a given input always compresses to identical bytes
/// within a pinned zstd library version, matching the bundle's determinism
/// contract.
pub const BUNDLE_ZSTD_LEVEL: i32 = 19;

/// The 4-byte zstd frame magic `0x28 0xB5 0x2F 0xFD` (little-endian
/// `0xFD2FB528`), sniffed at the head of an import file to tell a
/// zstd-wrapped bundle from a plain tar.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Valid package/version character set: alphanumerics, dot, dash, underscore,
/// with at least one alphanumeric character. `..`, `/`, `\`, and a lone `.`
/// are rejected so names can never escape the registry root or reduce to a
/// path no-op.
fn valid_component(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && name.chars().any(|c| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}
