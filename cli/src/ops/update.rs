//! `caddedit update` — CLI self-update via GitHub releases, distinct from `serve` web update.

use anyhow::Result;
use owo_colors::OwoColorize;

pub fn run(check: bool, channel: &str, yes: bool) -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let channel = if channel.trim().is_empty() {
        "stable"
    } else {
        channel.trim()
    };
    let target = crate::selfupdate::asset_name().ok_or_else(|| {
        anyhow::anyhow!(
            "auto-update not supported on this platform ({} {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    println!(
        "{} v{} ({}) channel={}",
        "caddedit".bright_cyan().bold(),
        current,
        target.dimmed(),
        channel
    );

    // fetch release info
    println!("checking GitHub releases ({}) ...", channel.dimmed());
    let info = crate::selfupdate::release_info_for(channel)?;
    let latest = info.version.clone();

    let is_newer = crate::selfupdate::is_newer_for(&latest, current, channel);
    let up_to_date = !is_newer;

    if up_to_date {
        println!(
            "{} v{} is up to date (channel {})",
            "✓".green(),
            current,
            channel
        );
        if let Some(notes) = info.notes.as_deref() {
            if !notes.trim().is_empty() && notes.len() < 2000 {
                println!("\n{}", notes.dimmed());
            }
        }
        return Ok(());
    }

    println!(
        "{} update available: {} -> {}",
        "»".bright_cyan(),
        format!("v{current}").dimmed(),
        format!("v{latest}").green().bold()
    );
    if let Some(published) = info.published_at.as_deref() {
        println!("  published: {}", published.dimmed());
    }
    if let Some(notes) = info.notes.as_deref() {
        let preview = if notes.len() > 3000 {
            format!("{}…", &notes[..3000])
        } else {
            notes.to_string()
        };
        println!("\n{}", preview);
    } else {
        // fallback: try compare notes
        let base = format!("v{current}");
        let head = if channel == "nightly" {
            "nightly".to_string()
        } else {
            format!("v{latest}")
        };
        if let Ok(Some(cmp)) = crate::selfupdate::compare_notes(&base, &head) {
            println!("\n{}", cmp);
        }
    }

    if check {
        println!("\n{} run without --check to install", "→".dimmed());
        return Ok(());
    }

    if !yes && !crate::ops::confirm(&format!("install v{latest} now?"))? {
        println!("aborted");
        return Ok(());
    }

    println!("installing v{} ...", latest.bright_cyan());
    let out = crate::selfupdate::install_cli_version(&latest)?;
    println!("{}", out.dimmed());
    println!(
        "{} updated to v{}",
        "✓".green().bold(),
        latest.green().bold()
    );
    println!(
        "{}",
        "restart your shell or run `caddedit --version` to verify".dimmed()
    );
    Ok(())
}
