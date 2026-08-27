use crate::config::Paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const ACCESS_TOKEN_TTL_MS: u64 = 24 * 60 * 60 * 1000; // 1 day

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RefreshToken {
    pub token: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_ua: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Persisted {
    refresh_token: Option<RefreshToken>,
}

pub fn token_file(paths: &Paths) -> PathBuf {
    // Store alongside the Caddyfile, e.g. /etc/caddy/.caddedit_tokens.json
    // Fallback to backup_dir parent if caddyfile has no parent.
    if let Some(parent) = paths.caddyfile.parent() {
        parent.join(".caddedit_tokens.json")
    } else {
        paths.backup_dir.join(".caddedit_tokens.json")
    }
}

pub fn load_refresh(paths: &Paths) -> Option<RefreshToken> {
    let file = token_file(paths);
    let data = std::fs::read_to_string(&file).ok()?;
    let persisted: Persisted = serde_json::from_str(&data).ok()?;
    persisted.refresh_token
}

pub fn save_refresh(paths: &Paths, rt: &Option<RefreshToken>) -> anyhow::Result<()> {
    let file = token_file(paths);
    let persisted = Persisted {
        refresh_token: rt.clone(),
    };
    let data = serde_json::to_string_pretty(&persisted)?;
    // write with 600 permissions on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true).truncate(true).mode(0o600);
        let mut f = opts.open(&file)?;
        use std::io::Write;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&file, data)?;
        Ok(())
    }
}

#[allow(dead_code)]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_paths(dir: &TempDir) -> crate::config::Paths {
        let caddyfile = dir.path().join("Caddyfile");
        std::fs::write(&caddyfile, "{ admin off }\n").unwrap();
        crate::config::Paths::resolve(Some(caddyfile), None)
    }

    #[test]
    fn access_token_ttl_is_one_day() {
        assert_eq!(ACCESS_TOKEN_TTL_MS, 86400000);
        assert_eq!(ACCESS_TOKEN_TTL_MS / 1000, 86400);
    }

    #[test]
    fn refresh_token_persists_and_rotates() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);
        assert!(load_refresh(&paths).is_none());

        let rt1 = RefreshToken {
            token: "refresh_one_time_abcdef123456".into(),
            created_at: 1000,
            created_by_ua: Some("caddedit-cli/0.5.2".into()),
        };
        save_refresh(&paths, &Some(rt1.clone())).unwrap();
        let loaded = load_refresh(&paths).unwrap();
        assert_eq!(loaded.token, rt1.token);
        assert_eq!(loaded.created_by_ua, rt1.created_by_ua);
        // User-Agent is stored (settings UI passes browser UA, CLI passes caddedit-cli)
        assert!(loaded.created_by_ua.as_ref().unwrap().contains("caddedit"));

        // rotation: new token invalidates old
        let rt2 = RefreshToken {
            token: "refresh_second_xyz7890".into(),
            created_at: 2000,
            created_by_ua: Some("Mozilla/5.0".into()),
        };
        save_refresh(&paths, &Some(rt2.clone())).unwrap();
        let loaded2 = load_refresh(&paths).unwrap();
        assert_eq!(loaded2.token, rt2.token);
        assert_ne!(loaded2.token, rt1.token);
        // old token no longer loadable
        assert!(loaded2.token != "refresh_one_time_abcdef123456");
    }

    #[test]
    fn token_file_uses_sibling_of_caddyfile() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);
        let tf = token_file(&paths);
        assert_eq!(tf, tmp.path().join(".caddedit_tokens.json"));
    }

    #[test]
    fn clearing_refresh_removes_file_content() {
        let tmp = TempDir::new().unwrap();
        let paths = test_paths(&tmp);
        let rt = RefreshToken {
            token: "t".into(),
            created_at: 1,
            created_by_ua: None,
        };
        save_refresh(&paths, &Some(rt)).unwrap();
        assert!(load_refresh(&paths).is_some());
        save_refresh(&paths, &None).unwrap();
        assert!(load_refresh(&paths).is_none());
    }
}
