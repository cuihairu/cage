//! Local registry CLI: publish, list (local/remote), export/import,
//! push/pull, verify, gc, and key management. Split out of the CLI entry.

use std::fs;
use std::path::{Path, PathBuf};

use cage_core::error::codes::distribution::E2101;
use cage_core::error::codes::registry::E1802;
use cage_core::manifest::ProjectConfig;

use crate::remote;
use crate::{build_project, load_project_config, pack_snapshot, BuildFailure};

/// Resolve the registry root for publish/list: the `--registry` flag wins,
/// else the project's `[registry].path` (relative to the project root).
pub(crate) fn registry_root(
    path: &Path,
    config: &ProjectConfig,
    flag: Option<&Path>,
) -> Result<PathBuf, String> {
    match flag {
        Some(r) => Ok(r.to_path_buf()),
        None => match &config.registry {
            Some(reg) => Ok(path.join(&reg.path)),
            None => Err(format!(
                "{E1802} no registry root (pass --registry or set '[registry] path' in cage.toml)"
            )),
        },
    }
}

/// `cage registry publish` — build the profile, pack its self-verifying
/// snapshot and enter it into the local registry as
/// `<package>/<version>`. Package defaults to `project.name`, version to
/// `project.version`. Only a ledger-verified snapshot is written; a
/// byte-identical re-publish is a no-op, different bytes for the same
/// version are an E1801 conflict (the registry never rewrites history).
/// With `--env` the entry's packed manifest names the environment; one
/// version stays one pack — a different environment packs differently and
/// the same E1801 conflict guards it.
pub(crate) fn run_registry_publish(
    path: &Path,
    profile: &str,
    env: Option<&str>,
    package: Option<&str>,
    version: Option<&str>,
    registry_flag: Option<&Path>,
) -> i32 {
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let package = package.map_or_else(|| config.project.name.clone(), str::to_string);
    let version = if let Some(v) = version {
        v.to_string()
    } else if let Some(v) = &config.project.version {
        v.clone()
    } else {
        eprintln!(
            "error: no version to publish (pass --version or set project.version in cage.toml)"
        );
        return 2;
    };
    let reg_root = match registry_root(path, &config, registry_flag) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // R3: remote registry roots are read-only — publish stays local.
    let root_spec = registry_flag.map_or_else(
        || config.registry.as_ref().map(|r| r.path.clone()),
        |p| Some(p.to_string_lossy().into_owned()),
    );
    if let Some(spec) = &root_spec {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — registry roots are read-only over \
                 the network; publish requires a local registry path"
            );
            return 2;
        }
    }

    // A publish is a fresh full build — no incremental carry-over.
    match build_project(path, "gamerule", profile, env, false, false) {
        Ok(out) => match pack_snapshot(path, &out) {
            Ok((snap_dir, files)) => {
                match cage_core::registry::publish(&reg_root, &package, &version, &snap_dir) {
                    Ok(report) => {
                        let env_note = env.map_or_else(String::new, |e| format!(", env '{e}'"));
                        println!(
                            "cage registry: published {package}/{version} (profile '{profile}'{env_note}, {files} files, build_id {}, content_hash {}){}",
                            &out.manifest.build_id[..12],
                            &out.manifest.content_hash[..12],
                            if report.already_identical {
                                " — identical, no-op"
                            } else {
                                ""
                            }
                        );
                        0
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        1
                    }
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                2
            }
        },
        Err(BuildFailure::Validation(diags)) => {
            println!("{}", diags.render(false));
            println!(
                "cage registry: FAILED validation ({} errors)",
                diags.errors().len()
            );
            1
        }
        Err(BuildFailure::Io(msg)) => {
            eprintln!("error: {msg}");
            2
        }
        Err(BuildFailure::UpToDate { .. }) => {
            // incremental is false for registry publishes — unreachable.
            eprintln!("error: internal: registry publish build did not rebuild");
            2
        }
    }
}

/// `cage registry list` — every package with its recorded versions, in
/// deterministic (name, dotted-numeric version) order. Local roots walk
/// package directories; remote http(s) roots list the `--package` names
/// given on the command line (§49 L1).
pub(crate) fn run_registry_list(
    registry_flag: Option<&Path>,
    packages: &[String],
    auth_env: Option<&str>,
    json: bool,
    path: Option<&Path>,
) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            return run_registry_list_remote(spec, packages, auth_env, json, path);
        }
    }
    match cage_core::registry::packages(reg_root) {
        Ok(indexes) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&indexes).expect("indexes serialize")
                );
                return 0;
            }
            if indexes.is_empty() {
                println!("cage registry: empty ({})", reg_root.display());
                return 0;
            }
            println!(
                "cage registry: {} package(s) in {}",
                indexes.len(),
                reg_root.display()
            );
            print_index_listing(&indexes);
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// The text shape local and remote listings share: one line per package,
/// then its versions with the 12-char build/content fingerprints.
pub(crate) fn print_index_listing(indexes: &[cage_core::registry::RegistryIndex]) {
    for index in indexes {
        println!("{}", index.package);
        if index.entries.is_empty() {
            println!("  (no versions)");
            continue;
        }
        for entry in &index.entries {
            println!(
                "  {:<12} build {}  content {}  {} files",
                entry.version,
                &entry.build_id[..entry.build_id.len().min(12)],
                &entry.content_hash[..entry.content_hash.len().min(12)],
                entry.files
            );
        }
    }
}

/// `cage registry list --registry <http(s)>` (§49 L1): one GET per named
/// package (the read-only protocol has no package-name enumeration — that
/// much stays true from R3; naming packages is the CI's job, it knows what
/// it published). `path` anchors the shared `.cage-cache` index cache and
/// enables the S4 offline fallback; without it every listing fetches fresh.
pub(crate) fn run_registry_list_remote(
    spec: &str,
    packages: &[String],
    auth_env: Option<&str>,
    json: bool,
    path: Option<&Path>,
) -> i32 {
    if packages.is_empty() {
        eprintln!(
            "error: listing a remote root needs at least one --package NAME — the read-only \
             protocol has no package-name enumeration (local roots list every package)"
        );
        return 2;
    }
    let mut indexes = Vec::new();
    let mut any_cached = false;
    for package in packages {
        match remote::fetch_remote_index(path, spec, package, auth_env) {
            Ok((index, from_cache)) => {
                any_cached |= from_cache;
                indexes.push(index);
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&indexes).expect("indexes serialize")
        );
        return 0;
    }
    println!(
        "cage registry: {} package(s) in {spec}{}",
        indexes.len(),
        if any_cached {
            " (offline, cached index)"
        } else {
            ""
        }
    );
    print_index_listing(&indexes);
    0
}

/// `cage registry export` — pack a published entry into a deterministic tar
/// bundle (A1). The bundle is byte-reproducible from the registry alone
/// (member-name order, zeroed mtime/uid/gid) and self-verifying — the entry
/// ledger rides along, and A2's import re-checks it before anything enters
/// a registry. `--compress zstd` wraps that same tar in a single zstd frame
/// at a fixed level, so the wrapped bytes stay reproducible; import sniffs
/// the frame magic, so both container forms load identically. Local
/// registry roots only: the remote read protocol has no file enumeration,
/// so remote consumers resolve `registry:` sources instead.
pub(crate) fn run_registry_export(
    package_spec: &str,
    output: &Path,
    compress: Option<&str>,
    sign: bool,
    key_env: Option<&str>,
    registry_flag: Option<&Path>,
) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — export packs a local entry; \
                 fetch remote packages via registry: source roots instead"
            );
            return 2;
        }
    }
    // clap's value_parser already restricts --compress to the literal
    // "zstd"; the guard keeps a non-CLI caller honest anyway.
    let compression = match compress {
        None => cage_core::registry::BundleCompression::Plain,
        Some("zstd") => cage_core::registry::BundleCompression::Zstd,
        Some(other) => {
            eprintln!("error: --compress only accepts 'zstd', got '{other}'");
            return 2;
        }
    };
    let (package, version) = match package_spec.split_once('@') {
        Some((p, v)) => (p, Some(v)),
        None => (package_spec, None),
    };
    match cage_core::registry::export_bundle(reg_root, package, version, compression, output) {
        Ok(report) => {
            println!(
                "cage registry: exported {}/{} → {} ({} entry files + index excerpt{})",
                report.package,
                report.version,
                output.display(),
                report.files,
                if matches!(compression, cage_core::registry::BundleCompression::Zstd) {
                    ", zstd container"
                } else {
                    ""
                }
            );
            // A6: detached ed25519 signature over the exact bundle bytes,
            // riding beside it as `<output>.sig`. The signing key resolves
            // from the env var named by --key-env only — never from
            // config, never echoed.
            if sign {
                let Some(key_env) = key_env else {
                    eprintln!("error: --sign requires --key-env");
                    return 2;
                };
                let signed = || -> Result<(), String> {
                    let key = cage_core::registry::signing_key_from_env(key_env)?;
                    let bytes = std::fs::read(output)
                        .map_err(|e| format!("cannot read back {}: {e}", output.display()))?;
                    let sig = cage_core::registry::sign_bundle_bytes(&bytes, &key);
                    cage_core::registry::write_bundle_signature(output, &sig)?;
                    Ok(())
                };
                match signed() {
                    Ok(()) => {
                        println!(
                            "cage registry: signed bundle → {}",
                            cage_core::registry::sidecar_path(output).display()
                        );
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 1;
                    }
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `cage registry import` — enter a bundle into the local registry (A2).
/// The bundle's riding ledger gates the entry exactly like publish does;
/// refused bytes never touch the target. The container form is sniffed from
/// the file head, not the name: plain tars load unchanged and a
/// `--compress zstd` frame decompresses first, so both import identically
/// under any extension. Local registry roots only — import writes, and
/// writes over the network stay out of scope.
pub(crate) fn run_registry_import(
    file: &Path,
    verify_sig: bool,
    key_env: Option<&str>,
    dry_run: bool,
    registry_flag: Option<&Path>,
) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — import writes a local registry; \
                 publish to a local root and host it statically instead"
            );
            return 2;
        }
    }
    // A6: the signature gate runs before the ledger trust gate — a
    // bundle that cannot be attributed to the trusted key is refused
    // before anything is staged, even though the ledger would also
    // catch byte-level tampering. The trusted key comes from --key-env;
    // the riding public key is never the anchor.
    if verify_sig {
        let Some(key_env) = key_env else {
            eprintln!("error: --verify-sig requires --key-env");
            return 2;
        };
        let gate = || -> Result<(), String> {
            let trusted = cage_core::registry::verifying_key_from_env(key_env)?;
            let bytes =
                std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            let sidecar = cage_core::registry::read_bundle_signature(file)?;
            cage_core::registry::verify_bundle_bytes(&bytes, &sidecar, &trusted)
        };
        if let Err(e) = gate() {
            eprintln!("error: {e}");
            return 1;
        }
        println!("cage registry: signature verified");
    }
    match cage_core::registry::import_bundle(reg_root, file, dry_run) {
        Ok(report) => {
            println!(
                "cage registry: {}{}/{} ({} files){}",
                if report.dry_run {
                    "would import "
                } else {
                    "imported "
                },
                report.package,
                report.version,
                report.files,
                if report.already_identical {
                    " — identical, no-op"
                } else {
                    ""
                }
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `cage registry keygen` (A6): generate an ed25519 signing keypair. The
/// seed goes only to the user-named file (owner-only permissions); stdout
/// carries the public key and the usage lines — the secret never appears
/// in any output.
pub(crate) fn run_registry_keygen(output: &Path) -> i32 {
    match cage_core::registry::generate_signing_key() {
        Ok(key) => {
            let (seed, public) = cage_core::registry::key_material(&key);
            if let Err(e) = std::fs::write(output, format!("{seed}\n")) {
                eprintln!("error: cannot write {}: {e}", output.display());
                return 1;
            }
            // Best-effort owner-only permissions (unix); failure is not
            // fatal — the file sits in the user's own workspace either way.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o600));
            }
            println!("cage registry keygen: seed (secret) → {}", output.display());
            println!("cage registry keygen: public key: {public}");
            println!(
                "export CAGE_SIGNING_KEY=$(cat {}) # then: cage registry export --sign --key-env CAGE_SIGNING_KEY",
                output.display()
            );
            println!(
                "export CAGE_VERIFYING_KEY={public} # consumers: cage registry import --verify-sig --key-env CAGE_VERIFYING_KEY"
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `cage registry push` — upload a published entry from the project's local
/// registry (`[registry].path`) to a remote http(s) registry root (A3): one
/// PUT per entry file, the package index written last and merged over the
/// remote's existing entries. The bearer token resolves from the
/// environment via `--auth-env` or `[registry].auth_env` (anonymous when
/// neither is set); the value never enters logs or the config.
pub(crate) fn run_registry_push(
    path: &Path,
    package_spec: Option<&str>,
    remote_flag: Option<&str>,
    presign_map_path: Option<&Path>,
    auth_env_flag: Option<&str>,
    dry_run: bool,
) -> i32 {
    let config = match load_project_config(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let Some(registry_cfg) = &config.registry else {
        eprintln!(
            "error: {E1802} no local registry to push from (set '[registry] path' in cage.toml — \
             the local registry is the push source)"
        );
        return 2;
    };
    let source_root = path.join(&registry_cfg.path);
    // Package/version defaults mirror publish: the project's own name, at
    // the source registry's latest when no @version is given.
    let (package, version) = match package_spec {
        Some(spec) => match spec.split_once('@') {
            Some((p, v)) => (p.to_string(), Some(v.to_string())),
            None => (spec.to_string(), None),
        },
        None => (config.project.name.clone(), None),
    };

    // Presigned route (design §47 A3): the map is the only credential —
    // no bearer token is read or sent, and --registry is meaningless.
    if let Some(map_path) = presign_map_path {
        let bytes = match fs::read(map_path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "error: {E2101} cannot read presign map {}: {e}",
                    map_path.display()
                );
                return 2;
            }
        };
        let map = match cage_core::registry::parse_presign_map(&bytes) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        return match cage_core::registry::push_entry_presigned(
            &source_root,
            &map,
            &package,
            version.as_deref(),
            dry_run,
        ) {
            Ok(report) => {
                println!(
                    "cage registry: {}{}/{} → presigned targets ({}) ({} file(s){}){}",
                    if report.dry_run {
                        "would push "
                    } else {
                        "pushed "
                    },
                    report.package,
                    report.version,
                    map_path.display(),
                    report.files,
                    if report.dry_run { ", zero PUT" } else { "" },
                    if report.already_identical {
                        " — identical, no-op"
                    } else {
                        ""
                    }
                );
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        };
    }

    // Direct route.
    let Some(remote_flag) = remote_flag else {
        eprintln!(
            "error: push needs a destination: --registry <http(s) root> or --presign-map <file>"
        );
        return 2;
    };
    // Flag overrides the config declaration; neither set pushes anonymously.
    let auth_env = auth_env_flag.or(registry_cfg.auth_env.as_deref());
    if !remote::is_remote_root(remote_flag) {
        eprintln!(
            "error: push targets a remote http(s) registry root, got '{remote_flag}' — local \
             destinations belong to 'cage registry publish'"
        );
        return 2;
    }
    match cage_core::registry::push_entry(
        &source_root,
        remote_flag,
        &package,
        version.as_deref(),
        auth_env,
        dry_run,
    ) {
        Ok(report) => {
            println!(
                "cage registry: {}{}/{} → {} ({} file(s){}){}",
                if report.dry_run {
                    "would push "
                } else {
                    "pushed "
                },
                report.package,
                report.version,
                remote_flag,
                report.files,
                if report.dry_run { ", zero PUT" } else { "" },
                if report.already_identical {
                    " — identical, no-op"
                } else {
                    ""
                }
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `cage registry verify` — the full-registry audit (R4): every recorded
/// entry exists and self-verifies (its bytes re-hashed against its ledger,
/// the index record matching the ledger's `build_id`/`content_hash`), and no
/// entry directory sits on disk without an index record. Read-only; a
/// finding is an E1803 and fails the command.
pub(crate) fn run_registry_verify(registry_flag: Option<&Path>) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — registry roots are read-only over \
                 the network; verify requires a local registry path"
            );
            return 2;
        }
    }
    match cage_core::registry::verify_registry(reg_root) {
        Ok(report) => {
            if report.ok() {
                println!(
                    "cage registry: OK — {} package(s), {} entry/ies verified in {}",
                    report.packages,
                    report.entries_checked,
                    reg_root.display()
                );
                0
            } else {
                println!(
                    "cage registry: {} problem(s) across {} package(s) in {}",
                    report.problems.len(),
                    report.packages,
                    reg_root.display()
                );
                for problem in &report.problems {
                    println!("  {problem}");
                }
                2
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// `cage registry gc [--keep N] [--dry-run]` — per package keep the newest
/// `keep` versions (the rollback window; consumers pin old @versions and
/// keep resolving), delete the older entries plus orphaned directories,
/// rewrite the touched indexes. --dry-run reports the identical removal
/// list without touching anything.
pub(crate) fn run_registry_gc(keep: usize, dry_run: bool, registry_flag: Option<&Path>) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — registry roots are read-only over \
                 the network; gc requires a local registry path"
            );
            return 2;
        }
    }
    match cage_core::registry::gc_registry(reg_root, keep, dry_run) {
        Ok(report) => {
            if report.removed.is_empty() {
                println!("cage registry: nothing to collect (keep {keep})");
            } else {
                println!(
                    "cage registry: {} to remove, {} index(es) rewritten{}",
                    report.removed.len(),
                    report.rewritten,
                    if dry_run {
                        " (dry run — nothing touched)"
                    } else {
                        ""
                    }
                );
                for removed in &report.removed {
                    println!("  {removed}");
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

/// `cage registry remove <package> <version> [--dry-run]` — explicitly
/// remove one entry (version directory + index record). The package index
/// stays even when empty, so the same snapshot can be re-published
/// afterwards; removing an unrecorded version is E1802.
pub(crate) fn run_registry_remove(
    package: &str,
    version: &str,
    dry_run: bool,
    registry_flag: Option<&Path>,
) -> i32 {
    let Some(reg_root) = registry_flag else {
        eprintln!("error: no registry root (pass --registry)");
        return 2;
    };
    if let Some(spec) = reg_root.to_str() {
        if remote::is_remote_root(spec) {
            eprintln!(
                "error: registry root '{spec}' is remote — registry roots are read-only over \
                 the network; remove requires a local registry path"
            );
            return 2;
        }
    }
    match cage_core::registry::remove_entry(reg_root, package, version, dry_run) {
        Ok(()) => {
            if dry_run {
                println!(
                    "cage registry: would remove {package}/{version} from {}",
                    reg_root.display()
                );
            } else {
                println!(
                    "cage registry: removed {package}/{version} (registry {})",
                    reg_root.display()
                );
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}
