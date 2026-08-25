pub mod check;
pub mod edit;
pub mod init;
pub mod ls;
pub mod new;
pub mod reload;
pub mod show;
pub mod toggle;

use anyhow::Result;
use owo_colors::OwoColorize;
use std::io::Write;

/// Minimal y/n confirmation read from stdin.
pub fn confirm(question: &str) -> Result<bool> {
    print!("{} {}", question.bright_white(), "[y/N]".dimmed());
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}
