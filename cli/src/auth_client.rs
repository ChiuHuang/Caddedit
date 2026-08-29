//! CLI auth client — login via refresh token, store access token, User-Agent.

use std::path::PathBuf;

const USER_AGENT: &str = concat!("caddedit-cli/", env!("CARGO_PKG_VERSION"));

fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("caddedit"))
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.is_empty() {
                return Some(PathBuf::from(xdg).join("caddedit"));
            }
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join(".config/caddedit"))
    }
}

fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("cli.json"))
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(serde::Serialize, serde::Deserialize, Default, Debug)]
pub(crate) struct CliConfig {
    pub url: Option<String>,
    pub access_token: Option<String>,
    pub expires_at: Option<u64>,
    pub refresh_token: Option<String>,
}

pub(crate) fn load_config() -> CliConfig {
    let Some(file) = config_file() else {
        return CliConfig::default();
    };
    let Ok(data) = std::fs::read_to_string(&file) else {
        return CliConfig::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

pub(crate) fn load_valid_config() -> Option<CliConfig> {
    let cfg = load_config();
    if cfg
        .url
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        return None;
    }
    if cfg
        .access_token
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        return None;
    }
    if let Some(exp) = cfg.expires_at {
        if exp != 0 && exp < now_ms() {
            return None;
        }
    }
    Some(cfg)
}

fn save_config(cfg: &CliConfig) -> anyhow::Result<()> {
    let Some(file) = config_file() else {
        anyhow::bail!("cannot determine config directory");
    };
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_string_pretty(cfg)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true).write(true).truncate(true).mode(0o600);
        let mut f = opts.open(&file)?;
        use std::io::Write;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&file, data)?;
    }
    Ok(())
}

fn curl_available() -> bool {
    std::process::Command::new("curl")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Perform POST /api/auth/refresh via curl (avoids adding reqwest dep).
fn exchange_via_curl(url: &str, refresh_token: &str) -> anyhow::Result<(String, u64)> {
    let url = normalize_url(url);
    let endpoint = format!("{}/api/auth/refresh", url.trim_end_matches('/'));
    let payload = serde_json::json!({ "refresh_token": refresh_token }).to_string();
    // Use curl -fsSL -H ... -d ...
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "30",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-H",
            &format!("User-Agent: {USER_AGENT}"),
            "-d",
            &payload,
            &endpoint,
        ])
        .output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let body = String::from_utf8_lossy(&out.stdout);
        let msg = if !body.is_empty() {
            body.trim()
        } else {
            err.trim()
        };
        anyhow::bail!(
            "refresh failed: {}",
            if msg.is_empty() { "unknown error" } else { msg }
        );
    }
    let body = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&body)?;
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        anyhow::bail!("{}", err);
    }
    let token = v
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing access_token in response"))?
        .to_string();
    let expires_at = v.get("expires_at").and_then(|e| e.as_u64()).unwrap_or(0);
    Ok((token, expires_at))
}

pub(crate) fn try_remote_ls(json: bool, query: Option<&str>) -> Option<anyhow::Result<()>> {
    let cfg = load_valid_config()?;
    let url_raw = cfg.url?.trim().to_string();
    let url = normalize_url(&url_raw);
    let token = cfg.access_token?.trim().to_string();
    if url.is_empty() || token.is_empty() {
        return None;
    }
    // fetch via curl GET /api/vhosts — normalized to https:// to avoid http->https 308 stripping Authorization
    let endpoint = format!("{}/api/vhosts", url.trim_end_matches('/'));
    let out = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "30",
            "-H",
            &format!("Authorization: Bearer {token}"),
            "-H",
            &format!("User-Agent: {USER_AGENT}"),
            "-H",
            "Accept: application/json",
            &endpoint,
        ])
        .output();
    let out = match out {
        Ok(o) => o,
        Err(e) => return Some(Err(anyhow::anyhow!("curl failed: {e}"))),
    };
    if !out.status.success() {
        let body = String::from_utf8_lossy(&out.stdout);
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = if !body.trim().is_empty() {
            body.trim().to_string()
        } else if !err.trim().is_empty() {
            err.trim().to_string()
        } else {
            format!("HTTP {}", out.status)
        };
        // token expired? suggest re-login
        if msg.contains("locked") || msg.contains("401") || msg.contains("Unauthorized") {
            return Some(Err(anyhow::anyhow!(
                "remote auth failed (token expired or invalid) — re-run `caddedit login --url {url} --refresh-token <token>`"
            )));
        }
        return Some(Err(anyhow::anyhow!("remote ls failed: {msg}")));
    }
    let body = String::from_utf8_lossy(&out.stdout);
    let mut vals: Vec<serde_json::Value> = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return Some(Err(anyhow::anyhow!("invalid JSON from server: {e}"))),
    };
    // query filter like local ls
    if let Some(q) = query.map(str::trim).filter(|s| !s.is_empty()) {
        let q = q.to_lowercase();
        vals.retain(|v| {
            let hay = [
                v.get("id").and_then(|x| x.as_str()).unwrap_or(""),
                &v.get("addresses")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default(),
                &v.get("upstreams")
                    .and_then(|a| a.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default(),
                v.get("kind").and_then(|x| x.as_str()).unwrap_or(""),
            ]
            .join(" ")
            .to_lowercase();
            hay.contains(&q)
        });
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&vals).unwrap_or_else(|_| "[]".into())
        );
        return Some(Ok(()));
    }
    if vals.is_empty() {
        println!("no vhosts (remote {})", url);
        return Some(Ok(()));
    }
    // render table similar to local ls
    use owo_colors::OwoColorize;
    let header = ["", "DOMAINS", "TYPE", "UPSTREAM", "TLS"];
    // build rows
    let mut table: Vec<[String; 5]> = Vec::new();
    for v in &vals {
        let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("off");
        let status_col = if status == "on" {
            "● on".green().to_string()
        } else {
            "○ off".dimmed().to_string()
        };
        let domains = v
            .get("addresses")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| {
                v.get("id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string()
            });
        let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("raw");
        let (kind_label, kind_color) = match kind {
            "proxy" => ("proxy", owo_colors::AnsiColors::Cyan),
            "php" => ("php", owo_colors::AnsiColors::Blue),
            "static" => ("static", owo_colors::AnsiColors::Yellow),
            "other" => ("other", owo_colors::AnsiColors::White),
            _ => ("raw", owo_colors::AnsiColors::Magenta),
        };
        let kind_col = kind_label.to_string().color(kind_color).to_string();
        let upstreams = v
            .get("upstreams")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let upstream_col = if upstreams.is_empty() {
            "-".dimmed().to_string()
        } else {
            upstreams
        };
        let tls_val = v.get("tls");
        let tls_col = match tls_val {
            None => "-".dimmed().to_string(),
            Some(t) if t.is_null() => "-".dimmed().to_string(),
            Some(t) => {
                let mode = t.get("mode").and_then(|x| x.as_str()).unwrap_or("");
                let detail = t.get("detail").and_then(|x| x.as_str()).unwrap_or("");
                if mode.is_empty() {
                    detail.to_string()
                } else if detail.is_empty() {
                    mode.to_string()
                } else {
                    format!("{mode} ({detail})")
                }
            }
        };
        let tls_col = if tls_col == "-" {
            tls_col
        } else {
            // color similar to local: internal yellow etc. Keep plain for remote
            tls_col
        };
        table.push([status_col, domains, kind_col, upstream_col, tls_col]);
    }
    // print table (strip ANSI for width)
    let plain: Vec<[String; 5]> = table
        .iter()
        .map(|r| {
            [
                strip_ansi(&r[0]),
                r[1].clone(),
                strip_ansi(&r[2]),
                r[3].clone(),
                strip_ansi(&r[4]),
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
    for (ri, row) in table.iter().enumerate() {
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
    println!("\n{} remote vhost(s) from {}", vals.len(), url.dimmed());
    Some(Ok(()))
}

fn strip_ansi(s: &str) -> String {
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

fn normalize_url(url: &str) -> String {
    let u = url.trim();
    if u.contains("://") {
        u.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", u.trim_end_matches('/'))
    }
}

pub fn run_login(url: &str, refresh_token: &str, save: bool) -> anyhow::Result<()> {
    if url.trim().is_empty() || refresh_token.trim().is_empty() {
        anyhow::bail!("url and refresh_token are required");
    }
    let url = normalize_url(url);
    // Prefer curl if available, else fallback to simple message
    if !curl_available() {
        anyhow::bail!("curl not found — install curl or use: curl -H \"User-Agent: {USER_AGENT}\" -H \"Content-Type: application/json\" -d '{{\"refresh_token\":\"{}\"}}' {}/api/auth/refresh", refresh_token, url.trim_end_matches('/'));
    }
    println!("caddedit exchanging refresh token at {} ...", url);
    let (access, expires_at) = exchange_via_curl(&url, refresh_token)?;
    println!("✓ access token issued (expires_at={})", expires_at);
    println!("  token: {}", access);
    println!("  You can now run `caddedit ls` or `caddedit list` to manage remote vhosts.");
    if save {
        let mut cfg = load_config();
        cfg.url = Some(url.trim().to_string());
        cfg.access_token = Some(access.clone());
        cfg.expires_at = Some(expires_at);
        cfg.refresh_token = Some(refresh_token.to_string());
        save_config(&cfg)?;
        if let Some(file) = config_file() {
            println!("✓ saved to {}", file.display());
        }
        println!("  Token valid for 24h. Re-run login after expiry or when refresh rotated.");
    } else {
        println!("  (not saved — pass --save to persist to config file)");
    }
    Ok(())
}

pub fn run_login_tui(save_default: bool) -> anyhow::Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
    use crossterm::execute;
    use crossterm::terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
    };
    use ratatui::backend::CrosstermBackend;
    use ratatui::layout::{Constraint, Layout};
    use ratatui::style::{Color, Style, Stylize};
    use ratatui::text::{Line, Span};
    use ratatui::widgets::{Block, Borders, Paragraph};
    use ratatui::Terminal;
    use std::io::{stdout, IsTerminal};

    if !stdout().is_terminal() || !std::io::stdin().is_terminal() {
        anyhow::bail!("interactive login requires a TTY; use `caddedit login --url <url> --refresh-token <token>`");
    }

    let existing = load_config();
    let mut url = existing.url.unwrap_or_default();
    // prefill token from existing refresh_token for convenience (masked)
    let mut token = String::new();
    let mut focus: usize = 0; // 0 url, 1 token, 2 save
    let mut save = save_default;
    let mut show_token = false;
    let mut message: Option<(String, bool)> = None;
    let mut done = false;

    // if url empty, focus url, else focus token
    if !url.is_empty() {
        focus = 1;
    }

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let res: anyhow::Result<()> = (|| {
        loop {
            terminal.draw(|f| {
                let area = f.area();
                let chunks = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(3),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(area);

                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(" caddedit login ", Style::new().bold().fg(Color::Cyan)),
                        Span::styled("— refresh token → access token (curl)", Style::new().dim()),
                    ])),
                    chunks[0],
                );

                let url_block = Block::default()
                    .borders(Borders::ALL)
                    .title(Line::from(vec![
                        Span::raw(" Server URL "),
                        Span::styled("e.g. https://caddedit.example.com", Style::new().dim()),
                    ]))
                    .border_style(if focus == 0 {
                        Style::new().fg(Color::Yellow)
                    } else {
                        Style::new().dim()
                    });
                let url_display = if focus == 0 {
                    format!("{}_", url)
                } else {
                    url.clone()
                };
                f.render_widget(Paragraph::new(url_display).block(url_block), chunks[1]);

                let token_title = if show_token {
                    " Refresh Token (visible) "
                } else {
                    " Refresh Token (masked) "
                };
                let token_block = Block::default()
                    .borders(Borders::ALL)
                    .title(token_title)
                    .border_style(if focus == 1 {
                        Style::new().fg(Color::Yellow)
                    } else {
                        Style::new().dim()
                    });
                let token_display = if show_token {
                    if focus == 1 {
                        format!("{}_", token)
                    } else {
                        token.clone()
                    }
                } else if token.is_empty() {
                    if focus == 1 {
                        "_".to_string()
                    } else {
                        String::new()
                    }
                } else {
                    let masked = "•".repeat(token.chars().count());
                    if focus == 1 {
                        format!("{masked}_")
                    } else {
                        masked
                    }
                };
                f.render_widget(Paragraph::new(token_display).block(token_block), chunks[2]);

                let save_line = if save {
                    Span::styled("[x] Save to cli.json", Style::new().fg(Color::Green))
                } else {
                    Span::styled("[ ] Save to cli.json (print only)", Style::new().dim())
                };
                let save_para = Paragraph::new(Line::from(vec![
                    if focus == 2 {
                        Span::styled("> ", Style::new().fg(Color::Yellow))
                    } else {
                        Span::raw("  ")
                    },
                    save_line,
                    Span::styled("  (Space toggle)", Style::new().dim()),
                ]));
                f.render_widget(save_para, chunks[3]);

                if let Some((msg, is_err)) = &message {
                    let style = if *is_err {
                        Style::new().fg(Color::Red)
                    } else {
                        Style::new().fg(Color::Green)
                    };
                    f.render_widget(
                        Paragraph::new(Line::from(Span::styled(msg.clone(), style))),
                        chunks[4],
                    );
                } else {
                    let cfg_path = config_file()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<unknown>".into());
                    f.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            format!("config: {}", cfg_path),
                            Style::new().dim(),
                        ))),
                        chunks[4],
                    );
                }

                // help bar
                let help = if done {
                    Line::from(Span::styled(
                        " success — press Enter or Esc to exit · Tab switch field ",
                        Style::new().fg(Color::Green),
                    ))
                } else {
                    Line::from(vec![
                        Span::styled("Tab", Style::new().bold().fg(Color::Yellow)),
                        Span::raw(" switch · "),
                        Span::styled("Enter", Style::new().bold().fg(Color::Yellow)),
                        Span::raw(" submit · "),
                        Span::styled("Space", Style::new().bold().fg(Color::Yellow)),
                        Span::raw(" toggle save · "),
                        Span::styled("Ctrl+R", Style::new().bold().fg(Color::Yellow)),
                        Span::raw(" show/hide token · "),
                        Span::styled("Esc", Style::new().bold().fg(Color::Yellow)),
                        Span::raw(" cancel"),
                    ])
                };
                f.render_widget(Paragraph::new(help), chunks[6]);
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
                // Ctrl+R toggle token visibility
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
                    show_token = !show_token;
                    continue;
                }
                match key.code {
                    KeyCode::Esc => {
                        if done {
                            return Ok(());
                        }
                        anyhow::bail!("cancelled");
                    }
                    KeyCode::Enter => {
                        if done {
                            return Ok(());
                        }
                        // submit if both fields non-empty
                        if url.trim().is_empty() {
                            message = Some(("URL is required".into(), true));
                            focus = 0;
                            continue;
                        }
                        if token.trim().is_empty() {
                            message = Some(("refresh token is required".into(), true));
                            focus = 1;
                            continue;
                        }
                        // attempt login (blocking)
                        message = Some(("exchanging refresh token ...".into(), false));
                        // draw one more frame to show message before blocking
                        terminal.draw(|f| {
                            let area = f.area();
                            let chunks = Layout::vertical([
                                Constraint::Length(1),
                                Constraint::Length(3),
                                Constraint::Length(3),
                                Constraint::Length(1),
                                Constraint::Length(1),
                                Constraint::Min(1),
                                Constraint::Length(1),
                            ])
                            .split(area);
                            f.render_widget(
                                Paragraph::new(Line::from(Span::styled(
                                    " exchanging refresh token ...",
                                    Style::new().fg(Color::Yellow),
                                ))),
                                chunks[4],
                            );
                        })?;
                        let normalized = normalize_url(&url);
                        match exchange_via_curl(&normalized, token.trim()) {
                            Ok((access, expires_at)) => {
                                if save {
                                    let mut cfg = load_config();
                                    cfg.url = Some(normalized.clone());
                                    cfg.access_token = Some(access.clone());
                                    cfg.expires_at = Some(expires_at);
                                    cfg.refresh_token = Some(token.trim().to_string());
                                    if let Err(e) = save_config(&cfg) {
                                        message = Some((format!("save failed: {e:#}"), true));
                                        continue;
                                    }
                                    let file = config_file()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_else(|| "<unknown>".into());
                                    message = Some((
                                        format!(
                                            "✓ access token issued (expires_at={}) saved to {}",
                                            expires_at, file
                                        ),
                                        false,
                                    ));
                                } else {
                                    message = Some((
                                        format!(
                                            "✓ access token issued (expires_at={}) token: {}",
                                            expires_at, access
                                        ),
                                        false,
                                    ));
                                }
                                done = true;
                            }
                            Err(e) => {
                                message = Some((format!("{e:#}"), true));
                            }
                        }
                    }
                    KeyCode::Tab | KeyCode::Down => {
                        focus = (focus + 1) % 3;
                    }
                    KeyCode::BackTab | KeyCode::Up => {
                        focus = (focus + 2) % 3;
                    }
                    KeyCode::Char(' ') if focus == 2 => {
                        save = !save;
                    }
                    KeyCode::Backspace => {
                        if focus == 0 {
                            url.pop();
                        } else if focus == 1 {
                            token.pop();
                        }
                        message = None;
                    }
                    KeyCode::Char(c) => {
                        if focus == 0 {
                            url.push(c);
                            message = None;
                        } else if focus == 1 {
                            token.push(c);
                            message = None;
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

pub fn run_logout() -> anyhow::Result<()> {
    let mut cfg = load_config();
    let had = cfg.access_token.is_some() || cfg.refresh_token.is_some();
    cfg.access_token = None;
    cfg.expires_at = None;
    // keep url but clear tokens
    save_config(&cfg)?;
    if had {
        println!("✓ cleared stored tokens");
    } else {
        println!("no stored tokens");
    }
    Ok(())
}

pub fn run_config_show() -> anyhow::Result<()> {
    let cfg = load_config();
    let file = config_file()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<unknown>".into());
    println!("config file: {}", file);
    println!("{}", serde_json::to_string_pretty(&json_masked(&cfg))?);
    Ok(())
}

fn json_masked(cfg: &CliConfig) -> serde_json::Value {
    let mut v = serde_json::to_value(cfg).unwrap_or(serde_json::json!({}));
    if let Some(obj) = v.as_object_mut() {
        for key in ["access_token", "refresh_token"] {
            if obj.contains_key(key) && obj[key].is_string() {
                let s = obj[key].as_str().unwrap_or("");
                if !s.is_empty() {
                    // mask all but first 4 chars
                    let masked = if s.len() > 8 {
                        format!("{}…{} ({} chars)", &s[..4], &s[s.len() - 4..], s.len())
                    } else {
                        "****".to_string()
                    };
                    obj[key] = serde_json::Value::String(masked);
                }
            }
        }
    }
    v
}

#[allow(dead_code)]
pub fn user_agent() -> &'static str {
    USER_AGENT
}
