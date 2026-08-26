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
pub fn latest_version() -> Result<String> {
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

fn version_tuple(v: &str) -> (u64, u64, u64) {
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

const INSTALL_SCRIPT: &str = r#"set -eu
dir="$(mktemp -d)"
cd "$dir"
base="https://github.com/{repo}/releases/download/v{ver}"
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
    let script = INSTALL_SCRIPT
        .replace("{repo}", REPO)
        .replace("{ver}", version)
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
}
