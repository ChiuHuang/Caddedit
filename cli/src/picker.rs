//! Shared single-select route picker for argument-less commands
//! (`show`, `on`, `off`, `rm`, `edit`). Falls back to a clear error when
//! stdin/stdout are not a terminal so scripting never hangs.

use crate::config::Paths;
use crate::vhost::{self, Status};
use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;
use std::io::{stdout, IsTerminal};

/// Pick one vhost interactively. `verb` appears in the header ("rm <route>").
pub fn select(paths: &Paths, verb: &str) -> Result<vhost::VhostFile> {
    let mut rows = vhost::scan(paths);
    if rows.is_empty() {
        bail!("no vhosts found under {}", paths.vhosts_dir.display());
    }
    rows.sort_by(|a, b| match (a.status, b.status) {
        (Status::On, Status::Off) => std::cmp::Ordering::Less,
        (Status::Off, Status::On) => std::cmp::Ordering::Greater,
        _ => a.id.cmp(&b.id),
    });

    if !std::io::stdin().is_terminal() || !stdout().is_terminal() {
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        bail!(
            "a route name is required in non-interactive shells; available: {}",
            ids.join(", ")
        );
    }

    enable_raw_mode().context("enabling raw mode")?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let result = picker_loop(&mut terminal, rows, verb);

    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn picker_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    rows: Vec<vhost::VhostFile>,
    verb: &str,
) -> Result<vhost::VhostFile> {
    let mut state = ListState::default();
    state.select(Some(0));

    loop {
        terminal.draw(|f| {
            let area = f.area();
            let [header_area, body_area] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);

            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!(" caddedit {verb} "),
                        Style::new().bold().fg(Color::Cyan),
                    ),
                    Span::styled("j/k move · enter select · esc cancel", Style::new().dim()),
                ])),
                header_area,
            );

            let items: Vec<ListItem> = rows
                .iter()
                .map(|r| {
                    let (dot, color) = match r.status {
                        Status::On => ("●", Color::Green),
                        Status::Off => ("○", Color::DarkGray),
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(dot.to_string(), Style::new().fg(color)),
                        Span::raw(" "),
                        Span::raw(r.id.clone()),
                    ]))
                })
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL))
                .highlight_style(Style::new().reversed())
                .highlight_symbol("> ");
            f.render_stateful_widget(list, body_area, &mut state);
        })?;

        if !event::poll(std::time::Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => bail!("cancelled"),
                KeyCode::Char('j') | KeyCode::Down => {
                    state.select_next();
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.select_previous();
                }
                KeyCode::Enter => {
                    let idx = state.selected().unwrap_or(0);
                    return Ok(rows[idx].clone());
                }
                _ => {}
            }
        }
    }
}
