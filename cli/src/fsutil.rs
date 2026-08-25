//! Small filesystem helpers: atomic writes and timestamped names.

use anyhow::Context;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Write via `<file>.tmp` + rename so a crash never leaves half a config.
pub fn atomic_write(path: &Path, contents: &str) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating directory {}", parent.display()))?;
    let tmp = path.with_extension("caddy.tmp");
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
    }
    // Windows rename-over-existing fails; remove target first (tiny race is
    // acceptable here because tmp+remove+rename happens within milliseconds).
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("replacing {}", path.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("activating {}", path.display()))?;
    Ok(())
}

pub fn timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn random_token() -> String {
    let mut buf = [0u8; 24];
    getrandom::fill(&mut buf).expect("system entropy unavailable");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
