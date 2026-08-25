//! `caddedit show [domain]` — dump one vhost's raw block.

use crate::config::Paths;
use crate::vhost;
use anyhow::Result;
use owo_colors::OwoColorize;

pub fn run(paths: &Paths, domain: Option<&str>) -> Result<()> {
    let vf = match domain {
        Some(d) => vhost::find(paths, d)?,
        None => crate::picker::select(paths, "show")?,
    };
    let text = vhost::read_raw(&vf)?;
    println!(
        "{} {}",
        match vf.status {
            crate::vhost::Status::On => "●".green().to_string(),
            crate::vhost::Status::Off => "○".dimmed().to_string(),
        },
        vf.path.display().to_string().dimmed()
    );
    println!("{}", text.trim_end());
    Ok(())
}
