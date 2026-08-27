//! CLI auth client — login via refresh token, store access token, User-Agent.

use std::path::PathBuf;

const USER_AGENT: &str = concat!("caddedit-cli/", env!("CARGO_PKG_VERSION"));

fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("caddedit"))
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Some(PathBuf::from(xdg).join("caddedit"));
            }
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join(".config/caddedit"))
    }
}

fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("cli.json"))
}

#[derive(serde::Serialize, serde::Deserialize, Default, Debug)]
struct CliConfig {
    url: Option<String>,
    access_token: Option<String>,
    expires_at: Option<u64>,
    refresh_token: Option<String>,
}

fn load_config() -> CliConfig {
    let Some(file) = config_file() else {
        return CliConfig::default();
    };
    let Ok(data) = std::fs::read_to_string(&file) else {
        return CliConfig::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

fn save_config(cfg: &CliConfig) -> anyhow::Result<()> {
    let Some(file) = config_file() else {
        anyhow::bail!("cannot determine config directory");
    };
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(cfg)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true).truncate(true).mode(0o600);
        let mut f = opts.open(&file)?;
        use std::io::Write;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&file, data)?;
    }
    Ok(())
}

fn curl_available() -> bool {
    std::process::Command::new("curl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Perform POST /api/auth/refresh via curl (avoids adding reqwest dep).
fn exchange_via_curl(url: &str, refresh_token: &str) -> anyhow::Result<(String, u64)> {
    let endpoint = format!("{}/api/auth/refresh", url.trim_end_matches('/'));
    let payload = serde_json::json!({ "refresh_token": refresh_token }).to_string();
    // Use curl -fsSL -H ... -d ...
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "30",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-H",
            &format!("User-Agent: {USER_AGENT}"),
            "-d",
            &payload,
            &endpoint,
        ])
        .output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let body = String::from_utf8_lossy(&out.stdout);
        let msg = if !body.is_empty() { body.trim() } else { err.trim() };
        anyhow::bail!("refresh failed: {}", if msg.is_empty() { "unknown error" } else { msg });
    }
    let body = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&body)?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        anyhow::bail!("{}", err);
    }
    let token = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing access_token in response"))?
        .to_string();
    let expires_at = v
        .get("expires_at")
        .and_then(|e| e.as_u64())
        .unwrap_or(0);
    Ok((token, expires_at))
}

pub fn run_login(url: &str, refresh_token: &str, save: bool) -> anyhow::Result<()> {
    if url.trim().is_empty() || refresh_token.trim().is_empty() {
        anyhow::bail!("url and refresh_token are required");
    }
    // Prefer curl if available, else fallback to simple message
    if !curl_available() {
        anyhow::bail!("curl not found — install curl or use: curl -H \"User-Agent: {USER_AGENT}\" -H \"Content-Type: application/json\" -d '{{\"refresh_token\":\"{}\"}}' {}/api/auth/refresh", refresh_token, url.trim_end_matches('/'));
    }
    println!("{} exchanging refresh token at {} ...", "caddedit".to_string(), url);
    let (access, expires_at) = exchange_via_curl(url, refresh_token)?;
    println!("✓ access token issued (expires_at={})", expires_at);
    println!("  token: {}", access);
    println!("  Use: curl -H \"Authorization: Bearer {}\" -H \"User-Agent: {USER_AGENT}\" {}/api/vhosts", access, url.trim_end_matches('/'));
    if save {
        let mut cfg = load_config();
        cfg.url = Some(url.trim().to_string());
        cfg.access_token = Some(access.clone());
        cfg.expires_at = Some(expires_at);
        cfg.refresh_token = Some(refresh_token.to_string());
        save_config(&cfg)?;
        if let Some(file) = config_file() {
            println!("✓ saved to {}", file.display());
        }
        println!("  Token valid for 24h. Re-run login after expiry or when refresh rotated.");
    } else {
        println!("  (not saved — pass --save to persist to config file)");
    }
    Ok(())
}

pub fn run_logout() -> anyhow::Result<()> {
    let mut cfg = load_config();
    let had = cfg.access_token.is_some() || cfg.refresh_token.is_some();
    cfg.access_token = None;
    cfg.expires_at = None;
    // keep url but clear tokens
    save_config(&cfg)?;
    if had {
        println!("✓ cleared stored tokens");
    } else {
        println!("no stored tokens");
    }
    Ok(())
}

pub fn run_config_show() -> anyhow::Result<()> {
    let cfg = load_config();
    let file = config_file()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<unknown>".into());
    println!("config file: {}", file);
    println!("{}", serde_json::to_string_pretty(&json_masked(&cfg))?);
    Ok(())
}

fn json_masked(cfg: &CliConfig) -> serde_json::Value {
    let mut v = serde_json::to_value(cfg).unwrap_or(serde_json::json!({}));
    if let Some(obj) = v.as_object_mut() {
        for key in ["access_token", "refresh_token"] {
            if obj.contains_key(key) && obj[key].is_string() {
                let s = obj[key].as_str().unwrap_or("");
                if !s.is_empty() {
                    // mask all but first 4 chars
                    let masked = if s.len() > 8 {
                        format!("{}…{} ({} chars)", &s[..4], &s[s.len()-4..], s.len())
                    } else {
                        "****".to_string()
                    };
                    obj[key] = serde_json::Value::String(masked);
                }
            }
        }
    }
    v
}

pub fn user_agent() -> &'static str {
    USER_AGENT
}
