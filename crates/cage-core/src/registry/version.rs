//! Version semantics: dotted-numeric ordering, requirement parsing and
//! matching (`^`, `~`, comparators), spec parsing, and version selection.
//! Split out of the registry module (LAY-002).

use super::{valid_component, RegistryIndex};
use crate::error::codes::registry::E1802;

/// One comparator of a version requirement (R2 dependency pin): `op` plus a
/// dotted version whose missing components compare as zero (`>=1.2` ≡
/// `>=1.2.0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparator {
    /// Comparison operator
    pub op: CmpOp,
    /// Dotted version the operand compares against
    pub version: String,
}

/// Comparison operators of a requirement comparator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `=`
    Eq,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `<`
    Lt,
    /// `<=`
    Le,
}

/// A parsed version requirement (R2): the AND of comparators, e.g.
/// `>=1.0, <2.0`. Syntax: comma-separated comparators; operators `=` `>`
/// `>=` `<` `<=` (a bare `1.2.3` is exact); `^` expands caret
/// (`^1.2` → `>=1.2.0, <2.0.0`; `^0.2.3` → `>=0.2.3, <0.3.0`), `~` expands
/// tilde (`~1.2` → `>=1.2.0, <1.3.0`; `~1` → `>=1.0.0, <2.0.0`). All
/// comparisons use the registry's dotted-numeric order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VersionReq {
    /// All comparators must hold
    pub comparators: Vec<Comparator>,
}

/// Parse a version requirement string (`E1802` on invalid syntax).
pub fn parse_version_req(spec: &str) -> Result<VersionReq, String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("E1802 empty version requirement".to_string());
    }
    let mut comparators = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!(
                "E1802 invalid version requirement '{spec}': empty clause"
            ));
        }
        let (op, version) = match part.chars().next() {
            Some('^') => {
                // caret: bump the left-most non-zero component
                let v = part[1..].trim();
                let base = version_parts(v)
                    .map_err(|e| format!("E1802 invalid caret version '{v}': {e}"))?;
                let (lower, upper) = caret_range(&base, spec, v)?;
                comparators.push(Comparator {
                    op: CmpOp::Ge,
                    version: lower,
                });
                comparators.push(Comparator {
                    op: CmpOp::Lt,
                    version: upper,
                });
                continue;
            }
            Some('~') => {
                // tilde: lock every component but the second-to-last given
                let v = part[1..].trim();
                let base = version_parts(v)
                    .map_err(|e| format!("E1802 invalid tilde version '{v}': {e}"))?;
                let (lower, upper) = tilde_range(&base, spec, v)?;
                comparators.push(Comparator {
                    op: CmpOp::Ge,
                    version: lower,
                });
                comparators.push(Comparator {
                    op: CmpOp::Lt,
                    version: upper,
                });
                continue;
            }
            Some('=') => (CmpOp::Eq, &part[1..]),
            Some('>') => {
                if part[1..].starts_with('=') {
                    (CmpOp::Ge, &part[2..])
                } else {
                    (CmpOp::Gt, &part[1..])
                }
            }
            Some('<') => {
                if part[1..].starts_with('=') {
                    (CmpOp::Le, &part[2..])
                } else {
                    (CmpOp::Lt, &part[1..])
                }
            }
            Some(c) if c.is_ascii_digit() => (CmpOp::Eq, part),
            _ => {
                return Err(format!(
                    "E1802 invalid version requirement '{spec}': bad comparator '{part}'"
                ))
            }
        };
        let version = version.trim();
        version_parts(version)
            .map_err(|e| format!("E1802 invalid version requirement '{spec}': {e}"))?;
        comparators.push(Comparator {
            op,
            version: version.to_string(),
        });
    }
    Ok(VersionReq { comparators })
}

/// Parse a dotted version into parts, validating each component.
fn version_parts(v: &str) -> Result<Vec<VersionComponent>, String> {
    if v.is_empty() {
        return Err("missing version".to_string());
    }
    let parts = version_key(v);
    if parts.is_empty()
        || parts
            .iter()
            .any(|p| matches!(p, VersionComponent::Label(l) if l.is_empty()))
    {
        return Err(format!("invalid version '{v}'"));
    }
    Ok(parts)
}

/// Render `parts` back to a dotted string (caret/tilde expansion output).
fn render_parts(parts: &[VersionComponent]) -> String {
    parts
        .iter()
        .map(|p| match p {
            VersionComponent::Number(n) => n.to_string(),
            VersionComponent::Label(l) => l.clone(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Zero-pad `parts` to `len` numeric components (missing = 0).
fn pad_zeros(parts: &[VersionComponent], len: usize) -> Vec<VersionComponent> {
    let mut out = parts.to_vec();
    while out.len() < len {
        out.push(VersionComponent::Number(0));
    }
    out
}

/// Caret range: bump the left-most non-zero component (all-zero base bumps
/// the last given one). `^1.2` → [1.2.0, 2.0.0); `^0.2.3` → [0.2.3, 0.3.0).
fn caret_range(
    base: &[VersionComponent],
    spec: &str,
    raw: &str,
) -> Result<(String, String), String> {
    let width = base.len();
    let mut upper = pad_zeros(base, width);
    let bump_at = base
        .iter()
        .position(|p| matches!(p, VersionComponent::Number(n) if *n > 0))
        .or_else(|| match base.last() {
            Some(VersionComponent::Number(_)) => Some(base.len() - 1),
            _ => None,
        });
    let Some(i) = bump_at else {
        return Err(format!(
            "E1802 invalid version requirement '{spec}': bad caret version '{raw}'"
        ));
    };
    let Some(VersionComponent::Number(n)) = upper.get_mut(i) else {
        return Err(format!(
            "E1802 invalid version requirement '{spec}': bad caret version '{raw}'"
        ));
    };
    *n += 1;
    // Everything after the bumped component drops to zero (^1.2 → <2.0.0,
    // not <2.2); trailing zeros are popped for a tidy upper bound ("2").
    for p in upper.iter_mut().skip(i + 1) {
        *p = VersionComponent::Number(0);
    }
    while matches!(upper.last(), Some(VersionComponent::Number(0))) && upper.len() > 1 {
        upper.pop();
    }
    Ok((render_parts(&pad_zeros(base, width)), render_parts(&upper)))
}

/// Tilde range: lock everything but the second-to-last given component
/// (`~1.2` → [1.2.0, 1.3.0); `~1` → [1.0.0, 2.0.0); `~1.2.3` →
/// [1.2.3, 1.3.0)).
fn tilde_range(
    base: &[VersionComponent],
    spec: &str,
    raw: &str,
) -> Result<(String, String), String> {
    let width = base.len().max(2);
    let lower = pad_zeros(base, width);
    let mut upper = lower.clone();
    let bump_at = usize::from(base.len() >= 2);
    let Some(VersionComponent::Number(n)) = upper.get_mut(bump_at) else {
        return Err(format!(
            "E1802 invalid version requirement '{spec}': bad tilde version '{raw}'"
        ));
    };
    *n += 1;
    // upper: zero out everything after the bumped component
    for p in upper.iter_mut().skip(bump_at + 1) {
        *p = VersionComponent::Number(0);
    }
    while matches!(upper.last(), Some(VersionComponent::Number(0))) && upper.len() > 1 {
        upper.pop();
    }
    Ok((render_parts(&lower), render_parts(&upper)))
}

/// Compare two dotted versions component-wise, treating missing trailing
/// components as zero (`1.2` ≡ `1.2.0`); labels sort after numbers.
pub(crate) fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let ka = version_key(a);
    let kb = version_key(b);
    let len = ka.len().max(kb.len());
    for i in 0..len {
        let pa = ka.get(i).cloned().unwrap_or(VersionComponent::Number(0));
        let pb = kb.get(i).cloned().unwrap_or(VersionComponent::Number(0));
        match pa.cmp(&pb) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Whether `version` satisfies every comparator of `req`.
pub fn satisfies(version: &str, req: &VersionReq) -> bool {
    use std::cmp::Ordering;
    req.comparators.iter().all(|c| {
        let ord = compare_versions(version, &c.version);
        match c.op {
            CmpOp::Eq => ord == Ordering::Equal,
            CmpOp::Gt => ord == Ordering::Greater,
            CmpOp::Ge => ord != Ordering::Less,
            CmpOp::Lt => ord == Ordering::Less,
            CmpOp::Le => ord != Ordering::Greater,
        }
    })
}

/// Dotted-numeric version component: either a number (compared
/// numerically — 1.9 < 1.10) or an arbitrary label (compared as text,
/// sorted after numbers).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum VersionComponent {
    Number(u64),
    Label(String),
}

/// Split a version into sortable components: dots/hyphens separate
/// (1.0.0-beta → [1, 0, 0, beta]). Shorter vectors sort first when all
/// shared components tie ("1.0" < "1.0.1", bare "1.0.0" < "1.0.0-beta").
pub(crate) fn version_key(version: &str) -> Vec<VersionComponent> {
    version
        .split(['.', '-'])
        .map(|part| match part.parse::<u64>() {
            Ok(n) => VersionComponent::Number(n),
            Err(_) => VersionComponent::Label(part.to_string()),
        })
        .collect()
}

/// Highest version by dotted-numeric order; ties (same numeric key, e.g.
/// "1.0" vs "1.0.0") break on the raw string for determinism.
pub(crate) fn latest_version(versions: &[String]) -> Option<&str> {
    versions
        .iter()
        .max_by(|a, b| version_key(a).cmp(&version_key(b)).then_with(|| a.cmp(b)))
        .map(String::as_str)
}

/// Parse a `registry:<package>[@<version>]` source-root spec.
/// A missing version means "latest".
pub fn parse_spec(spec: &str) -> Result<(String, Option<String>), String> {
    let rest = spec
        .strip_prefix("registry:")
        .ok_or_else(|| format!("registry spec must start with 'registry:': {spec}"))?;
    let (package, version) = match rest.split_once('@') {
        Some((p, v)) => (p, Some(v.to_string())),
        None => (rest, None),
    };
    if !valid_component(package) {
        return Err(format!(
            "{E1802} invalid registry package name '{package}' (allowed: letters, digits, '.', '-', '_')"
        ));
    }
    if let Some(v) = &version {
        if !valid_component(v) {
            return Err(format!(
                "{E1802} invalid registry version '{v}' (allowed: letters, digits, '.', '-', '_')"
            ));
        }
    }
    Ok((package.to_string(), version))
}

/// Render a requirement back to a compact comparator list (diagnostics).
pub(crate) fn render_req(req: &VersionReq) -> String {
    req.comparators
        .iter()
        .map(|c| {
            let op = match c.op {
                CmpOp::Eq => "=",
                CmpOp::Gt => ">",
                CmpOp::Ge => ">=",
                CmpOp::Lt => "<",
                CmpOp::Le => "<=",
            };
            format!("{op}{}", c.version)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Pick the entry version for a resolved index: an explicit `version` must
/// exist and satisfy `requirement`; with only a requirement, the highest
/// published version satisfying it wins; with neither, the latest
/// (dotted-numeric order). `registry_display` appears in the error text
/// (local roots show the path, remote roots the URL). Pure — shared by the
/// local resolver and the remote (R3) fetcher.
pub fn select_version(
    index: &RegistryIndex,
    version: Option<&str>,
    requirement: Option<&VersionReq>,
    registry_display: &str,
) -> Result<String, String> {
    let package = &index.package;
    if index.entries.is_empty() {
        return Err(format!(
            "{E1802} registry package not found: {package} (registry {registry_display})"
        ));
    }
    let versions: Vec<String> = index
        .entries
        .iter()
        .map(|e| e.version.clone())
        .collect::<Vec<_>>();
    match (version, requirement) {
        (Some(v), _) => {
            if !versions.iter().any(|e| e == v) {
                return Err(format!(
                    "{E1802} registry version not found: {package}@{v} (registry {registry_display}; published: {})",
                    versions.join(", ")
                ));
            }
            if let Some(req) = requirement {
                if !satisfies(v, req) {
                    return Err(format!(
                        "{E1802} registry version {package}@{v} does not satisfy requirement \
                         '{}' (registry {registry_display})",
                        render_req(req)
                    ));
                }
            }
            Ok(v.to_string())
        }
        (None, Some(req)) => {
            let req_text = render_req(req);
            let best = versions
                .iter()
                .filter(|v| satisfies(v, req))
                .max_by(|a, b| version_key(a).cmp(&version_key(b)).then_with(|| a.cmp(b)));
            let Some(best) = best else {
                return Err(format!(
                    "{E1802} no published version of {package} satisfies requirement \
                     '{req_text}' (registry {registry_display}; published: {})",
                    versions.join(", ")
                ));
            };
            Ok(best.clone())
        }
        (None, None) => Ok(latest_version(&versions)
            .expect("non-empty entries")
            .to_string()),
    }
}
