//! Interactions with the `caddy` binary: validate + reload.

use crate::config::Paths;
use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;

pub fn bin_name() -> String {
    std::env::var("CADDY_BIN").unwrap_or_else(|_| "caddy".to_string())
}

fn run(args: &[&str]) -> Result<String> {
    let out = Command::new(bin_name())
        .args(args)
        .output()
        .map_err(|e| anyhow!("cannot execute `{}`: {e}", bin_name()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        Err(anyhow!("{}", text.trim()))
    }
}

/// `caddy validate --adapter caddyfile --config <path>`
pub fn validate_file(path: &Path) -> Result<String> {
    run(&[
        "validate",
        "--adapter",
        "caddyfile",
        "--config",
        &path.to_string_lossy(),
    ])
    .map_err(|e| anyhow!(e))
}

/// Extract snippet definitions from the main Caddyfile, so that validating a
/// lone site block can resolve `import <snippet>` lines.
fn snippet_prelude(paths: &Paths) -> Option<String> {
    let src = fs::read_to_string(&paths.caddyfile).ok()?;
    let doc = crate::caddyfile::parser::Document::parse(&src);
    let parts: Vec<&str> = doc
        .items
        .iter()
        .filter_map(|it| match it {
            crate::caddyfile::parser::TopLevel::Snippet(s) => Some(&doc.src[s.full_span.clone()]),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

/// `caddy adapt` — syntax-level check that stops short of provisioning
/// modules, so env-dependent blocks (e.g. `{env.…}` DNS API tokens) don't
/// fail validation in environments where those variables aren't set.
pub fn adapt_file(path: &Path) -> Result<String> {
    run(&[
        "adapt",
        "--adapter",
        "caddyfile",
        "--config",
        &path.to_string_lossy(),
    ])
    .map(|_| String::new())
    .map_err(|e| anyhow!(e))
}

/// Validate a single site-block file. Snippets from the main Caddyfile are
/// prepended when the block imports one, since `caddy validate` on a lone
/// file cannot resolve those imports on its own. Provisioning failures that
/// only stem from a bare environment (missing API tokens) fall back to the
/// syntax-level `adapt` check.
pub fn validate_site(paths: &Paths, path: &Path) -> Result<String> {
    match validate_file(path) {
        Ok(out) => Ok(out),
        Err(standalone_err) => {
            let Some(prelude) = snippet_prelude(paths) else {
                return Err(standalone_err);
            };
            let body =
                fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
            let tmp = std::env::temp_dir().join(format!(
                "caddedit-validate-{}.caddy",
                crate::fsutil::random_token()
            ));
            fs::write(&tmp, format!("{prelude}\n\n{body}"))
                .with_context(|| format!("writing {}", tmp.display()))?;
            let result = match validate_file(&tmp) {
                Ok(out) => Ok(out),
                Err(e) => {
                    if e.to_string().contains("provision")
                        || e.to_string().contains("appears invalid")
                    {
                        match adapt_file(&tmp) {
                            Ok(_) => Ok(String::new()),
                            Err(adapt_err) => Err(adapt_err),
                        }
                    } else {
                        Err(e)
                    }
                }
            };
            let _ = fs::remove_file(&tmp);
            result
        }
    }
}

pub fn caddy_available() -> bool {
    Command::new(bin_name()).arg("version").output().is_ok()
}

/// Reload using CADDEDIT_RELOAD_COMMAND if set, otherwise the default
/// `caddy reload --config <main Caddyfile>`.
pub fn reload(paths: &Paths) -> Result<String> {
    if let Ok(cmd) = std::env::var("CADDEDIT_RELOAD_COMMAND") {
        let parts: Vec<String> = cmd.split_whitespace().map(String::from).collect();
        if parts.is_empty() {
            return Err(anyhow!("CADDEDIT_RELOAD_COMMAND is set but empty"));
        }
        let (prog, args) = parts.split_first().unwrap();
        let out = Command::new(prog)
            .args(args)
            .output()
            .map_err(|e| anyhow!("cannot execute `{prog}`: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        return if out.status.success() {
            Ok(text)
        } else {
            Err(anyhow!("{}", text.trim()))
        };
    }
    run(&["reload", "--config", &paths.caddyfile.to_string_lossy()])
        .with_context(|| format!("reloading {}", paths.caddyfile.display()))
}

/// Best-effort reload that only warns on failure.
pub fn try_reload(paths: &Paths, enabled: bool) {
    if !enabled {
        return;
    }
    match reload(paths) {
        Ok(_) => println!("{}", "  caddy reloaded".green()),
        Err(e) => println!("{}", format!("  warning: reload failed: {e:#}").yellow()),
    }
}

use owo_colors::OwoColorize;

/// Resolve $VISUAL / $EDITOR with platform fallbacks and block until exit.
pub fn open_editor(file: &Path) -> Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });
    let parts: Vec<String> = editor.split_whitespace().map(String::from).collect();
    let (prog, args) = parts.split_first().ok_or_else(|| anyhow!("empty EDITOR"))?;
    let status = Command::new(prog)
        .args(args)
        .arg(file)
        .status()
        .map_err(|e| anyhow!("cannot launch editor `{prog}`: {e}"))?;
    if !status.success() {
        return Err(anyhow!("editor exited with {status}"));
    }
    Ok(())
}
