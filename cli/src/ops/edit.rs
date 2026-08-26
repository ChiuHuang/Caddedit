//! `caddedit edit [domain]` — $EDITOR on the raw block, validated afterwards.

use crate::config::Paths;
use crate::{caddy, vhost};
use anyhow::Result;
use owo_colors::OwoColorize;

pub fn run(paths: &Paths, domain: Option<&str>, no_reload: bool) -> Result<()> {
    let vf = match domain {
        Some(d) => vhost::find(paths, d)?,
        None => crate::picker::select(paths, "edit")?,
    };
    caddy::open_editor(&vf.path)?;

    if caddy::caddy_available() {
        match caddy::validate_site(paths, &vf.path) {
            Ok(_) => println!("{}", "validated".green()),
            Err(e) => {
                eprintln!("{}", "validation failed — fix or revert:".red().bold());
                eprintln!("{}", format!("{e:#}").red());
                eprintln!("{}", vf.path.display().to_string().yellow());
                std::process::exit(1);
            }
        }
    }
    caddy::try_reload(paths, !no_reload);
    Ok(())
}
