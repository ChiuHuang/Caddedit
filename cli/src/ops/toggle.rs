//! `caddedit on/off/rm` — status flips and soft deletion, always validated.

use crate::config::Paths;
use crate::vhost;
use crate::{caddy, ops};
use anyhow::{bail, Result};
use owo_colors::OwoColorize;

pub fn enable(paths: &Paths, domains: &[String], no_reload: bool) -> Result<()> {
    flip(paths, domains, true, no_reload)
}

pub fn disable(paths: &Paths, domains: &[String], no_reload: bool) -> Result<()> {
    flip(paths, domains, false, no_reload)
}

fn flip(paths: &Paths, domains: &[String], on: bool, no_reload: bool) -> Result<()> {
    // bare invocation -> interactive picker
    let picked: Vec<String>;
    let domains: &[String] = if domains.is_empty() {
        picked = vec![crate::picker::select(paths, if on { "on" } else { "off" })?.id];
        &picked
    } else {
        domains
    };

    for d in domains {
        let vf = vhost::find(paths, d)?;
        let already = on == (vf.status == vhost::Status::On);
        if already {
            println!(
                "{} {} is already {}",
                "=".dimmed(),
                vf.id.bright_white(),
                if on { "on" } else { "off" }
            );
            continue;
        }
        // sanity-validate the block before it goes live
        if on && caddy::caddy_available() {
            if let Err(e) = caddy::validate_file(&vf.path) {
                eprintln!("{}", format!("✗ {}: validation failed", vf.id).red().bold());
                eprintln!("{}", format!("{e:#}").red());
                continue;
            }
        }
        let target = vhost::set_status(&vf, paths, on)?;
        println!(
            "{} {} -> {}",
            if on {
                "●".green().to_string()
            } else {
                "○".dimmed().to_string()
            },
            vf.id.bright_white(),
            target.display()
        );
        caddy::try_reload(paths, !no_reload);
    }
    Ok(())
}

pub fn remove(paths: &Paths, domain: Option<&str>, yes: bool, no_reload: bool) -> Result<()> {
    let vf = match domain {
        Some(d) => vhost::find(paths, d)?,
        None => crate::picker::select(paths, "rm")?,
    };
    if !yes && !ops::confirm(&format!("remove {}?", vf.path.display()))? {
        bail!("cancelled");
    }
    let target = vhost::soft_delete(&vf, paths)?;
    println!(
        "{} moved to {}",
        vf.id.bright_white(),
        target.display().dimmed()
    );
    caddy::try_reload(paths, !no_reload);
    Ok(())
}
