//! `caddedit init` — split the monolithic Caddyfile into per-site vhost files.

use crate::caddyfile::parser::{Document, TopLevel};
use crate::config::{ensure_dirs, Paths};
use crate::{caddy, fsutil, vhost};
use anyhow::{bail, Context, Result};
use owo_colors::OwoColorize;
use std::fs;

const HEADER: &str = "// managed by caddedit — site blocks live in vhosts/enabled/*.caddy";

pub fn run(paths: &Paths, force: bool, no_reload: bool) -> Result<()> {
    if !paths.caddyfile.exists() {
        bail!("config not found: {}", paths.caddyfile.display());
    }
    ensure_dirs(paths)?;

    let src = fs::read_to_string(&paths.caddyfile)
        .with_context(|| format!("reading {}", paths.caddyfile.display()))?;
    let doc = Document::parse(&src);

    // Refuse to double-init.
    let already = src.contains("vhosts/enabled") || src.contains(paths.import_line().as_str());
    if already && !force {
        bail!("config already imports vhosts (use --force to split again)");
    }

    let sites = doc.sites();
    if sites.is_empty() {
        bail!(
            "no site blocks found in {} — nothing to split",
            paths.caddyfile.display()
        );
    }

    // 1. Backup the original.
    let stamp = fsutil::timestamp();
    let backup = paths.backup_dir.join(format!("Caddyfile.bak.{stamp}"));
    fs::copy(&paths.caddyfile, &backup)
        .with_context(|| format!("backing up to {}", backup.display()))?;
    println!(
        "{}",
        format!("  backed up -> {}", backup.display()).dimmed()
    );

    // 2. Write each site block verbatim into enabled/.
    let mut created: Vec<String> = Vec::new();
    for site in &sites {
        let stem = vhost::sanitize_address(site.primary_address().unwrap_or("site"));
        let target = paths.enabled_dir().join(format!("{stem}.caddy"));
        let target = if target.exists() && !force {
            println!(
                "{}",
                format!(
                    "  skipped {} (already exists, use --force)",
                    target.display()
                )
                .yellow()
            );
            continue;
        } else {
            target
        };
        fsutil::atomic_write(&target, &doc.site_text(site))?;
        created.push(target.display().to_string());
        println!("  {} {}", "+".green(), target.display());
    }
    if created.is_empty() {
        bail!(
            "nothing written; aborting without touching {}",
            paths.caddyfile.display()
        );
    }

    // 3. Compose the new main Caddyfile.
    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str("\n\n");
    for item in &doc.items {
        match item {
            TopLevel::Other { span, .. } => {
                let text = src[span.clone()].trim_end().to_string();
                if !text.is_empty() {
                    out.push_str(&text);
                    out.push('\n');
                    out.push('\n');
                }
            }
            TopLevel::Snippet(s) => {
                out.push_str(src[s.full_span.clone()].trim_end());
                out.push('\n');
                out.push('\n');
            }
            TopLevel::Site(_) => {} // moved out
        }
    }
    out.push_str("# routes\n");
    out.push_str(&paths.import_line());
    out.push('\n');

    // 4. Validate before swapping in (needs the vhost files we just wrote).
    let tmp = paths.caddyfile.with_extension("caddedit.tmp");
    fsutil::atomic_write(&tmp, &out)?;
    if caddy::caddy_available() {
        if let Err(e) = caddy::validate_file(&tmp) {
            eprintln!("{}", "validation failed — rolling back:".red().bold());
            eprintln!("{}", format!("{e:#}").red());
            for file in &created {
                let _ = fs::remove_file(file);
            }
            let _ = fs::remove_file(&tmp);
            bail!("{} left untouched", paths.caddyfile.display());
        }
    } else {
        println!(
            "{}",
            "  note: caddy binary not found, skipping validation".yellow()
        );
    }
    fsutil::atomic_write(&paths.caddyfile, &out)?;
    let _ = fs::remove_file(&tmp);

    println!(
        "\n{} {} site(s) split; main config now imports {}",
        "done.".green().bold(),
        created.len(),
        paths.enabled_dir().display()
    );

    caddy::try_reload(paths, !no_reload);
    Ok(())
}
