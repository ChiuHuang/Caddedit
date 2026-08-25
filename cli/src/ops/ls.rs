//! `caddedit ls` ??the money shot: every route at a glance.

use crate::caddyfile::analyze::SiteKind;
use crate::config::Paths;
use anyhow::Result;
use owo_colors::OwoColorize;

pub fn run(paths: &Paths, json: bool, query: Option<&str>) -> Result<()> {
    let mut rows = crate::vhost::summarize(paths);

    // optional substring filter across every visible field
    if let Some(q) = query.map(str::trim).filter(|s| !s.is_empty()) {
        let q = q.to_lowercase();
        rows.retain(|(_, s)| {
            let hay = [
                s.id.as_str(),
                s.info.addresses.join(" ").as_str(),
                s.info.upstreams.join(" ").as_str(),
                s.info.directives.join(" ").as_str(),
                tls_plain(&s.info).as_str(),
            ]
            .join(" ")
            .to_lowercase();
            hay.contains(&q)
        });
    }

    if json {
        let vals: Vec<serde_json::Value> = rows
            .iter()
            .map(|(_, s)| serde_json::to_value(s).expect("serializable"))
            .collect();
        println!("{}", serde_json::to_string_pretty(&vals)?);
        return Ok(());
    }

    if rows.is_empty() {
        println!(
            "no vhosts under {} ??run `{}` first",
            paths.vhosts_dir.display(),
            "caddedit init".bright_cyan()
        );
        return Ok(());
    }

    // (status, domains, kind, upstream, tls) columns
    let header = ["", "DOMAINS", "TYPE", "UPSTREAM", "TLS"];
    let mut table: Vec<[String; 5]> = Vec::new();
    for (_, s) in &rows {
        table.push([
            match s.status {
                crate::vhost::Status::On => "??on".green().to_string(),
                crate::vhost::Status::Off => "??off".dimmed().to_string(),
            },
            s.info.addresses.join(", "),
            kind_label(s.info.kind)
                .0
                .to_string()
                .color(kind_label(s.info.kind).1)
                .to_string(),
            if s.info.upstreams.is_empty() {
                "-".dimmed().to_string()
            } else {
                s.info.upstreams.join(", ")
            },
            tls_label(&s.info),
        ]);
    }

    print_table(header, &table);
    println!(
        "\n{} vhost(s): {} enabled, {} disabled",
        rows.len(),
        rows.iter()
            .filter(|(v, _)| v.status == crate::vhost::Status::On)
            .count(),
        rows.iter()
            .filter(|(v, _)| v.status == crate::vhost::Status::Off)
            .count()
    );
    Ok(())
}

fn kind_label(kind: SiteKind) -> (&'static str, owo_colors::AnsiColors) {
    match kind {
        SiteKind::Proxy => ("proxy", owo_colors::AnsiColors::Cyan),
        SiteKind::Php => ("php", owo_colors::AnsiColors::Blue),
        SiteKind::Static => ("static", owo_colors::AnsiColors::Yellow),
        SiteKind::Other => ("other", owo_colors::AnsiColors::White),
        SiteKind::Raw => ("raw", owo_colors::AnsiColors::Magenta),
    }
}

/// Color-free TLS description, reused by the search filter.
use crate::caddyfile::analyze::TlsMode;
fn tls_plain(info: &crate::caddyfile::analyze::SiteInfo) -> String {
    match &info.tls {
        None => "-".to_string(),
        Some(t) => match (&t.detail, t.mode.label()) {
            (Some(d), label) => format!("{label} ({d})"),
            (None, label) => label.to_string(),
        },
    }
}

fn tls_label(info: &crate::caddyfile::analyze::SiteInfo) -> String {
    match &info.tls {
        None => "-".dimmed().to_string(),
        Some(t) => match t.mode {
            TlsMode::Internal => "internal".yellow().to_string(),
            TlsMode::AcmeEmail | TlsMode::Dns | TlsMode::Manual | TlsMode::Custom => {
                let base = t.mode.label();
                match &t.detail {
                    Some(d) => format!("{base} ({d})"),
                    None => base.to_string(),
                }
            }
        },
    }
}

fn print_table(header: [&str; 5], rows: &[[String; 5]]) {
    // strip ANSI for width computation
    let plain: Vec<[String; 5]> = rows
        .iter()
        .map(|r| {
            [
                console_strip(&r[0]),
                r[1].clone(),
                console_strip(&r[2]),
                r[3].clone(),
                console_strip(&r[4]),
            ]
        })
        .collect();

    let mut widths = [0usize; 5];
    for (i, h) in header.iter().enumerate() {
        widths[i] = h.len();
    }
    for row in &plain {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let sep = "  ";
    let mut line = String::new();
    for (i, h) in header.iter().enumerate() {
        line.push_str(&format!("{:<width$}", h, width = widths[i]));
        line.push_str(sep);
    }
    println!("{}", line.trim_end().dimmed());

    for (ri, row) in rows.iter().enumerate() {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            let visible = plain[ri][i].chars().count();
            let pad = widths[i].saturating_sub(visible);
            line.push_str(cell);
            for _ in 0..pad {
                line.push(' ');
            }
            line.push_str(sep);
        }
        println!("{}", line.trim_end());
    }
}

fn console_strip(s: &str) -> String {
    // cheap ANSI escape stripper: \x1b[..m
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for n in chars.by_ref() {
                if n == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
