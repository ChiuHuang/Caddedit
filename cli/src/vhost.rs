//! Virtual-host file management: the `enabled/` + `disabled/` split.

use crate::caddyfile::analyze::{analyze_site, SiteInfo};
use crate::config::Paths;
use crate::fsutil;
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    On,
    Off,
}

#[derive(Debug, Clone)]
pub struct VhostFile {
    /// Filename stem — used as stable id in CLI and API.
    pub id: String,
    pub path: PathBuf,
    pub status: Status,
}

#[derive(Debug, Clone, Serialize)]
pub struct VhostSummary {
    pub id: String,
    pub status: Status,
    #[serde(flatten)]
    pub info: SiteInfo,
    pub file: String,
}

/// Turn a site address into a safe filename stem.
pub fn sanitize_address(addr: &str) -> String {
    let mut s = addr.trim().to_lowercase();
    for prefix in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.to_string();
        }
    }
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '*' => out.push_str("_wildcard_"),
            ':' | '/' | '\\' | '?' | '&' | '=' | '@' => out.push('_'),
            c if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') => out.push(c),
            _ => out.push('_'),
        }
    }
    let out = out.trim_matches('.').to_string();
    if out.is_empty() {
        "site".to_string()
    } else {
        out
    }
}

fn unique_target(dir: &std::path::Path, stem: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem}.caddy"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem}-{n}.caddy"));
        n += 1;
    }
    candidate
}

pub fn scan(paths: &Paths) -> Vec<VhostFile> {
    let mut out = Vec::new();
    for (status, dir) in [
        (Status::On, paths.enabled_dir()),
        (Status::Off, paths.disabled_dir()),
    ] {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<VhostFile> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "caddy").unwrap_or(false))
            .map(|p| VhostFile {
                id: p
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: p,
                status,
            })
            .collect();
        files.sort_by(|a, b| a.id.cmp(&b.id));
        out.extend(files);
    }
    out
}

pub fn summarize(paths: &Paths) -> Vec<(VhostFile, VhostSummary)> {
    scan(paths)
        .into_iter()
        .filter_map(|vf| {
            let text = fs::read_to_string(&vf.path).ok()?;
            let info = analyze_site(&text);
            let sum = VhostSummary {
                id: vf.id.clone(),
                status: vf.status,
                file: vf.path.display().to_string(),
                info,
            };
            Some((vf, sum))
        })
        .collect()
}

/// Find a vhost by filename stem or by (substring of) any address.
pub fn find(paths: &Paths, query: &str) -> Result<VhostFile> {
    let q = query.trim().trim_end_matches(".caddy").to_lowercase();
    let all = scan(paths);
    if all.is_empty() {
        return Err(anyhow!(
            "no vhosts found under {}",
            paths.vhosts_dir.display()
        ));
    }

    // exact stem match first
    if let Some(vf) = all.iter().find(|v| v.id.to_lowercase() == q) {
        return Ok(vf.clone());
    }
    // then address substring
    let matches: Vec<VhostFile> = all
        .into_iter()
        .filter(|v| {
            let text = fs::read_to_string(&v.path).unwrap_or_default();
            let info = analyze_site(&text);
            info.addresses.iter().any(|a| a.to_lowercase().contains(&q))
        })
        .collect();
    match matches.len() {
        0 => Err(anyhow!("no vhost matching `{query}`")),
        1 => Ok(matches.into_iter().next().unwrap()),
        n => Err(anyhow!(
            "`{query}` is ambiguous ({n} matches); use the file name instead"
        )),
    }
}

/// Move a file between enabled/ and disabled/.
pub fn set_status(vf: &VhostFile, paths: &Paths, on: bool) -> Result<PathBuf> {
    let target_dir = if on {
        paths.enabled_dir()
    } else {
        paths.disabled_dir()
    };
    std::fs::create_dir_all(&target_dir)
        .with_context(|| format!("creating {}", target_dir.display()))?;
    let target = unique_target(&target_dir, &vf.id);
    fs::rename(&vf.path, &target)
        .with_context(|| format!("moving {} to {}", vf.path.display(), target_dir.display()))?;
    Ok(target)
}

/// "Delete" = move into the backup dir so mistakes are recoverable.
pub fn soft_delete(vf: &VhostFile, paths: &Paths) -> Result<PathBuf> {
    std::fs::create_dir_all(&paths.backup_dir)?;
    let base = format!("deleted-{}-{}", crate::fsutil::timestamp(), vf.id);
    let mut target = paths.backup_dir.join(format!("{base}.caddy"));
    let mut n = 2;
    while target.exists() {
        target = paths.backup_dir.join(format!("{base}-{n}.caddy"));
        n += 1;
    }
    fs::rename(&vf.path, &target)
        .with_context(|| format!("moving {} to backups", vf.path.display()))?;
    Ok(target)
}

pub fn read_raw(vf: &VhostFile) -> Result<String> {
    fs::read_to_string(&vf.path).with_context(|| format!("reading {}", vf.path.display()))
}

/// Generate a fresh site block for `caddedit new` and the web UI.
pub fn scaffold_block(domains: &[String], upstream: &str, tls: &str, watch_log: bool) -> String {
    let mut s = format!("{} {{\n", domains.join(", "));
    if watch_log {
        s.push_str("\timport request_watch_log\n");
    }
    if !upstream.is_empty() {
        s.push_str(&format!("\treverse_proxy {upstream}\n"));
    }
    match tls {
        "internal_explicit" => s.push_str("\ttls internal\n"),
        "cloudflare" => {
            s.push_str("\ttls {\n\t\tdns cloudflare {$CF_API_TOKEN}\n\t}\n");
        }
        _ => {}
    }
    if upstream.is_empty() {
        s.push_str("\trespond \"It works!\"\n");
    }
    s.push_str("}\n");
    s
}

/// Write a raw site block into enabled/, validating before it sticks.
/// Used by the dashboard's "Raw" create tab.
pub fn create_vhost_source(paths: &Paths, source: &str) -> Result<PathBuf> {
    let doc = crate::caddyfile::parser::Document::parse(source);
    let sites = doc.sites();
    if sites.len() != 1 {
        anyhow::bail!("content must contain exactly one site block");
    }
    let stem = sites[0]
        .primary_address()
        .map(sanitize_address)
        .unwrap_or_else(|| "site".to_string());
    std::fs::create_dir_all(paths.enabled_dir())?;
    let mut target = paths.enabled_dir().join(format!("{stem}.caddy"));
    let mut n = 2;
    while target.exists() {
        target = paths.enabled_dir().join(format!("{stem}-{n}.caddy"));
        n += 1;
    }
    let mut text = source.to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    fsutil::atomic_write(&target, &text)?;
    if crate::caddy::caddy_available() {
        if let Err(e) = crate::caddy::validate_site(paths, &target) {
            let _ = fs::remove_file(&target);
            anyhow::bail!("validation failed:\n{e}");
        }
    }
    Ok(target)
}

/// Write a new vhost file into enabled/, validating before it sticks.
pub fn create_vhost_file(
    paths: &Paths,
    domains: &[String],
    upstream: &str,
    tls: &str,
    watch_log: bool,
) -> Result<PathBuf> {
    let stem = sanitize_address(&domains[0]);
    std::fs::create_dir_all(paths.enabled_dir())?;
    let mut target = paths.enabled_dir().join(format!("{stem}.caddy"));
    let mut n = 2;
    while target.exists() {
        target = paths.enabled_dir().join(format!("{stem}-{n}.caddy"));
        n += 1;
    }
    let block = scaffold_block(domains, upstream, tls, watch_log);
    fsutil::atomic_write(&target, &block)?;
    if crate::caddy::caddy_available() {
        if let Err(e) = crate::caddy::validate_site(paths, &target) {
            let _ = fs::remove_file(&target);
            anyhow::bail!("validation failed:\n{e}");
        }
    }
    Ok(target)
}
