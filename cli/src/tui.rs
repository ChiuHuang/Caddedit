//! Bare `caddedit` — interactive route browser (ratatui).
//!
//! j/k move · space toggle · e edit · d rm · r reload · R refresh · q quit

use crate::caddyfile::analyze::{SiteInfo, SiteKind};
use crate::config::Paths;
use crate::{caddy, vhost};
use anyhow::Context;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Terminal;
use std::io::stdout;

struct App {
    paths: Paths,
    rows: Vec<vhost::VhostSummary>,
    selected: usize,
    message: Option<(String, bool)>, // (text, is_error)
    pending_delete: Option<String>,
}

pub fn run(paths: &Paths) -> Result<()> {
    enable_raw_mode().context("enabling raw mode")?;
    execute!(stdout(), EnterAlternateScreen).context("entering alternate screen")?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend).context("creating terminal")?;

    let mut app = App {
        paths: paths.clone(),
        rows: Vec::new(),
        selected: 0,
        message: None,
        pending_delete: None,
    };
    app.refresh();

    let result = event_loop(&mut terminal, &mut app);

    // always restore the terminal
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        if !event::poll(std::time::Duration::from_millis(250))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                return Ok(());
            }
            match key.code {
                KeyCode::Char('q') => return Ok(()),
                KeyCode::Esc if app.pending_delete.is_some() => {
                    app.pending_delete = None;
                    app.set_msg("cancelled", false);
                }
                KeyCode::Esc => return Ok(()),
                KeyCode::Char('y') => {
                    if app.pending_delete.take().is_some() {
                        app.delete_selected()?;
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => app.next(),
                KeyCode::Char('k') | KeyCode::Up => app.prev(),
                KeyCode::Home | KeyCode::Char('g') => app.selected = 0,
                KeyCode::End | KeyCode::Char('G') => {
                    app.selected = app.rows.len().saturating_sub(1)
                }
                KeyCode::Char('R') => {
                    app.refresh();
                    app.set_msg(format!("{} routes", app.rows.len()), false);
                }
                KeyCode::Char('r') => app.reload_caddy(),
                KeyCode::Char(' ') | KeyCode::Enter => app.toggle()?,
                KeyCode::Char('e') => app.edit_selected()?,
                KeyCode::Char('d') => {
                    if let Some(id) = app.selected_id() {
                        app.pending_delete = Some(id);
                        app.message = None;
                    }
                }
                _ => {}
            }
        }
    }
}

impl App {
    fn refresh(&mut self) {
        self.rows = vhost::summarize(&self.paths)
            .into_iter()
            .map(|(_, s)| s)
            .collect();
        if self.selected >= self.rows.len() {
            self.selected = self.rows.len().saturating_sub(1);
        }
    }

    fn set_msg(&mut self, text: impl Into<String>, is_error: bool) {
        self.message = Some((text.into(), is_error));
    }

    fn next(&mut self) {
        if self.selected + 1 < self.rows.len() {
            self.selected += 1;
        }
    }

    fn prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn selected_id(&self) -> Option<String> {
        self.rows.get(self.selected).map(|r| r.id.clone())
    }

    /// Re-scan to get an owned handle for the currently selected row.
    fn selected_vf(&self) -> Option<vhost::VhostFile> {
        let id = self.selected_id()?;
        vhost::scan(&self.paths).into_iter().find(|vf| vf.id == id)
    }

    fn toggle(&mut self) -> Result<()> {
        self.pending_delete = None;
        let Some(vf) = self.selected_vf() else {
            return Ok(());
        };
        let turning_on = vf.status == vhost::Status::Off;

        if turning_on && caddy::caddy_available() {
            if let Err(e) = caddy::validate_file(&vf.path) {
                self.set_msg(format!("validation failed:\n{e}"), true);
                return Ok(());
            }
        }
        match vhost::set_status(&vf, &self.paths, turning_on) {
            Ok(_) => {
                self.reload_quietly();
                self.refresh();
                self.set_msg(
                    format!("{} is now {}", vf.id, if turning_on { "on" } else { "off" }),
                    false,
                );
            }
            Err(e) => self.set_msg(format!("{e:#}"), true),
        }
        Ok(())
    }

    fn edit_selected(&mut self) -> Result<()> {
        self.pending_delete = None;
        let Some(vf) = self.selected_vf() else {
            return Ok(());
        };

        // suspend the TUI so $EDITOR gets a normal terminal
        disable_raw_mode()?;
        execute!(stdout(), LeaveAlternateScreen)?;
        let outcome = caddy::open_editor(&vf.path).and_then(|_| {
            if caddy::caddy_available() {
                caddy::validate_file(&vf.path).map(|_| ())
            } else {
                Ok(())
            }
        });
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen)?;

        match outcome {
            Ok(()) => {
                self.reload_quietly();
                self.refresh();
                self.set_msg(format!("saved {}", vf.id), false);
            }
            Err(e) => self.set_msg(format!("{e:#}"), true),
        }
        Ok(())
    }

    fn delete_selected(&mut self) -> Result<()> {
        let Some(vf) = self.selected_vf() else {
            return Ok(());
        };
        match vhost::soft_delete(&vf, &self.paths) {
            Ok(target) => {
                self.reload_quietly();
                self.refresh();
                self.set_msg(format!("deleted {} -> {}", vf.id, target.display()), false);
            }
            Err(e) => self.set_msg(format!("{e:#}"), true),
        }
        Ok(())
    }

    fn reload_caddy(&mut self) {
        match caddy::reload(&self.paths) {
            Ok(_) => self.set_msg("caddy reloaded", false),
            Err(e) => self.set_msg(format!("reload failed: {e:#}"), true),
        }
    }

    /// Best-effort reload; failures surface in the message line.
    fn reload_quietly(&mut self) {
        if caddy::caddy_available() {
            if let Err(e) = caddy::reload(&self.paths) {
                self.set_msg(format!("warning: reload failed: {e:#}"), true);
            }
        }
    }
}

fn kind_style(kind: SiteKind) -> Style {
    match kind {
        SiteKind::Proxy => Style::new().fg(Color::Cyan),
        SiteKind::Static => Style::new().fg(Color::Yellow),
        SiteKind::Other => Style::new().fg(Color::Gray),
        SiteKind::Raw => Style::new().fg(Color::Magenta),
    }
}

fn kind_label(kind: SiteKind) -> &'static str {
    match kind {
        SiteKind::Proxy => "proxy",
        SiteKind::Static => "static",
        SiteKind::Other => "other",
        SiteKind::Raw => "raw",
    }
}

fn tls_label(info: &SiteInfo) -> String {
    match &info.tls {
        None => "-".to_string(),
        Some(t) => match (&t.detail, t.mode.label()) {
            (Some(d), label) => format!("{label} ({d})"),
            (None, label) => label.to_string(),
        },
    }
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    let area = f.area();
    let [title_area, table_area, msg_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .areas(area);

    let on = app
        .rows
        .iter()
        .filter(|r| r.status == vhost::Status::On)
        .count();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" caddedit ", Style::new().bold().fg(Color::Cyan)),
            Span::raw(app.paths.caddyfile.display().to_string()),
            Span::raw("   "),
            Span::styled(
                format!("{on} on / {} total", app.rows.len()),
                Style::new().dim(),
            ),
        ])),
        title_area,
    );

    let header = Row::new(["", "DOMAINS", "TYPE", "UPSTREAM", "TLS"])
        .style(Style::new().bold().underlined());

    let rows = app.rows.iter().enumerate().map(|(i, r)| {
        let selected = i == app.selected;
        let base = if selected {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        let status_cell = match r.status {
            vhost::Status::On => Cell::from("*").fg(Color::Green),
            vhost::Status::Off => Cell::from("o").style(Style::new().dim()),
        };
        Row::new(vec![
            status_cell,
            Cell::from(r.info.addresses.join(", ")).style(base),
            Cell::from(kind_label(r.info.kind)).style(if selected {
                base
            } else {
                kind_style(r.info.kind)
            }),
            Cell::from(if r.info.upstreams.is_empty() {
                "-".to_string()
            } else {
                r.info.upstreams.join(", ")
            })
            .style(base),
            Cell::from(tls_label(&r.info)).style(base),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Percentage(34),
            Constraint::Length(8),
            Constraint::Percentage(30),
            Constraint::Percentage(26),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(table, table_area);

    let help =
        "j/k move   space toggle   e edit   d rm   y confirm   r reload   R refresh   q quit";
    let line = if let Some(id) = &app.pending_delete {
        Line::from(Span::styled(
            format!(" delete {id}? y to confirm, esc to cancel"),
            Style::new().fg(Color::Red).bold(),
        ))
    } else {
        match &app.message {
            Some((text, true)) => Line::from(Span::styled(
                format!(" {text}"),
                Style::new().fg(Color::Red),
            )),
            Some((text, false)) => Line::from(Span::styled(
                format!(" {text}"),
                Style::new().fg(Color::Green),
            )),
            None => Line::from(Span::styled(help, Style::new().dim())),
        }
    };
    f.render_widget(Paragraph::new(line), msg_area);
}
