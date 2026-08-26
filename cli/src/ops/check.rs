//! `caddedit check` — validate main config + every enabled vhost.

use crate::caddy;
use crate::config::Paths;
use crate::vhost::{scan, Status};
use anyhow::Result;
use owo_colors::OwoColorize;

pub fn run(paths: &Paths) -> Result<bool> {
    let mut ok = true;

    print!("main config ... ");
    if !paths.caddyfile.exists() {
        println!("{}", "missing".red().bold());
        return Ok(false);
    }
    match caddy::validate_file(&paths.caddyfile) {
        Ok(_) => println!("{}", "ok".green()),
        Err(e) => {
            ok = false;
            println!("{}", "FAILED".red().bold());
            eprintln!("{}", format!("{e:#}").red());
        }
    }

    let files = scan(paths);
    for vf in files {
        if vf.status == Status::Off {
            continue;
        }
        print!("{} {} ... ", "●".green(), vf.id.bright_white());
        match caddy::validate_site(paths, &vf.path) {
            Ok(_) => println!("{}", "ok".green()),
            Err(e) => {
                ok = false;
                println!("{}", "FAILED".red().bold());
                eprintln!("{}", format!("{e:#}").red());
            }
        }
    }

    Ok(ok)
}
