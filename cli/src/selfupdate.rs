//! Self-update: check GitHub releases, download + verify + install the new
//! binary, then restart the systemd service from outside our cgroup.

use anyhow::{anyhow, Context, Result};

pub const REPO: &str = "ChiuHuang/Caddedit";

/// Release asset matching the platform this binary was built for.
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-musl"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

fn curl(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("curl")
        .args(["-fsSL", "--max-time", "90"])
        .args(args)
        .output()
        .map_err(|e| anyhow!("curl unavailable: {e}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Latest release tag without the leading `v`.
#[allow(dead_code)]
pub fn latest_version() -> Result<String> {
    latest_version_for("stable")
}

/// Latest version for a channel: "stable" (latest) or "nightly" (tag nightly).
pub fn latest_version_for(channel: &str) -> Result<String> {
    if channel == "nightly" {
        latest_nightly_version()
    } else {
        latest_stable_version()
    }
}

fn latest_stable_version() -> Result<String> {
    let json = curl(&[
        "-H",
        "Accept: application/vnd.github+json",
        &format!("https://api.github.com/repos/{REPO}/releases/latest"),
    ])?;
    let v: serde_json::Value =
        serde_json::from_str(&json).context("parsing GitHub release response")?;
    let tag = v["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("release response missing tag_name"))?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// Latest nightly version as `x.x.x_rnd<date>` where `x.x.x` is latest stable + random date suffix.
pub fn latest_nightly_version() -> Result<String> {
    // Verify nightly tag exists (rolling release)
    let json = curl(&[
        "-H",
        "Accept: application/vnd.github+json",
        &format!("https://api.github.com/repos/{REPO}/releases/tags/nightly"),
    ])?;
    let v: serde_json::Value =
        serde_json::from_str(&json).context("parsing GitHub nightly response")?;
    if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
        if msg.contains("Not Found") {
            return Err(anyhow!("no nightly release found (tag nightly)"));
        }
    }
    v["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("nightly response missing tag_name"))?;
    // Use current stable version + _rnd + date suffix (e.g., 0.5.9_rnd20260827)
    let stable = latest_stable_version()?;
    let published_at = v.get("published_at").and_then(|p| p.as_str()).unwrap_or("");
    let date_suffix = published_at
        .split('T')
        .next()
        .unwrap_or("")
        .replace('-', "");
    let suffix = if date_suffix.is_empty() {
        // fallback to random 4 digits from getrandom
        let mut buf = [0u8; 2];
        let _ = getrandom::fill(&mut buf);
        format!("{:04}", u16::from_le_bytes(buf) % 10000)
    } else {
        date_suffix
    };
    Ok(format!("{}_rnd{}", stable, suffix))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub notes: Option<String>,
    pub published_at: Option<String>,
}

/// Compare two refs (tags/branches) and return a markdown list of commits.
/// e.g., base="v0.5.5", head="nightly" or "v0.5.6"
pub fn compare_notes(base: &str, head: &str) -> Result<Option<String>> {
    let url = format!("https://api.github.com/repos/{REPO}/compare/{base}...{head}");
    let json = match curl(&["-H", "Accept: application/vnd.github+json", &url]) {
        Ok(j) => j,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("404") || msg.contains("Not Found") {
                return Ok(None);
            }
            return Err(e);
        }
    };
    let v: serde_json::Value = serde_json::from_str(&json).context("parsing compare response")?;
    if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
        if msg.contains("Not Found") {
            return Ok(None);
        }
    }
    let ahead_by = v.get("ahead_by").and_then(|n| n.as_u64()).unwrap_or(0);
    let behind_by = v.get("behind_by").and_then(|n| n.as_u64()).unwrap_or(0);
    if ahead_by == 0 {
        // head is not ahead of base — no new commits
        if behind_by > 0 {
            return Ok(Some(format!(
                "Already ahead of nightly by {behind_by} commit(s) — no update needed."
            )));
        }
        return Ok(None);
    }
    let commits = v
        .get("commits")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    if commits.is_empty() {
        return Ok(None);
    }
    let mut lines = Vec::new();
    for c in commits.iter().take(12) {
        let msg = c
            .get("commit")
            .and_then(|m| m.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let first_line = msg.lines().next().unwrap_or("").trim();
        if first_line.is_empty() {
            continue;
        }
        let sha = c.get("sha").and_then(|s| s.as_str()).unwrap_or("");
        let short = if sha.len() >= 7 { &sha[..7] } else { sha };
        // skip merge commits noise
        if first_line.starts_with("Merge ") {
            continue;
        }
        lines.push(format!("- {first_line} ({short})"));
    }
    if lines.is_empty() {
        return Ok(None);
    }
    let mut out = lines.join("\n");
    if commits.len() > 12 {
        out.push_str(&format!("\n… and {} more commits", commits.len() - 12));
    }
    out.push_str(&format!(
        "\n\n[Full diff](https://github.com/{REPO}/compare/{base}...{head})"
    ));
    Ok(Some(out))
}

pub fn release_info_for(channel: &str) -> Result<ReleaseInfo> {
    if channel == "nightly" {
        let json = curl(&[
            "-H",
            "Accept: application/vnd.github+json",
            &format!("https://api.github.com/repos/{REPO}/releases/tags/nightly"),
        ])?;
        let v: serde_json::Value =
            serde_json::from_str(&json).context("parsing GitHub release response")?;
        if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
            if msg.contains("Not Found") {
                return Err(anyhow!("no nightly release found (tag nightly)"));
            }
        }
        let published_at = v
            .get("published_at")
            .and_then(|p| p.as_str())
            .map(|s| s.to_string());
        let body = v
            .get("body")
            .and_then(|b| b.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // nightly version is stable + _rnd + date/random (e.g., 0.5.9_rnd20260827)
        let stable = latest_stable_version().unwrap_or_else(|_| "0.0.0".to_string());
        let date_suffix = published_at
            .as_deref()
            .unwrap_or("")
            .split('T')
            .next()
            .unwrap_or("")
            .replace('-', "");
        let suffix = if date_suffix.is_empty() {
            let mut buf = [0u8; 2];
            let _ = getrandom::fill(&mut buf);
            format!("{:04}", u16::from_le_bytes(buf) % 10000)
        } else {
            date_suffix
        };
        let version = format!("{}_rnd{}", stable, suffix);
        let notes = body.map(|b| {
            if b.len() > 4000 {
                format!(
                    "{}…\n\n[Full notes](https://github.com/{REPO}/releases/tag/nightly)",
                    &b[..4000]
                )
            } else {
                b
            }
        });
        return Ok(ReleaseInfo {
            version,
            notes,
            published_at,
        });
    }
    let json = curl(&[
        "-H",
        "Accept: application/vnd.github+json",
        &format!("https://api.github.com/repos/{REPO}/releases/latest"),
    ])?;
    let v: serde_json::Value =
        serde_json::from_str(&json).context("parsing GitHub release response")?;
    if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
        if msg.contains("Not Found") {
            return Err(anyhow!("no stable release found"));
        }
    }
    let tag = v["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("release response missing tag_name"))?
        .trim_start_matches('v')
        .to_string();
    let body = v
        .get("body")
        .and_then(|b| b.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let published_at = v
        .get("published_at")
        .and_then(|p| p.as_str())
        .map(|s| s.to_string());
    let notes = body.map(|b| {
        if b.len() > 4000 {
            format!(
                "{}…\n\n[Full notes](https://github.com/{REPO}/releases/tag/v{tag})",
                &b[..4000]
            )
        } else {
            b
        }
    });
    Ok(ReleaseInfo {
        version: tag,
        notes,
        published_at,
    })
}

fn version_tuple(v: &str) -> (u64, u64, u64) {
    // strip _rnd and -nightly suffixes before parsing
    let v = v.split('_').next().unwrap_or(v);
    let v = v.split('-').next().unwrap_or(v);
    let mut it = v.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
        it.next().unwrap_or(0),
    )
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    version_tuple(latest) > version_tuple(current)
}

pub fn is_newer_for(latest: &str, current: &str, channel: &str) -> bool {
    if channel == "nightly" {
        if latest == "nightly" {
            return true;
        }
        if latest.contains("_rnd") {
            // x.x.x_rnd* is newer than x.x.x stable, equal _rnd* is not newer
            if version_tuple(latest) > version_tuple(current) {
                return true;
            }
            if version_tuple(latest) == version_tuple(current)
                && latest.contains("_rnd")
                && !current.contains("_rnd")
            {
                return true;
            }
            return false;
        }
        // handle "0.5.3-nightly" or "0.5.3-nightly.20260828"
        let base = latest.split('-').next().unwrap_or(latest);
        return version_tuple(base) >= version_tuple(current);
    }
    is_newer(latest, current)
}

const INSTALL_SCRIPT: &str = r#"set -eu
dir="$(mktemp -d)"
cd "$dir"
base="https://github.com/{repo}/releases/download/{tag}"
asset="caddedit-{target}"
curl -fsSL --max-time 120 "$base/$asset.tar.gz" -O
expected="$(curl -fsSL --max-time 30 --retry 3 "$base/$asset.tar.gz.sha256" | awk '{print $1}')"
actual="$(sha256sum "$asset.tar.gz" 2>/dev/null | awk '{print $1}' || shasum -a 256 "$asset.tar.gz" | awk '{print $1}')"
[ -n "$expected" ] || { echo "empty checksum (download of .sha256 failed)"; exit 1; }
[ "$expected" = "$actual" ] || { echo "checksum mismatch"; exit 1; }
tar xzf "$asset.tar.gz"
install -m 0755 "$asset/caddedit" /usr/local/bin/caddedit
echo "installed $(/usr/local/bin/caddedit --version)"
"#;

/// Download, checksum-verify and install `version` over /usr/local/bin/caddedit.
pub fn install_version(version: &str) -> Result<String> {
    let target =
        asset_name().ok_or_else(|| anyhow!("auto-update not supported on this platform"))?;
    let tag = if version == "nightly" || version.starts_with("nightly") || version.contains("_rnd")
    {
        "nightly".to_string()
    } else {
        format!("v{version}")
    };
    let script = INSTALL_SCRIPT
        .replace("{repo}", REPO)
        .replace("{tag}", &tag)
        .replace("{target}", target);
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&script)
        .output()
        .map_err(|e| anyhow!("cannot run sh: {e}"))?;
    if !out.status.success() {
        // failure details may land on either stream (e.g. `echo`-based errors)
        let detail = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout).trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        return Err(anyhow!(
            "update script failed: {}",
            if detail.is_empty() {
                "unknown error".to_string()
            } else {
                detail
            }
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Restart the dashboard service from a transient systemd unit so the new
/// binary takes over even though we're the process being replaced.
/// Returns Err when systemd isn't available (manual installs).
pub fn schedule_restart() -> Result<()> {
    for unit in ["caddedit-dashboard.service", "caddedit.service"] {
        let check = std::process::Command::new("systemctl")
            .args(["cat", unit])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if check.map(|s| s.success()).unwrap_or(false) {
            std::process::Command::new("systemd-run")
                .args([
                    "--collect",
                    "--unit=caddedit-selfupdate",
                    "--description=caddedit self-update restart",
                    "bash",
                    "-c",
                    &format!("sleep 1; systemctl restart {unit}"),
                ])
                .spawn()
                .map_err(|e| anyhow!("systemd-run failed: {e}"))?;
            return Ok(());
        }
    }
    Err(anyhow!(
        "no caddedit systemd unit found — update installed, restart manually"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_matches_build_target() {
        // whatever CI builds must be downloadable by itself
        let expected = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") | ("macos", "x86_64") => true,
            ("linux" | "macos", "aarch64") => true,
            _ => asset_name().is_none(),
        };
        assert!(expected);
        assert!(asset_name()
            .map(|a| a.contains(std::env::consts::ARCH))
            .unwrap_or(true));
    }

    #[test]
    fn semver_compare() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
    }

    #[test]
    fn nightly_is_newer() {
        assert!(is_newer_for("nightly", "0.5.3", "nightly"));
        assert!(is_newer_for("0.5.3-nightly", "0.5.3", "nightly"));
        assert!(is_newer_for("0.5.4-nightly", "0.5.3", "nightly"));
        assert!(!is_newer_for("0.5.3", "0.5.3", "stable"));
    }
}
