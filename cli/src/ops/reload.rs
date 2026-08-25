//! `caddedit reload` — manual reload.

use crate::caddy;
use crate::config::Paths;
use anyhow::Result;
use owo_colors::OwoColorize;

pub fn run(paths: &Paths) -> Result<()> {
    caddy::reload(paths)?;
    println!("{}", "reloaded".green().bold());
    Ok(())
}
