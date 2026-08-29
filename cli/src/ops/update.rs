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

pub fn run_tui(initial_channel: &str) -> Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::{stdout, IsTerminal};

    if !stdout().is_terminal() || !std::io::stdin().is_terminal() {
        // fallback to non-interactive check
        return run(false, initial_channel, false);
    }

    let current = env!("CARGO_PKG_VERSION").to_string();
    let target = crate::selfupdate::asset_name()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "auto-update not supported on this platform ({} {})",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?
        .to_string();
    let mut channel = if initial_channel.trim().is_empty() {
        "stable".to_string()
    } else {
        initial_channel.trim().to_string()
    };
    let mut info: Option<crate::selfupdate::ReleaseInfo> = None;
    let mut message: Option<(String, bool)> = None;
    let mut checking = true;
    let mut installing = false;

    // helper to fetch
    let do_fetch = |chan: &str| -> Result<crate::selfupdate::ReleaseInfo> {
        crate::selfupdate::release_info_for(chan)
    };

    // initial fetch (blocking, before TUI to show spinner we fetch inside loop)
    // we will fetch inside loop on first draw

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut first = true;

    let res: Result<()> = (|| {
        loop {
            // fetch on first iteration or when requested
            if first {
                first = false;
                checking = true;
                message = Some((format!("checking {} ...", channel), false));
                // draw one frame before blocking so user sees checking
                terminal.draw(|f| {
                    draw_update(
                        f,
                        &current,
                        &target,
                        &channel,
                        info.as_ref(),
                        checking,
                        installing,
                        message.as_ref(),
                    )
                })?;
                match do_fetch(&channel) {
                    Ok(i) => {
                        info = Some(i);
                        message = None;
                    }
                    Err(e) => {
                        message = Some((format!("check failed: {e:#}"), true));
                        info = None;
                    }
                }
                checking = false;
            }

            terminal.draw(|f| {
                draw_update(
                    f,
                    &current,
                    &target,
                    &channel,
                    info.as_ref(),
                    checking,
                    installing,
                    message.as_ref(),
                )
            })?;

            if !event::poll(std::time::Duration::from_millis(200))? {
                continue;
            }
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    anyhow::bail!("cancelled");
                }
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                    KeyCode::Char('s') => {
                        if channel != "stable" {
                            channel = "stable".to_string();
                            info = None;
                            first = true;
                        }
                    }
                    KeyCode::Char('n') => {
                        if channel != "nightly" {
                            channel = "nightly".to_string();
                            info = None;
                            first = true;
                        }
                    }
                    KeyCode::Char('r') | KeyCode::Char('c') => {
                        info = None;
                        first = true;
                    }
                    KeyCode::Char('u') | KeyCode::Enter => {
                        // install if update available
                        let latest = match info.as_ref() {
                            Some(i) => i.version.clone(),
                            None => {
                                message = Some(("no release info — press r to check".into(), true));
                                continue;
                            }
                        };
                        let is_newer = crate::selfupdate::is_newer_for(&latest, &current, &channel);
                        if !is_newer {
                            message = Some(("already up to date".into(), false));
                            continue;
                        }
                        if installing || checking {
                            continue;
                        }
                        installing = true;
                        message = Some((format!("installing v{latest} ..."), false));
                        terminal.draw(|f| {
                            draw_update(
                                f,
                                &current,
                                &target,
                                &channel,
                                info.as_ref(),
                                checking,
                                installing,
                                message.as_ref(),
                            )
                        })?;
                        // leave TUI for install on Windows? keep raw mode for now, but install may need to print
                        // we run install blocking
                        match crate::selfupdate::install_cli_version(&latest) {
                            Ok(out) => {
                                message = Some((format!("✓ updated to v{latest}\n{out}\nrestart shell or run `caddedit --version`"), false));
                                installing = false;
                            }
                            Err(e) => {
                                message = Some((format!("install failed: {e:#}"), true));
                                installing = false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    })();

    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    res
}

#[allow(clippy::too_many_arguments)]
fn draw_update(
    f: &mut ratatui::Frame,
    current: &str,
    target: &str,
    channel: &str,
    info: Option<&crate::selfupdate::ReleaseInfo>,
    checking: bool,
    installing: bool,
    message: Option<&(String, bool)>,
) {
    use ratatui::layout::{Constraint, Layout};
    use ratatui::style::{Color, Style, Stylize};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

    let area = f.area();
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" caddedit update ", Style::new().bold().fg(Color::Cyan)),
            Span::styled(format!("v{current}"), Style::new().dim()),
            Span::raw("  "),
            Span::styled(format!("({target})"), Style::new().dim()),
        ])),
        chunks[0],
    );

    // channel + version info
    let (latest, status_line) = if checking {
        (
            "…".to_string(),
            Span::styled(" checking ...", Style::new().fg(Color::Yellow)),
        )
    } else if let Some(info) = info {
        let is_newer = crate::selfupdate::is_newer_for(&info.version, current, channel);
        if is_newer {
            (
                format!("v{}", info.version),
                Span::styled(
                    format!(" → v{} available", info.version),
                    Style::new().fg(Color::Green).bold(),
                ),
            )
        } else {
            (
                format!("v{}", info.version),
                Span::styled(" up to date", Style::new().fg(Color::Green)),
            )
        }
    } else {
        (
            "—".to_string(),
            Span::styled(" no info", Style::new().dim()),
        )
    };

    let channel_line = Paragraph::new(Line::from(vec![
        Span::styled(" channel: ", Style::new().dim()),
        Span::styled(
            channel.to_string(),
            if channel == "nightly" {
                Style::new().fg(Color::Magenta).bold()
            } else {
                Style::new().fg(Color::Cyan).bold()
            },
        ),
        Span::raw("   "),
        Span::styled(format!("current v{current}"), Style::new().dim()),
        Span::raw(" → "),
        Span::raw(format!("latest {latest}")),
        status_line,
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Channel / Version "),
    );
    f.render_widget(channel_line, chunks[1]);

    // notes
    let notes_text = if let Some(info) = info {
        if let Some(n) = &info.notes {
            let mut txt = n.clone();
            if txt.len() > 4000 {
                txt.truncate(4000);
                txt.push_str("\n…");
            }
            if let Some(p) = &info.published_at {
                format!("published: {p}\n\n{txt}")
            } else {
                txt
            }
        } else {
            // try compare
            let base = format!("v{current}");
            let head = if channel == "nightly" {
                "nightly".to_string()
            } else {
                format!("v{}", info.version)
            };
            crate::selfupdate::compare_notes(&base, &head)
                .ok()
                .flatten()
                .unwrap_or_else(|| "no release notes".to_string())
        }
    } else if checking {
        "fetching release info from GitHub ...".to_string()
    } else {
        "press r to check for updates".to_string()
    };

    // sanitize: replace tabs, ensure not too wide
    let para = Paragraph::new(notes_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Release Notes "),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(para, chunks[2]);

    // message line
    if let Some((msg, is_err)) = message {
        let style = if *is_err {
            Style::new().fg(Color::Red)
        } else if installing {
            Style::new().fg(Color::Yellow)
        } else {
            Style::new().fg(Color::Green)
        };
        // take first line for status bar, rest already in notes
        let first = msg.lines().next().unwrap_or(msg).to_string();
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(first, style))),
            chunks[3],
        );
    } else if installing {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " installing ...",
                Style::new().fg(Color::Yellow),
            ))),
            chunks[3],
        );
    } else {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                if info.is_some()
                    && crate::selfupdate::is_newer_for(&info.unwrap().version, current, channel)
                {
                    " update available — press u or Enter to install"
                } else {
                    " "
                },
                Style::new().dim(),
            ))),
            chunks[3],
        );
    }

    let help = Line::from(vec![
        Span::styled("s", Style::new().bold().fg(Color::Yellow)),
        Span::raw(" stable · "),
        Span::styled("n", Style::new().bold().fg(Color::Yellow)),
        Span::raw(" nightly · "),
        Span::styled("r", Style::new().bold().fg(Color::Yellow)),
        Span::raw(" refresh · "),
        Span::styled("u", Style::new().bold().fg(Color::Yellow)),
        Span::raw("/Enter install · "),
        Span::styled("q", Style::new().bold().fg(Color::Yellow)),
        Span::raw(" quit"),
    ]);
    f.render_widget(Paragraph::new(help), chunks[4]);
}
