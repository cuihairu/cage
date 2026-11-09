//! Signed bundles (A6, design §47): detached ed25519 signatures over the
//! bundle file bytes, riding as `<bundle>.sig` sidecars. Split out of the
//! registry module (LAY-002).

use super::{fs, Deserialize, PathBuf, Serialize, E2106, E2107};
use std::path::Path;

// ---------------------------------------------------------------------------
// Signed bundles (A6, design §47 留待实现期 → delivered): a detached ed25519
// signature over the bundle **file bytes**. The blake3 ledger is the
// integrity anchor (the importer re-verifies it either way); the signature
// adds non-repudiation — a consumer can prove which key produced a bundle.
// The signature rides beside the bundle as `<bundle>.sig` (JSON); the trust
// anchor is always the verifying key the consumer brings out of band
// (`--key-env`), never the riding public key. Keys only ever travel as
// env-var bytes — never into cage.toml, never into logs.
// ---------------------------------------------------------------------------

/// Sidecar signature file name: `<bundle>.sig`.
pub const BUNDLE_SIG_SUFFIX: &str = ".sig";

/// The `algorithm` value written into every [`BundleSignature`]; a verifier
/// refuses any other value instead of guessing.
pub const BUNDLE_SIG_ALGORITHM: &str = "ed25519";

/// Detached ed25519 signature riding beside a bundle as `<bundle>.sig`
/// (JSON). The signature covers the bundle's exact file bytes (plain tar
/// or zstd frame alike), so tampering anywhere in the file is caught; the
/// public key travels along for human convenience, but is not a trust
/// anchor — verification always pins the key from the consumer's env.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleSignature {
    /// Signature scheme; only [`BUNDLE_SIG_ALGORITHM`] is accepted.
    pub algorithm: String,
    /// base64 (std) of the signing key's 32-byte public half.
    pub public_key: String,
    /// base64 (std) of the 64-byte detached signature.
    pub signature: String,
}

/// Resolve the signing (secret) key from an environment variable holding
/// base64 of the 32-byte Ed25519 seed. E2106 when the variable is unset,
/// empty, not base64, or not exactly 32 bytes.
pub fn signing_key_from_env(env_name: &str) -> Result<ed25519_dalek::SigningKey, String> {
    let raw = std::env::var(env_name)
        .map_err(|_| format!("{E2106} signing key env '{env_name}' is not set"))?;
    if raw.trim().is_empty() {
        return Err(format!("{E2106} signing key env '{env_name}' is empty"));
    }
    let bytes = base64_decode_32(&raw).map_err(|e| {
        format!("{E2106} signing key env '{env_name}' is not base64 of a 32-byte seed: {e}")
    })?;
    Ok(ed25519_dalek::SigningKey::from_bytes(&bytes))
}

/// Resolve the verifying (public) key from an environment variable holding
/// base64 of the 32-byte compressed Ed25519 public key. E2106 on the same
/// failure classes as [`signing_key_from_env`].
pub fn verifying_key_from_env(env_name: &str) -> Result<ed25519_dalek::VerifyingKey, String> {
    let raw = std::env::var(env_name)
        .map_err(|_| format!("{E2106} verifying key env '{env_name}' is not set"))?;
    if raw.trim().is_empty() {
        return Err(format!("{E2106} verifying key env '{env_name}' is empty"));
    }
    let bytes = base64_decode_32(&raw).map_err(|e| {
        format!("{E2106} verifying key env '{env_name}' is not base64 of a 32-byte key: {e}")
    })?;
    ed25519_dalek::VerifyingKey::from_bytes(&bytes)
        .map_err(|e| format!("{E2106} verifying key bytes rejected: {e}"))
}

/// base64 (std, no extra whitespace tolerance beyond trim) → exactly 32 bytes.
pub(crate) fn base64_decode_32(raw: &str) -> Result<[u8; 32], String> {
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(raw.trim())
        .map_err(|e| format!("bad base64: {e}"))?;
    decoded
        .try_into()
        .map_err(|v: Vec<u8>| format!("expected 32 bytes, got {}", v.len()))
}

/// base64 (std) encode helper for keys and signatures.
pub(crate) fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Generate a fresh signing key from the OS entropy source (`rand_core`'s
/// OsRng-backed generator). Only used by `cage registry keygen` — nothing
/// generates keys implicitly.
pub fn generate_signing_key() -> Result<ed25519_dalek::SigningKey, String> {
    use rand_core::OsRng;
    let mut rng = OsRng;
    Ok(ed25519_dalek::SigningKey::generate(&mut rng))
}

/// The two base64 strings a key is carried as: (seed, public key). The
/// seed is the secret — it only ever goes to the `-o` file the user named,
/// never to stdout.
pub fn key_material(key: &ed25519_dalek::SigningKey) -> (String, String) {
    (b64(&key.to_bytes()), b64(&key.verifying_key().to_bytes()))
}

/// Sign the exact bundle bytes with the given key.
pub fn sign_bundle_bytes(bundle: &[u8], key: &ed25519_dalek::SigningKey) -> BundleSignature {
    use ed25519_dalek::Signer as _;
    let sig = key.sign(bundle);
    BundleSignature {
        algorithm: BUNDLE_SIG_ALGORITHM.to_string(),
        public_key: b64(&key.verifying_key().to_bytes()),
        signature: b64(&sig.to_bytes()),
    }
}

/// Verify the detached signature against the bundle bytes, pinning the
/// trust anchor to `trusted`. E2107 when the sidecar's algorithm is
/// unknown, its base64 does not decode to a 64-byte signature, or the
/// signature does not verify under the trusted key.
pub fn verify_bundle_bytes(
    bundle: &[u8],
    sidecar: &BundleSignature,
    trusted: &ed25519_dalek::VerifyingKey,
) -> Result<(), String> {
    use base64::Engine as _;
    use ed25519_dalek::Verifier as _;
    if sidecar.algorithm != BUNDLE_SIG_ALGORITHM {
        return Err(format!(
            "{E2107} signature algorithm '{}' is not '{BUNDLE_SIG_ALGORITHM}'",
            sidecar.algorithm
        ));
    }
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(sidecar.signature.trim())
        .map_err(|e| format!("{E2107} signature is not base64: {e}"))?;
    let sig: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("{E2107} signature is {} bytes, expected 64", v.len()))?;
    trusted
        .verify(bundle, &ed25519_dalek::Signature::from_bytes(&sig))
        .map_err(|_| {
            "{E2107} bundle signature does not verify under the trusted key — the bundle \
             was signed by a different key or its bytes were altered"
                .to_string()
        })
}

/// Read and parse a sidecar `<bundle>.sig` file. E2107 when absent or
/// malformed JSON (a present-but-unreadable signature is a verification
/// failure, not a transport error — the bundle cannot be trusted).
pub fn read_bundle_signature(bundle_path: &Path) -> Result<BundleSignature, String> {
    let sig_path = sidecar_path(bundle_path);
    let raw = fs::read(&sig_path).map_err(|_| {
        format!(
            "{E2107} signature sidecar {} is missing — cannot verify the bundle",
            sig_path.display()
        )
    })?;
    serde_json::from_slice(&raw).map_err(|e| {
        format!(
            "{E2107} signature sidecar {} is malformed: {e}",
            sig_path.display()
        )
    })
}

/// Write the sidecar signature file next to the bundle.
pub fn write_bundle_signature(
    bundle_path: &Path,
    sig: &BundleSignature,
) -> Result<PathBuf, String> {
    let sig_path = sidecar_path(bundle_path);
    let json = serde_json::to_vec_pretty(sig)
        .map_err(|e| format!("{E2106} cannot encode signature: {e}"))?;
    fs::write(&sig_path, json)
        .map_err(|e| format!("{E2106} cannot write {}: {e}", sig_path.display()))?;
    Ok(sig_path)
}

/// The sidecar path for a bundle file: `<bundle>.sig`.
pub fn sidecar_path(bundle_path: &Path) -> PathBuf {
    let mut name = bundle_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(BUNDLE_SIG_SUFFIX);
    bundle_path.with_file_name(name)
}
