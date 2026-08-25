//! `caddedit new` — scaffold a fresh site block, interactively or via flags.

use crate::config::Paths;
use crate::vhost;
use anyhow::{bail, Result};
use owo_colors::OwoColorize;
use std::io::{BufRead, Write};

pub const TLS_PRESETS: &[(&str, &str)] = &[
    ("auto", "automatic (Let's Encrypt / ZeroSSL, no directive)"),
    ("internal_explicit", "tls internal (self-managed CA)"),
    ("cloudflare", "Cloudflare DNS-01 challenge"),
    ("none", "HTTP only"),
];

pub fn run(
    paths: &Paths,
    cli_domains: Option<&str>,
    cli_upstream: Option<&str>,
    cli_tls: Option<&str>,
    watch_log: bool,
    no_reload: bool,
) -> Result<()> {
    let domains_raw = match cli_domains {
        Some(d) => d.to_string(),
        None => prompt("domains (comma separated)")?,
    };
    let domains: Vec<String> = domains_raw
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if domains.is_empty() {
        bail!("at least one domain is required");
    }

    let upstream = match cli_upstream {
        Some(u) => u.trim().to_string(),
        None => prompt_with_default("upstream (empty = static page)", "")?,
    };

    let tls = match cli_tls {
        Some(t) => {
            validate_preset(t)?;
            t.to_string()
        }
        None => choose_tls()?,
    };

    let watch_log = if !watch_log && cli_domains.is_none() {
        crate::ops::confirm("add 'import request_watch_log'")?
    } else {
        watch_log
    };

    let target = vhost::create_vhost_file(paths, &domains, &upstream, &tls, watch_log)?;
    println!("{} {}", "+".green(), target.display());
    println!("{}", std::fs::read_to_string(&target)?.dimmed());

    if !no_reload && crate::caddy::caddy_available() {
        crate::caddy::try_reload(paths, true);
    }
    Ok(())
}

fn validate_preset(t: &str) -> Result<()> {
    if TLS_PRESETS.iter().any(|(k, _)| *k == t) {
        Ok(())
    } else {
        bail!(
            "unknown tls preset `{t}`; one of: {}",
            TLS_PRESETS
                .iter()
                .map(|(k, _)| *k)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    let v = prompt(&format!("{label} [{default}]"))?;
    Ok(if v.is_empty() { default.to_string() } else { v })
}

fn choose_tls() -> Result<String> {
    println!("tls policy:");
    for (i, (_, desc)) in TLS_PRESETS.iter().enumerate() {
        println!("  {}) {desc}", i + 1);
    }
    loop {
        let answer = prompt("choice [1]")?;
        let idx: usize = if answer.is_empty() {
            1
        } else {
            answer
                .parse()
                .map_err(|_| anyhow::anyhow!("not a number"))?
        };
        if idx >= 1 && idx <= TLS_PRESETS.len() {
            return Ok(TLS_PRESETS[idx - 1].0.to_string());
        }
        println!("{}", format!("pick 1..{}", TLS_PRESETS.len()).yellow());
    }
}
